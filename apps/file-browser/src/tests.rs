//! The File Browser is driven through the SDK's native harness: the test plays both the user
//! (clicking buttons by label) and the server (answering `ListDir`), so the whole loop the app
//! runs in production — ask, wait, render, navigate, re-ask — runs here without a browser.

use ccosel_proto::archive::{Archive, ArchiveStatus};
use ccosel_proto::fs::{Access, AccessReply, DirEntry, DirListing, EntryKind, ListDir, Remove};
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
fn every_file_has_a_right_click_menu_and_folders_one_to_compress_or_delete_them() {
    let h = browsing(sample());
    // The folder's menu, then the whole menu for `notes.md` and for `link`.
    assert_eq!(
        h.context_menu_items(),
        [
            COMPRESS,
            DELETE,
            OPEN_WITH_VIEWER,
            SHARE_WITH_VIEWER,
            DOWNLOAD,
            COMPRESS,
            GZIP,
            DELETE,
            OPEN_WITH_VIEWER,
            SHARE_WITH_VIEWER,
            DOWNLOAD,
            COMPRESS,
            GZIP,
            DELETE
        ]
    );
    assert_eq!(
        h.open_apps(),
        [
            (
                OPEN_WITH_VIEWER.to_owned(),
                "viewer".to_owned(),
                "/notes.md".to_owned()
            ),
            (
                OPEN_WITH_VIEWER.to_owned(),
                "viewer".to_owned(),
                "/link".to_owned()
            ),
        ]
    );
    assert_eq!(
        h.copy_links()[0],
        (
            SHARE_WITH_VIEWER.to_owned(),
            "/app/viewer?open=%2Fnotes.md".to_owned()
        )
    );
    // Download moved into the menu: there is no Download button on the row any more.
    assert_eq!(
        h.open_urls(),
        [
            (DOWNLOAD.to_owned(), "/files/notes.md".to_owned()),
            (DOWNLOAD.to_owned(), "/files/link".to_owned()),
        ]
    );
    assert!(!h.has_button("Download"));
}

#[test]
fn a_file_in_a_folder_is_opened_and_shared_by_its_whole_path() {
    let mut h = browsing(sample());
    press(&mut h, "docs");
    h.reply::<ListDir>(&listing(vec![entry("a b.txt", EntryKind::File, 3)]));
    h.frame();
    answer_access(&mut h, false, None);
    h.frame();
    assert_eq!(
        h.open_apps(),
        [(
            OPEN_WITH_VIEWER.to_owned(),
            "viewer".to_owned(),
            "/docs/a b.txt".to_owned()
        )]
    );
    assert_eq!(
        h.copy_links(),
        [(
            SHARE_WITH_VIEWER.to_owned(),
            "/app/viewer?open=%2Fdocs%2Fa%20b.txt".to_owned()
        )]
    );
    assert_eq!(
        h.open_urls(),
        [(DOWNLOAD.to_owned(), "/files/docs/a%20b.txt".to_owned())]
    );
}

#[test]
fn join_puts_one_slash_between() {
    assert_eq!(join("/", "a"), "/a");
    assert_eq!(join("/docs", "a"), "/docs/a");
    assert_eq!(join("/docs/", "a"), "/docs/a");
}

#[test]
fn the_labels_use_the_sdk_icons() {
    assert_eq!(SHARED_TAB, format!("{}  Shared", icons::USERS));
    assert_eq!(
        OPEN_WITH_VIEWER,
        format!("{}  Open with Viewer", icons::EYE)
    );
    assert_eq!(
        SHARE_WITH_VIEWER,
        format!("{}  Share with Viewer", icons::LINK)
    );
    assert_eq!(DOWNLOAD, format!("{}  Download", icons::DOWNLOAD_SIMPLE));
    assert_eq!(MINE_TAB, format!("{}  My files", icons::HOUSE));
    assert_eq!(CAN_CHANGE, format!("{}  Can change", icons::PENCIL_SIMPLE));
    assert_eq!(READ_ONLY, format!("{}  Read-only", icons::LOCK_SIMPLE));
    assert_eq!(DELETE, format!("{}  Delete", icons::TRASH));
    assert_eq!(CONFIRM_DELETE, format!("{}  Delete it", icons::TRASH));
    assert_eq!(
        COMPRESS,
        format!("{}  Compress (.tar.gz)", icons::ARROWS_IN_SIMPLE)
    );
    assert_eq!(GZIP, format!("{}  Gzip (.gz)", icons::FILE_ZIP));
    assert_eq!(
        EXTRACT,
        format!("{}  Extract here", icons::ARROWS_OUT_SIMPLE)
    );
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

/// Narrow the listing to `name`, so a click on a row's menu entry lands on that row.
fn only(h: &mut Harness<FileBrowser>, name: &str) {
    h.type_text(0, name);
    h.frame();
}

#[test]
fn the_upload_files_button_sits_beside_upload_folder_and_follows_the_folder() {
    let mut h = browsing(sample());
    assert_eq!(h.upload_buttons(), vec!["/"]);
    assert_eq!(h.file_upload_buttons(), vec!["/"]);
    press(&mut h, "docs");
    answer_access(&mut h, true, None);
    h.frame();
    assert_eq!(h.file_upload_buttons(), vec!["/docs"]);
}

#[test]
fn a_finished_file_upload_re_lists_the_folder() {
    let mut h = browsing(sample());
    h.finish_file_upload();
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ListDir>(), 1);
    h.reply::<ListDir>(&sample());
    h.frame();
    h.frame();
    assert_eq!(
        h.outstanding::<ListDir>(),
        0,
        "the same count is not a new upload"
    );
}

