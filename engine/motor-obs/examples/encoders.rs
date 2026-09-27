//! The encoder ids libobs registered off OBS.app's plugins.
use std::ffi::{c_char, CStr};
extern "C" {
    fn obs_enum_encoder_types(idx: usize, id: *mut *const c_char) -> bool;
}
fn main() {
    let obs = motor_obs::Obs::start("en-US").expect("libobs");
    obs.set_up().expect("set up");
    let mut i = 0;
    loop {
        let mut id: *const c_char = std::ptr::null();
        if !unsafe { obs_enum_encoder_types(i, &mut id) } {
            break;
        }
        println!("{}", unsafe { CStr::from_ptr(id) }.to_string_lossy());
        i += 1;
    }
}
