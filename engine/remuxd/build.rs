//! With `--features obs`, the binary finds libobs where OBS.app keeps it
//! (`OBS_APP` overrides `/Applications/OBS.app`): a dependency's build
//! script cannot set a binary's rpath, so this one does.

fn main() {
    println!("cargo:rerun-if-env-changed=OBS_APP");
    if std::env::var_os("CARGO_FEATURE_OBS").is_some() && cfg!(target_os = "macos") {
        let app = std::env::var("OBS_APP").unwrap_or_else(|_| "/Applications/OBS.app".into());
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{app}/Contents/Frameworks");
        // An installed engine: `Frameworks` beside `bin/` is a link to the
        // OBS.app on that machine (install.sh makes it).
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path/../Frameworks");
    }
}