#[test]
fn a_read_only_folder_offers_neither_upload_button() {
    let h = browsing_as(sample(), false, None);
    assert!(h.upload_buttons().is_empty());
    assert!(h.file_upload_buttons().is_empty());
}

#[test]
fn nothing_read_only_can_be_deleted() {
    let mut ro = sample();
    for e in &mut ro.entries {
        e.writable = false;
    }
    let h = browsing_as(ro, false, None);
    let items = h.context_menu_items();
    assert!(!items.iter().any(|i| i == DELETE), "{items:?}");
    // The files keep the rest of their menus; the read-only folder has none at all.
    assert_eq!(items.iter().filter(|i| *i == DOWNLOAD).count(), 2);
}

#[test]
fn delete_asks_first_and_cancel_leaves_the_file_alone() {
    let mut h = browsing(sample());
    only(&mut h, "notes");
    press(&mut h, DELETE);
    assert!(
        h.has_label("Delete notes.md? This can't be undone."),
        "{:?}",
        h.labels()
    );
    assert_eq!(
        h.outstanding::<Remove>(),
        0,
        "nothing is deleted before the answer"
    );

    press(&mut h, CANCEL);
    assert!(!h.has_button(CONFIRM_DELETE));
    assert_eq!(h.outstanding::<Remove>(), 0);
}

#[test]
fn a_confirmed_delete_removes_it_and_re_lists_the_folder() {
    let mut h = browsing(sample());
    only(&mut h, "notes");
    press(&mut h, "notes.md");
    assert!(h.has_label("notes.md"), "selected");
    press(&mut h, DELETE);
    press(&mut h, CONFIRM_DELETE);
    assert_eq!(h.outstanding::<Remove>(), 1);
    assert!(h.has_label("Deleting notes.md…"));
    assert!(
        !h.has_button(CONFIRM_DELETE),
        "the question goes once answered"
    );

    h.reply::<Remove>(&());
    h.frame();
    h.frame();
    assert!(h.has_label("Deleted notes.md"), "{:?}", h.labels());
    assert_eq!(h.outstanding::<ListDir>(), 1, "the folder is listed again");
    assert!(
        h.app.selected.is_none(),
        "a deleted file is no longer selected"
    );
}

#[test]
fn deleting_a_folder_warns_about_what_is_in_it() {
    let mut h = browsing(sample());
    only(&mut h, "docs");
    press(&mut h, DELETE);
    assert!(h.has_label("Delete the folder docs and everything in it? This can't be undone."));
}

#[test]
fn a_refused_delete_says_why_and_the_entry_stays() {
    let mut h = browsing(sample());
    only(&mut h, "notes");
    press(&mut h, DELETE);
    press(&mut h, CONFIRM_DELETE);
    h.fail::<Remove>(rpc_error::DENIED);
    h.frame();
    h.frame();
    assert!(
        h.has_label("Couldn't delete notes.md: permission denied"),
        "{:?}",
        h.labels()
    );
}

#[test]
fn a_deletion_in_a_subfolder_names_the_whole_path_and_re_lists_that_folder() {
    let mut h = browsing(sample());
    press(&mut h, "docs");
    answer_access(&mut h, true, None);
    h.reply::<ListDir>(&listing(vec![entry("a.txt", EntryKind::File, 1)]));
    h.frame();
    press(&mut h, DELETE);
    press(&mut h, CONFIRM_DELETE);
    assert_eq!(
        h.app.deleting.as_ref().map(|(p, _)| p.as_str()),
        Some("/docs/a.txt")
    );
    h.reply::<Remove>(&());
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ListDir>(), 1);
}

#[test]
fn leaving_the_folder_drops_an_unanswered_question() {
    let mut h = browsing(sample());
    only(&mut h, "docs");
    press(&mut h, DELETE);
    assert!(h.has_button(CONFIRM_DELETE));
    // Opening the very folder it asked about: the question was about the one left behind.
    press(&mut h, "docs");
    assert!(!h.has_button(CONFIRM_DELETE));
    assert!(h.app.confirm.is_none());
}

fn running(done: u64, total: u64) -> ArchiveStatus {
    ArchiveStatus {
        finished: false,
        done_bytes: done,
        total_bytes: total,
        result: None,
    }
}

fn finished(result: ArchiveResult) -> ArchiveStatus {
    ArchiveStatus {
        finished: true,
        done_bytes: 0,
        total_bytes: 0,
        result: Some(result),
    }
}

