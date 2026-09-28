//! The encoder ids libobs registered off OBS.app's plugins.
use std::ffi::{c_char, CStr};

use libobs as sys;

fn main() {
    let obs = motor_obs::Obs::start("en-US").expect("libobs");
    obs.set_up().expect("set up");
    let mut i = 0;
    loop {
        let mut id: *const c_char = std::ptr::null();
        // SAFETY: libobs fills `id` with a string it owns while `i` is in range.
        if !unsafe { sys::obs_enum_encoder_types(i, &mut id) } {
            break;
        }
        // SAFETY: as above.
        println!("{}", unsafe { CStr::from_ptr(id) }.to_string_lossy());
        i += 1;
    }
}
