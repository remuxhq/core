//! The motor's libobs, set up as the engine sets it up, and the lists a face
//! needs read off the capture sources' properties: displays, windows,
//! applications, cameras, microphones.

use std::ffi::CStr;

use libobs as sys;
use motor_obs::c;

fn list(source: &str, property: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // SAFETY: a source made and released here; its properties are read and
    // destroyed before it goes, every string copied out.
    unsafe {
        let made = sys::obs_source_create(
            c(source).as_ptr(),
            c"probe".as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if made.is_null() {
            println!("  {source}: not created (module not loaded?)");
            return out;
        }
        let props = sys::obs_source_properties(made);
        let p = if props.is_null() {
            std::ptr::null_mut()
        } else {
            sys::obs_properties_get(props, c(property).as_ptr())
        };
        if p.is_null() {
            println!("  {source}: no property {property}");
        } else {
            for i in 0..sys::obs_property_list_item_count(p) {
                let name = CStr::from_ptr(sys::obs_property_list_item_name(p, i))
                    .to_string_lossy()
                    .into_owned();
                let value = sys::obs_property_list_item_string(p, i);
                let value = if value.is_null() {
                    sys::obs_property_list_item_int(p, i).to_string()
                } else {
                    CStr::from_ptr(value).to_string_lossy().into_owned()
                };
                out.push((name, value));
            }
        }
        if !props.is_null() {
            sys::obs_properties_destroy(props);
        }
        sys::obs_source_release(made);
    }
    out
}

fn main() {
    let obs = motor_obs::Obs::start("en-US").expect("libobs");
    obs.set_up().expect("libobs sets up");
    let table = motor_obs::platform::TABLE;
    println!(
        "displays: {:?}",
        list(table.screen.source, table.screen.displays)
    );
    let windows = list(table.screen.window_source, table.screen.windows);
    println!("windows: {:?}", &windows[..windows.len().min(3)]);
    if let Some(apps) = table.screen.apps {
        let apps = list(table.screen.source, apps);
        println!("apps: {:?}", &apps[..apps.len().min(3)]);
    }
    println!(
        "cameras: {:?}",
        list(table.camera.source, table.camera.devices)
    );
    println!("mics: {:?}", list(table.mic.source, table.mic.devices));
}
