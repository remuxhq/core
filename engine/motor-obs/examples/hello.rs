//! `cargo run --example hello`, from `engine/motor-obs`: starts libobs off
//! OBS.app, prints its version, stops it.

fn main() {
    let obs = motor_obs::Obs::start("en-US").expect("libobs starts");
    println!(
        "libobs {} started: {}",
        motor_obs::Obs::version(),
        motor_obs::Obs::initialized()
    );
    drop(obs);
    println!("stopped: initialized now {}", motor_obs::Obs::initialized());
}
