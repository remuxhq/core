//! The modules loaded off OBS.app, video and audio
//! reset, and the three lists a face needs read off the capture sources'
//! properties: displays, cameras, microphones.

use std::ffi::{c_char, c_void, CStr, CString};

#[repr(C)]
struct VideoInfo {
    graphics_module: *const c_char,
    fps_num: u32,
    fps_den: u32,
    base_width: u32,
    base_height: u32,
    output_width: u32,
    output_height: u32,
    output_format: i32,
    adapter: u32,
    gpu_conversion: bool,
    colorspace: i32,
    range: i32,
    scale_type: i32,
}

#[repr(C)]
struct AudioInfo {
    samples_per_sec: u32,
    speakers: i32,
}

extern "C" {
    fn obs_add_module_path(bin: *const c_char, data: *const c_char);
    fn obs_add_data_path(path: *const c_char);
    fn obs_open_module(
        module: *mut *mut c_void,
        path: *const c_char,
        data_path: *const c_char,
    ) -> i32;
    fn obs_init_module(module: *mut c_void) -> bool;
    fn obs_log_loaded_modules();
    fn obs_reset_video(info: *mut VideoInfo) -> i32;
    fn obs_reset_audio(info: *const AudioInfo) -> bool;
    fn obs_source_create(
        id: *const c_char,
        name: *const c_char,
        settings: *mut c_void,
        hotkeys: *mut c_void,
    ) -> *mut c_void;
    fn obs_source_properties(source: *mut c_void) -> *mut c_void;
    fn obs_source_release(source: *mut c_void);
    fn obs_properties_get(props: *mut c_void, name: *const c_char) -> *mut c_void;
    fn obs_properties_destroy(props: *mut c_void);
    fn obs_property_list_item_count(p: *mut c_void) -> usize;
    fn obs_property_list_item_name(p: *mut c_void, idx: usize) -> *const c_char;
    fn obs_property_list_item_string(p: *mut c_void, idx: usize) -> *const c_char;
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn list(source: &str, property: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    unsafe {
        println!("  creating {source}");
        let made = obs_source_create(
            c(source).as_ptr(),
            c("probe").as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if made.is_null() {
            println!("  {source}: not created (module not loaded?)");
            return out;
        }
        println!("  asking {source} for its properties");
        let props = obs_source_properties(made);
        if props.is_null() {
            println!("  {source}: no properties");
            obs_source_release(made);
            return out;
        }
        let p = obs_properties_get(props, c(property).as_ptr());
        if p.is_null() {
            println!("  {source}: no property {property}");
        } else {
            for i in 0..obs_property_list_item_count(p) {
                let name = CStr::from_ptr(obs_property_list_item_name(p, i))
                    .to_string_lossy()
                    .into_owned();
                let value = obs_property_list_item_string(p, i);
                let value = if value.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(value).to_string_lossy().into_owned()
                };
                out.push((name, value));
            }
        }
        obs_properties_destroy(props);
        obs_source_release(made);
    }
    out
}

fn main() {
    let app = std::env::var("OBS_APP").unwrap_or_else(|_| "/Applications/OBS.app".into());
    let _obs = motor_obs::Obs::start("en-US").expect("libobs");
    unsafe {
        obs_add_data_path(
            c(&format!(
                "{app}/Contents/Frameworks/libobs.framework/Resources/"
            ))
            .as_ptr(),
        );
        obs_add_module_path(
            c(&format!(
                "{app}/Contents/PlugIns/%module%.plugin/Contents/MacOS"
            ))
            .as_ptr(),
            c(&format!(
                "{app}/Contents/PlugIns/%module%.plugin/Contents/Resources"
            ))
            .as_ptr(),
        );
        let mut video = VideoInfo {
            graphics_module: c(&format!("{app}/Contents/Frameworks/libobs-opengl.dylib"))
                .into_raw(),
            fps_num: 30,
            fps_den: 1,
            base_width: 1920,
            base_height: 1080,
            output_width: 1920,
            output_height: 1080,
            output_format: 2, // VIDEO_FORMAT_NV12
            adapter: 0,
            gpu_conversion: true,
            colorspace: 2, // VIDEO_CS_709
            range: 2,      // VIDEO_RANGE_PARTIAL
            scale_type: 2, // OBS_SCALE_BICUBIC
        };
        let video_ok = obs_reset_video(&mut video);
        let audio_ok = obs_reset_audio(&AudioInfo {
            samples_per_sec: 48000,
            speakers: 2,
        });
        println!("reset video {video_ok} (0 is ok), audio {audio_ok}");
        // Only the plugins a motor needs: the frontend ones want Qt callbacks.
        for name in [
            "mac-capture",
            "mac-avcapture",
            "obs-ffmpeg",
            "obs-outputs",
            "mac-videotoolbox",
            "coreaudio-encoder",
            "obs-x264",
        ] {
            let mut module: *mut c_void = std::ptr::null_mut();
            let bin = format!("{app}/Contents/PlugIns/{name}.plugin/Contents/MacOS/{name}");
            let data = format!("{app}/Contents/PlugIns/{name}.plugin/Contents/Resources");
            let opened = obs_open_module(&mut module, c(&bin).as_ptr(), c(&data).as_ptr());
            let up = opened == 0 && obs_init_module(module);
            println!("module {name}: open {opened}, init {up}");
        }
        obs_log_loaded_modules();
    }
    println!("displays: {:?}", list("screen_capture", "display_uuid"));
    println!("windows: {:?}", &list("screen_capture", "window")[..3]);
    println!("apps: {:?}", &list("screen_capture", "application")[..2]);
    println!("cameras: {:?}", list("macos-avcapture", "device"));
    println!("mics: {:?}", list("coreaudio_input_capture", "device_id"));
}
