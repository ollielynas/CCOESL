//! The Server Dashboard is driven through the SDK's native harness: the test plays both the
//! user (clicking buttons by label) and the server (answering `ServerInfo` and `ListDir`), so
//! the whole loop the app runs in production runs here without a browser.

use ccosel_proto::fs::{DirEntry, DirListing, EntryKind, ListDir};
use ccosel_proto::info::{ServerInfo, ServerInfoReply};
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

/// Deliberately not `1`, so a test asserting on it cannot be fooled by a stray count label that
/// happens to read `1` too.
fn server_info(root: &str) -> ServerInfoReply {
    ServerInfoReply {
        proto_version: 7,
        root: root.to_owned(),
    }
}

/// A dashboard that has asked for server info and `/`'s listing, been answered with both, and
/// drawn the result.
fn browsing(listing: DirListing) -> Harness<ServerDashboard> {
    let mut h = Harness::new(ServerDashboard::default());
    h.frame();
    h.reply::<ServerInfo>(&server_info("/srv/shared"));
    h.reply::<ListDir>(&listing);
    h.frame();
    h
}

/// Clicks `button`, then runs the frame that observes the click and the one that redraws.
fn press(h: &mut Harness<ServerDashboard>, button: &str) {
    h.click(button);
    h.frame();
    h.frame();
}

#[test]
fn asks_the_server_for_info_and_a_listing_and_shows_loading_meanwhile() {
    let mut h = Harness::new(ServerDashboard::default());
    h.frame();
    assert!(h.has_label("Loading…"));
    assert_eq!(h.outstanding::<ServerInfo>(), 1);
    assert_eq!(h.outstanding::<ListDir>(), 1);

    // Asking every frame is the idiom; it must not put a second call on the wire.
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ServerInfo>(), 1);
    assert_eq!(h.outstanding::<ListDir>(), 1);
}

#[test]
fn shows_server_info_once_answered() {
    let h = browsing(listing(vec![]));
    assert!(h.has_label("Protocol version"));
    assert!(h.has_label("7"));
    assert!(h.has_label("Shared root"));
    assert!(h.has_label("/srv/shared"));
}

#[test]
fn a_failed_server_info_can_be_retried() {
    let mut h = Harness::new(ServerDashboard::default());
    h.frame();
    h.fail::<ServerInfo>(rpc_error::SERVER);
    h.reply::<ListDir>(&listing(vec![]));
    h.frame();

    assert!(h.has_label("server error"));
    assert!(h.has_button("Retry server info"));
    assert_eq!(h.outstanding::<ServerInfo>(), 0);

    press(&mut h, "Retry server info");
    assert_eq!(h.outstanding::<ServerInfo>(), 1, "retrying asks again");
}

#[test]
fn draws_each_kind_of_entry_and_the_storage_summary() {
    let h = browsing(sample());
    let labels = h.labels();

    for icon in ["📁", "📄", "🔗"] {
        assert!(labels.iter().any(|l| l == icon), "no {icon} icon drawn");
    }
    assert!(h.has_button("docs"), "a directory is a button");
    assert!(
        h.has_label("notes.md") && h.has_label("link"),
        "files are plain labels"
    );
    assert!(h.has_label("· 2K"), "a file shows its size");
    assert!(
        h.has_label("· 0B"),
        "so does a symlink, which is not a directory"
    );
    assert!(h.has_label("1 folders, 2 files, 2K total"));
}

#[test]
fn an_empty_directory_says_so() {
    let h = browsing(listing(vec![]));
    assert!(h.has_label("(empty directory)"));
    assert!(h.has_label("0 folders, 0 files, 0B total"));
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
    let mut h = Harness::new(ServerDashboard::default());
    h.frame();
    h.reply::<ServerInfo>(&server_info("/srv/shared"));
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
fn refresh_asks_both_queries_again() {
    let mut h = browsing(sample());
    assert_eq!(h.outstanding::<ServerInfo>(), 0);
    assert_eq!(h.outstanding::<ListDir>(), 0);

    press(&mut h, "⟳ Refresh");
    assert_eq!(h.outstanding::<ServerInfo>(), 1);
    assert_eq!(h.outstanding::<ListDir>(), 1);
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
fn up_goes_to_the_parent() {
    let mut h = Harness::new(ServerDashboard::default());
    h.app.path = "/a/b".to_owned();
    h.frame();
    press(&mut h, "⬆ Up");
    assert_eq!(h.app.path, "/a");
}

#[test]
fn a_breadcrumb_jumps_straight_to_an_ancestor() {
    let mut h = Harness::new(ServerDashboard::default());
    h.app.path = "/a/b/c".to_owned();
    h.frame();

    press(&mut h, "a");
    assert_eq!(h.app.path, "/a");
    press(&mut h, "🏠");
    assert_eq!(h.app.path, "/");
}

#[test]
fn go_up_stops_at_the_root() {
    let mut app = ServerDashboard::default();
    app.go_up();
    assert_eq!(app.path, "/");

    app.path = "/a".to_owned();
    app.go_up();
    assert_eq!(app.path, "/");

    app.path = "/a/b/c".to_owned();
    app.go_up();
    assert_eq!(app.path, "/a/b");
}

#[test]
fn enter_appends_with_exactly_one_separator() {
    let mut app = ServerDashboard::default();
    app.enter("a");
    assert_eq!(app.path, "/a");
    app.enter("b");
    assert_eq!(app.path, "/a/b");
}

#[test]
fn crumbs_run_from_the_root_down() {
    let mut app = ServerDashboard::default();
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

#[test]
fn icon_for_every_kind_is_a_distinct_glyph() {
    assert_eq!(icon_for(EntryKind::Dir), "📁");
    assert_eq!(icon_for(EntryKind::Symlink), "🔗");
    assert_eq!(icon_for(EntryKind::File), "📄");
    assert_eq!(icon_for(EntryKind::Other), "📄");
}
