//! The motor's libobs, set up as the engine sets it up, and the lists a face
//! needs, read the way the engine reads them: displays, windows,
//! applications, cameras, microphones.

use motor_obs::sources::list;

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
