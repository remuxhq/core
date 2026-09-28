//! The two kinds of source this motor adds to libobs.
//!
//! `remux_filter`: an operator's OBS effect file (HLSL, the language of OBS's
//! own `.effect` files) as a filter, on a layer's source or on the scene. It
//! draws with the file's `Draw` technique and fills two uniforms when the
//! file declares them: `time`, seconds since the motor started drawing, and
//! `resolution`, the size in pixels of what it filters.
//!
//! `remux_element`: a text or a timer as a picture of its own, exactly the
//! element's width by height, the words centred and shrunk to fit. A filter
//! on it sees that box and nothing larger, the same pixels whatever the words.

use std::ffi::{c_char, c_void, CStr};
use std::sync::OnceLock;
use std::time::Instant;

use crate::{c, ffi};

pub const FILTER: &str = "remux_filter";
pub const ELEMENT: &str = "remux_element";

/// The clock `time` reads: one for every filter, from the first registration.
fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Both kinds, once per process, after the modules have loaded.
pub fn register() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        epoch();
        let filter = ffi::SourceInfo {
            id: c"remux_filter".as_ptr(),
            kind: ffi::OBS_SOURCE_TYPE_FILTER,
            output_flags: ffi::OBS_SOURCE_VIDEO,
            get_name: Some(filter_name),
            create: Some(filter_create),
            destroy: Some(filter_destroy),
            get_width: None,
            get_height: None,
            get_defaults: None,
            get_properties: None,
            update: None,
            activate: None,
            deactivate: None,
            show: None,
            hide: None,
            video_tick: None,
            video_render: Some(filter_render),
        };
        let element = ffi::SourceInfo {
            id: c"remux_element".as_ptr(),
            kind: ffi::OBS_SOURCE_TYPE_INPUT,
            output_flags: ffi::OBS_SOURCE_VIDEO | ffi::OBS_SOURCE_CUSTOM_DRAW,
            get_name: Some(element_name),
            create: Some(element_create),
            destroy: Some(element_destroy),
            get_width: Some(element_width),
            get_height: Some(element_height),
            get_defaults: None,
            get_properties: None,
            update: Some(element_update),
            activate: None,
            deactivate: None,
            show: None,
            hide: None,
            video_tick: None,
            video_render: Some(element_render),
        };
        // SAFETY: libobs copies the struct; the ids and callbacks are static.
        unsafe {
            ffi::obs_register_source_s(&filter, std::mem::size_of::<ffi::SourceInfo>());
            ffi::obs_register_source_s(&element, std::mem::size_of::<ffi::SourceInfo>());
        }
    });
}

/// Whether the file builds, with libobs's own complaint when it does not.
/// Asked before a filter is put anywhere, so a bad file is the reply to the
/// command that named it and never a picture that went quietly unfiltered.
pub fn check(path: &str) -> Result<(), String> {
    if !std::path::Path::new(path).is_file() {
        return Err(format!("no effect file at {path}"));
    }
    // SAFETY: inside the graphics context; the effect and the error string
    // are freed here.
    unsafe {
        ffi::obs_enter_graphics();
        let mut error: *mut c_char = std::ptr::null_mut();
        let effect = ffi::gs_effect_create_from_file(c(path).as_ptr(), &mut error);
        let said = if error.is_null() {
            String::new()
        } else {
            let said = CStr::from_ptr(error).to_string_lossy().trim().to_string();
            ffi::bfree(error.cast());
            said
        };
        if !effect.is_null() {
            ffi::gs_effect_destroy(effect);
        }
        ffi::obs_leave_graphics();
        if effect.is_null() {
            Err(if said.is_empty() {
                format!("{path} is not an effect libobs can build")
            } else {
                format!("{path}: {said}")
            })
        } else {
            Ok(())
        }
    }
}

/// A filter on `source` that draws with the effect at `path`.
pub fn filter(path: &str) -> Result<*mut c_void, String> {
    check(path)?;
    // SAFETY: settings made and released around the create.
    unsafe {
        let settings = ffi::obs_data_create();
        ffi::obs_data_set_string(settings, c"path".as_ptr(), c(path).as_ptr());
        let made = ffi::obs_source_create(
            c(FILTER).as_ptr(),
            c"filter".as_ptr(),
            settings,
            std::ptr::null_mut(),
        );
        ffi::obs_data_release(settings);
        if made.is_null() {
            return Err("libobs could not make the filter".into());
        }
        Ok(made)
    }
}

