//! `cargo run --example effect`, from `engine/motor-obs`: libobs off OBS.app,
//! the motor's two sources registered, a red square inverted by
//! `examples/invert.effect` and a text element, read back off the output.

use std::ffi::c_void;
use std::sync::Mutex;

use libobs as sys;
use motor_obs::effect;

static FRAME: Mutex<Vec<u8>> = Mutex::new(Vec::new());

unsafe extern "C" fn on_frame(_: *mut c_void, frame: *mut sys::video_data) {
    // SAFETY: libobs's frame for the call, BGRA 1920x1080.
    unsafe {
        let frame = &*frame;
        let stride = frame.linesize[0] as usize;
        let bytes = std::slice::from_raw_parts(frame.data[0], stride * 1080);
        let mut kept = Vec::with_capacity(1920 * 4 * 1080);
        for y in 0..1080 {
            kept.extend_from_slice(&bytes[y * stride..y * stride + 1920 * 4]);
        }
        *FRAME.lock().unwrap() = kept;
    }
}

fn pixel(x: usize, y: usize) -> [u8; 4] {
    let frame = FRAME.lock().unwrap();
    let at = (y * 1920 + x) * 4;
    [frame[at], frame[at + 1], frame[at + 2], frame[at + 3]]
}

fn main() {
    let obs = motor_obs::Obs::start("en-US").expect("libobs starts");
    obs.set_up().expect("libobs sets up");
    effect::register();
    let here = std::env::current_dir().unwrap();
    let invert = here.join("examples/invert.effect").display().to_string();
    assert!(effect::check("/nowhere.effect").is_err());
    let broken = std::env::temp_dir().join("remux-broken.effect");
    std::fs::write(&broken, "technique Draw { pass { nonsense } }").unwrap();
    let said = effect::check(&broken.display().to_string());
    println!("a broken file: {said:?}");
    assert!(said.is_err());
    effect::check(&invert).expect("the example builds");
    // SAFETY: an example driving libobs by hand.
    unsafe {
        let scene = sys::obs_scene_create(c"example".as_ptr());
        sys::obs_set_output_source(0, sys::obs_scene_get_source(scene));
        let settings = sys::obs_data_create();
        sys::obs_data_set_int(settings, c"color".as_ptr(), 0xFF0000FF); // ABGR: red
        sys::obs_data_set_int(settings, c"width".as_ptr(), 400);
        sys::obs_data_set_int(settings, c"height".as_ptr(), 400);
        let red = sys::obs_source_create(
            c"color_source_v3".as_ptr(),
            c"red".as_ptr(),
            settings,
            std::ptr::null_mut(),
        );
        sys::obs_data_release(settings);
        let red_item = sys::obs_scene_add(scene, red);
        let filter = effect::filter(&invert).expect("the filter");
        sys::obs_source_filter_add(red, filter);
        // A camera's circle: the mask the motor draws, stretched over a
        // 1280x720 source, cut from its middle square.
        let settings = sys::obs_data_create();
        sys::obs_data_set_int(settings, c"color".as_ptr(), 0xFF00FF00); // ABGR: green
        sys::obs_data_set_int(settings, c"width".as_ptr(), 1280);
        sys::obs_data_set_int(settings, c"height".as_ptr(), 720);
        let green = sys::obs_source_create(
            c"color_source_v3".as_ptr(),
            c"green".as_ptr(),
            settings,
            std::ptr::null_mut(),
        );
        sys::obs_data_release(settings);
        let region = motor_obs::place::Region {
            x: 280,
            y: 0,
            width: 720,
            height: 720,
        };
        let file = std::env::temp_dir().join("remux-circle.png");
        motor_obs::text::circle_mask(&file, (1280, 720), region).unwrap();
        let settings = sys::obs_data_create();
        sys::obs_data_set_string(
            settings,
            c"type".as_ptr(),
            c"mask_alpha_filter.effect".as_ptr(),
        );
        sys::obs_data_set_string(
            settings,
            c"image_path".as_ptr(),
            motor_obs::c(&file.display().to_string()).as_ptr(),
        );
        sys::obs_data_set_bool(settings, c"stretch".as_ptr(), true);
        let mask = sys::obs_source_create(
            c"mask_filter_v2".as_ptr(),
            c"shape".as_ptr(),
            settings,
            std::ptr::null_mut(),
        );
        sys::obs_data_release(settings);
        sys::obs_source_filter_add(green, mask);
        let green_item = sys::obs_scene_add(scene, green);
        sys::obs_sceneitem_set_pos(green_item, &motor_obs::vec2(500.0, 0.0));
        sys::obs_sceneitem_set_crop(
            green_item,
            &sys::obs_sceneitem_crop {
                left: 280,
                top: 0,
                right: 280,
                bottom: 0,
            },
        );
        let words = effect::element(600, 200, "Hello").expect("the element");
        let item = sys::obs_scene_add(scene, words);
        sys::obs_sceneitem_set_pos(item, &motor_obs::vec2(1000.0, 500.0));
        let scale = sys::video_scale_info {
            format: sys::video_format_VIDEO_FORMAT_BGRA,
            width: 1920,
            height: 1080,
            range: sys::video_range_type_VIDEO_RANGE_PARTIAL,
            colorspace: sys::video_colorspace_VIDEO_CS_709,
        };
        sys::obs_add_raw_video_callback(&scale, Some(on_frame), std::ptr::null_mut());
        std::thread::sleep(std::time::Duration::from_millis(1500));
        println!(
            "element {}x{}",
            sys::obs_source_get_width(words),
            sys::obs_source_get_height(words)
        );
        let inside = pixel(200, 200);
        println!("inverted red, BGRA: {inside:?}");
        let mut lit = 0;
        for y in 500..700 {
            for x in 1000..1600 {
                if pixel(x, y)[0] > 200 {
                    lit += 1;
                }
            }
        }
        println!("white pixels in the element's box: {lit}");
        // The cropped square sits at 500..1220 x 0..720: green in its
        // middle, nothing in its corners.
        let middle = pixel(860, 360);
        let corner = pixel(505, 5);
        let rim = pixel(860, 3);
        println!("circle: middle {middle:?}, corner {corner:?}, top of the rim {rim:?}");
        sys::obs_remove_raw_video_callback(Some(on_frame), std::ptr::null_mut());
        sys::obs_set_output_source(0, std::ptr::null_mut());
        sys::obs_source_filter_remove(red, filter);
        sys::obs_source_release(filter);
        sys::obs_sceneitem_remove(red_item);
        sys::obs_sceneitem_remove(green_item);
        sys::obs_source_filter_remove(green, mask);
        sys::obs_source_release(mask);
        sys::obs_source_release(green);
        sys::obs_sceneitem_remove(item);
        sys::obs_source_release(red);
        sys::obs_source_release(words);
        sys::obs_scene_release(scene);
        // libobs destroys on a thread of its own; the text inside the element
        // has to be gone before the plugin that drew it is.
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(
            inside[0] > 200 && inside[1] > 200 && inside[2] < 60,
            "red inverts to cyan"
        );
        assert!(lit > 500, "the words are drawn");
        assert!(
            middle[1] > 200 && middle[2] < 60,
            "the circle shows the camera"
        );
        assert!(corner[1] < 30, "the corner is cut away");
        assert!(rim[1] > 150, "the circle reaches the top of the square");
    }
    drop(obs);
    println!("ok");
}
