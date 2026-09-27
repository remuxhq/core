//! The daemon with the libobs motor in it. `remuxd::boot` is the daemon;
//! this is the motor it gets.

fn main() {
    if let Some(other) = remuxd_domain::config::motor().filter(|m| m != "obs") {
        eprintln!("remuxd: this build has the obs motor only, not {other} (remux config)");
        std::process::exit(1);
    }
    // The libobs motor finds OBS through this; the config says where.
    if std::env::var_os("OBS_APP").is_none() {
        std::env::set_var("OBS_APP", remuxd_domain::config::obs_app());
    }
    remuxd::boot::boot(
        || {
            let (started, sources, pipeline) = motor_obs::Motor::start()?;
            // Kept for the life of the process: dropping it shuts libobs down.
            std::mem::forget(started);
            Ok(remuxd::boot::Motor {
                // The libobs this engine runs on, in the status: two distros
                // and two majors apart is the range that has to keep working.
                name: format!("obs {}", motor_obs::Obs::version()),
                sources,
                pipeline,
            })
        },
        remuxd::boot::wait,
    );
}
