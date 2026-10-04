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
        writable: true,
    }
}

fn file(name: &str) -> DirEntry {
    DirEntry {
        name: name.to_owned(),
        kind: EntryKind::File,
        size: 100,
        mtime_s: 0,
        writable: true,
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
    assert!(h.has_label(NOTHING_TO_BUILD));
}

/// Shows `entries` as the folder on screen.
fn showing(entries: Vec<DirEntry>) -> Harness<RustCompiler> {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(entries));
    h.frame();
    h
}

#[test]
fn says_which_kind_of_project_it_found() {
    let h = showing(vec![dir("src"), file("Cargo.toml")]);
    assert!(h.has_label("Found: Rust project (Cargo.toml)"));

    let h = showing(vec![file("Makefile"), file("main.c")]);
    assert!(h.has_label("Found: C/C++ project (Makefile)"));
    assert!(h.has_button("\u{1f528} Build"));

    let h = showing(vec![file("main.cpp"), file("util.h")]);
    assert!(h.has_label("Found: C/C++ sources"));
    assert!(h.has_button("\u{1f528} Build"));
}

#[test]
fn headers_alone_or_a_folder_named_like_a_source_are_not_a_project() {
    let h = showing(vec![file("util.h"), dir("main.c")]);
    assert!(!h.has_button("\u{1f528} Build"));
    assert!(h.has_label(NOTHING_TO_BUILD));
}

/// Presses Build on `entries` and answers the first poll with `status`.
fn building(entries: Vec<DirEntry>, status: CompileStatus) -> Harness<RustCompiler> {
    let mut h = showing(entries);
    h.click("\u{1f528} Build");
    h.frame();
    h.reply::<Compile>(&status);
    h.frame();
    h
}

#[test]
fn progress_counts_what_the_kind_of_build_is_made_of() {
    let h = building(vec![file("Cargo.toml")], status_in_progress(2, 5, "serde"));
    assert!(h.has_label("Compiling… 2/5 crates"), "{:?}", h.labels());

    let h = building(
        vec![file("a.c"), file("b.c")],
        status_in_progress(1, 3, "b.c"),
    );
    assert!(h.has_label("Compiling… 1/3 steps"), "{:?}", h.labels());
    assert!(h.has_label("b.c"));

    let h = building(
        vec![file("Makefile")],
        status_in_progress(4, 0, "gcc -c x.c"),
    );
    assert!(
        h.has_label("Compiling… 4 files compiled"),
        "{:?}",
        h.labels()
    );
}

#[test]
fn a_c_build_lists_its_program_for_download() {
    let h = building(
        vec![file("main.c")],
        status_succeeded(vec![BuiltBinary {
            name: "hello".to_owned(),
            path: "/hello/target/hello".to_owned(),
            size: 16_000,
        }]),
    );
    assert!(h.has_label("✅ Build succeeded"));
    assert!(h.has_label("hello"));
    assert_eq!(
        h.open_urls(),
        [(
            "Download".to_owned(),
            "/files/hello/target/hello".to_owned()
        )]
    );
}

#[test]
fn build_tooltips_say_what_runs() {
    assert!(build_tooltip(ProjectKind::Cargo).contains("cargo build"));
    assert!(build_tooltip(ProjectKind::Make).contains("make"));
    assert!(build_tooltip(ProjectKind::Sources).contains("gcc"));
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
    assert_eq!(
        h.open_urls(),
        vec![(
            "Download".to_owned(),
            "/files/project/target/release/myapp".to_owned()
        )]
    );
}

#[test]
fn download_urls_are_percent_encoded_per_segment() {
    assert_eq!(
        download_url("/p/target/release/app"),
        "/files/p/target/release/app"
    );
    assert_eq!(
        download_url("/my proj #2/target/release/a?b"),
        "/files/my%20proj%20%232/target/release/a%3Fb"
    );
    assert_eq!(download_url("/café/x"), "/files/caf%C3%A9/x");
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

/// A compiler whose upload of `hello` into scratch folder 42 has just finished, answered all
/// the way to the project's own listing.
fn uploaded_hello() -> Harness<RustCompiler> {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.reply::<ListDir>(&listing(vec![]));
    h.frame();
    assert!(h.has_project_upload());
    h.finish_project_upload(42);
    h.frame(); // sees the upload, switches to /.scratch/42
    h.frame(); // asks for its listing
    h.reply::<ListDir>(&listing(vec![dir("hello")]));
    h.frame(); // sees the single project, opens it
    h.frame(); // asks for the project's listing
    h.reply::<ListDir>(&listing(vec![file("Cargo.toml"), dir("src")]));
    h.frame();
    h
}

#[test]
fn has_an_upload_project_button() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    assert!(h.has_project_upload());
}

