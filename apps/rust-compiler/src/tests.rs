use ccosel_proto::build::{BuiltBinary, CompileResult, CompileStatus};
use ccosel_proto::fs::{DirEntry, DirListing, EntryKind};
use ccosel_sdk::testing::{Harness, rpc_error};

use super::*;

fn listing(entries: Vec<DirEntry>) -> DirListing {
    DirListing {
        entries,
        truncated: false,
    }
}

fn dir(name: &str) -> DirEntry {
    DirEntry {
        name: name.to_owned(),
        kind: EntryKind::Dir,
        size: 0,
        mtime_s: 0,
    }
}

fn file(name: &str) -> DirEntry {
    DirEntry {
        name: name.to_owned(),
        kind: EntryKind::File,
        size: 100,
        mtime_s: 0,
    }
}

fn status_in_progress(units_done: u32, units_total: u32, current: &str) -> CompileStatus {
    CompileStatus {
        finished: false,
        units_done,
        units_total,
        current: current.to_owned(),
        elapsed_ms: 1000,
        result: None,
    }
}

fn status_succeeded(binaries: Vec<BuiltBinary>) -> CompileStatus {
    CompileStatus {
        finished: true,
        units_done: 10,
        units_total: 10,
        current: String::new(),
        elapsed_ms: 5000,
        result: Some(CompileResult {
            success: true,
            output: String::new(),
            output_truncated: false,
            binaries,
        }),
    }
}

fn status_failed(output: &str, truncated: bool) -> CompileStatus {
    CompileStatus {
        finished: true,
        units_done: 5,
        units_total: 10,
        current: String::new(),
        elapsed_ms: 3000,
        result: Some(CompileResult {
            success: false,
            output: output.to_owned(),
            output_truncated: truncated,
            binaries: Vec::new(),
        }),
    }
}

#[test]
fn shows_loading_while_listing_is_pending() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    assert!(h.has_label("Loading…"));
}

#[test]
fn shows_error_when_listing_fails() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.fail::<ListDir>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
}

#[test]
fn shows_directories_from_listing() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![dir("src"), dir("tests"), file("Cargo.toml")]));
    h.frame();
    assert!(h.has_button("src"));
    assert!(h.has_button("tests"));
}

#[test]
fn build_button_appears_when_cargo_toml_is_present() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![dir("src"), file("Cargo.toml")]));
    h.frame();
    assert!(h.has_button("\u{1f528} Build"));
}

#[test]
fn no_build_button_without_cargo_toml() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![dir("src")]));
    h.frame();
    assert!(!h.has_button("\u{1f528} Build"));
    assert!(h.has_label("No Cargo.toml here \u{2014} open a crate directory to build it."));
}

#[test]
fn build_triggers_compile_polling() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![file("Cargo.toml")]));
    h.frame();
    h.click("\u{1f528} Build");
    h.frame();
    // A compile call should have been issued
    assert!(
        h.outstanding::<Compile>() >= 1,
        "Build click should issue a Compile RPC"
    );
    // Answer the compile with an in-progress status
    h.reply::<Compile>(&status_in_progress(1, 5, "cc"));
    h.frame();
    // The app invalidates the cache so the next frame re-issues the poll
    h.frame();
    assert!(
        h.outstanding::<Compile>() >= 1,
        "Compile should be re-polled each frame while in progress"
    );
}

#[test]
fn build_issues_a_compile_request() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![file("Cargo.toml")]));
    h.frame();
    assert!(
        h.has_button("\u{1f528} Build"),
        "Build button should be visible"
    );
    h.click("\u{1f528} Build");
    h.frame();
    // After clicking Build, a Compile call should be in the outbox
    assert!(
        h.outstanding::<Compile>() >= 1,
        "Build click should issue a Compile RPC"
    );
}

#[test]
fn shows_success_with_binaries() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![file("Cargo.toml")]));
    h.frame();
    h.click("\u{1f528} Build");
    h.frame();
    h.reply::<Compile>(&status_succeeded(vec![BuiltBinary {
        name: "myapp".to_owned(),
        path: "/project/target/release/myapp".to_owned(),
        size: 2048,
    }]));
    h.frame();
    assert!(h.has_label("\u{2705} Build succeeded"));
    assert!(h.has_label("myapp"));
    assert!(h.has_label("2K"));
    assert!(h.has_label("/files/project/target/release/myapp"));
}

