//! With the desktop app built in (feature `app`), the bare `functor` binary
//! carries the app's Info.plist on macOS, as `functor-app` does: macOS shows
//! its microphone purpose when the composer's mic first records.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let app = std::env::var_os("CARGO_FEATURE_APP").is_some();
    if app && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let plist = dir.join("../app/functor-app/Info.plist");
        println!("cargo:rerun-if-changed={}", plist.display());
        println!(
            "cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
}