#[test]
fn a_finished_upload_opens_the_project_ready_to_build() {
    let h = uploaded_hello();
    assert_eq!(h.app.path, "/.scratch/42/hello");
    assert!(h.has_button("\u{1f528} Build"));
    assert!(h.has_label("Uploaded for this build only: deleted after an hour unused."));
}

#[test]
fn the_scratch_folder_shows_as_one_uploaded_crumb() {
    let mut h = uploaded_hello();
    assert!(h.has_button("⬆ Uploaded: hello"), "{:?}", h.buttons());
    assert!(!h.has_button(".scratch") && !h.has_button("42"));
    h.click("src");
    h.frame();
    assert_eq!(h.app.path, "/.scratch/42/hello/src");
    h.click("⬆ Uploaded: hello");
    h.frame();
    assert_eq!(h.app.path, "/.scratch/42/hello");
}

#[test]
fn going_up_out_of_an_uploaded_project_goes_home() {
    let mut h = uploaded_hello();
    h.click("⬆ Up");
    h.frame();
    assert_eq!(h.app.path, "/");
    // Not bounced back in on later frames (`/`'s listing is still cached from the start).
    h.frame();
    h.frame();
    assert_eq!(h.app.path, "/");
}

#[test]
fn up_from_deeper_inside_steps_one_level() {
    let mut h = uploaded_hello();
    h.click("src");
    h.frame();
    h.click("⬆ Up");
    h.frame();
    assert_eq!(h.app.path, "/.scratch/42/hello");
}

#[test]
fn the_same_upload_is_opened_once_not_every_frame() {
    let mut h = uploaded_hello();
    h.click("⬆ Up");
    h.frame();
    for _ in 0..3 {
        h.frame();
    }
    assert_eq!(
        h.app.path, "/",
        "the upload's count is unchanged, so no re-open"
    );
}

#[test]
fn a_new_upload_replaces_the_last_build() {
    let mut h = uploaded_hello();
    h.click("\u{1f528} Build");
    h.frame();
    assert!(h.app.building.is_some());
    h.finish_project_upload(43);
    h.frame();
    assert_eq!(h.app.path, "/.scratch/43");
    assert!(h.app.building.is_none());
}

#[test]
fn a_scratch_folder_with_more_than_one_thing_is_left_to_the_user() {
    let mut h = Harness::new(RustCompiler::default());
    h.frame();
    h.finish_project_upload(7);
    h.frame();
    h.frame();
    h.reply::<ListDir>(&listing(vec![dir("a"), dir("b")]));
    h.frame();
    h.frame();
    assert_eq!(h.app.path, "/.scratch/7");
}

#[test]
fn scratch_helpers() {
    assert_eq!(scratch_root("/.scratch/5"), Some(5));
    assert_eq!(scratch_root("/.scratch/5/"), Some(5));
    assert_eq!(scratch_root("/.scratch/5/p"), None);
    assert_eq!(scratch_root("/x"), None);
    assert_eq!(scratch_dir(), "/.scratch");
}

#[test]
fn an_uploaded_build_names_the_project_not_the_scratch_folder() {
    let mut h = uploaded_hello();
    h.click("\u{1f528} Build");
    h.frame();
    assert!(h.has_label("⬆ Uploaded: hello"), "{:?}", h.labels());
    assert!(!h.labels().iter().any(|l| l.contains(".scratch")));
    assert_eq!(display_path("/proj"), "/proj");
    assert_eq!(display_path("/.scratch/9/p/q"), "⬆ Uploaded: p/q");
}