/// A folder holding an archive, a plain file and a folder.
fn with_archive() -> Harness<FileBrowser> {
    browsing(listing(vec![
        entry("photos", EntryKind::Dir, 0),
        entry("notes.txt", EntryKind::File, 10),
        entry("old.tar.gz", EntryKind::File, 300),
    ]))
}

#[test]
fn archives_offer_extract_files_gzip_and_everything_compresses() {
    let h = with_archive();
    let items = h.context_menu_items();
    let count = |label: &str| items.iter().filter(|i| *i == label).count();
    assert_eq!(count(EXTRACT), 1, "only the archive extracts: {items:?}");
    assert_eq!(count(GZIP), 1, "only the plain file gzips: {items:?}");
    assert_eq!(count(COMPRESS), 3, "everything compresses: {items:?}");
}

#[test]
fn archive_actions_by_kind() {
    let actions = |name, dir| {
        archive_actions(name, dir)
            .into_iter()
            .map(|(_, a)| a)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        actions("a.tgz", false),
        [ArchiveAction::Extract, ArchiveAction::Compress]
    );
    assert_eq!(
        actions("a.txt", false),
        [ArchiveAction::Compress, ArchiveAction::Gzip]
    );
    assert_eq!(actions("a.tar", true), [ArchiveAction::Compress]);
}

#[test]
fn nothing_can_be_archived_in_a_read_only_folder() {
    let mut h = Harness::new(FileBrowser::default());
    h.frame();
    answer_access(&mut h, false, None);
    h.reply::<ListDir>(&listing(vec![
        entry("notes.txt", EntryKind::File, 10),
        entry("old.tar.gz", EntryKind::File, 300),
    ]));
    h.frame();
    let items = h.context_menu_items();
    assert!(
        !items
            .iter()
            .any(|i| i == COMPRESS || i == GZIP || i == EXTRACT),
        "{items:?}"
    );
}

#[test]
fn compressing_polls_shows_progress_then_says_what_it_made() {
    let mut h = with_archive();
    only(&mut h, "photos");
    press(&mut h, COMPRESS);
    assert_eq!(h.outstanding::<Archive>(), 1);
    assert!(h.has_label("Compressing photos…"), "{:?}", h.labels());
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        POLL_MS,
        "polls while it runs"
    );

    h.reply::<Archive>(&running(40, 100));
    h.frame();
    assert!(h.has_label("Compressing photos… 40%"), "{:?}", h.labels());
    h.frame();
    assert_eq!(h.outstanding::<Archive>(), 1, "asked again");

    h.reply::<Archive>(&finished(ArchiveResult::Made("/photos.tar.gz".to_owned())));
    h.frame();
    h.frame();
    assert!(
        h.has_label("Compressed photos into photos.tar.gz"),
        "{:?}",
        h.labels()
    );
    assert_eq!(h.outstanding::<ListDir>(), 1, "the folder is listed again");
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        ccosel_sdk::REPAINT_ON_INPUT_ONLY
    );
}

#[test]
fn extract_and_gzip_name_what_they_did() {
    let mut h = with_archive();
    only(&mut h, "old");
    press(&mut h, EXTRACT);
    assert!(h.has_label("Extracting old.tar.gz…"));
    h.reply::<Archive>(&finished(ArchiveResult::Made("/old".to_owned())));
    h.frame();
    h.frame();
    assert!(
        h.has_label("Extracted old.tar.gz into old"),
        "{:?}",
        h.labels()
    );

    let mut h = with_archive();
    only(&mut h, "notes");
    press(&mut h, GZIP);
    assert!(h.has_label("Gzipping notes.txt…"));
    h.reply::<Archive>(&running(0, 0));
    h.frame();
    assert!(
        h.has_label("Gzipping notes.txt…"),
        "no percentage before a size"
    );
}

#[test]
fn a_refused_extract_says_why() {
    let mut h = with_archive();
    only(&mut h, "old");
    press(&mut h, EXTRACT);
    h.reply::<Archive>(&finished(ArchiveResult::Failed(
        "it holds an absolute path (/etc/passwd)".to_owned(),
    )));
    h.frame();
    h.frame();
    assert!(
        h.has_label("Couldn't extract old.tar.gz: it holds an absolute path (/etc/passwd)"),
        "{:?}",
        h.labels()
    );
}

#[test]
fn a_failed_call_says_why_and_stops_polling() {
    let mut h = with_archive();
    only(&mut h, "notes");
    press(&mut h, COMPRESS);
    h.fail::<Archive>(rpc_error::DENIED);
    h.frame();
    h.frame();
    assert!(
        h.has_label("Couldn't compress notes.txt: permission denied"),
        "{:?}",
        h.labels()
    );
    assert!(h.app.archiving.is_none());
}

#[test]
fn one_job_at_a_time_and_it_outlives_leaving_the_folder() {
    let mut h = with_archive();
    only(&mut h, "photos");
    press(&mut h, COMPRESS);
    assert!(
        !h.context_menu_items().iter().any(|i| i == COMPRESS),
        "no second job while one runs"
    );
    // Leaving keeps the job and its status line.
    press(&mut h, "photos");
    assert!(h.app.archiving.is_some());
    assert!(h.has_label("Compressing photos…"));
}
