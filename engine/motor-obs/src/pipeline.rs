//! The ports over libobs: the sound, the air and what they share. The
//! picture is `crate::picture`.

use std::ffi::{c_void, CStr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use remuxd_domain::engine::{Air, Pipeline, Sound, SoundLevels};
use remuxd_domain::protocol::{Grant, Hearing, Mixing, Outgoing};
use remuxd_domain::sound::mixer::gate::GateParams;
use remuxd_domain::sound::music::Track;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::c;
use crate::sources::Known;
use libobs as sys;

/// The one audio track the encoders take: bit 0.
pub(crate) const MIXER_STREAM: u32 = 1;
/// A track nothing encodes: the music the operator hears but the live does not.
pub(crate) const MIXER_NOBODY: u32 = 1 << 5;

pub struct ObsPipeline {
    pub(crate) known: Arc<Mutex<Known>>,
    /// The scene on output channel 0, what goes out.
    pub(crate) scene: *mut sys::obs_scene_t,
    /// The picture's layers and elements, shared with the tick.
    pub(crate) picture: Box<Mutex<crate::picture::Drawn>>,
    /// The scene's own filter, over everything composed.
    pub(crate) scene_filter: Option<(String, *mut sys::obs_source_t)>,
    /// Filters asked for an element before its picture exists.
    pub(crate) element_filters: std::collections::BTreeMap<String, String>,
    pub(crate) ticking: bool,
    /// Whether a face is drawing the preview (the watch lease).
    pub(crate) previewing: bool,
    /// The panel's self-view flip, which is not the broadcast's.
    pub(crate) mirror: bool,
    /// The microphone, on output channel 1, with its gate and its denoiser
    /// (libobs filters) and a meter on it.
    mic: *mut sys::obs_source_t,
    gate: *mut sys::obs_source_t,
    denoiser: *mut sys::obs_source_t,
    meter: *mut sys::obs_volmeter_t,
    heard: Box<Heard>,
    /// The mix that leaves, metered off the raw audio, and the music on its
    /// own meter.
    mixed: Box<Mixed>,
    music_meter: *mut sys::obs_volmeter_t,
    music_heard: Box<Heard>,
    /// The music, on output channel 2; whether the live gets it is its
    /// mixer mask, whether the speakers do is its monitoring.
    music: *mut sys::obs_source_t,
    music_to_stream: bool,
    monitoring: bool,
    music_was_playing: bool,
    /// The screen's sound, on channel 3: one app's, or off. `sck_audio_capture`
    /// hears every app but this one; with none named, the whole screen.
    screen_sound: *mut sys::obs_source_t,
    screen_heard: Followed,
    hearing_apps: Vec<String>,
    screen_sound_on: bool,
    /// A clip, on channel 4, played once over the mix.
    clip: *mut sys::obs_source_t,
    gate_params: GateParams,
    denoise_on: bool,
    /// The preview ring, made when a face first asks.
    pub(crate) preview: Option<Box<crate::preview::Ring>>,
    width: u32,
    height: u32,
    /// An application's sound on its own, on channel 5, and its fader.
    app_audio: *mut sys::obs_source_t,
    app_heard: Followed,
    app_audio_volume: f64,
    /// The independent audio captures, each on its own channel from 8, and
    /// whether it steps back under the voice.
    audio_layers: Vec<(String, *mut sys::obs_source_t, u32, bool)>,
    /// H264 and AAC (the OS's encoders, from the table), made on first use and shared
    /// by the stream and the recording.
    video_encoder: *mut sys::obs_encoder_t,
    audio_encoder: *mut sys::obs_encoder_t,
    recording: Option<Output>,
    /// One RTMP output per destination, all on the same encoders.
    publishing: Vec<(i64, Output)>,
    /// The duck: a compressor keyed by the microphone on each sound that
    /// plays under the voice, with the sound it is on.
    ducks: Vec<(*mut sys::obs_source_t, *mut sys::obs_source_t)>,
    duck_db: f64,
}

/// What the meter on the microphone last said, in millibels off the
/// callback thread.
#[derive(Default)]
struct Heard {
    level_mdb: AtomicU64,
    peak_mdb: AtomicU64,
    updates: AtomicU64,
    /// Samples the source handed over, both channels, when followed.
    samples: AtomicU64,
}

impl Heard {
    unsafe extern "C" fn on_level(
        param: *mut c_void,
        magnitude: *const f32,
        peak: *const f32,
        _input: *const f32,
    ) {
        // SAFETY: `param` is the boxed `Heard`; the arrays are libobs's for
        // the call and hold eight channels.
        unsafe {
            let heard = &*(param as *const Self);
            let to_mdb = |db: f32| ((db.max(-120.0) + 120.0) * 1000.0) as u64;
            heard.level_mdb.store(to_mdb(*magnitude), Ordering::Relaxed);
            heard.peak_mdb.store(to_mdb(*peak), Ordering::Relaxed);
            heard.updates.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn db(&self, mdb: &AtomicU64) -> f64 {
        mdb.load(Ordering::Relaxed) as f64 / 1000.0 - 120.0
    }
    unsafe extern "C" fn on_audio(
        param: *mut c_void,
        _source: *mut sys::obs_source_t,
        data: *const sys::audio_data,
        _muted: bool,
    ) {
        // SAFETY: `param` is the boxed `Heard`; `data` is libobs's for the
        // call. Counted in both channels, as the native motor counts them.
        unsafe {
            let heard = &*(param as *const Self);
            heard
                .samples
                .fetch_add(u64::from((*data).frames) * 2, Ordering::Relaxed);
        }
    }
}

/// A meter that follows whichever source is there now: the level off a
/// volmeter (after the source's fader), the samples off the source's own
/// audio, kept across sources.
struct Followed {
    meter: *mut sys::obs_volmeter_t,
    heard: Box<Heard>,
}

impl Default for Followed {
    fn default() -> Self {
        Self {
            meter: std::ptr::null_mut(),
            heard: Box::default(),
        }
    }
}

impl Followed {
    fn param(&self) -> *mut c_void {
        &*self.heard as *const Heard as *mut c_void
    }
    /// SAFETY: `source` is live, and is left before it is released.
    unsafe fn follow(&mut self, source: *mut sys::obs_source_t) {
        unsafe {
            if self.meter.is_null() {
                self.meter = sys::obs_volmeter_create(sys::obs_fader_type_OBS_FADER_LOG);
                sys::obs_volmeter_add_callback(self.meter, Some(Heard::on_level), self.param());
            }
            sys::obs_volmeter_attach_source(self.meter, source);
            sys::obs_source_add_audio_capture_callback(source, Some(Heard::on_audio), self.param());
        }
    }
    /// SAFETY: `source` is the live one `follow` was given.
    unsafe fn leave(&mut self, source: *mut sys::obs_source_t) {
        unsafe {
            sys::obs_source_remove_audio_capture_callback(
                source,
                Some(Heard::on_audio),
                self.param(),
            );
            if !self.meter.is_null() {
                sys::obs_volmeter_detach_source(self.meter);
            }
        }
        // Nothing followed is silence, not the last level heard.
        self.heard.level_mdb.store(0, Ordering::Relaxed);
        self.heard.peak_mdb.store(0, Ordering::Relaxed);
    }
    fn samples(&self) -> u64 {
        self.heard.samples.load(Ordering::Relaxed)
    }
    fn db(&self) -> f64 {
        self.heard.db(&self.heard.level_mdb)
    }
}

impl Drop for Followed {
    fn drop(&mut self) {
        if !self.meter.is_null() {
            // SAFETY: ours; its source was left first.
            unsafe { sys::obs_volmeter_destroy(self.meter) };
        }
    }
}

/// The mix that leaves, measured off libobs's raw audio: the level of the
/// last block and a peak that decays.
#[derive(Default)]
struct Mixed {
    frames: AtomicU64,
    level_mdb: AtomicU64,
    peak_mdb: AtomicU64,
    on: bool,
}

impl Mixed {
    unsafe extern "C" fn on_audio(param: *mut c_void, _mix: usize, data: *mut sys::audio_data) {
        // SAFETY: `param` is the boxed `Mixed`; `data` is libobs's for the
        // call, float planar, two channels.
        unsafe {
            let mixed = &*(param as *const Self);
            let data = &*data;
            let n = data.frames as usize;
            if n == 0 || data.data[0].is_null() {
                return;
            }
            let mut sum = 0.0f64;
            let mut peak = 0.0f32;
            for ch in data.data.iter().take(2) {
                if ch.is_null() {
                    continue;
                }
                let samples = std::slice::from_raw_parts(*ch as *const f32, n);
                for s in samples {
                    sum += f64::from(*s) * f64::from(*s);
                    peak = peak.max(s.abs());
                }
            }
            let rms = (sum / (2.0 * n as f64)).sqrt();
            let to_mdb =
                |x: f64| (((20.0 * x.max(1e-6).log10()).max(-120.0) + 120.0) * 1000.0) as u64;
            mixed.level_mdb.store(to_mdb(rms), Ordering::Relaxed);
            let was = mixed.peak_mdb.load(Ordering::Relaxed);
            let now = to_mdb(f64::from(peak));
            // The peak holds and falls a little each block.
            mixed
                .peak_mdb
                .store(now.max(was.saturating_sub(300)), Ordering::Relaxed);
            mixed.frames.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
}

/// One libobs output, running: the file's muxer or the RTMP push.
struct Output {
    output: *mut sys::obs_output_t,
    service: *mut sys::obs_service_t,
    since: Instant,
    /// Stopped on a thread of its own: an RTMP output still connecting
    /// joins its connect thread on stop, which held the engine, and every
    /// face with it, until the door gave up. A recording is stopped in place.
    stops_apart: bool,
}

/// An output's handles, sent to the thread that stops it.
struct Stopping(*mut sys::obs_output_t, *mut sys::obs_service_t);

// SAFETY: libobs is thread-safe about stopping and releasing an output, and
// nothing else holds these once the `Output` is gone.
unsafe impl Send for Stopping {}

impl Stopping {
    fn stop(self) {
        // SAFETY: ours; stopping waits for the muxer to close the file.
        unsafe {
            sys::obs_output_stop(self.0);
            sys::obs_output_release(self.0);
            if !self.1.is_null() {
                sys::obs_service_release(self.1);
            }
        }
    }
}

impl Output {
    fn active(&self) -> bool {
        // SAFETY: a pure read on a live output.
        unsafe { sys::obs_output_active(self.output) }
    }

    /// Whether this output still counts as on its way: active, reconnecting,
    /// or connecting. libobs connects an RTMP output on its own thread and
    /// says `active` only once the door answered; a tick in that window
    /// read "not active" as "the stream ended" and tore a live down two
    /// seconds after it started (OBS 30 on Linux, where the connect took
    /// longer than the tick). Ten seconds is the connect's own timeout.
    fn alive(&self) -> bool {
        // SAFETY: pure reads on a live output.
        let reconnecting = unsafe { sys::obs_output_reconnecting(self.output) };
        self.active()
            || reconnecting
            || (self.since.elapsed() < Duration::from_secs(10) && self.complaint().is_empty())
    }

    fn complaint(&self) -> String {
        // SAFETY: libobs hands out a string it owns, or null.
        unsafe {
            let said = sys::obs_output_get_last_error(self.output);
            if said.is_null() {
                String::new()
            } else {
                CStr::from_ptr(said).to_string_lossy().into_owned()
            }
        }
    }

    fn outgoing(&self, width: u32, height: u32) -> Outgoing {
        // SAFETY: pure reads.
        let (frames, bytes) = unsafe {
            (
                sys::obs_output_get_total_frames(self.output).max(0) as u64,
                sys::obs_output_get_total_bytes(self.output),
            )
        };
        let secs = self.since.elapsed().as_secs_f64().max(0.5);
        Outgoing {
            width,
            height,
            fps: (frames as f64 / secs).round() as u32,
            video_kbps: (bytes as f64 * 8.0 / 1000.0 / secs).round() as u32,
            audio_kbps: 160,
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let stopping = Stopping(self.output, self.service);
        if self.stops_apart {
            std::thread::spawn(move || stopping.stop());
        } else {
            stopping.stop();
        }
    }
}

// SAFETY: the libobs handles are used from the engine's one thread at a
// time (the engine is behind a mutex), and libobs is thread-safe about them.
unsafe impl Send for ObsPipeline {}

impl ObsPipeline {
    pub fn new(known: Arc<Mutex<Known>>) -> Self {
        crate::effect::register();
        crate::gate::register();
        Self {
            known,
            scene: std::ptr::null_mut(),
            picture: Box::default(),
            scene_filter: None,
            element_filters: Default::default(),
            ticking: false,
            previewing: false,
            mirror: false,
            mic: std::ptr::null_mut(),
            gate: std::ptr::null_mut(),
            denoiser: std::ptr::null_mut(),
            meter: std::ptr::null_mut(),
            heard: Box::new(Heard::default()),
            mixed: Box::new(Mixed::default()),
            music_meter: std::ptr::null_mut(),
            music_heard: Box::new(Heard::default()),
            music: std::ptr::null_mut(),
            music_to_stream: true,
            monitoring: false,
            music_was_playing: false,
            screen_sound: std::ptr::null_mut(),
            screen_heard: Followed::default(),
            hearing_apps: Vec::new(),
            screen_sound_on: false,
            clip: std::ptr::null_mut(),
            gate_params: GateParams::default(),
            denoise_on: false,
            preview: None,
            width: 1920,
            height: 1080,
            app_audio: std::ptr::null_mut(),
            app_heard: Followed::default(),
            app_audio_volume: 1.0,
            audio_layers: Vec::new(),
            video_encoder: std::ptr::null_mut(),
            audio_encoder: std::ptr::null_mut(),
            recording: None,
            publishing: Vec::new(),
            ducks: Vec::new(),
            duck_db: -24.0,
        }
    }

    /// The encoders, made once: 6 Mbps constant, a keyframe every 2 s, AAC
    /// at 160 kbps.
    fn encoders(&mut self) -> Result<(*mut sys::obs_encoder_t, *mut sys::obs_encoder_t), String> {
        if self.video_encoder.is_null() {
            // SAFETY: settings created and released here; the encoders are
            // kept until the pipeline drops.
            unsafe {
                let video = sys::obs_data_create();
                sys::obs_data_set_int(video, c("bitrate").as_ptr(), 6000);
                sys::obs_data_set_string(video, c("rate_control").as_ptr(), c("CBR").as_ptr());
                sys::obs_data_set_int(video, c("keyint_sec").as_ptr(), 2);
                // No B-frames, as the native motor encodes: a reordered frame
                // parts presentation from decode time, which RTMP's muxers
                // assume are one. VideoToolbox reads `bframes`, x264 its opts;
                // each ignores the other's.
                sys::obs_data_set_bool(video, c("bframes").as_ptr(), false);
                sys::obs_data_set_string(video, c("x264opts").as_ptr(), c("bframes=0").as_ptr());
                let table = &crate::platform::TABLE;
                let encoder = sys::obs_video_encoder_create(
                    c(table.video_encoder).as_ptr(),
                    c("h264").as_ptr(),
                    video,
                    std::ptr::null_mut(),
                );
                sys::obs_data_release(video);
                if encoder.is_null() {
                    return Err(format!("libobs has no {} encoder", table.video_encoder));
                }
                sys::obs_encoder_set_video(encoder, sys::obs_get_video());
                let audio = sys::obs_data_create();
                sys::obs_data_set_int(audio, c("bitrate").as_ptr(), 160);
                let aac = sys::obs_audio_encoder_create(
                    c(table.audio_encoder).as_ptr(),
                    c("aac").as_ptr(),
                    audio,
                    0,
                    std::ptr::null_mut(),
                );
                sys::obs_data_release(audio);
                if aac.is_null() {
                    sys::obs_encoder_release(encoder);
                    return Err(format!("libobs has no {} encoder", table.audio_encoder));
                }
                sys::obs_encoder_set_audio(aac, sys::obs_get_audio());
                self.video_encoder = encoder;
                self.audio_encoder = aac;
            }
        }
        Ok((self.video_encoder, self.audio_encoder))
    }

    /// An output of this kind with these settings, on the encoders, started.
    fn start_output(
        &mut self,
        kind: &str,
        settings: *mut sys::obs_data_t,
        service: *mut sys::obs_service_t,
    ) -> Result<Output, String> {
        let (video, audio) = self.encoders()?;
        // SAFETY: the output takes its own references to the encoders and
        // the service; `settings` is released here after the create.
        unsafe {
            let output = sys::obs_output_create(
                c(kind).as_ptr(),
                c(kind).as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            sys::obs_data_release(settings);
            if output.is_null() {
                return Err(format!("libobs has no {kind} output"));
            }
            sys::obs_output_set_video_encoder(output, video);
            sys::obs_output_set_audio_encoder(output, audio, 0);
            if !service.is_null() {
                sys::obs_output_set_service(output, service);
            }
            let made = Output {
                output,
                service,
                since: Instant::now(),
                stops_apart: false,
            };
            if !sys::obs_output_start(output) {
                let why = made.complaint();
                return Err(if why.is_empty() {
                    format!("the {kind} output did not start")
                } else {
                    why
                });
            }
            Ok(made)
        }
    }

    /// A filter of this kind on the microphone, with these settings.
    fn filter_on_mic(
        &mut self,
        kind: &str,
        name: &str,
        settings: *mut sys::obs_data_t,
    ) -> *mut sys::obs_source_t {
        if self.mic.is_null() {
            // SAFETY: settings created by the caller, released here.
            unsafe { sys::obs_data_release(settings) };
            return std::ptr::null_mut();
        }
        // SAFETY: the filter is ours; adding it takes libobs's own reference.
        unsafe {
            let filter = sys::obs_source_create(
                c(kind).as_ptr(),
                c(name).as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            sys::obs_data_release(settings);
            if !filter.is_null() {
                sys::obs_source_filter_add(self.mic, filter);
            }
            filter
        }
    }

    fn drop_filter(&mut self, filter: &mut *mut sys::obs_source_t) {
        if !filter.is_null() {
            // SAFETY: ours, on the mic.
            unsafe {
                if !self.mic.is_null() {
                    sys::obs_source_filter_remove(self.mic, *filter);
                }
                sys::obs_source_release(*filter);
            }
            *filter = std::ptr::null_mut();
        }
    }

    /// The screen's sound source, remade for what is heard now.
    fn apply_screen_sound(&mut self) -> Result<(), String> {
        // SAFETY: the old one comes off channel 3 before release.
        unsafe {
            if !self.screen_sound.is_null() {
                self.unduck(self.screen_sound);
                sys::obs_set_output_source(3, std::ptr::null_mut());
                self.screen_heard.leave(self.screen_sound);
                sys::obs_source_release(self.screen_sound);
                self.screen_sound = std::ptr::null_mut();
            }
            if !self.screen_sound_on {
                return Ok(());
            }
            // `sck_audio_capture`: type 0 is the whole desktop, 1 one app by
            // its bundle id (mac-sck-common.h); anything else is a crash.
            let settings = sys::obs_data_create();
            let table = &crate::platform::TABLE;
            match self
                .hearing_apps
                .first()
                .filter(|_| table.screen_sound.per_app)
            {
                Some(app) => {
                    let bundle = self
                        .known
                        .lock()
                        .ok()
                        .and_then(|k| k.apps.get(app).cloned())
                        .ok_or_else(|| format!("no running application called {app}"))?;
                    sys::obs_data_set_int(settings, c("type").as_ptr(), 1);
                    sys::obs_data_set_string(
                        settings,
                        c("application").as_ptr(),
                        c(&bundle).as_ptr(),
                    );
                }
                None if table.screen_sound.per_app => {
                    sys::obs_data_set_int(settings, c("type").as_ptr(), 0)
                }
                None => {}
            }
            let source = sys::obs_source_create(
                c(table.screen_sound.source).as_ptr(),
                c("screen sound").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            sys::obs_data_release(settings);
            if source.is_null() {
                return Err("libobs could not hear the screen".into());
            }
            sys::obs_set_output_source(3, source);
            self.screen_heard.follow(source);
            self.screen_sound = source;
        }
        self.apply_duck();
        Ok(())
    }

    /// Whatever plays under the voice steps back when it speaks: the music,
    /// the screen's sound, an application's, and the audio layers whose kind
    /// ducks; never a microphone or a clip. A compressor on each, keyed by
    /// the microphone (libobs's sidechain), its threshold set so the step is
    /// about `duck_db` when the voice is at a normal level.
    fn apply_duck(&mut self) {
        let ducked: Vec<*mut sys::obs_source_t> = self.ducks.iter().map(|(on, _)| *on).collect();
        for on in ducked {
            self.unduck(on);
        }
        if self.mic.is_null() || self.duck_db >= 0.0 {
            return;
        }
        let under = [self.music, self.screen_sound, self.app_audio]
            .into_iter()
            .chain(
                self.audio_layers
                    .iter()
                    .filter(|(_, _, _, ducks)| *ducks)
                    .map(|(_, source, _, _)| *source),
            )
            .filter(|source| !source.is_null())
            .collect::<Vec<_>>();
        for on in under {
            // SAFETY: `on` is ours and live; settings released after create.
            unsafe {
                let settings = sys::obs_data_create();
                sys::obs_data_set_double(settings, c("ratio").as_ptr(), 32.0);
                // A voice at about -20 dBFS compressed 32:1 above this
                // threshold steps the sound down by about the duck.
                sys::obs_data_set_double(settings, c("threshold").as_ptr(), -20.0 + self.duck_db);
                sys::obs_data_set_int(settings, c("attack_time").as_ptr(), 10);
                sys::obs_data_set_int(settings, c("release_time").as_ptr(), 400);
                sys::obs_data_set_string(
                    settings,
                    c("sidechain_source").as_ptr(),
                    c("mic").as_ptr(),
                );
                let filter = sys::obs_source_create(
                    c("compressor_filter").as_ptr(),
                    c("duck").as_ptr(),
                    settings,
                    std::ptr::null_mut(),
                );
                sys::obs_data_release(settings);
                if !filter.is_null() {
                    sys::obs_source_filter_add(on, filter);
                    self.ducks.push((on, filter));
                }
            }
        }
    }

    /// The duck off a sound, before the sound is released: a filter left on
    /// a source that is gone is ours to release, and nobody's to remove.
    fn unduck(&mut self, on: *mut sys::obs_source_t) {
        let (off, kept) = std::mem::take(&mut self.ducks)
            .into_iter()
            .partition(|(there, _)| *there == on);
        self.ducks = kept;
        for (_, filter) in off {
            // SAFETY: both ours; the sound is still live.
            unsafe {
                sys::obs_source_filter_remove(on, filter);
                sys::obs_source_release(filter);
            }
        }
    }

    fn apply_music_routing(&self) {
        if self.music.is_null() {
            return;
        }
        // SAFETY: ours and live.
        unsafe {
            sys::obs_source_set_audio_mixers(
                self.music,
                if self.music_to_stream {
                    MIXER_STREAM
                } else {
                    MIXER_NOBODY
                },
            );
            sys::obs_source_set_monitoring_type(
                self.music,
                if self.monitoring {
                    sys::obs_monitoring_type_OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT
                } else {
                    sys::obs_monitoring_type_OBS_MONITORING_TYPE_NONE
                },
            );
        }
    }

    /// The mix's meter, on from the first thing that makes sound.
    fn meter_the_mix(&mut self) {
        if self.mixed.on {
            return;
        }
        let convert = sys::audio_convert_info {
            samples_per_sec: 48_000,
            format: sys::audio_format_AUDIO_FORMAT_FLOAT_PLANAR,
            speakers: sys::speaker_layout_SPEAKERS_STEREO,
            // Measured as it leaves: a peak over 0 dBFS is read, never clipped away.
            allow_clipping: true,
        };
        // SAFETY: the box outlives the registration, removed in drop.
        unsafe {
            sys::obs_add_raw_audio_callback(
                0,
                &convert,
                Some(Mixed::on_audio),
                &*self.mixed as *const Mixed as *mut c_void,
            );
        }
        self.mixed.on = true;
    }

    /// The portal's restore token, once the source has it, written beside
    /// the socket so the next boot restores the pick without the dialog.
    /// Polled from `flowing` because the portal answers on its own time.
    pub(crate) fn keep_portal_token(&self) {
        let table = crate::platform::screen();
        let Some(key) = table.token_key.filter(|_| table.portal) else {
            return;
        };
        let screen = self.first_screen();
        if screen.is_null() {
            return;
        }
        let path = crate::platform::portal_token_path();
        let kept = std::fs::read_to_string(&path).unwrap_or_default();
        // SAFETY: a new reference to the settings, released here; the string
        // is copied out before that.
        let token = unsafe {
            let settings = sys::obs_source_get_settings(screen);
            if settings.is_null() {
                return;
            }
            let said = sys::obs_data_get_string(settings, c(key).as_ptr());
            let token = if said.is_null() {
                String::new()
            } else {
                CStr::from_ptr(said).to_string_lossy().into_owned()
            };
            sys::obs_data_release(settings);
            token
        };
        if !token.is_empty() && token != kept.trim() {
            let _ = std::fs::write(&path, &token);
        }
    }

    /// One RTMP output to this url, for this destination.
    fn push(&mut self, id: i64, url: &str) -> Result<(), String> {
        if self.publishing.iter().any(|(there, _)| *there == id) {
            return Err("this engine is already live there".into());
        }
        let mut output = if url.starts_with("rtmp://") || url.starts_with("rtmps://") {
            self.rtmp(url)?
        } else {
            // Anything else is a file, as the native motor's ffmpeg takes it:
            // a test sends a live to one. An FLV is libobs's own writer of
            // what RTMP would carry (its ffmpeg muxer wrote nothing to one).
            let kind = if url.ends_with(".flv") {
                "flv_output"
            } else {
                "ffmpeg_muxer"
            };
            // SAFETY: released by `start_output`.
            let settings = unsafe {
                let settings = sys::obs_data_create();
                sys::obs_data_set_string(settings, c("path").as_ptr(), c(url).as_ptr());
                settings
            };
            self.start_output(kind, settings, std::ptr::null_mut())?
        };
        output.stops_apart = true;
        self.publishing.push((id, output));
        Ok(())
    }

    /// An RTMP output to `rtmp://host/app/key`: libobs wants the door and
    /// the key apart, so the last segment is the key.
    fn rtmp(&mut self, url: &str) -> Result<Output, String> {
        let (server, key) = url
            .rsplit_once('/')
            .ok_or("a destination is rtmp://host/app/key")?;
        // SAFETY: settings created and released around the create.
        let service = unsafe {
            let settings = sys::obs_data_create();
            sys::obs_data_set_string(settings, c("server").as_ptr(), c(server).as_ptr());
            sys::obs_data_set_string(settings, c("key").as_ptr(), c(key).as_ptr());
            let service = sys::obs_service_create(
                c("rtmp_custom").as_ptr(),
                c("destination").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            sys::obs_data_release(settings);
            service
        };
        if service.is_null() {
            return Err("libobs has no rtmp_custom service".into());
        }
        // SAFETY: an empty settings object, released by `start_output`.
        let settings = unsafe { sys::obs_data_create() };
        self.start_output("rtmp_output", settings, service)
    }

    /// The preview ring, made on first use.
    pub(crate) fn ring(&mut self) -> Option<&mut crate::preview::Ring> {
        if self.preview.is_none() {
            self.preview = crate::preview::Ring::new().ok();
        }
        self.preview.as_deref_mut()
    }

    /// The scene, made on first use and put on output channel 0, with the
    /// picture's tick on from its birth.
    pub(crate) fn scene(&mut self) -> *mut sys::obs_scene_t {
        if self.scene.is_null() {
            // SAFETY: the scene is ours until drop; its source is what the
            // output shows. The tick's pointer is the boxed picture, which
            // outlives the registration (removed in drop).
            unsafe {
                self.scene = sys::obs_scene_create(c("remux").as_ptr());
                sys::obs_set_output_source(0, sys::obs_scene_get_source(self.scene));
                sys::obs_add_tick_callback(Some(crate::picture::tick), self.tick_param());
                self.ticking = true;
            }
        }
        self.scene
    }

    fn tick_param(&self) -> *mut c_void {
        &*self.picture as *const Mutex<crate::picture::Drawn> as *mut c_void
    }

    pub(crate) fn width(&self) -> u32 {
        self.width
    }

    pub(crate) fn height(&self) -> u32 {
        self.height
    }

    /// The screen's sound on or off; what is heard of it is `hear`'s.
    pub(crate) fn set_screen_sound(&mut self, on: bool) -> Result<(), String> {
        if on == self.screen_sound_on {
            return Ok(());
        }
        self.screen_sound_on = on;
        let made = self.apply_screen_sound();
        if made.is_err() {
            self.screen_sound_on = false;
        }
        made
    }

    /// An audio source of this platform's kind: `(source id, settings)` for
    /// a microphone by id, one application by name, or the whole screen's.
    fn audio_source(
        &self,
        kind: remuxd_domain::sound::audio_layers::Kind,
        said: &str,
    ) -> Result<*mut sys::obs_source_t, String> {
        use remuxd_domain::sound::audio_layers::Kind;
        let table = &crate::platform::TABLE;
        // SAFETY: settings released after the create.
        unsafe {
            let settings = sys::obs_data_create();
            let id = match kind {
                Kind::Mic => {
                    // An id as the device list gives it, or a name as it reads.
                    let device = crate::sources::list(table.mic.source, table.mic.devices)
                        .into_iter()
                        .find(|(name, id)| id == said || name.eq_ignore_ascii_case(said))
                        .map_or_else(|| said.to_string(), |(_, id)| id);
                    sys::obs_data_set_string(
                        settings,
                        c(table.mic.device_key).as_ptr(),
                        c(&device).as_ptr(),
                    );
                    table.mic.source
                }
                Kind::App => {
                    if !table.screen_sound.per_app {
                        sys::obs_data_release(settings);
                        return Err(
                            "this platform hears the screen whole, never one application".into(),
                        );
                    }
                    let bundle = self
                        .known
                        .lock()
                        .ok()
                        .and_then(|k| {
                            k.apps
                                .iter()
                                .find(|(name, _)| name.eq_ignore_ascii_case(said))
                                .map(|(_, bundle)| bundle.clone())
                        })
                        .ok_or_else(|| format!("no running application called {said}"));
                    let bundle = match bundle {
                        Ok(bundle) => bundle,
                        Err(why) => {
                            sys::obs_data_release(settings);
                            return Err(why);
                        }
                    };
                    // `sck_audio_capture`: type 1 is one app by its bundle id.
                    sys::obs_data_set_int(settings, c("type").as_ptr(), 1);
                    sys::obs_data_set_string(
                        settings,
                        c("application").as_ptr(),
                        c(&bundle).as_ptr(),
                    );
                    table.screen_sound.source
                }
                Kind::Screen => {
                    if table.screen_sound.per_app {
                        sys::obs_data_set_int(settings, c("type").as_ptr(), 0);
                    }
                    table.screen_sound.source
                }
            };
            let source = sys::obs_source_create(
                c(id).as_ptr(),
                c(said).as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            sys::obs_data_release(settings);
            if source.is_null() {
                return Err(format!("libobs could not hear {said}"));
            }
            Ok(source)
        }
    }
}

impl Drop for ObsPipeline {
    fn drop(&mut self) {
        if self.ticking {
            // SAFETY: registered in `scene` with this pointer.
            unsafe { sys::obs_remove_tick_callback(Some(crate::picture::tick), self.tick_param()) };
        }
        self.recording = None;
        self.publishing.clear();
        self.clear_picture();
        let _ = self.app_audio(None);
        for (id, _, _, _) in self.audio_layers.clone() {
            self.audio_layer_remove(&id);
        }
        let _ = self.play(None);
        let _ = self.mic(None);
        // SAFETY: registered with these pointers; the meter is ours.
        unsafe {
            if self.mixed.on {
                sys::obs_remove_raw_audio_callback(
                    0,
                    Some(Mixed::on_audio),
                    &*self.mixed as *const Mixed as *mut c_void,
                );
            }
            if !self.music_meter.is_null() {
                sys::obs_volmeter_destroy(self.music_meter);
            }
        }
        // SAFETY: ours; off the channel first.
        unsafe {
            if !self.clip.is_null() {
                sys::obs_set_output_source(4, std::ptr::null_mut());
                sys::obs_source_release(self.clip);
            }
        }
        self.screen_sound_on = false;
        let _ = self.apply_screen_sound();
        // SAFETY: ours; the channels are emptied before the release.
        unsafe {
            if !self.mic.is_null() {
                sys::obs_set_output_source(1, std::ptr::null_mut());
                sys::obs_source_release(self.mic);
            }
            if !self.scene.is_null() {
                sys::obs_set_output_source(0, std::ptr::null_mut());
                sys::obs_scene_release(self.scene);
            }
        }
        // SAFETY: the outputs that held them are gone.
        unsafe {
            if !self.audio_encoder.is_null() {
                sys::obs_encoder_release(self.audio_encoder);
            }
            if !self.video_encoder.is_null() {
                sys::obs_encoder_release(self.video_encoder);
            }
        }
    }
}

impl Sound for ObsPipeline {
    fn mic(&mut self, device: Option<&str>) -> Result<(), String> {
        // SAFETY: the old one comes off channel 1 before it is released; the
        // new one is ours until then.
        unsafe {
            if !self.mic.is_null() {
                let (mut gate, mut denoiser) = (self.gate, self.denoiser);
                self.drop_filter(&mut gate);
                self.drop_filter(&mut denoiser);
                self.gate = gate;
                self.denoiser = denoiser;
                if !self.meter.is_null() {
                    sys::obs_volmeter_destroy(self.meter);
                    self.meter = std::ptr::null_mut();
                }
                sys::obs_set_output_source(1, std::ptr::null_mut());
                sys::obs_source_release(self.mic);
                self.mic = std::ptr::null_mut();
            }
            let Some(id) = device else { return Ok(()) };
            let table = &crate::platform::TABLE.mic;
            let settings = sys::obs_data_create();
            sys::obs_data_set_string(settings, c(table.device_key).as_ptr(), c(id).as_ptr());
            let source = sys::obs_source_create(
                c(table.source).as_ptr(),
                c("mic").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            sys::obs_data_release(settings);
            if source.is_null() {
                return Err("libobs could not open the microphone".into());
            }
            sys::obs_set_output_source(1, source);
            self.mic = source;
            self.meter_the_mix();
            let meter = sys::obs_volmeter_create(sys::obs_fader_type_OBS_FADER_LOG);
            sys::obs_volmeter_attach_source(meter, source);
            sys::obs_volmeter_add_callback(
                meter,
                Some(Heard::on_level),
                &*self.heard as *const Heard as *mut c_void,
            );
            self.meter = meter;
        }
        let params = self.gate_params;
        self.gate(params);
        if self.denoise_on {
            self.denoise(true);
        }
        // With no microphone nothing ducks; with a new one, all of it again.
        self.apply_duck();
        Ok(())
    }
    fn gate(&mut self, params: GateParams) {
        self.gate_params = params;
        let mut gate = self.gate;
        self.drop_filter(&mut gate);
        self.gate = self.filter_on_mic(crate::gate::GATE, "gate", crate::gate::settings(params));
        if !self.mic.is_null() {
            // The gate hands the voice back a block and its lookahead late;
            // the microphone's offset takes that back, as the native motor
            // stamps the voice from the moment it was heard.
            // SAFETY: our microphone.
            unsafe { sys::obs_source_set_sync_offset(self.mic, -crate::gate::latency_ns()) };
        }
    }
    fn denoise(&mut self, on: bool) {
        self.denoise_on = on;
        let mut denoiser = self.denoiser;
        self.drop_filter(&mut denoiser);
        self.denoiser = denoiser;
        if on {
            // SAFETY: as in `gate`.
            let settings = unsafe {
                let settings = sys::obs_data_create();
                sys::obs_data_set_string(settings, c("method").as_ptr(), c("rnnoise").as_ptr());
                settings
            };
            self.denoiser = self.filter_on_mic("noise_suppress_filter", "denoise", settings);
            // libobs runs a source's filters in the order they were added, so
            // a denoiser added after the gate cleaned what the gate had
            // already let through, and the gate heard the room. It goes back
            // on after the denoiser, which is the order `denoise` promises.
            let params = self.gate_params;
            self.gate(params);
        }
    }
    fn hearing(&self) -> Hearing {
        let heard = Hearing {
            screen_samples: self.screen_heard.samples(),
            app_samples: self.app_heard.samples(),
            ..Hearing::default()
        };
        if self.mic.is_null() {
            return heard;
        }
        // libobs meters after the filters, so a closed gate reads as silence
        // here. Silence is the domain's floor, as the native motor says it.
        let level_db = self.heard.db(&self.heard.level_mdb);
        let floor = remuxd_domain::sound::mixer::levels::Meter::FLOOR_DB;
        Hearing {
            samples: self.heard.updates.load(Ordering::Relaxed) * 480,
            level_db: level_db.max(floor),
            peak_db: self.heard.db(&self.heard.peak_mdb).max(floor),
            gate_open: crate::gate::heard().0,
            gate_levels: crate::gate::heard().1,
            ..heard
        }
    }
    fn play(&mut self, track: Option<&Track>) -> Result<(), String> {
        // SAFETY: the old one comes off channel 2 before release; the new
        // one is ours until then.
        unsafe {
            if !self.music.is_null() {
                self.unduck(self.music);
                sys::obs_set_output_source(2, std::ptr::null_mut());
                sys::obs_source_release(self.music);
                self.music = std::ptr::null_mut();
            }
            let Some(track) = track else {
                self.music_was_playing = false;
                return Ok(());
            };
            let settings = sys::obs_data_create();
            sys::obs_data_set_bool(settings, c("is_local_file").as_ptr(), true);
            sys::obs_data_set_string(settings, c("local_file").as_ptr(), c(&track.url).as_ptr());
            sys::obs_data_set_bool(settings, c("looping").as_ptr(), false);
            sys::obs_data_set_bool(settings, c("clear_on_media_end").as_ptr(), true);
            let source = sys::obs_source_create(
                c("ffmpeg_source").as_ptr(),
                c("music").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            sys::obs_data_release(settings);
            if source.is_null() {
                return Err("libobs could not open the track".into());
            }
            sys::obs_set_output_source(2, source);
            self.music = source;
            if self.music_meter.is_null() {
                let meter = sys::obs_volmeter_create(sys::obs_fader_type_OBS_FADER_LOG);
                sys::obs_volmeter_add_callback(
                    meter,
                    Some(Heard::on_level),
                    &*self.music_heard as *const Heard as *mut c_void,
                );
                self.music_meter = meter;
            }
            sys::obs_volmeter_attach_source(self.music_meter, source);
        }
        self.meter_the_mix();
        self.music_was_playing = true;
        self.apply_duck();
        self.apply_music_routing();
        Ok(())
    }
    fn music_ended(&mut self) -> bool {
        if self.music.is_null() || !self.music_was_playing {
            return false;
        }
        // SAFETY: ours and live.
        let ended = unsafe { sys::obs_source_media_get_state(self.music) }
            == sys::obs_media_state_OBS_MEDIA_STATE_ENDED;
        if ended {
            self.music_was_playing = false;
        }
        ended
    }
    fn clip(&mut self, path: &std::path::Path) -> Result<(), String> {
        // SAFETY: the last clip comes off channel 4 before release.
        unsafe {
            if !self.clip.is_null() {
                sys::obs_set_output_source(4, std::ptr::null_mut());
                sys::obs_source_release(self.clip);
                self.clip = std::ptr::null_mut();
            }
            let settings = sys::obs_data_create();
            sys::obs_data_set_bool(settings, c("is_local_file").as_ptr(), true);
            sys::obs_data_set_string(
                settings,
                c("local_file").as_ptr(),
                c(&path.display().to_string()).as_ptr(),
            );
            sys::obs_data_set_bool(settings, c("looping").as_ptr(), false);
            sys::obs_data_set_bool(settings, c("clear_on_media_end").as_ptr(), true);
            let source = sys::obs_source_create(
                c("ffmpeg_source").as_ptr(),
                c("clip").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            sys::obs_data_release(settings);
            if source.is_null() {
                return Err("libobs could not open the clip".into());
            }
            sys::obs_set_output_source(4, source);
            self.clip = source;
        }
        Ok(())
    }
    fn hear(&mut self, apps: &[String]) -> Result<(), String> {
        self.hearing_apps = apps.to_vec();
        if self.screen_sound_on {
            self.apply_screen_sound()?;
        }
        Ok(())
    }
    fn monitor(&mut self, on: bool) -> Result<(), String> {
        self.monitoring = on;
        // SAFETY: static strings; libobs copies them.
        unsafe {
            sys::obs_set_audio_monitoring_device(c("Default").as_ptr(), c("default").as_ptr());
        }
        self.apply_music_routing();
        Ok(())
    }
    fn levels(&mut self, levels: SoundLevels) -> Result<(), String> {
        if !self.mic.is_null() {
            // SAFETY: ours and live.
            unsafe {
                sys::obs_source_set_volume(self.mic, levels.mic as f32);
                sys::obs_source_set_muted(self.mic, levels.muted);
            }
        }
        self.music_to_stream = levels.music_to_stream;
        if (levels.duck_db - self.duck_db).abs() > 0.01 {
            self.duck_db = levels.duck_db;
            self.apply_duck();
        }
        self.set_screen_sound(levels.screen_sound)?;
        if !self.music.is_null() {
            // SAFETY: ours and live.
            unsafe { sys::obs_source_set_volume(self.music, levels.music as f32) };
        }
        self.app_audio_volume = levels.app_audio_volume;
        if !self.app_audio.is_null() {
            // SAFETY: ours and live.
            unsafe { sys::obs_source_set_volume(self.app_audio, levels.app_audio_volume as f32) };
        }
        self.apply_music_routing();
        Ok(())
    }
    /// One application's sound on channel 5, apart from the screen's. The
    /// new one opens before the old one goes, so a name that is not running
    /// leaves the one that is.
    fn app_audio(&mut self, app: Option<&str>) -> Result<Option<String>, String> {
        let made = match app {
            Some(app) => {
                Some(self.audio_source(remuxd_domain::sound::audio_layers::Kind::App, app)?)
            }
            None => None,
        };
        // SAFETY: the old one comes off channel 5 before release; the new
        // one is ours until the next.
        unsafe {
            if !self.app_audio.is_null() {
                self.unduck(self.app_audio);
                sys::obs_set_output_source(5, std::ptr::null_mut());
                self.app_heard.leave(self.app_audio);
                sys::obs_source_release(self.app_audio);
                self.app_audio = std::ptr::null_mut();
            }
            if let Some(source) = made {
                sys::obs_source_set_volume(source, self.app_audio_volume as f32);
                sys::obs_set_output_source(5, source);
                self.app_heard.follow(source);
                self.app_audio = source;
                self.meter_the_mix();
            }
        }
        self.apply_duck();
        Ok(app.map(String::from))
    }
    /// Each audio layer on a channel of its own, from 8: libobs mixes every
    /// output channel into what leaves.
    fn audio_layer_add(
        &mut self,
        layer: &remuxd_domain::sound::audio_layers::Layer,
    ) -> Result<(), String> {
        use remuxd_domain::sound::audio_layers::Kind;
        let said = match layer.source.kind {
            Kind::Mic => layer.source.device.clone(),
            Kind::App => layer.source.name.clone(),
            Kind::Screen => layer.source.display.map(|d| d.to_string()),
        }
        .ok_or("an audio layer names its source")?;
        let channel = (8..64)
            .find(|ch| self.audio_layers.iter().all(|(_, _, used, _)| used != ch))
            .ok_or("every audio channel is taken")?;
        let source = self.audio_source(layer.source.kind, &said)?;
        // SAFETY: ours until removed; on its own channel.
        unsafe {
            sys::obs_source_set_volume(source, layer.volume as f32);
            sys::obs_source_set_muted(source, layer.muted);
            sys::obs_set_output_source(channel, source);
        }
        self.meter_the_mix();
        self.audio_layers
            .push((layer.id.clone(), source, channel, layer.ducks()));
        self.apply_duck();
        Ok(())
    }
    fn audio_layer_remove(&mut self, id: &str) {
        if let Some(at) = self
            .audio_layers
            .iter()
            .position(|(there, _, _, _)| there == id)
        {
            let (_, source, channel, _) = self.audio_layers.remove(at);
            self.unduck(source);
            // SAFETY: off its channel before release.
            unsafe {
                sys::obs_set_output_source(channel, std::ptr::null_mut());
                sys::obs_source_release(source);
            }
        }
    }
    fn audio_layer_duck(&mut self, id: &str, ducks: bool) {
        if let Some(layer) = self
            .audio_layers
            .iter_mut()
            .find(|(there, _, _, _)| there == id)
        {
            layer.3 = ducks;
            self.apply_duck();
        }
    }
    fn audio_layer_levels(&mut self, id: &str, volume: f64, muted: bool) {
        if let Some((_, source, _, _)) = self
            .audio_layers
            .iter()
            .find(|(there, _, _, _)| there == id)
        {
            // SAFETY: ours and live.
            unsafe {
                sys::obs_source_set_volume(*source, volume as f32);
                sys::obs_source_set_muted(*source, muted);
            }
        }
    }
    fn speakers(&self) -> Option<String> {
        self.monitoring.then(|| "Default".to_string())
    }
    fn mixing(&self) -> Mixing {
        // Silence is the domain's floor, as the native motor reports it, not
        // libobs's -120.
        let floor = remuxd_domain::sound::mixer::levels::Meter::FLOOR_DB;
        let db = |mdb: &AtomicU64| (mdb.load(Ordering::Relaxed) as f64 / 1000.0 - 120.0).max(floor);
        let music = |mdb: &AtomicU64| {
            if self.music.is_null() {
                floor
            } else {
                self.music_heard.db(mdb).max(floor)
            }
        };
        let music_out = |db: f64| if self.music_to_stream { db } else { floor };
        Mixing {
            playing: self.music_was_playing,
            music_out_db: music_out(music(&self.music_heard.level_mdb)),
            music_out_peak_db: music_out(music(&self.music_heard.peak_mdb)),
            frames: self.mixed.frames.load(Ordering::Relaxed),
            level_db: db(&self.mixed.level_mdb),
            peak_db: db(&self.mixed.peak_mdb),
            music_db: music(&self.music_heard.level_mdb),
            music_peak_db: music(&self.music_heard.peak_mdb),
            app_db: if self.app_audio.is_null() {
                floor
            } else {
                self.app_heard.db().max(floor)
            },
            // Only the music is monitored (`apply_music_routing`): the
            // speakers hear it, as you hear it, or nothing.
            monitor_db: if self.monitoring {
                music(&self.music_heard.level_mdb)
            } else {
                floor
            },
            ..Mixing::default()
        }
    }
}

impl Air for ObsPipeline {
    /// The url is `rtmp://host/app/key`: libobs wants the door and the key
    /// apart, so the last segment is the key (a query on it stays with it).
    fn publish(&mut self, url: &str) -> Result<(), String> {
        self.publish_to(0, url)
    }
    fn publish_to(&mut self, id: i64, url: &str) -> Result<(), String> {
        self.push(id, url)
    }
    fn publishing(&self) -> Vec<i64> {
        self.publishing
            .iter()
            .filter(|(_, out)| out.alive())
            .map(|(id, _)| *id)
            .collect()
    }
    fn troubles(&self) -> Vec<(i64, String)> {
        self.publishing
            .iter()
            .filter(|(_, out)| !out.alive())
            .map(|(id, out)| {
                let why = out.complaint();
                (
                    *id,
                    if why.is_empty() {
                        "the stream ended".into()
                    } else {
                        why
                    },
                )
            })
            .collect()
    }
    fn unpublish(&mut self) {
        self.publishing.clear();
    }
    fn still_publishing(&mut self) -> bool {
        self.publishing.iter().any(|(_, out)| out.alive())
    }
    fn outgoing(&self) -> Outgoing {
        self.publishing
            .first()
            .map(|(_, out)| out.outgoing(self.width, self.height))
            .unwrap_or_default()
    }
    fn record(&mut self, into: &str) -> Result<String, String> {
        if self.recording.is_some() {
            return Err("this engine is already recording".into());
        }
        std::fs::create_dir_all(into).map_err(|e| format!("cannot write into {into}: {e}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs() as i64)
            .unwrap_or(0);
        let path = format!("{into}/{}", remuxd_domain::air::recording::name(now));
        // SAFETY: released by `start_output`.
        let settings = unsafe {
            let settings = sys::obs_data_create();
            sys::obs_data_set_string(settings, c("path").as_ptr(), c(&path).as_ptr());
            settings
        };
        self.recording = Some(self.start_output("ffmpeg_muxer", settings, std::ptr::null_mut())?);
        Ok(path)
    }
    fn stop_recording(&mut self) {
        self.recording = None;
    }
}

impl Pipeline for ObsPipeline {
    fn grants(&self) -> (Grant, Grant, Grant) {
        // Where there is no grant to ask for, a device is there or it is not.
        if !crate::platform::TABLE.grants {
            return (Grant::Granted, Grant::Granted, Grant::Granted);
        }
        // libobs asks the system itself and never says what it was told; a
        // device that delivers is a device that was granted. A display on the
        // list is the screen's grant, frames the camera's, samples the mic's.
        let granted = |delivering: bool| {
            if delivering {
                Grant::Granted
            } else {
                Grant::NotAsked
            }
        };
        let screen = self
            .known
            .lock()
            .map(|k| !k.displays.is_empty())
            .unwrap_or(false);
        (
            granted(screen),
            granted(self.a_camera_delivers()),
            granted(self.heard.updates.load(Ordering::Relaxed) > 0),
        )
    }
}
