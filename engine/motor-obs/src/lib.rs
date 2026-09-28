//! The libobs motor: the domain's ports (`Sources`, `Picture`, `Sound`,
//! `Air`) over the libobs OBS ships. `remuxd --features obs` boots it
//! under `REMUX_MOTOR=obs`. Linking libobs puts that binary under the GPL.
//!
//! One port at a time: what is not ported yet answers as `NoPipeline` does.

pub mod effect;
pub mod ffi;
mod pipeline;
pub mod platform;
mod preview;
mod sources;
pub mod text;

use std::ffi::{CStr, CString};
use std::sync::{Arc, Mutex};

pub use pipeline::ObsPipeline;
use remuxd_domain::engine::{Pipeline, Sources};
pub use sources::ObsSources;

pub use platform::app;

pub(crate) fn c(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

/// libobs, started and set up (video 1080p30, audio 48 kHz, the modules);
/// dropping it shuts it down. One per process.
pub struct Obs;

impl Obs {
    pub fn start(locale: &str) -> Result<Self, String> {
        let locale = c(locale);
        // The plugins keep their own files (rtmp-services caches its list)
        // under the module config path: beside the socket, never the cwd.
        let config = remuxd_domain::socket::default_path().with_file_name("obs");
        let _ = std::fs::create_dir_all(&config);
        let config = c(&config.display().to_string());
        // SAFETY: the pointers are valid for the call; libobs copies what it
        // keeps. A null store is what OBS documents for a process with no
        // profiler.
        let up =
            unsafe { ffi::obs_startup(locale.as_ptr(), config.as_ptr(), std::ptr::null_mut()) };
        if !up {
            return Err("libobs refused to start".into());
        }
        Ok(Self)
    }

    /// The graphics, the audio and the modules, off the OBS on this machine.
    pub fn set_up(&self) -> Result<(), String> {
        let app = app();
        let table = &platform::TABLE;
        let graphics = c(&(table.graphics)(&app));
        let mut video = ffi::VideoInfo {
            graphics_module: graphics.as_ptr(),
            fps_num: 30,
            fps_den: 1,
            base_width: 1920,
            base_height: 1080,
            output_width: 1920,
            output_height: 1080,
            output_format: ffi::VIDEO_FORMAT_NV12,
            adapter: 0,
            gpu_conversion: true,
            colorspace: ffi::VIDEO_CS_709,
            range: ffi::VIDEO_RANGE_PARTIAL,
            scale_type: ffi::OBS_SCALE_BICUBIC,
        };
        // SAFETY: every string outlives its call; libobs copies them.
        unsafe {
            ffi::obs_add_data_path(c(&(table.data)(&app)).as_ptr());
            let video_said = ffi::obs_reset_video(&mut video);
            if video_said != 0 {
                return Err(format!("libobs could not set up video: {video_said}"));
            }
            let audio = ffi::AudioInfo {
                samples_per_sec: 48_000,
                speakers: ffi::SPEAKERS_STEREO,
            };
            if !ffi::obs_reset_audio(&audio) {
                return Err("libobs could not set up audio".into());
            }
            for name in table.modules.iter().chain(table.optional_modules) {
                let mut module = std::ptr::null_mut();
                let (bin, data) = (table.module)(&app, name);
                let (bin, data) = (c(&bin), c(&data));
                if ffi::obs_open_module(&mut module, bin.as_ptr(), data.as_ptr()) != 0
                    || !ffi::obs_init_module(module)
                {
                    if table.optional_modules.contains(name) {
                        remuxd_domain::log::note(&format!(
                            "the plugin {name} did not load from {app}; going on without it"
                        ));
                        continue;
                    }
                    return Err(format!("the plugin {name} did not load from {app}"));
                }
            }
            // VideoToolbox registers its encoders here, not on load.
            ffi::obs_post_load_modules();
        }
        Ok(())
    }

    pub fn initialized() -> bool {
        // SAFETY: a pure read.
        unsafe { ffi::obs_initialized() }
    }

    pub fn version() -> String {
        // SAFETY: libobs hands out a static string.
        unsafe { CStr::from_ptr(ffi::obs_get_version_string()) }
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for Obs {
    fn drop(&mut self) {
        // SAFETY: started in `start`, and once.
        unsafe { ffi::obs_shutdown() }
    }
}

/// What a boot gets: the motor to keep alive, and the two ports.
pub type Started = (Motor, Box<dyn Sources>, Box<dyn Pipeline>);

/// The motor, started: what the daemon boots with under `REMUX_MOTOR=obs`.
pub struct Motor {
    _obs: Obs,
}

impl Motor {
    pub fn start() -> Result<Started, String> {
        let obs = Obs::start("en-US")?;
        obs.set_up()?;
        (platform::TABLE.helper)()?;
        let known = Arc::new(Mutex::new(sources::Known::default()));
        Ok((
            Motor { _obs: obs },
            Box::new(ObsSources::new(Arc::clone(&known))),
            Box::new(ObsPipeline::new(known)),
        ))
    }
}
