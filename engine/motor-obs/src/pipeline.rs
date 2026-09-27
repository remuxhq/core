//! The ports over libobs. What is ported answers with libobs; the rest
//! answers as `NoPipeline` does, until its turn.

use std::ffi::{c_void, CStr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use remuxd_domain::engine::{Air, Behind, NoPipeline, Picture, Pipeline, Sound};
use remuxd_domain::gate::GateParams;
use remuxd_domain::music::Track;
use remuxd_domain::protocol::{Card, Flowing, Framed, Grant, Hearing, Mixing, Outgoing};
use remuxd_domain::scene::Layout;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::sources::Known;
use crate::{c, ffi};

pub struct ObsPipeline {
    rest: NoPipeline,
    known: Arc<Mutex<Known>>,
    /// The scene on output channel 0: the screen at the back, the camera
    /// where the layout puts it.
    scene: *mut c_void,
    /// The source behind the picture and its item in the scene.
    behind: *mut c_void,
    behind_item: *mut c_void,
    camera_source: *mut c_void,
    camera_item: *mut c_void,
    /// A whole display behind the picture: the self-view window on it is
    /// the camera on the stream, and the camera item is hidden. See
    /// `scene::camera_is_composited`.
    whole_display: bool,
    /// Whether a face is drawing the preview (the watch lease), which is
    /// when a self-view window may be on the display.
    previewing: bool,
    camera_id: Option<String>,
    /// The camera's look (a colour filter) and its shape (a mask filter),
    /// libobs filters on the camera source.
    look: *mut c_void,
    mask: *mut c_void,
    layout: Layout,
    mirror: bool,
    bounce: remuxd_domain::scene::Bounce,
    /// The tick is on from the scene's birth: it places what had no size
    /// yet, and moves the bounce.
    ticking: bool,
    place_pending: bool,
    /// A card over everything (its PNG on an image source), the words and
    /// the clock's start, so the tick can rewrite the seconds.
    card: *mut c_void,
    card_item: *mut c_void,
    card_words: Option<(String, Option<(Instant, Duration)>)>,
    card_last_second: u64,
    card_words_source: (*mut c_void, *mut c_void),
    card_clock_source: (*mut c_void, *mut c_void),
    card_share: f64,
    /// The microphone, on output channel 1, with its gate and its denoiser
    /// (libobs filters) and a meter on it.
    mic: *mut c_void,
    gate: *mut c_void,
    denoiser: *mut c_void,
    meter: *mut c_void,
    heard: Box<Heard>,
    /// The mix that leaves, metered off the raw audio, and the music on its
    /// own meter.
    mixed: Box<Mixed>,
    music_meter: *mut c_void,
    music_heard: Box<Heard>,
    /// The music, on output channel 2; whether the live gets it is its
    /// mixer mask, whether the speakers do is its monitoring.
    music: *mut c_void,
    music_to_stream: bool,
    monitoring: bool,
    music_was_playing: bool,
    /// The screen's sound, on channel 3: one app's, or off. `sck_audio_capture`
    /// hears every app but this one; with none named, the whole screen.
    screen_sound: *mut c_void,
    hearing_apps: Vec<String>,
    screen_sound_on: bool,
    /// A clip, on channel 4, played once over the mix.
    clip: *mut c_void,
    gate_params: GateParams,
    denoise_on: bool,
    /// The preview ring, made when a face first asks.
    preview: Option<Box<crate::preview::Ring>>,
    width: u32,
    height: u32,
    /// H264 and AAC (the OS's encoders, from the table), made on first use and shared
    /// by the stream and the recording.
    video_encoder: *mut c_void,
    audio_encoder: *mut c_void,
    recording: Option<Output>,
    /// One RTMP output per destination, all on the same encoders.
    publishing: Vec<(i64, Output)>,
    /// The duck: a compressor on the music keyed by the microphone.
    duck: *mut c_void,
    duck_db: f64,
}

/// What the meter on the microphone last said, in millibels off the
/// callback thread.
#[derive(Default)]
struct Heard {
    level_mdb: AtomicU64,
    peak_mdb: AtomicU64,
    updates: AtomicU64,
}

impl Heard {
    extern "C" fn on_level(
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
    extern "C" fn on_audio(param: *mut c_void, _mix: usize, data: *mut ffi::AudioData) {
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
    output: *mut c_void,
    service: *mut c_void,
    since: Instant,
}

impl Output {
    fn active(&self) -> bool {
        // SAFETY: a pure read on a live output.
        unsafe { ffi::obs_output_active(self.output) }
    }

    /// Whether this output still counts as on its way: active, reconnecting,
    /// or connecting. libobs connects an RTMP output on its own thread and
    /// says `active` only once the door answered; a tick in that window
    /// read "not active" as "the stream ended" and tore a live down two
    /// seconds after it started (OBS 30 on Linux, where the connect took
    /// longer than the tick). Ten seconds is the connect's own timeout.
    fn alive(&self) -> bool {
        // SAFETY: pure reads on a live output.
        let reconnecting = unsafe { ffi::obs_output_reconnecting(self.output) };
        self.active()
            || reconnecting
            || (self.since.elapsed() < Duration::from_secs(10) && self.complaint().is_empty())
    }

    fn complaint(&self) -> String {
        // SAFETY: libobs hands out a string it owns, or null.
        unsafe {
            let said = ffi::obs_output_get_last_error(self.output);
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
                ffi::obs_output_get_total_frames(self.output).max(0) as u64,
                ffi::obs_output_get_total_bytes(self.output),
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
        // SAFETY: ours; stopping waits for the muxer to close the file.
        unsafe {
            ffi::obs_output_stop(self.output);
            ffi::obs_output_release(self.output);
            if !self.service.is_null() {
                ffi::obs_service_release(self.service);
            }
        }
    }
}

// SAFETY: the libobs handles are used from the engine's one thread at a
// time (the engine is behind a mutex), and libobs is thread-safe about them.
unsafe impl Send for ObsPipeline {}

impl ObsPipeline {
    pub fn new(known: Arc<Mutex<Known>>) -> Self {
        Self {
            rest: NoPipeline::default(),
            known,
            scene: std::ptr::null_mut(),
            behind: std::ptr::null_mut(),
            behind_item: std::ptr::null_mut(),
            camera_source: std::ptr::null_mut(),
            camera_item: std::ptr::null_mut(),
            whole_display: false,
            previewing: false,
            camera_id: None,
            look: std::ptr::null_mut(),
            mask: std::ptr::null_mut(),
            layout: Layout::default(),
            mirror: false,
            bounce: remuxd_domain::scene::Bounce::default(),
            ticking: false,
            place_pending: false,
            card: std::ptr::null_mut(),
            card_item: std::ptr::null_mut(),
            card_words: None,
            card_last_second: 0,
            card_words_source: (std::ptr::null_mut(), std::ptr::null_mut()),
            card_clock_source: (std::ptr::null_mut(), std::ptr::null_mut()),
            card_share: remuxd_domain::card::TEXT_HEIGHT_SHARE,
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
            hearing_apps: Vec::new(),
            screen_sound_on: false,
            clip: std::ptr::null_mut(),
            gate_params: GateParams::default(),
            denoise_on: false,
            preview: None,
            width: 1920,
            height: 1080,
            video_encoder: std::ptr::null_mut(),
            audio_encoder: std::ptr::null_mut(),
            recording: None,
            publishing: Vec::new(),
            duck: std::ptr::null_mut(),
            duck_db: -24.0,
        }
    }

    /// The encoders, made once: 6 Mbps constant, a keyframe every 2 s, AAC
    /// at 160 kbps.
    fn encoders(&mut self) -> Result<(*mut c_void, *mut c_void), String> {
        if self.video_encoder.is_null() {
            // SAFETY: settings created and released here; the encoders are
            // kept until the pipeline drops.
            unsafe {
                let video = ffi::obs_data_create();
                ffi::obs_data_set_int(video, c("bitrate").as_ptr(), 6000);
                ffi::obs_data_set_string(video, c("rate_control").as_ptr(), c("CBR").as_ptr());
                ffi::obs_data_set_int(video, c("keyint_sec").as_ptr(), 2);
                let table = &crate::platform::TABLE;
                let encoder = ffi::obs_video_encoder_create(
                    c(table.video_encoder).as_ptr(),
                    c("h264").as_ptr(),
                    video,
                    std::ptr::null_mut(),
                );
                ffi::obs_data_release(video);
                if encoder.is_null() {
                    return Err(format!("libobs has no {} encoder", table.video_encoder));
                }
                ffi::obs_encoder_set_video(encoder, ffi::obs_get_video());
                let audio = ffi::obs_data_create();
                ffi::obs_data_set_int(audio, c("bitrate").as_ptr(), 160);
                let aac = ffi::obs_audio_encoder_create(
                    c(table.audio_encoder).as_ptr(),
                    c("aac").as_ptr(),
                    audio,
                    0,
                    std::ptr::null_mut(),
                );
                ffi::obs_data_release(audio);
                if aac.is_null() {
                    ffi::obs_encoder_release(encoder);
                    return Err(format!("libobs has no {} encoder", table.audio_encoder));
                }
                ffi::obs_encoder_set_audio(aac, ffi::obs_get_audio());
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
        settings: *mut c_void,
        service: *mut c_void,
    ) -> Result<Output, String> {
        let (video, audio) = self.encoders()?;
        // SAFETY: the output takes its own references to the encoders and
        // the service; `settings` is released here after the create.
        unsafe {
            let output = ffi::obs_output_create(
                c(kind).as_ptr(),
                c(kind).as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            ffi::obs_data_release(settings);
            if output.is_null() {
                return Err(format!("libobs has no {kind} output"));
            }
            ffi::obs_output_set_video_encoder(output, video);
            ffi::obs_output_set_audio_encoder(output, audio, 0);
            if !service.is_null() {
                ffi::obs_output_set_service(output, service);
            }
            let made = Output {
                output,
                service,
                since: Instant::now(),
            };
            if !ffi::obs_output_start(output) {
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
    fn filter_on_mic(&mut self, kind: &str, name: &str, settings: *mut c_void) -> *mut c_void {
        if self.mic.is_null() {
            // SAFETY: settings created by the caller, released here.
            unsafe { ffi::obs_data_release(settings) };
            return std::ptr::null_mut();
        }
        // SAFETY: the filter is ours; adding it takes libobs's own reference.
        unsafe {
            let filter = ffi::obs_source_create(
                c(kind).as_ptr(),
                c(name).as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            ffi::obs_data_release(settings);
            if !filter.is_null() {
                ffi::obs_source_filter_add(self.mic, filter);
            }
            filter
        }
    }

    fn drop_filter(&mut self, filter: &mut *mut c_void) {
        if !filter.is_null() {
            // SAFETY: ours, on the mic.
            unsafe {
                if !self.mic.is_null() {
                    ffi::obs_source_filter_remove(self.mic, *filter);
                }
                ffi::obs_source_release(*filter);
            }
            *filter = std::ptr::null_mut();
        }
    }

    /// The screen's sound source, remade for what is heard now.
    fn apply_screen_sound(&mut self) -> Result<(), String> {
        // SAFETY: the old one comes off channel 3 before release.
        unsafe {
            if !self.screen_sound.is_null() {
                ffi::obs_set_output_source(3, std::ptr::null_mut());
                ffi::obs_source_release(self.screen_sound);
                self.screen_sound = std::ptr::null_mut();
            }
            if !self.screen_sound_on {
                return Ok(());
            }
            // `sck_audio_capture`: type 0 is the whole desktop, 1 one app by
            // its bundle id (mac-sck-common.h); anything else is a crash.
            let settings = ffi::obs_data_create();
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
                    ffi::obs_data_set_int(settings, c("type").as_ptr(), 1);
                    ffi::obs_data_set_string(
                        settings,
                        c("application").as_ptr(),
                        c(&bundle).as_ptr(),
                    );
                }
                None if table.screen_sound.per_app => {
                    ffi::obs_data_set_int(settings, c("type").as_ptr(), 0)
                }
                None => {}
            }
            let source = ffi::obs_source_create(
                c(table.screen_sound.source).as_ptr(),
                c("screen sound").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            ffi::obs_data_release(settings);
            if source.is_null() {
                return Err("libobs could not hear the screen".into());
            }
            ffi::obs_set_output_source(3, source);
            self.screen_sound = source;
        }
        Ok(())
    }

    /// The music steps under the voice: a compressor on the music keyed by
    /// the microphone (libobs's sidechain), its threshold set so the step
    /// is about `duck_db` when the voice is at a normal level.
    fn apply_duck(&mut self) {
        if self.music.is_null() {
            return;
        }
        // SAFETY: the old filter is ours; settings released after create.
        unsafe {
            if !self.duck.is_null() {
                ffi::obs_source_filter_remove(self.music, self.duck);
                ffi::obs_source_release(self.duck);
                self.duck = std::ptr::null_mut();
            }
            if self.mic.is_null() || self.duck_db >= 0.0 {
                return;
            }
            let settings = ffi::obs_data_create();
            ffi::obs_data_set_double(settings, c("ratio").as_ptr(), 32.0);
            // A voice at about -20 dBFS compressed 32:1 above this threshold
            // steps the music down by about the duck.
            ffi::obs_data_set_double(settings, c("threshold").as_ptr(), -20.0 + self.duck_db);
            ffi::obs_data_set_int(settings, c("attack_time").as_ptr(), 10);
            ffi::obs_data_set_int(settings, c("release_time").as_ptr(), 400);
            ffi::obs_data_set_string(settings, c("sidechain_source").as_ptr(), c("mic").as_ptr());
            let filter = ffi::obs_source_create(
                c("compressor_filter").as_ptr(),
                c("duck").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            ffi::obs_data_release(settings);
            if !filter.is_null() {
                ffi::obs_source_filter_add(self.music, filter);
            }
            self.duck = filter;
        }
    }

    fn apply_music_routing(&self) {
        if self.music.is_null() {
            return;
        }
        // SAFETY: ours and live.
        unsafe {
            ffi::obs_source_set_audio_mixers(
                self.music,
                if self.music_to_stream {
                    ffi::MIXER_STREAM
                } else {
                    ffi::MIXER_NOBODY
                },
            );
            ffi::obs_source_set_monitoring_type(
                self.music,
                if self.monitoring {
                    ffi::OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT
                } else {
                    ffi::OBS_MONITORING_TYPE_NONE
                },
            );
        }
    }

    /// The card as it reads now: a colour behind, the words in the middle,
    /// the clock under them while it counts. Text sources (FreeType), so a
    /// second's change is an update, not a file.
    fn draw_card(&mut self) {
        let Some((line, counting)) = self.card_words.clone() else {
            return;
        };
        let clock = counting.map(|(since, length)| {
            let remaining = length.as_secs_f64() - since.elapsed().as_secs_f64();
            remuxd_domain::card::countdown(remaining.ceil().max(0.0) as i64)
        });
        let (bg, glow) = (remuxd_domain::card::BACKGROUND, remuxd_domain::card::GLOW);
        let abgr = |c: (f64, f64, f64)| {
            (0xFF00_0000u32
                | ((c.2 * 255.0) as u32) << 16
                | ((c.1 * 255.0) as u32) << 8
                | (c.0 * 255.0) as u32) as i64
        };
        let words_size = (self.card_share * self.height as f64) as i64;
        let clock_size = (remuxd_domain::card::CLOCK_HEIGHT_SHARE * self.height as f64) as i64;
        let face = crate::platform::TABLE.card_font;
        let text_json = |text: &str, size: i64, color: i64| {
            format!(
                r#"{{"text":{},"font":{{"face":"{face}","style":"Regular","size":{size},"flags":0}},"color1":{color},"color2":{color},"antialiasing":true}}"#,
                serde_json::to_string(text).unwrap_or_default()
            )
        };
        // SAFETY: every settings object is released after use; the sources
        // and items are ours until `stop`.
        unsafe {
            if self.card.is_null() {
                let settings = ffi::obs_data_create();
                ffi::obs_data_set_int(settings, c("color").as_ptr(), abgr(bg));
                ffi::obs_data_set_int(settings, c("width").as_ptr(), i64::from(self.width));
                ffi::obs_data_set_int(settings, c("height").as_ptr(), i64::from(self.height));
                let back = ffi::obs_source_create(
                    c("color_source_v3").as_ptr(),
                    c("card").as_ptr(),
                    settings,
                    std::ptr::null_mut(),
                );
                ffi::obs_data_release(settings);
                // A plugin that refuses is a card that is not drawn, never a
                // null pointer handed to libobs.
                if back.is_null() {
                    remuxd_domain::log::note("card: libobs could not make the colour source");
                    return;
                }
                let scene = self.scene();
                let item = ffi::obs_scene_add(scene, back);
                if item.is_null() {
                    remuxd_domain::log::note(
                        "card: libobs could not add the colour source to the scene",
                    );
                    ffi::obs_source_release(back);
                    return;
                }
                ffi::obs_sceneitem_set_order(item, ffi::ORDER_MOVE_TOP);
                self.card = back;
                self.card_item = item;
                for (slot, which) in [
                    (&mut self.card_words_source, "words"),
                    (&mut self.card_clock_source, "clock"),
                ] {
                    let settings =
                        ffi::obs_data_create_from_json(c(&text_json("", 10, abgr(glow))).as_ptr());
                    let text = ffi::obs_source_create(
                        c("text_ft2_source_v2").as_ptr(),
                        c(which).as_ptr(),
                        settings,
                        std::ptr::null_mut(),
                    );
                    ffi::obs_data_release(settings);
                    if text.is_null() {
                        remuxd_domain::log::note(&format!(
                            "card: libobs could not make the text source ({which}); the card has no words"
                        ));
                        continue;
                    }
                    let item = ffi::obs_scene_add(scene, text);
                    if item.is_null() {
                        remuxd_domain::log::note(&format!(
                            "card: libobs could not add {which} to the scene"
                        ));
                        ffi::obs_source_release(text);
                        continue;
                    }
                    ffi::obs_sceneitem_set_order(item, ffi::ORDER_MOVE_TOP);
                    *slot = (text, item);
                }
            }
            let (width, height) = (self.width as f32, self.height as f32);
            let place_text = |(source, item): (*mut c_void, *mut c_void),
                              text: &str,
                              size: i64,
                              color: i64,
                              y_share: f32| {
                if source.is_null() || item.is_null() {
                    return;
                }
                let settings =
                    ffi::obs_data_create_from_json(c(&text_json(text, size, color)).as_ptr());
                ffi::obs_source_update(source, settings);
                ffi::obs_data_release(settings);
                // Centred: the text source knows its size once it has the words.
                let (w, h) = (
                    ffi::obs_source_get_width(source) as f32,
                    ffi::obs_source_get_height(source) as f32,
                );
                ffi::obs_sceneitem_set_pos(
                    item,
                    &ffi::Vec2 {
                        x: (width - w) / 2.0,
                        y: height * y_share - h / 2.0,
                    },
                );
            };
            let (words_y, clock_y) = if clock.is_some() {
                (0.42, 0.58)
            } else {
                (0.5, 0.5)
            };
            place_text(
                self.card_words_source,
                &line,
                words_size,
                abgr(glow),
                words_y,
            );
            place_text(
                self.card_clock_source,
                clock.as_deref().unwrap_or(""),
                clock_size,
                0xFFFF_FFFF,
                clock_y,
            );
        }
    }

    /// The mix's meter, on from the first thing that makes sound.
    fn meter_the_mix(&mut self) {
        if self.mixed.on {
            return;
        }
        let convert = ffi::AudioConvertInfo {
            samples_per_sec: 48_000,
            format: ffi::AUDIO_FORMAT_FLOAT_PLANAR,
            speakers: ffi::SPEAKERS_STEREO,
        };
        // SAFETY: the box outlives the registration, removed in drop.
        unsafe {
            ffi::obs_add_raw_audio_callback(
                0,
                &convert,
                Mixed::on_audio,
                &*self.mixed as *const Mixed as *mut c_void,
            );
        }
        self.mixed.on = true;
    }

    /// The portal's restore token, once the source has it, written beside
    /// the socket so the next boot restores the pick without the dialog.
    /// Polled from `flowing` because the portal answers on its own time.
    fn keep_portal_token(&self) {
        let table = crate::platform::screen();
        let Some(key) = table.token_key.filter(|_| table.portal) else {
            return;
        };
        if self.behind.is_null() {
            return;
        }
        let path = crate::platform::portal_token_path();
        let kept = std::fs::read_to_string(&path).unwrap_or_default();
        // SAFETY: a new reference to the settings, released here; the string
        // is copied out before that.
        let token = unsafe {
            let settings = ffi::obs_source_get_settings(self.behind);
            if settings.is_null() {
                return;
            }
            let said = ffi::obs_data_get_string(settings, c(key).as_ptr());
            let token = if said.is_null() {
                String::new()
            } else {
                CStr::from_ptr(said).to_string_lossy().into_owned()
            };
            ffi::obs_data_release(settings);
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
        let (server, key) = url
            .rsplit_once('/')
            .ok_or("a destination is rtmp://host/app/key")?;
        // SAFETY: settings created and released around the create.
        let service = unsafe {
            let settings = ffi::obs_data_create();
            ffi::obs_data_set_string(settings, c("server").as_ptr(), c(server).as_ptr());
            ffi::obs_data_set_string(settings, c("key").as_ptr(), c(key).as_ptr());
            let service = ffi::obs_service_create(
                c("rtmp_custom").as_ptr(),
                c("destination").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            ffi::obs_data_release(settings);
            service
        };
        if service.is_null() {
            return Err("libobs has no rtmp_custom service".into());
        }
        // SAFETY: an empty settings object, released by `start_output`.
        let settings = unsafe { ffi::obs_data_create() };
        let output = self.start_output("rtmp_output", settings, service)?;
        self.publishing.push((id, output));
        Ok(())
    }

    /// The preview ring, made on first use.
    fn ring(&mut self) -> Option<&mut crate::preview::Ring> {
        if self.preview.is_none() {
            self.preview = crate::preview::Ring::new().ok();
        }
        self.preview.as_deref_mut()
    }

    /// The scene, made on first use and put on output channel 0.
    fn scene(&mut self) -> *mut c_void {
        if self.scene.is_null() {
            // SAFETY: the scene is ours until drop; its source is what the
            // output shows.
            unsafe {
                self.scene = ffi::obs_scene_create(c("remux").as_ptr());
                ffi::obs_set_output_source(0, ffi::obs_scene_get_source(self.scene));
                ffi::obs_add_tick_callback(Self::on_tick, self as *mut Self as *mut c_void);
                self.ticking = true;
            }
        }
        self.scene
    }

    /// A filter of this kind on the camera, with these settings; the old one
    /// of that slot goes first.
    fn filter_on_camera(
        &mut self,
        slot: &mut *mut c_void,
        kind: &str,
        name: &str,
        settings: *mut c_void,
    ) {
        // SAFETY: the old filter is ours, on the camera; the new one is ours
        // until replaced; settings released here.
        unsafe {
            if !slot.is_null() {
                if !self.camera_source.is_null() {
                    ffi::obs_source_filter_remove(self.camera_source, *slot);
                }
                ffi::obs_source_release(*slot);
                *slot = std::ptr::null_mut();
            }
            if settings.is_null() || self.camera_source.is_null() {
                if !settings.is_null() {
                    ffi::obs_data_release(settings);
                }
                return;
            }
            let filter = ffi::obs_source_create(
                c(kind).as_ptr(),
                c(name).as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            ffi::obs_data_release(settings);
            if !filter.is_null() {
                ffi::obs_source_filter_add(self.camera_source, filter);
            }
            *slot = filter;
        }
    }

    /// The camera's look and shape, as the layout says: a colour filter for
    /// sepia, mono and noir; a circle is an alpha mask, a square a crop.
    fn dress_camera(&mut self) {
        use remuxd_domain::scene::{Filter, Shape};
        if self.camera_source.is_null() {
            return;
        }
        let mut look = self.look;
        // SAFETY: settings are made here and handed to the filter.
        let settings = unsafe {
            let (saturation, contrast, brightness, multiply): (f64, f64, f64, i64) =
                match self.layout.filter {
                    Filter::Plain => (0.0, 0.0, 0.0, 0),
                    Filter::Sepia => (-0.7, 0.05, 0.0, 0xFF99_CCFF),
                    Filter::Mono => (-1.0, 0.0, 0.0, 0),
                    Filter::Noir => (-1.0, 0.5, -0.1, 0),
                };
            if self.layout.filter == Filter::Plain {
                std::ptr::null_mut()
            } else {
                let settings = ffi::obs_data_create();
                ffi::obs_data_set_double(settings, c("saturation").as_ptr(), saturation);
                ffi::obs_data_set_double(settings, c("contrast").as_ptr(), contrast);
                ffi::obs_data_set_double(settings, c("brightness").as_ptr(), brightness);
                if multiply != 0 {
                    ffi::obs_data_set_int(settings, c("color_multiply").as_ptr(), multiply);
                }
                settings
            }
        };
        self.filter_on_camera(&mut look, "color_filter_v2", "look", settings);
        self.look = look;

        let mut mask = self.mask;
        let settings = if self.layout.shape == Shape::Circle {
            let file = crate::text::folder().join("circle.png");
            match crate::text::circle_mask(&file, (720, 720)) {
                // SAFETY: as above.
                Ok(file) => unsafe {
                    let settings = ffi::obs_data_create();
                    ffi::obs_data_set_string(
                        settings,
                        c("type").as_ptr(),
                        c("mask_alpha_filter.effect").as_ptr(),
                    );
                    ffi::obs_data_set_string(
                        settings,
                        c("image_path").as_ptr(),
                        c(&file.display().to_string()).as_ptr(),
                    );
                    settings
                },
                Err(_) => std::ptr::null_mut(),
            }
        } else {
            std::ptr::null_mut()
        };
        self.filter_on_camera(&mut mask, "mask_filter_v2", "shape", settings);
        self.mask = mask;

        // A square, or a circle, is the camera cut to its middle: a crop on
        // the item, in the source's own pixels.
        if !self.camera_item.is_null() {
            // SAFETY: ours and live.
            unsafe {
                let (w, h) = (
                    ffi::obs_source_get_width(self.camera_source) as i32,
                    ffi::obs_source_get_height(self.camera_source) as i32,
                );
                let crop = if self.layout.shape == Shape::Rectangle || w == 0 || h == 0 {
                    ffi::Crop {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    }
                } else {
                    let side = w.min(h);
                    ffi::Crop {
                        left: (w - side) / 2,
                        top: (h - side) / 2,
                        right: (w - side) / 2,
                        bottom: (h - side) / 2,
                    }
                };
                ffi::obs_sceneitem_set_crop(self.camera_item, &crop);
            }
        }
    }

    /// The rings follow the sources of now.
    fn point_rings(&mut self) {
        let (camera, screen) = (self.camera_source, self.behind);
        if let Some(ring) = self.preview.as_deref() {
            ring.alone(camera, screen);
        }
    }

    fn clear_behind(&mut self) {
        if !self.behind.is_null() {
            self.point_rings_at(std::ptr::null_mut(), self.camera_source);
            // SAFETY: the item and the source are ours; the item goes first.
            unsafe {
                ffi::obs_sceneitem_remove(self.behind_item);
                ffi::obs_source_release(self.behind);
            }
            self.behind = std::ptr::null_mut();
            self.behind_item = std::ptr::null_mut();
        }
    }

    fn point_rings_at(&mut self, screen: *mut c_void, camera: *mut c_void) {
        if let Some(ring) = self.preview.as_deref() {
            ring.alone(camera, screen);
        }
    }

    fn clear_camera(&mut self) {
        if !self.camera_source.is_null() {
            self.point_rings_at(self.behind, std::ptr::null_mut());
            let (mut look, mut mask) = (self.look, self.mask);
            self.filter_on_camera(&mut look, "", "", std::ptr::null_mut());
            self.filter_on_camera(&mut mask, "", "", std::ptr::null_mut());
            self.look = look;
            self.mask = mask;
            // SAFETY: as `clear_behind`.
            unsafe {
                ffi::obs_sceneitem_remove(self.camera_item);
                ffi::obs_source_release(self.camera_source);
            }
            self.camera_source = std::ptr::null_mut();
            self.camera_item = std::ptr::null_mut();
        }
    }

    /// Everything where the layout puts it (`Layout::placed`): the screen's
    /// rectangle and the camera's slot, in the domain's y-up pixels; libobs
    /// counts y down. Mirroring is a negative scale, so the item is placed
    /// at its far edge. A source that has not said its size yet is left
    /// where libobs put it (bounds on a 0x0 source render nothing) and
    /// placed on a later tick.
    fn place(&mut self) {
        let output = (self.width as f64, self.height as f64);
        // SAFETY: the items and sources are ours and live; a null item is
        // skipped.
        unsafe {
            let size_of = |source: *mut c_void| {
                if source.is_null() {
                    return None;
                }
                let (w, h) = (
                    ffi::obs_source_get_width(source),
                    ffi::obs_source_get_height(source),
                );
                (w > 0 && h > 0).then_some((w as f64, h as f64))
            };
            let screen = size_of(self.behind);
            let camera = size_of(self.camera_source);
            self.place_pending = (!self.behind.is_null() && screen.is_none())
                || (!self.camera_source.is_null() && camera.is_none());
            let placed = self
                .layout
                .placed(output, screen, camera, Some(&self.bounce));
            let put = |item: *mut c_void, rect: remuxd_domain::scene::Rect, mirror: bool| {
                ffi::obs_sceneitem_set_bounds_type(item, ffi::OBS_BOUNDS_SCALE_INNER);
                ffi::obs_sceneitem_set_bounds(
                    item,
                    &ffi::Vec2 {
                        x: rect.width as f32,
                        y: rect.height as f32,
                    },
                );
                ffi::obs_sceneitem_set_scale(
                    item,
                    &ffi::Vec2 {
                        x: if mirror { -1.0 } else { 1.0 },
                        y: 1.0,
                    },
                );
                ffi::obs_sceneitem_set_pos(
                    item,
                    &ffi::Vec2 {
                        x: rect.x as f32,
                        y: (output.1 - rect.y - rect.height) as f32,
                    },
                );
            };
            if screen.is_some() {
                put(self.behind_item, placed.screen, false);
            }
            if let (Some((slot, _)), Some(_)) = (placed.camera, camera) {
                put(self.camera_item, slot, self.mirror);
                // Just above the picture behind it: to the bottom, then one up.
                // Two movements, never a position the list may not reach.
                ffi::obs_sceneitem_set_order(self.camera_item, ffi::ORDER_MOVE_BOTTOM);
                ffi::obs_sceneitem_set_order(self.camera_item, ffi::ORDER_MOVE_UP);
                ffi::obs_sceneitem_set_visible(
                    self.camera_item,
                    remuxd_domain::scene::camera_is_composited(self.whole_display, self.previewing),
                );
            }
        }
    }

    /// Every frame: what had no size yet is placed once it has one, and the
    /// bounce moves.
    extern "C" fn on_tick(param: *mut c_void, _seconds: f32) {
        // SAFETY: `param` is this pipeline, boxed by the engine and alive
        // while the callback is registered (removed in drop).
        let me = unsafe { &mut *(param as *mut Self) };
        let bouncing =
            me.layout.mode == remuxd_domain::scene::Mode::Bounce && !me.camera_item.is_null();
        if bouncing {
            let output = (me.width as f64, me.height as f64);
            if let Some((slot, _)) = me.layout.slot(output, (16.0, 9.0)) {
                me.bounce.step(output, (slot.width, slot.height));
            }
        }
        if bouncing || me.place_pending {
            me.place();
        }
        if let Some((_, Some((since, _)))) = &me.card_words {
            let second = since.elapsed().as_secs();
            if second != me.card_last_second {
                me.card_last_second = second;
                me.draw_card();
            }
        }
    }
}

impl Drop for ObsPipeline {
    fn drop(&mut self) {
        if self.ticking {
            // SAFETY: registered in `layout` with this pointer.
            unsafe {
                ffi::obs_remove_tick_callback(Self::on_tick, self as *mut Self as *mut c_void)
            };
        }
        self.recording = None;
        self.publishing.clear();
        self.stop();
        self.clear_camera();
        self.clear_behind();
        let _ = self.play(None);
        let _ = self.mic(None);
        // SAFETY: registered with these pointers; the meter is ours.
        unsafe {
            if self.mixed.on {
                ffi::obs_remove_raw_audio_callback(
                    0,
                    Mixed::on_audio,
                    &*self.mixed as *const Mixed as *mut c_void,
                );
            }
            if !self.music_meter.is_null() {
                ffi::obs_volmeter_destroy(self.music_meter);
            }
        }
        // SAFETY: ours; off the channel first.
        unsafe {
            if !self.clip.is_null() {
                ffi::obs_set_output_source(4, std::ptr::null_mut());
                ffi::obs_source_release(self.clip);
            }
        }
        self.screen_sound_on = false;
        let _ = self.apply_screen_sound();
        // SAFETY: ours; the channels are emptied before the release.
        unsafe {
            if !self.mic.is_null() {
                ffi::obs_set_output_source(1, std::ptr::null_mut());
                ffi::obs_source_release(self.mic);
            }
            if !self.scene.is_null() {
                ffi::obs_set_output_source(0, std::ptr::null_mut());
                ffi::obs_scene_release(self.scene);
            }
        }
        // SAFETY: the outputs that held them are gone.
        unsafe {
            if !self.audio_encoder.is_null() {
                ffi::obs_encoder_release(self.audio_encoder);
            }
            if !self.video_encoder.is_null() {
                ffi::obs_encoder_release(self.video_encoder);
            }
        }
    }
}

impl Picture for ObsPipeline {
    fn capture(&mut self, behind: Behind) -> Result<(), String> {
        self.clear_behind();
        self.whole_display = matches!(behind, Behind::Screen(_));
        match behind {
            Behind::Nothing => Ok(()),
            Behind::Screen(id) => {
                let uuid = self
                    .known
                    .lock()
                    .map_err(|_| "the display list is poisoned")?
                    .displays
                    .get(&id.0)
                    .cloned()
                    .ok_or_else(|| format!("no display {}: remux devices lists them", id.0))?;
                // SAFETY: settings and source are created here and owned by
                // this pipeline until `clear_behind`.
                unsafe {
                    let table = crate::platform::screen();
                    let settings = ffi::obs_data_create();
                    if let Some(kind) = table.kind_key {
                        ffi::obs_data_set_int(settings, c(kind).as_ptr(), 0);
                    }
                    if table.portal {
                        // The portal picks; a token from last time skips its dialog.
                        if let (Some(key), Ok(token)) = (
                            table.token_key,
                            std::fs::read_to_string(crate::platform::portal_token_path()),
                        ) {
                            if !token.trim().is_empty() {
                                ffi::obs_data_set_string(
                                    settings,
                                    c(key).as_ptr(),
                                    c(token.trim()).as_ptr(),
                                );
                            }
                        }
                    } else {
                        // macOS names a display by uuid, Linux by number.
                        match uuid.parse::<i64>() {
                            Ok(n) if table.kind_key.is_none() => {
                                ffi::obs_data_set_int(settings, c(table.display_key).as_ptr(), n)
                            }
                            _ => ffi::obs_data_set_string(
                                settings,
                                c(table.display_key).as_ptr(),
                                c(&uuid).as_ptr(),
                            ),
                        }
                    }
                    let source = ffi::obs_source_create(
                        c(table.source).as_ptr(),
                        c("screen").as_ptr(),
                        settings,
                        std::ptr::null_mut(),
                    );
                    ffi::obs_data_release(settings);
                    if source.is_null() {
                        return Err("libobs could not open the screen".into());
                    }
                    let scene = self.scene();
                    let item = ffi::obs_scene_add(scene, source);
                    ffi::obs_sceneitem_set_order(item, ffi::ORDER_MOVE_BOTTOM);
                    self.behind = source;
                    self.behind_item = item;
                }
                self.point_rings();
                self.place();
                Ok(())
            }
            Behind::Window(id) => {
                // SAFETY: as for a screen.
                unsafe {
                    let table = crate::platform::screen();
                    if table.portal {
                        return Err("under Wayland the portal picks a window: remux screen 1, then choose it in the dialog".into());
                    }
                    let settings = ffi::obs_data_create();
                    if let Some(kind) = table.kind_key {
                        ffi::obs_data_set_int(settings, c(kind).as_ptr(), 1);
                        ffi::obs_data_set_int(
                            settings,
                            c(table.window_key).as_ptr(),
                            i64::from(id.0),
                        );
                    } else {
                        // xcomposite names a window "id\r\nname\r\nclass": the
                        // id alone matches the window.
                        ffi::obs_data_set_string(
                            settings,
                            c(table.window_key).as_ptr(),
                            c(&format!("{}\r\n\r\n", id.0)).as_ptr(),
                        );
                    }
                    let source = ffi::obs_source_create(
                        c(table.window_source).as_ptr(),
                        c("window").as_ptr(),
                        settings,
                        std::ptr::null_mut(),
                    );
                    ffi::obs_data_release(settings);
                    if source.is_null() {
                        return Err("libobs could not open the window".into());
                    }
                    let scene = self.scene();
                    let item = ffi::obs_scene_add(scene, source);
                    ffi::obs_sceneitem_set_order(item, ffi::ORDER_MOVE_BOTTOM);
                    self.behind = source;
                    self.behind_item = item;
                }
                self.point_rings();
                self.place();
                Ok(())
            }
        }
    }
    fn camera(&mut self, device: Option<&str>) -> Result<(), String> {
        self.clear_camera();
        self.camera_id = device.map(String::from);
        let Some(id) = device else { return Ok(()) };
        // SAFETY: settings released after the create; the source and its
        // item are ours until `clear_camera`.
        unsafe {
            let table = &crate::platform::TABLE.camera;
            let settings = ffi::obs_data_create();
            ffi::obs_data_set_string(settings, c(table.device_key).as_ptr(), c(id).as_ptr());
            if let Some((key, preset)) = table.preset {
                ffi::obs_data_set_string(settings, c(key).as_ptr(), c(preset).as_ptr());
            }
            // Held at the picture's 30.
            ffi::obs_data_set_frames_per_second(
                settings,
                c("frame_rate").as_ptr(),
                ffi::FramesPerSecond {
                    numerator: 30,
                    denominator: 1,
                },
                std::ptr::null(),
            );
            let source = ffi::obs_source_create(
                c(table.source).as_ptr(),
                c("camera").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            ffi::obs_data_release(settings);
            if source.is_null() {
                return Err("libobs could not open the camera".into());
            }
            let scene = self.scene();
            let item = ffi::obs_scene_add(scene, source);
            self.camera_source = source;
            self.camera_item = item;
        }
        self.dress_camera();
        self.point_rings();
        self.place();
        Ok(())
    }
    fn layout(&mut self, layout: Layout) {
        let dress = layout.filter != self.layout.filter || layout.shape != self.layout.shape;
        self.layout = layout;
        if dress {
            self.dress_camera();
        }
        self.place();
    }
    /// A card replaces the picture: the screen and the camera are hidden
    /// under it, not composited with it.
    fn show(&mut self, _card: Card, line: &str, counting: Option<Duration>) -> Result<(), String> {
        // `Live` is no card at all: the picture comes back.
        if _card == Card::Live {
            self.stop();
            return Ok(());
        }
        self.card_share = if _card == Card::NothingShared {
            remuxd_domain::card::NOTHING_SHARED_HEIGHT_SHARE
        } else {
            remuxd_domain::card::TEXT_HEIGHT_SHARE
        };
        self.card_words = Some((
            line.to_string(),
            counting.map(|length| (Instant::now(), length)),
        ));
        self.card_last_second = u64::MAX;
        self.draw_card();
        // SAFETY: ours, or null and skipped.
        unsafe {
            for item in [self.behind_item, self.camera_item] {
                if !item.is_null() {
                    ffi::obs_sceneitem_set_visible(item, false);
                }
            }
        }
        Ok(())
    }
    fn stop(&mut self) {
        self.card_words = None;
        // SAFETY: ours; items removed before the sources go.
        unsafe {
            for (source, item) in [
                (self.card, self.card_item),
                self.card_words_source,
                self.card_clock_source,
            ] {
                if !source.is_null() {
                    ffi::obs_sceneitem_remove(item);
                    ffi::obs_source_release(source);
                }
            }
        }
        self.card = std::ptr::null_mut();
        self.card_item = std::ptr::null_mut();
        self.card_words_source = (std::ptr::null_mut(), std::ptr::null_mut());
        self.card_clock_source = (std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: ours, or null and skipped.
        unsafe {
            for item in [self.behind_item, self.camera_item] {
                if !item.is_null() {
                    ffi::obs_sceneitem_set_visible(item, true);
                }
            }
        }
    }
    fn shot(&mut self, of: Framed) -> Option<(Vec<u8>, u32, u32)> {
        let (camera, screen) = (self.camera_source, self.behind);
        let ring = self.ring()?;
        ring.alone(camera, screen);
        let ready = match of {
            Framed::Scene => ring.shot().is_some(),
            Framed::Camera => ring.shot_alone(true).is_some(),
            Framed::Screen => ring.shot_alone(false).is_some(),
        };
        if !ready {
            ring.watch(true);
            ring.render(true);
            std::thread::sleep(Duration::from_millis(200));
        }
        let ring = self.preview.as_ref()?;
        match of {
            Framed::Scene => ring.shot(),
            Framed::Camera => ring.shot_alone(true),
            Framed::Screen => ring.shot_alone(false),
        }
    }
    fn preview(&self) -> Option<remuxd_domain::preview::Preview> {
        self.preview.as_ref().map(|ring| ring.said().clone())
    }
    fn previewing(&mut self, on: bool) {
        if self.previewing != on {
            self.previewing = on;
            self.place();
        }
        let (camera, screen) = (self.camera_source, self.behind);
        if let Some(ring) = self.ring() {
            ring.alone(camera, screen);
            ring.watch(on);
            ring.render(on);
        }
    }
    fn mirror(&mut self, on: bool) {
        self.mirror = on;
        self.place();
    }
    fn flowing(&self) -> Flowing {
        self.keep_portal_token();
        // libobs renders an empty scene as steadily as a full one; a frame
        // counts only when there is something in the picture, or the engine
        // would go live with black. What is behind, the camera or a card.
        let something =
            !self.behind.is_null() || !self.camera_source.is_null() || !self.card.is_null();
        // SAFETY: pure reads of libobs counters.
        let frames = if something {
            unsafe { ffi::obs_get_total_frames() as u64 }
        } else {
            0
        };
        Flowing {
            captured: if self.behind.is_null() { 0 } else { frames },
            frames,
            width: self.width,
            height: self.height,
            held: None,
        }
    }
    fn camera_flowing(&self) -> Flowing {
        if self.camera_source.is_null() {
            return Flowing::default();
        }
        // SAFETY: pure reads on a live source.
        let (width, height) = unsafe {
            (
                ffi::obs_source_get_width(self.camera_source),
                ffi::obs_source_get_height(self.camera_source),
            )
        };
        Flowing {
            captured: u64::from(width > 0),
            frames: unsafe { ffi::obs_get_total_frames() } as u64,
            width,
            height,
            held: (width > 0).then_some(30),
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
                    ffi::obs_volmeter_destroy(self.meter);
                    self.meter = std::ptr::null_mut();
                }
                ffi::obs_set_output_source(1, std::ptr::null_mut());
                ffi::obs_source_release(self.mic);
                self.mic = std::ptr::null_mut();
            }
            let Some(id) = device else { return Ok(()) };
            let table = &crate::platform::TABLE.mic;
            let settings = ffi::obs_data_create();
            ffi::obs_data_set_string(settings, c(table.device_key).as_ptr(), c(id).as_ptr());
            let source = ffi::obs_source_create(
                c(table.source).as_ptr(),
                c("mic").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            ffi::obs_data_release(settings);
            if source.is_null() {
                return Err("libobs could not open the microphone".into());
            }
            ffi::obs_set_output_source(1, source);
            self.mic = source;
            self.meter_the_mix();
            let meter = ffi::obs_volmeter_create(ffi::OBS_FADER_LOG);
            ffi::obs_volmeter_attach_source(meter, source);
            ffi::obs_volmeter_add_callback(
                meter,
                Heard::on_level,
                &*self.heard as *const Heard as *mut c_void,
            );
            self.meter = meter;
        }
        let params = self.gate_params;
        self.gate(params);
        if self.denoise_on {
            self.denoise(true);
        }
        Ok(())
    }
    fn gate(&mut self, params: GateParams) {
        self.gate_params = params;
        // The domain's thresholds are amplitudes; the filter's are dBFS.
        let db = |amplitude: f64| 20.0 * amplitude.max(1e-6).log10();
        let mut gate = self.gate;
        self.drop_filter(&mut gate);
        // SAFETY: settings handed to `filter_on_mic`, which releases them.
        let settings = unsafe {
            let settings = ffi::obs_data_create();
            ffi::obs_data_set_double(settings, c("open_threshold").as_ptr(), db(params.full));
            ffi::obs_data_set_double(settings, c("close_threshold").as_ptr(), db(params.floor));
            ffi::obs_data_set_int(settings, c("hold_time").as_ptr(), params.hold_ms as i64);
            ffi::obs_data_set_int(settings, c("attack_time").as_ptr(), params.attack_ms as i64);
            settings
        };
        self.gate = self.filter_on_mic("noise_gate_filter", "gate", settings);
    }
    fn denoise(&mut self, on: bool) {
        self.denoise_on = on;
        let mut denoiser = self.denoiser;
        self.drop_filter(&mut denoiser);
        self.denoiser = denoiser;
        if on {
            // SAFETY: as in `gate`.
            let settings = unsafe {
                let settings = ffi::obs_data_create();
                ffi::obs_data_set_string(settings, c("method").as_ptr(), c("rnnoise").as_ptr());
                settings
            };
            self.denoiser = self.filter_on_mic("noise_suppress_filter", "denoise", settings);
        }
    }
    fn hearing(&self) -> Hearing {
        if self.mic.is_null() {
            return Hearing::default();
        }
        // libobs meters after the filters, so a closed gate reads as silence
        // here.
        let level_db = self.heard.db(&self.heard.level_mdb);
        Hearing {
            samples: self.heard.updates.load(Ordering::Relaxed) * 480,
            level_db,
            peak_db: self.heard.db(&self.heard.peak_mdb),
            gate_open: level_db > -100.0,
            ..Hearing::default()
        }
    }
    fn play(&mut self, track: Option<&Track>) -> Result<(), String> {
        // SAFETY: the old one comes off channel 2 before release; the new
        // one is ours until then.
        unsafe {
            if !self.music.is_null() {
                ffi::obs_set_output_source(2, std::ptr::null_mut());
                ffi::obs_source_release(self.music);
                self.music = std::ptr::null_mut();
            }
            let Some(track) = track else {
                self.music_was_playing = false;
                return Ok(());
            };
            let settings = ffi::obs_data_create();
            ffi::obs_data_set_bool(settings, c("is_local_file").as_ptr(), true);
            ffi::obs_data_set_string(settings, c("local_file").as_ptr(), c(&track.url).as_ptr());
            ffi::obs_data_set_bool(settings, c("looping").as_ptr(), false);
            ffi::obs_data_set_bool(settings, c("clear_on_media_end").as_ptr(), true);
            let source = ffi::obs_source_create(
                c("ffmpeg_source").as_ptr(),
                c("music").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            ffi::obs_data_release(settings);
            if source.is_null() {
                return Err("libobs could not open the track".into());
            }
            ffi::obs_set_output_source(2, source);
            self.music = source;
            self.duck = std::ptr::null_mut();
            if self.music_meter.is_null() {
                let meter = ffi::obs_volmeter_create(ffi::OBS_FADER_LOG);
                ffi::obs_volmeter_add_callback(
                    meter,
                    Heard::on_level,
                    &*self.music_heard as *const Heard as *mut c_void,
                );
                self.music_meter = meter;
            }
            ffi::obs_volmeter_attach_source(self.music_meter, source);
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
        let ended =
            unsafe { ffi::obs_source_media_get_state(self.music) } == ffi::OBS_MEDIA_STATE_ENDED;
        if ended {
            self.music_was_playing = false;
        }
        ended
    }
    fn clip(&mut self, path: &std::path::Path) -> Result<(), String> {
        // SAFETY: the last clip comes off channel 4 before release.
        unsafe {
            if !self.clip.is_null() {
                ffi::obs_set_output_source(4, std::ptr::null_mut());
                ffi::obs_source_release(self.clip);
                self.clip = std::ptr::null_mut();
            }
            let settings = ffi::obs_data_create();
            ffi::obs_data_set_bool(settings, c("is_local_file").as_ptr(), true);
            ffi::obs_data_set_string(
                settings,
                c("local_file").as_ptr(),
                c(&path.display().to_string()).as_ptr(),
            );
            ffi::obs_data_set_bool(settings, c("looping").as_ptr(), false);
            ffi::obs_data_set_bool(settings, c("clear_on_media_end").as_ptr(), true);
            let source = ffi::obs_source_create(
                c("ffmpeg_source").as_ptr(),
                c("clip").as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            ffi::obs_data_release(settings);
            if source.is_null() {
                return Err("libobs could not open the clip".into());
            }
            ffi::obs_set_output_source(4, source);
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
            ffi::obs_set_audio_monitoring_device(c("Default").as_ptr(), c("default").as_ptr());
        }
        self.apply_music_routing();
        Ok(())
    }
    fn levels(
        &mut self,
        mic: f64,
        music: f64,
        duck_db: f64,
        muted: bool,
        music_to_stream: bool,
        screen_sound: bool,
    ) -> Result<(), String> {
        if !self.mic.is_null() {
            // SAFETY: ours and live.
            unsafe {
                ffi::obs_source_set_volume(self.mic, mic as f32);
                ffi::obs_source_set_muted(self.mic, muted);
            }
        }
        self.music_to_stream = music_to_stream;
        if (duck_db - self.duck_db).abs() > 0.01 {
            self.duck_db = duck_db;
            self.apply_duck();
        }
        if screen_sound != self.screen_sound_on {
            self.screen_sound_on = screen_sound;
            self.apply_screen_sound()?;
        }
        if !self.music.is_null() {
            // SAFETY: ours and live.
            unsafe { ffi::obs_source_set_volume(self.music, music as f32) };
        }
        self.apply_music_routing();
        self.rest
            .levels(mic, music, duck_db, muted, music_to_stream, screen_sound)
    }
    fn speakers(&self) -> Option<String> {
        self.monitoring.then(|| "Default".to_string())
    }
    fn mixing(&self) -> Mixing {
        let db = |mdb: &AtomicU64| mdb.load(Ordering::Relaxed) as f64 / 1000.0 - 120.0;
        let music_out = |db: f64| if self.music_to_stream { db } else { -120.0 };
        Mixing {
            playing: self.music_was_playing,
            music_out_db: music_out(if self.music.is_null() {
                -120.0
            } else {
                self.music_heard.db(&self.music_heard.level_mdb)
            }),
            frames: self.mixed.frames.load(Ordering::Relaxed),
            level_db: db(&self.mixed.level_mdb),
            peak_db: db(&self.mixed.peak_mdb),
            music_db: if self.music.is_null() {
                -120.0
            } else {
                self.music_heard.db(&self.music_heard.level_mdb)
            },
            music_peak_db: if self.music.is_null() {
                -120.0
            } else {
                self.music_heard.db(&self.music_heard.peak_mdb)
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
        let path = format!("{into}/{}", remuxd_domain::recording::name(now));
        // SAFETY: released by `start_output`.
        let settings = unsafe {
            let settings = ffi::obs_data_create();
            ffi::obs_data_set_string(settings, c("path").as_ptr(), c(&path).as_ptr());
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
            granted(self.camera_flowing().frames > 0),
            granted(self.heard.updates.load(Ordering::Relaxed) > 0),
        )
    }
}
