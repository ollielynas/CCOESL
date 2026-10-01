//! The File Browser is driven through the SDK's native harness: the test plays both the user
//! (clicking buttons by label) and the server (answering `ListDir`), so the whole loop the app
//! runs in production — ask, wait, render, navigate, re-ask — runs here without a browser.

use ccosel_proto::fs::{Access, AccessReply, DirEntry, DirListing, EntryKind, ListDir};
use ccosel_sdk::icons;
use ccosel_sdk::testing::{Harness, rpc_error};

use super::*;

fn entry(name: &str, kind: EntryKind, size: u64) -> DirEntry {
    DirEntry {
        name: name.to_owned(),
        kind,
        size,
        mtime_s: 0,
        writable: true,
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

/// Answers every `Access` call the app is waiting on: it may change what it asked about, and
/// is signed in as `user`.
fn answer_access(h: &mut Harness<FileBrowser>, write: bool, user: Option<&str>) {
    while h.outstanding::<Access>() > 0 {
        h.reply::<Access>(&AccessReply {
            read: true,
            write,
            user: user.map(str::to_owned),
        });
    }
}

/// A browser that has asked for `/` and been answered with `listing`, then drawn it. `/` is a
/// folder it may change, and nobody is signed in.
fn browsing(listing: DirListing) -> Harness<FileBrowser> {
    browsing_as(listing, true, None)
}

fn browsing_as(listing: DirListing, write: bool, user: Option<&str>) -> Harness<FileBrowser> {
    let mut h = Harness::new(FileBrowser::default());
    h.frame();
    answer_access(&mut h, write, user);
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

#[test]
fn the_upload_button_targets_the_folder_on_screen() {
    let mut h = browsing(sample());
    assert_eq!(h.upload_buttons(), vec!["/"]);
    press(&mut h, "docs");
    answer_access(&mut h, true, None);
    h.frame();
    assert_eq!(h.upload_buttons(), vec!["/docs"]);
}

#[test]
fn a_finished_upload_re_lists_the_folder_once() {
    let mut h = browsing(sample());
    h.finish_upload();
    h.frame(); // the app sees the new count and invalidates the listing
    h.frame(); // and re-asks
    assert_eq!(h.outstanding::<ListDir>(), 1);

    h.reply::<ListDir>(&listing(vec![entry("photos", EntryKind::Dir, 0)]));
    h.frame();
    assert!(h.has_button("photos"), "{:?}", h.buttons());

    // The same count on later frames is not another upload.
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ListDir>(), 0);

    h.finish_upload();
    h.frame();
    h.frame();
    assert_eq!(
        h.outstanding::<ListDir>(),
        1,
        "a second upload re-lists again"
    );
}

#[test]
fn every_file_gets_a_download_link_and_folders_do_not() {
    let h = browsing(sample());
    assert_eq!(
        h.open_urls(),
        vec![
            ("Download".to_owned(), "/files/notes.md".to_owned()),
            ("Download".to_owned(), "/files/link".to_owned()),
        ]
    );
}

#[test]
fn download_urls_are_percent_encoded_per_segment() {
    assert_eq!(download_url("/", "notes.md"), "/files/notes.md");
    assert_eq!(
        download_url("/Documents", "notes #1.md"),
        "/files/Documents/notes%20%231.md"
    );
    assert_eq!(
        download_url("/a b/c?d", "e&f=g%h"),
        "/files/a%20b/c%3Fd/e%26f%3Dg%25h"
    );
    assert_eq!(
        download_url("/café/", "résumé.pdf"),
        "/files/caf%C3%A9/r%C3%A9sum%C3%A9.pdf"
    );
}

#[test]
fn the_labels_use_the_sdk_icons() {
    assert_eq!(SHARED_TAB, format!("{}  Shared", icons::USERS));
    assert_eq!(MINE_TAB, format!("{}  My files", icons::HOUSE));
    assert_eq!(CAN_CHANGE, format!("{}  Can change", icons::PENCIL_SIMPLE));
    assert_eq!(READ_ONLY, format!("{}  Read-only", icons::LOCK_SIMPLE));
}

#[test]
fn a_signed_in_user_has_a_tab_for_their_own_files() {
    let mut h = browsing_as(sample(), true, Some("alice"));
    assert_eq!(
        h.selectables(),
        vec![(SHARED_TAB.to_owned(), true), (MINE_TAB.to_owned(), false)]
    );

    press(&mut h, MINE_TAB);
    assert_eq!(h.app.place, Place::Mine);
    assert_eq!(h.app.path, "/home/alice");
    assert_eq!(h.outstanding::<ListDir>(), 1, "it lists the private folder");
    assert_eq!(
        h.selectables(),
        vec![(SHARED_TAB.to_owned(), false), (MINE_TAB.to_owned(), true)]
    );

    // Up never leaves it: the top of "My files" is the private folder.
    press(&mut h, "⬆ Up");
    assert_eq!(h.app.path, "/home/alice");

    press(&mut h, SHARED_TAB);
    assert_eq!(h.app.place, Place::Shared);
    assert_eq!(h.app.path, "/");
}

#[test]
fn someone_not_signed_in_has_no_files_of_their_own() {
    let h = browsing(sample());
    assert_eq!(h.selectables(), vec![(SHARED_TAB.to_owned(), true)]);
    assert!(h.has_text("Sign in to have files of your own"));
}

#[test]
fn signing_out_while_in_my_files_goes_back_to_shared() {
    let mut h = Harness::new(FileBrowser::default());
    h.app.place = Place::Mine;
    h.app.path = "/home/alice/Work".to_owned();
    h.frame();
    answer_access(&mut h, true, None);
    h.frame();
    assert_eq!(h.app.place, Place::Shared);
    assert_eq!(h.app.path, "/");
}

#[test]
fn my_files_breadcrumbs_start_at_the_private_folder() {
    let mut app = FileBrowser {
        place: Place::Mine,
        path: "/home/alice/Work/2026".to_owned(),
        ..FileBrowser::default()
    };
    assert_eq!(
        app.crumbs(),
        [
            ("🏠".to_owned(), "/home/alice".to_owned()),
            ("Work".to_owned(), "/home/alice/Work".to_owned()),
            ("2026".to_owned(), "/home/alice/Work/2026".to_owned()),
        ]
    );
    app.go_up();
    app.go_up();
    assert_eq!(app.path, "/home/alice");
    app.go_up();
    assert_eq!(app.path, "/home/alice", "and no further");
}

#[test]
fn the_top_of_shared_leaves_out_everyones_home_folders() {
    let h = browsing_as(
        listing(vec![
            entry("Docs", EntryKind::Dir, 0),
            entry("home", EntryKind::Dir, 0),
        ]),
        true,
        Some("alice"),
    );
    assert!(h.has_button("Docs"));
    assert!(!h.has_button("home"), "that is what My files is for");
    assert!(h.has_label("1 of 1 items"));

    // A folder called home anywhere else is just a folder.
    let mut h = browsing(listing(vec![entry("home", EntryKind::Dir, 0)]));
    h.app.path = "/Projects".to_owned();
    h.frame();
    answer_access(&mut h, true, None);
    h.reply::<ListDir>(&listing(vec![entry("home", EntryKind::Dir, 0)]));
    h.frame();
    assert!(h.has_button("home"));
}

#[test]
fn every_entry_says_whether_it_can_be_changed() {
    let mut notes = entry("notes.md", EntryKind::File, 10);
    notes.writable = false;
    let h = browsing(listing(vec![entry("docs", EntryKind::Dir, 0), notes]));
    assert!(h.has_label("Permissions"));
    let labels = h.labels();
    let perms: Vec<_> = labels
        .iter()
        .filter(|l| *l == CAN_CHANGE || *l == READ_ONLY)
        .collect();
    assert_eq!(perms, [CAN_CHANGE, READ_ONLY]);
}

#[test]
fn a_read_only_folder_says_so_and_offers_no_upload() {
    let h = browsing_as(sample(), false, Some("alice"));
    assert!(h.upload_buttons().is_empty());
    assert!(h.has_label(&format!(
        "{READ_ONLY}: you can open and download files here, but not change them"
    )));
}

#[test]
fn a_folder_that_can_be_changed_says_so() {
    let h = browsing(sample());
    assert!(h.has_label(&format!("{CAN_CHANGE}: you can add and change files here")));
    assert_eq!(h.upload_buttons(), vec!["/"]);
}

#[test]
fn nothing_is_said_about_a_folder_until_the_server_answers() {
    let mut h = Harness::new(FileBrowser::default());
    h.frame();
    assert!(h.upload_buttons().is_empty());
    assert!(!h.labels().iter().any(|l| l.contains("you can")));
}
