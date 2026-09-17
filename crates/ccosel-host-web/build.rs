//! Builds the guest app so the browser-backend tests have a real module to load.
//!
//! `apps/` is a separate cargo workspace with its own profile and its own target dir, so this
//! is a plain sub-invocation rather than a dependency edge. It is the same work `xtask` will
//! eventually do as part of the asset pipeline.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let apps = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("apps");

    println!("cargo:rerun-if-changed={}", apps.join("file-browser/src").display());

    let artifact = apps.join("target/wasm32-unknown-unknown/release/file_browser.wasm");

    // Guard against recursion: the guest build must not re-enter this build script.
    if std::env::var("CCOSEL_BUILDING_GUEST").is_err() {
        let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .current_dir(&apps)
            .env("CCOSEL_BUILDING_GUEST", "1")
            .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => println!("cargo:warning=guest build failed; web backend tests will not run"),
        }
    }

    println!("cargo:rustc-env=CCOSEL_GUEST_WASM={}", artifact.display());
}