#[test]
fn shows_failure_with_output() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![file("Cargo.toml")]));
    h.frame();
    h.click("\u{1f528} Build");
    h.frame();
    h.reply::<Compile>(&status_failed(
        "error[E0425]: cannot find value `x` in this scope\n  --> src/main.rs:2:5",
        false,
    ));
    h.frame();
    assert!(h.has_label("\u{274c} Build failed"));
    assert!(h.has_label("error[E0425]: cannot find value `x` in this scope"));
}

#[test]
fn shows_truncated_output_notice() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![file("Cargo.toml")]));
    h.frame();
    h.click("\u{1f528} Build");
    h.frame();
    h.reply::<Compile>(&status_failed("error: something went wrong", true));
    h.frame();
    assert!(h.has_label("(earlier output trimmed)"));
}

#[test]
fn shows_error_when_compile_rpc_fails() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![file("Cargo.toml")]));
    h.frame();
    h.click("\u{1f528} Build");
    h.frame();
    h.fail::<Compile>(rpc_error::SERVER);
    h.frame();
    assert!(h.has_label("server error"));
}

#[test]
fn navigates_into_directory() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![dir("src")]));
    h.frame();
    h.click("src");
    h.frame();
    assert_eq!(h.app.path, "/src");
}

#[test]
fn go_up_navigates_to_parent() {
    let mut h = Harness::new(RustCompiler::default());
    h.app.path = "/a/b".to_owned();
    h.frame();
    h.reply::<ListDir>(&listing(vec![]));
    h.frame();
    h.click("\u{2b06} Up");
    h.frame();
    assert_eq!(h.app.path, "/a");
}

#[test]
fn go_up_at_root_stays_at_root() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![]));
    h.frame();
    h.click("\u{2b06} Up");
    h.frame();
    assert_eq!(h.app.path, "/");
}

#[test]
fn a_breadcrumb_jumps_straight_to_an_ancestor() {
    let mut h = Harness::new(RustCompiler::default());
    h.app.path = "/a/b/c".to_owned();
    h.frame();
    h.reply::<ListDir>(&listing(vec![]));
    h.frame();
    assert!(h.buttons().starts_with(&[
        "\u{2b06} Up".to_owned(),
        "\u{1f3e0}".to_owned(),
        "a".to_owned(),
        "b".to_owned(),
        "c".to_owned(),
    ]));

    // Two frames, as clicking any button does: one for the app to observe the click and update
    // `path`, one more for the resulting re-request to land on the wire.
    h.click("a");
    h.frame();
    h.frame();
    assert_eq!(h.app.path, "/a");

    h.reply::<ListDir>(&listing(vec![]));
    h.frame();
    h.click("\u{1f3e0}");
    h.frame();
    h.frame();
    assert_eq!(h.app.path, "/");
}

#[test]
fn crumbs_run_from_the_root_down() {
    let app = RustCompiler::default();
    assert_eq!(app.crumbs(), [("\u{1f3e0}".to_owned(), "/".to_owned())]);

    let app = RustCompiler {
        path: "/a/b".to_owned(),
        ..Default::default()
    };
    assert_eq!(
        app.crumbs(),
        [
            ("\u{1f3e0}".to_owned(), "/".to_owned()),
            ("a".to_owned(), "/a".to_owned()),
            ("b".to_owned(), "/a/b".to_owned()),
        ]
    );
}

#[test]
fn itoa_handles_zero_and_common_sizes() {
    assert_eq!(itoa(0), "0");
    assert_eq!(itoa(42), "42");
    assert_eq!(itoa(1023), "1023");
}

#[test]
fn human_size_uses_appropriate_units() {
    assert_eq!(human_size(0), "0B");
    assert_eq!(human_size(512), "512B");
    assert_eq!(human_size(1024), "1K");
    assert_eq!(human_size(1048576), "1M");
}

#[test]
fn progress_fraction_without_total_is_indeterminate() {
    let s = status_in_progress(5, 0, "");
    assert!(s.fraction() < 0.0, "should be negative when no total");
}

#[test]
fn progress_fraction_capped_below_one() {
    let s = CompileStatus {
        finished: false,
        units_done: 99,
        units_total: 100,
        current: String::new(),
        elapsed_ms: 0,
        result: None,
    };
    assert!(s.fraction() <= 0.99);
}

#[test]
fn finished_build_shows_full_progress() {
    let s = status_succeeded(vec![]);
    assert_eq!(s.fraction(), 1.0);
}
