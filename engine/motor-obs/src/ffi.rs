//! The slice of libobs this motor calls, declared by hand: OBS ships no
//! headers, and the dozen functions here are stable C API. Struct layouts
//! are libobs 32's (`obs.h`).

use std::ffi::{c_char, c_void};

#[repr(C)]
pub struct VideoInfo {
    pub graphics_module: *const c_char,
    pub fps_num: u32,
    pub fps_den: u32,
    pub base_width: u32,
    pub base_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub output_format: i32,
    pub adapter: u32,
    pub gpu_conversion: bool,
    pub colorspace: i32,
    pub range: i32,
    pub scale_type: i32,
}

#[repr(C)]
pub struct AudioInfo {
    pub samples_per_sec: u32,
    pub speakers: i32,
}

pub const VIDEO_FORMAT_NV12: i32 = 2;
/// `enum obs_order_movement`.
pub const ORDER_MOVE_UP: i32 = 0;
pub const ORDER_MOVE_TOP: i32 = 2;
pub const ORDER_MOVE_BOTTOM: i32 = 3;
pub const VIDEO_CS_709: i32 = 2;
pub const VIDEO_RANGE_PARTIAL: i32 = 2;
pub const OBS_SCALE_BICUBIC: i32 = 2;
pub const SPEAKERS_STEREO: i32 = 2;
pub const OBS_BOUNDS_SCALE_INNER: i32 = 2;
pub const VIDEO_FORMAT_BGRA: i32 = 7;
pub const OBS_MONITORING_TYPE_NONE: i32 = 0;
pub const OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT: i32 = 2;
pub const OBS_MEDIA_STATE_ENDED: i32 = 6;
pub const OBS_FADER_LOG: i32 = 2;
pub const AUDIO_FORMAT_FLOAT_PLANAR: i32 = 8;
pub const GS_BGRA: i32 = 5;
pub const GS_ZS_NONE: i32 = 0;
pub const GS_CLEAR_COLOR: u32 = 1;

#[repr(C)]
pub struct FramesPerSecond {
    pub numerator: u32,
    pub denominator: u32,
}

