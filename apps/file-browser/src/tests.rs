//! The File Browser is driven through the SDK's native harness: the test plays both the user
//! (clicking buttons by label) and the server (answering `ListDir`), so the whole loop the app
//! runs in production — ask, wait, render, navigate, re-ask — runs here without a browser.

use ccosel_proto::fs::{DirEntry, DirListing, EntryKind, ListDir};
use ccosel_sdk::testing::{Harness, rpc_error};

use super::*;

fn entry(name: &str, kind: EntryKind, size: u64) -> DirEntry {
    DirEntry {
        name: name.to_owned(),
        kind,
        size,
        mtime_s: 0,
    }
}

fn listing(entries: Vec<DirEntry>) -> DirListing {
    DirListing {
        entries,
        truncated: false,
    }
}

/// A directory holding one of each kind the app draws differently.
fn sample() -> DirListing {
    listing(vec![
        entry("docs", EntryKind::Dir, 0),
        entry("notes.md", EntryKind::File, 2048),
        entry("link", EntryKind::Symlink, 0),
    ])
}

/// A browser that has asked for `/` and been answered with `listing`, then drawn it.
fn browsing(listing: DirListing) -> Harness<FileBrowser> {
    let mut h = Harness::new(FileBrowser::default());
    h.frame();
    h.reply::<ListDir>(&listing);
    h.frame();
    h
}

/// Clicks `button`, then runs the frame that observes the click and the one that redraws.
fn press(h: &mut Harness<FileBrowser>, button: &str) {
    h.click(button);
    h.frame();
    h.frame();
}

#[test]
fn asks_the_server_once_and_shows_loading_meanwhile() {
    let mut h = Harness::new(FileBrowser::default());
    h.frame();
    assert!(h.has_label("Loading…"));
    assert!(h.has_label("nothing selected"));
    assert_eq!(h.outstanding::<ListDir>(), 1);

    // Asking every frame is the idiom; it must not put a second call on the wire.
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ListDir>(), 1);
}

#[test]
fn draws_each_kind_of_entry() {
    let h = browsing(sample());
    let labels = h.labels();

    for icon in ["📁", "📄", "🔗"] {
        assert!(labels.iter().any(|l| l == icon), "no {icon} icon drawn");
    }
    assert!(h.has_label("Name") && h.has_label("Size"));
    assert!(h.has_button("docs") && h.has_button("notes.md") && h.has_button("link"));
    assert!(h.has_label("· 2K"), "a file shows its size");
    assert!(
        h.has_label("· 0B"),
        "so does a symlink, which is not a directory"
    );
    // Three entries but two sizes: the directory is the one that draws none.
    let sizes = labels.iter().filter(|l| l.starts_with("· ")).count();
    assert_eq!(sizes, 2);
    assert!(h.has_label("3 of 3 items"));
}

#[test]
fn an_empty_directory_says_so() {
    let h = browsing(listing(vec![]));
    assert!(h.has_label("(empty directory)"));
    assert!(!h.has_label("Name"), "no header over an empty table");
    assert!(h.has_label("nothing selected"));
}

#[test]
fn a_truncated_listing_says_so() {
    let mut l = sample();
    l.truncated = true;
    let h = browsing(l);
    assert!(h.has_label("(listing truncated)"));
}

#[test]
fn a_failed_listing_can_be_retried() {
    let mut h = Harness::new(FileBrowser::default());
    h.frame();
    h.fail::<ListDir>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
    assert!(h.has_button("Retry"));
    assert_eq!(h.outstanding::<ListDir>(), 0);

    press(&mut h, "Retry");
    assert_eq!(h.outstanding::<ListDir>(), 1, "Retry asks again");
    assert!(h.has_label("Loading…"));
}

#[test]
fn refresh_asks_again() {
    let mut h = browsing(sample());
    assert_eq!(h.outstanding::<ListDir>(), 0);
    press(&mut h, "⟳ Refresh");
    assert_eq!(h.outstanding::<ListDir>(), 1);
    assert!(h.has_label("Loading…"));
}