struct Filter {
    me: *mut c_void,
    effect: *mut c_void,
    time: *mut c_void,
    resolution: *mut c_void,
}

extern "C" fn filter_name(_: *mut c_void) -> *const c_char {
    c"remux filter".as_ptr()
}

extern "C" fn filter_create(settings: *mut c_void, me: *mut c_void) -> *mut c_void {
    // SAFETY: libobs's settings and source for the call; the effect is built
    // inside the graphics context and kept until destroy.
    unsafe {
        let path = ffi::obs_data_get_string(settings, c"path".as_ptr());
        let (mut effect, mut time, mut resolution) = (
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if !path.is_null() {
            ffi::obs_enter_graphics();
            effect = ffi::gs_effect_create_from_file(path, std::ptr::null_mut());
            if !effect.is_null() {
                time = ffi::gs_effect_get_param_by_name(effect, c"time".as_ptr());
                resolution = ffi::gs_effect_get_param_by_name(effect, c"resolution".as_ptr());
            }
            ffi::obs_leave_graphics();
        }
        Box::into_raw(Box::new(Filter {
            me,
            effect,
            time,
            resolution,
        }))
        .cast()
    }
}

extern "C" fn filter_destroy(data: *mut c_void) {
    // SAFETY: the box made in create, once.
    unsafe {
        let filter = Box::from_raw(data.cast::<Filter>());
        if !filter.effect.is_null() {
            ffi::obs_enter_graphics();
            ffi::gs_effect_destroy(filter.effect);
            ffi::obs_leave_graphics();
        }
    }
}

extern "C" fn filter_render(data: *mut c_void, _effect: *mut c_void) {
    // SAFETY: on libobs's render thread, inside the graphics context, with
    // the filter's own data.
    unsafe {
        let filter = &*data.cast::<Filter>();
        let target = ffi::obs_filter_get_target(filter.me);
        let (width, height) = if target.is_null() {
            (0, 0)
        } else {
            (
                ffi::obs_source_get_base_width(target),
                ffi::obs_source_get_base_height(target),
            )
        };
        if filter.effect.is_null() || width == 0 || height == 0 {
            ffi::obs_source_skip_video_filter(filter.me);
            return;
        }
        if !ffi::obs_source_process_filter_begin(
            filter.me,
            ffi::GS_RGBA,
            ffi::OBS_ALLOW_DIRECT_RENDERING,
        ) {
            return;
        }
        if !filter.time.is_null() {
            ffi::gs_effect_set_float(filter.time, epoch().elapsed().as_secs_f32());
        }
        if !filter.resolution.is_null() {
            ffi::gs_effect_set_vec2(
                filter.resolution,
                &ffi::Vec2 {
                    x: width as f32,
                    y: height as f32,
                },
            );
        }
        ffi::obs_source_process_filter_end(filter.me, filter.effect, width, height);
    }
}

/// An element's picture: `width` by `height`, the words centred in it.
pub fn element(width: u32, height: u32, words: &str) -> Result<*mut c_void, String> {
    // SAFETY: settings made and released around the create.
    unsafe {
        let settings = element_settings(width, height, words);
        let made = ffi::obs_source_create(
            c(ELEMENT).as_ptr(),
            c"element".as_ptr(),
            settings,
            std::ptr::null_mut(),
        );
        ffi::obs_data_release(settings);
        if made.is_null() {
            return Err("libobs could not make the element".into());
        }
        Ok(made)
    }
}

/// New words or a new size for an element made by [`element`].
pub(crate) fn reword(source: *mut c_void, width: u32, height: u32, words: &str) {
    // SAFETY: the source is live; the settings are released after the update.
    unsafe {
        let settings = element_settings(width, height, words);
        ffi::obs_source_update(source, settings);
        ffi::obs_data_release(settings);
    }
}

unsafe fn element_settings(width: u32, height: u32, words: &str) -> *mut c_void {
    let settings = ffi::obs_data_create();
    ffi::obs_data_set_int(settings, c"width".as_ptr(), i64::from(width));
    ffi::obs_data_set_int(settings, c"height".as_ptr(), i64::from(height));
    ffi::obs_data_set_string(settings, c"words".as_ptr(), c(words).as_ptr());
    settings
}

/// The words, in white, at four fifths of the box's height: what the text
/// element was on the compositor this replaces.
fn text_json(words: &str, height: u32) -> String {
    let face = crate::platform::TABLE.card_font;
    let size = (f64::from(height) * 0.8).round().max(1.0) as i64;
    format!(
        r#"{{"text":{},"font":{{"face":"{face}","style":"Regular","size":{size},"flags":0}},"color1":4294967295,"color2":4294967295,"antialiasing":true}}"#,
        serde_json::to_string(words).unwrap_or_default()
    )
}

struct Element {
    width: u32,
    height: u32,
    text: *mut c_void,
}

extern "C" fn element_name(_: *mut c_void) -> *const c_char {
    c"remux element".as_ptr()
}

extern "C" fn element_create(settings: *mut c_void, _me: *mut c_void) -> *mut c_void {
    let element = Box::into_raw(Box::new(Element {
        width: 0,
        height: 0,
        text: std::ptr::null_mut(),
    }));
    element_update(element.cast(), settings);
    element.cast()
}

extern "C" fn element_destroy(data: *mut c_void) {
    // SAFETY: the box made in create, once; the text source is its own.
    unsafe {
        let element = Box::from_raw(data.cast::<Element>());
        if !element.text.is_null() {
            ffi::obs_source_release(element.text);
        }
    }
}

extern "C" fn element_update(data: *mut c_void, settings: *mut c_void) {
    // SAFETY: the element's own data and libobs's settings for the call.
    unsafe {
        let element = &mut *data.cast::<Element>();
        element.width = ffi::obs_data_get_int(settings, c"width".as_ptr()).clamp(0, 8192) as u32;
        element.height = ffi::obs_data_get_int(settings, c"height".as_ptr()).clamp(0, 8192) as u32;
        let words = ffi::obs_data_get_string(settings, c"words".as_ptr());
        let words = if words.is_null() {
            String::new()
        } else {
            CStr::from_ptr(words).to_string_lossy().into_owned()
        };
        let text = ffi::obs_data_create_from_json(c(&text_json(&words, element.height)).as_ptr());
        if element.text.is_null() {
            element.text = ffi::obs_source_create(
                c"text_ft2_source_v2".as_ptr(),
                c"words".as_ptr(),
                text,
                std::ptr::null_mut(),
            );
        } else {
            ffi::obs_source_update(element.text, text);
        }
        ffi::obs_data_release(text);
    }
}

extern "C" fn element_width(data: *mut c_void) -> u32 {
    // SAFETY: the element's own data.
    unsafe { (*data.cast::<Element>()).width }
}

extern "C" fn element_height(data: *mut c_void) -> u32 {
    // SAFETY: the element's own data.
    unsafe { (*data.cast::<Element>()).height }
}

extern "C" fn element_render(data: *mut c_void, _effect: *mut c_void) {
    // SAFETY: on libobs's render thread, inside the graphics context.
    unsafe {
        let element = &*data.cast::<Element>();
        if element.text.is_null() {
            return;
        }
        let (w, h) = (
            ffi::obs_source_get_width(element.text) as f32,
            ffi::obs_source_get_height(element.text) as f32,
        );
        if w == 0.0 || h == 0.0 {
            return;
        }
        let (bw, bh) = (element.width as f32, element.height as f32);
        let scale = (bw / w).min(bh / h).min(1.0);
        ffi::gs_matrix_push();
        ffi::gs_matrix_translate3f((bw - w * scale) / 2.0, (bh - h * scale) / 2.0, 0.0);
        ffi::gs_matrix_scale3f(scale, scale, 1.0);
        ffi::obs_source_video_render(element.text);
        ffi::gs_matrix_pop();
    }
}