#[repr(C)]
pub struct Crop {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[repr(C)]
pub struct AudioConvertInfo {
    pub samples_per_sec: u32,
    pub format: i32,
    pub speakers: i32,
}

#[repr(C)]
pub struct AudioData {
    pub data: [*mut u8; 8],
    pub frames: u32,
    pub timestamp: u64,
}
/// The one audio track the encoders take: bit 0.
pub const MIXER_STREAM: u32 = 1;
/// A track nothing encodes: the music the operator hears but the live does not.
pub const MIXER_NOBODY: u32 = 1 << 5;

#[repr(C)]
pub struct VideoScaleInfo {
    pub format: i32,
    pub width: u32,
    pub height: u32,
    pub range: i32,
    pub colorspace: i32,
}

#[repr(C)]
pub struct VideoData {
    pub data: [*mut u8; 8],
    pub linesize: [u32; 8],
    pub timestamp: u64,
}

#[repr(C)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

extern "C" {
    pub fn obs_startup(
        locale: *const c_char,
        module_config_path: *const c_char,
        store: *mut c_void,
    ) -> bool;
    pub fn obs_initialized() -> bool;
    pub fn obs_get_version_string() -> *const c_char;
    pub fn obs_shutdown();
    pub fn obs_add_data_path(path: *const c_char);
    pub fn obs_open_module(
        module: *mut *mut c_void,
        path: *const c_char,
        data_path: *const c_char,
    ) -> i32;
    pub fn obs_init_module(module: *mut c_void) -> bool;
    pub fn obs_post_load_modules();
    pub fn obs_reset_video(info: *mut VideoInfo) -> i32;
    pub fn obs_reset_audio(info: *const AudioInfo) -> bool;
    pub fn obs_get_total_frames() -> u32;
    pub fn obs_add_raw_video_callback(
        conversion: *const VideoScaleInfo,
        callback: extern "C" fn(*mut c_void, *mut VideoData),
        param: *mut c_void,
    );
    pub fn obs_remove_raw_video_callback(
        callback: extern "C" fn(*mut c_void, *mut VideoData),
        param: *mut c_void,
    );
    pub fn obs_get_lagged_frames() -> u32;

    pub fn obs_data_create() -> *mut c_void;
    pub fn obs_data_set_string(data: *mut c_void, name: *const c_char, value: *const c_char);
    pub fn obs_data_set_int(data: *mut c_void, name: *const c_char, value: i64);
    pub fn obs_data_get_string(data: *mut c_void, name: *const c_char) -> *const c_char;
    pub fn obs_data_release(data: *mut c_void);
    /// A new reference to the source's settings; released by the caller.
    pub fn obs_source_get_settings(source: *mut c_void) -> *mut c_void;

    pub fn obs_source_create(
        id: *const c_char,
        name: *const c_char,
        settings: *mut c_void,
        hotkeys: *mut c_void,
    ) -> *mut c_void;
    pub fn obs_source_release(source: *mut c_void);
    pub fn obs_source_properties(source: *mut c_void) -> *mut c_void;
    pub fn obs_properties_get(props: *mut c_void, name: *const c_char) -> *mut c_void;
    pub fn obs_properties_destroy(props: *mut c_void);
    pub fn obs_property_list_item_count(p: *mut c_void) -> usize;
    pub fn obs_property_list_item_name(p: *mut c_void, idx: usize) -> *const c_char;
    pub fn obs_property_list_item_string(p: *mut c_void, idx: usize) -> *const c_char;
    pub fn obs_property_list_item_int(p: *mut c_void, idx: usize) -> i64;

    pub fn obs_set_output_source(channel: u32, source: *mut c_void);
    pub fn obs_source_get_width(source: *mut c_void) -> u32;
    pub fn obs_source_get_height(source: *mut c_void) -> u32;
    pub fn obs_source_set_muted(source: *mut c_void, muted: bool);
    pub fn obs_source_set_volume(source: *mut c_void, volume: f32);
    pub fn obs_source_set_audio_mixers(source: *mut c_void, mixers: u32);
    pub fn obs_source_set_monitoring_type(source: *mut c_void, kind: i32);
    pub fn obs_set_audio_monitoring_device(name: *const c_char, id: *const c_char) -> bool;
    pub fn obs_source_media_get_state(source: *mut c_void) -> i32;
    pub fn obs_source_filter_add(source: *mut c_void, filter: *mut c_void);
    pub fn obs_source_filter_remove(source: *mut c_void, filter: *mut c_void);
    pub fn obs_source_update(source: *mut c_void, settings: *mut c_void);
    pub fn obs_data_set_bool(data: *mut c_void, name: *const c_char, value: bool);
    pub fn obs_data_set_double(data: *mut c_void, name: *const c_char, value: f64);
    pub fn obs_data_create_from_json(json: *const c_char) -> *mut c_void;
    pub fn obs_data_set_frames_per_second(
        data: *mut c_void,
        name: *const c_char,
        fps: FramesPerSecond,
        option: *const c_char,
    );
    pub fn obs_volmeter_create(kind: i32) -> *mut c_void;
    pub fn obs_volmeter_destroy(meter: *mut c_void);
    pub fn obs_volmeter_attach_source(meter: *mut c_void, source: *mut c_void) -> bool;
    pub fn obs_add_raw_audio_callback(
        mix_idx: usize,
        conversion: *const AudioConvertInfo,
        callback: extern "C" fn(*mut c_void, usize, *mut AudioData),
        param: *mut c_void,
    );
    pub fn obs_remove_raw_audio_callback(
        mix_idx: usize,
        callback: extern "C" fn(*mut c_void, usize, *mut AudioData),
        param: *mut c_void,
    );
    pub fn obs_enter_graphics();
    pub fn obs_leave_graphics();
    pub fn obs_add_main_render_callback(
        draw: extern "C" fn(*mut c_void, u32, u32),
        param: *mut c_void,
    );
    pub fn obs_remove_main_render_callback(
        draw: extern "C" fn(*mut c_void, u32, u32),
        param: *mut c_void,
    );
    pub fn obs_source_video_render(source: *mut c_void);
    pub fn gs_texrender_create(format: i32, zstencil: i32) -> *mut c_void;
    pub fn gs_texrender_destroy(texrender: *mut c_void);
    pub fn gs_texrender_reset(texrender: *mut c_void);
    pub fn gs_texrender_begin(texrender: *mut c_void, cx: u32, cy: u32) -> bool;
    pub fn gs_texrender_end(texrender: *mut c_void);
    pub fn gs_texrender_get_texture(texrender: *const c_void) -> *mut c_void;
    pub fn gs_ortho(left: f32, right: f32, top: f32, bottom: f32, znear: f32, zfar: f32);
    pub fn gs_clear(flags: u32, color: *const Vec4, depth: f32, stencil: u8);
    pub fn gs_stagesurface_create(width: u32, height: u32, format: i32) -> *mut c_void;
    pub fn gs_stagesurface_destroy(surface: *mut c_void);
    pub fn gs_stage_texture(surface: *mut c_void, texture: *mut c_void);
    pub fn gs_stagesurface_map(
        surface: *mut c_void,
        data: *mut *mut u8,
        linesize: *mut u32,
    ) -> bool;
    pub fn gs_stagesurface_unmap(surface: *mut c_void);
    pub fn obs_volmeter_add_callback(
        meter: *mut c_void,
        callback: extern "C" fn(*mut c_void, *const f32, *const f32, *const f32),
        param: *mut c_void,
    );

    pub fn obs_scene_create(name: *const c_char) -> *mut c_void;
    pub fn obs_scene_release(scene: *mut c_void);
    pub fn obs_scene_get_source(scene: *const c_void) -> *mut c_void;
    pub fn obs_scene_add(scene: *mut c_void, source: *mut c_void) -> *mut c_void;
    pub fn obs_sceneitem_remove(item: *mut c_void);
    pub fn obs_sceneitem_set_pos(item: *mut c_void, pos: *const Vec2);
    pub fn obs_sceneitem_set_bounds(item: *mut c_void, bounds: *const Vec2);
    pub fn obs_sceneitem_set_bounds_type(item: *mut c_void, kind: i32);
    pub fn obs_sceneitem_set_order_position(item: *mut c_void, position: i32);
    /// `enum obs_order_movement`: up 0, down 1, top 2, bottom 3. Unlike a
    /// position, a movement is never out of range: OBS 30 walks off the
    /// end of the list on a position past the last item.
    pub fn obs_sceneitem_set_order(item: *mut c_void, movement: i32);
    pub fn obs_sceneitem_set_scale(item: *mut c_void, scale: *const Vec2);
    pub fn obs_sceneitem_set_crop(item: *mut c_void, crop: *const Crop);
    pub fn obs_sceneitem_set_visible(item: *mut c_void, visible: bool) -> bool;
    pub fn obs_add_tick_callback(tick: extern "C" fn(*mut c_void, f32), param: *mut c_void);
    pub fn obs_remove_tick_callback(tick: extern "C" fn(*mut c_void, f32), param: *mut c_void);

    pub fn obs_get_video() -> *mut c_void;
    pub fn obs_get_audio() -> *mut c_void;
    pub fn obs_video_encoder_create(
        id: *const c_char,
        name: *const c_char,
        settings: *mut c_void,
        hotkeys: *mut c_void,
    ) -> *mut c_void;
    pub fn obs_audio_encoder_create(
        id: *const c_char,
        name: *const c_char,
        settings: *mut c_void,
        mixer_idx: usize,
        hotkeys: *mut c_void,
    ) -> *mut c_void;
    pub fn obs_encoder_set_video(encoder: *mut c_void, video: *mut c_void);
    pub fn obs_encoder_set_audio(encoder: *mut c_void, audio: *mut c_void);
    pub fn obs_encoder_release(encoder: *mut c_void);
    pub fn obs_output_create(
        id: *const c_char,
        name: *const c_char,
        settings: *mut c_void,
        hotkeys: *mut c_void,
    ) -> *mut c_void;
    pub fn obs_output_set_video_encoder(output: *mut c_void, encoder: *mut c_void);
    pub fn obs_output_set_audio_encoder(output: *mut c_void, encoder: *mut c_void, idx: usize);
    pub fn obs_output_set_service(output: *mut c_void, service: *mut c_void);
    pub fn obs_output_start(output: *mut c_void) -> bool;
    pub fn obs_output_stop(output: *mut c_void);
    pub fn obs_output_active(output: *const c_void) -> bool;
    pub fn obs_output_get_last_error(output: *const c_void) -> *const c_char;
    pub fn obs_output_reconnecting(output: *const c_void) -> bool;
    pub fn obs_output_get_total_frames(output: *const c_void) -> i32;
    pub fn obs_output_get_total_bytes(output: *const c_void) -> u64;
    pub fn obs_output_release(output: *mut c_void);
    pub fn obs_service_create(
        id: *const c_char,
        name: *const c_char,
        settings: *mut c_void,
        hotkeys: *mut c_void,
    ) -> *mut c_void;
    pub fn obs_service_release(service: *mut c_void);
}
