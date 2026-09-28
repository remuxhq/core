//! `remux_gate`: the mixer's gate (`remux-mixer`) as a libobs audio filter on
//! the microphone, hosted under the mixer's `Filter` contract.
//!
//! libobs's own noise gate has an open and a close threshold, a hold and an
//! attack, and nothing else. The domain's gate also hears the highs apart (a
//! key press from behind the microphone), lifts a keyboard with no voice by
//! the keys boost, closes to a floor rather than to silence, and looks 60 ms
//! ahead so a word from silence keeps its first syllable. So the motor hosts
//! the remux gate, the one every motor runs, and the settings every face
//! shows are the ones that act.

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;

use libobs as sys;
use remux_mixer::gate::{Gate, GateLevels, GateParams};
use remux_mixer::Filter as _;

pub const GATE: &str = "remux_gate";

/// The settings keys, one per field of [`GateParams`].
const KEYS: [&std::ffi::CStr; 7] = [
    c"full",
    c"hf",
    c"floor",
    c"hold_ms",
    c"attack_ms",
    c"hf_attack_ms",
    c"keys_boost",
];

/// What the last block decided, for the status. One microphone, one gate.
static OPEN: AtomicBool = AtomicBool::new(false);
static FULL: AtomicU64 = AtomicU64::new(0);
static HF: AtomicU64 = AtomicU64::new(0);

/// Whether the gate is open, and what its two detectors read.
pub fn heard() -> (bool, GateLevels) {
    (
        OPEN.load(Ordering::Relaxed),
        GateLevels {
            full: f64::from_bits(FULL.load(Ordering::Relaxed)),
            hf: f64::from_bits(HF.load(Ordering::Relaxed)),
        },
    )
}

/// How late the filter hands the voice back, in nanoseconds: one block and
/// the lookahead. The microphone's sync offset takes it back, so the voice
/// is not later against the lips.
pub fn latency_ns() -> i64 {
    let rate = sample_rate();
    let frames = Gate::new(rate, channels().max(1), GateParams::default()).latency_frames();
    (frames as f64 / rate * 1e9) as i64
}

/// The settings a `remux_gate` is created with.
pub fn settings(params: GateParams) -> *mut sys::obs_data_t {
    let values = [
        params.full,
        params.hf,
        params.floor,
        params.hold_ms,
        params.attack_ms,
        params.hf_attack_ms,
        params.keys_boost,
    ];
    // SAFETY: a fresh obs_data, handed to the caller, who releases it.
    unsafe {
        let settings = sys::obs_data_create();
        for (key, value) in KEYS.iter().zip(values) {
            sys::obs_data_set_double(settings, key.as_ptr(), value);
        }
        settings
    }
}

fn params(settings: *mut sys::obs_data_t) -> GateParams {
    // SAFETY: libobs's settings for the call.
    let [full, hf, floor, hold_ms, attack_ms, hf_attack_ms, keys_boost] =
        KEYS.map(|key| unsafe { sys::obs_data_get_double(settings, key.as_ptr()) });
    GateParams {
        full,
        hf,
        floor,
        hold_ms,
        attack_ms,
        hf_attack_ms,
        keys_boost,
    }
}

fn sample_rate() -> f64 {
    // SAFETY: the audio output exists once libobs is reset, before any source.
    unsafe { sys::audio_output_get_sample_rate(sys::obs_get_audio()) as f64 }
}

fn channels() -> usize {
    // SAFETY: as above.
    unsafe { sys::audio_output_get_channels(sys::obs_get_audio()) }
}

/// Once per process, after the modules have loaded.
pub fn register() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        // SAFETY: an all-zero obs_source_info is a valid empty one.
        let mut gate: sys::obs_source_info = unsafe { std::mem::zeroed() };
        gate.id = c"remux_gate".as_ptr();
        gate.type_ = sys::obs_source_type_OBS_SOURCE_TYPE_FILTER;
        gate.output_flags = sys::OBS_SOURCE_AUDIO;
        gate.get_name = Some(name);
        gate.create = Some(create);
        gate.destroy = Some(destroy);
        gate.filter_audio = Some(filter_audio);
        // Only as far as `filter_audio`, the last callback set here, as
        // `effect::register` does for the same reason.
        let size = std::mem::offset_of!(sys::obs_source_info, filter_audio)
            + std::mem::size_of_val(&gate.filter_audio);
        // SAFETY: the struct and its strings are 'static; libobs copies it.
        unsafe { sys::obs_register_source_s(&gate, size) };
    });
}

struct Hosted {
    gate: Gate,
    channels: usize,
    interleaved: Vec<f32>,
}

unsafe extern "C" fn name(_: *mut c_void) -> *const c_char {
    c"remux gate".as_ptr()
}

unsafe extern "C" fn create(
    settings: *mut sys::obs_data_t,
    _: *mut sys::obs_source_t,
) -> *mut c_void {
    let channels = channels().max(1);
    Box::into_raw(Box::new(Hosted {
        gate: Gate::new(sample_rate(), channels, params(settings)),
        channels,
        interleaved: Vec::new(),
    }))
    .cast()
}

unsafe extern "C" fn destroy(data: *mut c_void) {
    // SAFETY: the box made in `create`, dropped once.
    drop(unsafe { Box::from_raw(data.cast::<Hosted>()) });
}

/// libobs's audio is float, one plane per channel; the gate's is
/// interleaved. Through it and back, in place.
unsafe extern "C" fn filter_audio(
    data: *mut c_void,
    audio: *mut sys::obs_audio_data,
) -> *mut sys::obs_audio_data {
    // SAFETY: our filter, and libobs's buffer of `frames` floats per plane,
    // for the call.
    unsafe {
        let filter = &mut *data.cast::<Hosted>();
        let frames = (*audio).frames as usize;
        let planes: Vec<*mut f32> = (0..filter.channels)
            .map(|channel| (*audio).data[channel].cast::<f32>())
            .collect();
        if planes.iter().any(|plane| plane.is_null()) {
            return audio;
        }
        filter.interleaved.clear();
        for frame in 0..frames {
            for plane in &planes {
                filter.interleaved.push(*plane.add(frame));
            }
        }
        filter.gate.process(&mut filter.interleaved);
        if let Some((frame, levels)) = filter.gate.heard() {
            OPEN.store(frame.open, Ordering::Relaxed);
            FULL.store(levels.full.to_bits(), Ordering::Relaxed);
            HF.store(levels.hf.to_bits(), Ordering::Relaxed);
        }
        for (at, sample) in filter.interleaved.iter().enumerate() {
            *planes[at % filter.channels].add(at / filter.channels) = *sample;
        }
        audio
    }
}
