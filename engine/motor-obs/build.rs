//! Links libobs wherever this OS keeps it. macOS: the frameworks inside
//! OBS.app (`OBS_APP` overrides `/Applications/OBS.app`). Linux: the
//! distribution's `libobs.so.0` in the multiarch folder (`OBS_APP` is the
//! prefix, `/usr`). The functions and types come from the `libobs` crate,
//! generated from OBS 32.1's headers.

fn main() {
    println!("cargo:rerun-if-env-changed=OBS_APP");
    if cfg!(target_os = "macos") {
        let app = std::env::var("OBS_APP").unwrap_or_else(|_| "/Applications/OBS.app".into());
        let frameworks = format!("{app}/Contents/Frameworks");
        println!("cargo:rustc-link-search=framework={frameworks}");
        println!("cargo:rustc-link-lib=framework=libobs");
        println!("cargo:rustc-link-arg=-Wl,-rpath,{frameworks}");
    } else {
        let prefix = std::env::var("OBS_APP").unwrap_or_else(|_| "/usr".into());
        let triplet = if cfg!(target_arch = "aarch64") {
            "aarch64-linux-gnu"
        } else {
            "x86_64-linux-gnu"
        };
        println!("cargo:rustc-link-search=native={prefix}/lib/{triplet}");
        // The package ships the versioned name and no `libobs.so` link.
        println!("cargo:rustc-link-lib=dylib:+verbatim=libobs.so.0");
    }
}