#[test]
fn clicking_a_file_selects_it() {
    let mut h = browsing(sample());
    press(&mut h, "notes.md");

    assert_eq!(h.app.selected.as_deref(), Some("notes.md"));
    assert!(h.has_button("▸ notes.md"), "the selected row is marked");
    assert!(h.has_label("Selected:") && h.has_label("notes.md"));
    assert!(
        !h.has_label("3 of 3 items"),
        "the selection replaces the count"
    );
}

#[test]
fn clicking_a_directory_enters_it_and_asks_for_its_listing() {
    let mut h = browsing(sample());
    press(&mut h, "docs");

    assert_eq!(h.app.path, "/docs");
    assert_eq!(h.outstanding::<ListDir>(), 1, "a new path is a new request");
    assert!(h.has_label("Loading…"));
    assert!(h.has_button("docs"), "the breadcrumb for the new directory");
}

#[test]
fn entering_a_directory_clears_the_selection() {
    let mut h = browsing(sample());
    press(&mut h, "notes.md");
    press(&mut h, "docs");
    assert_eq!(h.app.selected, None);
}

#[test]
fn up_goes_to_the_parent() {
    let mut h = Harness::new(FileBrowser::default());
    h.app.path = "/a/b".to_owned();
    h.frame();
    press(&mut h, "⬆ Up");
    assert_eq!(h.app.path, "/a");
}

#[test]
fn a_breadcrumb_jumps_straight_to_an_ancestor() {
    let mut h = Harness::new(FileBrowser::default());
    h.app.path = "/a/b/c".to_owned();
    h.frame();
    assert_eq!(h.buttons()[2..], ["🏠", "a", "b", "c"]);

    press(&mut h, "a");
    assert_eq!(h.app.path, "/a");
    press(&mut h, "🏠");
    assert_eq!(h.app.path, "/");
}

#[test]
fn the_filter_narrows_the_listing_case_insensitively() {
    let mut h = browsing(sample());
    h.app.filter.set("NOTE");
    h.frame();

    assert!(h.has_button("notes.md"));
    assert!(!h.has_button("docs") && !h.has_button("link"));
    assert!(h.has_label("1 of 3 items"));
}

#[test]
fn a_filter_matching_nothing_says_so() {
    let mut h = browsing(sample());
    h.app.filter.set("zzz");
    h.frame();
    assert!(h.has_label("(nothing matches the filter)"));
}

#[test]
fn go_up_stops_at_the_root_and_clears_the_selection() {
    let mut app = FileBrowser::default();
    app.go_up();
    assert_eq!(app.path, "/");

    app.path = "/a".to_owned();
    app.selected = Some("x".to_owned());
    app.go_up();
    assert_eq!(app.path, "/");
    assert_eq!(app.selected, None);

    app.path = "/a/b/c".to_owned();
    app.go_up();
    assert_eq!(app.path, "/a/b");
}

#[test]
fn enter_appends_with_exactly_one_separator() {
    let mut app = FileBrowser::default();
    app.enter("a");
    assert_eq!(app.path, "/a");
    app.enter("b");
    assert_eq!(app.path, "/a/b");
}

#[test]
fn crumbs_run_from_the_root_down() {
    let mut app = FileBrowser::default();
    assert_eq!(app.crumbs(), [("🏠".to_owned(), "/".to_owned())]);

    app.path = "/a/b".to_owned();
    assert_eq!(
        app.crumbs(),
        [
            ("🏠".to_owned(), "/".to_owned()),
            ("a".to_owned(), "/a".to_owned()),
            ("b".to_owned(), "/a/b".to_owned()),
        ]
    );
}

#[test]
fn sizes_use_the_largest_whole_unit() {
    assert_eq!(human_size(0), "0B");
    assert_eq!(human_size(1023), "1023B");
    assert_eq!(human_size(1024), "1K");
    assert_eq!(human_size(1024 * 1024 - 1), "1023K");
    assert_eq!(human_size(5 * 1024 * 1024), "5M");
}

#[test]
fn itoa_handles_zero_and_the_widest_value() {
    assert_eq!(itoa(0), "0");
    assert_eq!(itoa(u64::MAX), "18446744073709551615");
}
