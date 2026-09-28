//! The app, driven the way a user and the server would.

use ccosel_proto::fs::{
    Access, AccessReply, CreateDir, DirEntry, DirListing, EntryKind, FileText, ListDir, ReadFile,
    Search, SearchHit, SearchReply, WriteFile,
};
use ccosel_sdk::TextStyle;
use ccosel_sdk::testing::{Harness, rpc_error};

use ccosel_sdk::icons;

use crate::{DOCS_ROOT, Docs, View, label as l};

/// Whether the sidebar shows a row reading `text`.
fn row(h: &Harness<Docs>, text: &str) -> bool {
    h.selectables().iter().any(|(t, _)| t == text)
}

fn entry(name: &str, kind: EntryKind) -> DirEntry {
    DirEntry {
        name: name.to_owned(),
        kind,
        size: 10,
        mtime_s: 0,
    }
}

fn listing(entries: &[(&str, EntryKind)]) -> DirListing {
    DirListing {
        entries: entries.iter().map(|(n, k)| entry(n, *k)).collect(),
        truncated: false,
    }
}

fn access(write: bool, user: Option<&str>) -> AccessReply {
    AccessReply {
        read: true,
        write,
        user: user.map(str::to_owned),
    }
}

fn file(text: &str, writable: bool) -> FileText {
    FileText {
        text: text.to_owned(),
        writable,
    }
}

/// Answer every outstanding `Access` call.
fn grant(h: &mut Harness<Docs>, write: bool, user: Option<&str>) {
    while h.outstanding::<Access>() > 0 {
        h.reply::<Access>(&access(write, user));
    }
}

/// The app open at `/Docs`, signed in as alice, with the folder listed.
fn started(write: bool) -> Harness<Docs> {
    let mut h = Harness::new(Docs::default());
    h.frame();
    grant(&mut h, write, Some("alice"));
    h.reply::<ListDir>(&listing(&[
        ("Apps", EntryKind::Dir),
        ("Guide.md", EntryKind::File),
        ("logo.png", EntryKind::File),
    ]));
    h.frame();
    h
}

/// Open a document by clicking it in the listing, and answer its read.
fn open_doc(h: &mut Harness<Docs>, button: &str, text: &str, writable: bool) {
    h.click(button);
    h.frame(); // the app sees the click and changes view
    h.frame(); // and draws the new one, asking for the document
    h.reply::<ReadFile>(&file(text, writable));
    h.frame();
}

#[test]
fn starts_in_the_documentation_folder_showing_only_folders_and_documents() {
    let mut h = Harness::new(Docs::default());
    h.frame();
    assert_eq!(h.app.view, View::Browse(DOCS_ROOT.to_owned()));
    assert!(h.has_label("Loading…"));
    grant(&mut h, false, None);
    h.reply::<ListDir>(&listing(&[
        ("Apps", EntryKind::Dir),
        ("Guide.md", EntryKind::File),
        ("logo.png", EntryKind::File),
    ]));
    h.frame();

    assert!(row(&h, &l(icons::FOLDER, "Apps")));
    assert!(row(&h, &l(icons::FILE_TEXT, "Guide")));
    assert!(!h.selectables().iter().any(|(t, _)| t.contains("logo")));
    // Anonymous, and the folder is read-only: no private folder, nothing to create.
    assert!(h.has_text("Sign in for a private folder"));
    assert!(!h.has_button(&l(icons::HOUSE, "My documents")));
    assert!(h.has_text(&l(icons::LOCK_SIMPLE, "Read-only folder")));
    assert!(!h.has_button(&l(icons::FILE_PLUS, "New document")));
}

#[test]
fn places_lead_to_the_shared_root_and_the_users_home() {
    let mut h = started(true);
    h.click(&l(icons::HOUSE, "My documents"));
    h.frame();
    assert_eq!(h.app.view, View::Browse("/home/alice".into()));

    h.click(&l(icons::USERS, "Shared"));
    h.frame();
    assert_eq!(h.app.view, View::Browse("/".into()));

    h.click(&l(icons::BOOK_OPEN, "Documentation"));
    h.frame();
    assert_eq!(h.app.view, View::Browse(DOCS_ROOT.into()));

    // Back retraces the way.
    h.click(&l(icons::ARROW_LEFT, "Back"));
    h.frame();
    assert_eq!(h.app.view, View::Browse("/".into()));
}

#[test]
fn folders_and_breadcrumbs_navigate() {
    let mut h = started(false);
    h.click(&l(icons::FOLDER, "Apps"));
    h.frame();
    assert_eq!(h.app.view, View::Browse("/Docs/Apps".into()));
    h.frame();
    grant(&mut h, false, Some("alice"));
    h.reply::<ListDir>(&listing(&[]));
    h.frame();
    assert!(h.has_text("No documents here yet."));
    // The trail starts at the place, then this folder as plain text.
    let docs = l(icons::BOOK_OPEN, "Documentation");
    assert!(h.has_button(&docs), "the place is a crumb, not a bare /");
    assert!(h.styled().contains(&("Apps".into(), TextStyle::STRONG)));
    h.click(&docs);
    h.frame();
    assert_eq!(h.app.view, View::Browse("/Docs".into()));
    h.frame();
    assert!(
        !h.has_button(&docs),
        "at the top of a place, it is plain text"
    );
}

#[test]
fn a_document_is_rendered_and_its_links_followed() {
    let mut h = started(false);
    open_doc(
        &mut h,
        &l(icons::FILE_TEXT, "Guide"),
        "# Welcome\n\nRead about [Files](Apps/files.md), fetch [the logo](logo.png) or visit <https://example.com>.\n\n- one\n1. two\n- [x] done\n\n> note\n\n```\ncode\n\nmore\n```\n\n---\n\n| A | B |\n|---|---|\n| [c](c.md) | d |\n\n[top](#top)",
        false,
    );
    assert_eq!(h.app.view, View::Read("/Docs/Guide.md".into()));
    assert!(
        h.styled()
            .contains(&("Welcome".into(), TextStyle::heading(1)))
    );
    assert!(h.has_text(&l(icons::LOCK_SIMPLE, "Read-only")));
    assert!(!h.has_button(&l(icons::PENCIL_SIMPLE, "Edit")));
    assert!(h.has_text("    1. "));
    assert!(h.has_text("code"));
    assert!(h.has_text("top"), "an anchor-only link is shown as text");

    let urls = h.open_urls();
    assert!(urls.contains(&(
        l(icons::DOWNLOAD_SIMPLE, "Download"),
        "/files/Docs/Guide.md".into()
    )));
    assert!(urls.contains(&(
        l(icons::PAPERCLIP, "the logo"),
        "/files/Docs/logo.png".into()
    )));
    assert!(urls.contains(&(
        l(icons::LINK, "https://example.com"),
        "https://example.com".into()
    )));

    h.click_link("Files");
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/Apps/files.md".into()));
    h.frame();
    h.reply::<ReadFile>(&file("", false));
    h.frame();
    assert!(h.has_text("This document is empty."));

    // A link inside a table works too.
    h.click(&l(icons::ARROW_LEFT, "Back"));
    h.frame();
    h.frame(); // the README is still cached, so it draws at once
    h.click_link("c");
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/c.md".into()));

    h.click(&l(icons::BOOK_OPEN, "Documentation"));
    h.frame();
    assert_eq!(h.app.view, View::Browse("/Docs".into()));
}

#[test]
fn a_failed_read_can_be_retried() {
    let mut h = started(false);
    h.click(&l(icons::FILE_TEXT, "Guide"));
    h.frame();
    h.frame();
    h.fail::<ReadFile>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
    h.click(&l(icons::ARROW_CLOCKWISE, "Retry"));
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ReadFile>(), 1, "asked again");
}

#[test]
fn editing_saving_and_the_unsaved_marker() {
    let mut h = started(true);
    open_doc(&mut h, &l(icons::FILE_TEXT, "Guide"), "# Old\n", true);
    h.click(&l(icons::PENCIL_SIMPLE, "Edit"));
    h.frame();
    assert_eq!(h.app.view, View::Edit("/Docs/Guide.md".into()));
    // The editor opens on the cached text, without asking the server again.
    assert_eq!(h.outstanding::<ReadFile>(), 0);
    h.frame();
    assert_eq!(h.text_fields()[1], "# Old\n");
    assert!(!h.app.dirty());

    // Typing shows up in the live preview and marks the document unsaved.
    h.type_text(1, "# New\n\nBody");
    h.frame();
    // The marker is drawn above the editor, so it shows from the frame after.
    h.frame();
    assert!(h.app.dirty());
    assert!(h.has_text("• Unsaved changes"));
    assert!(h.has_text("New"));
    assert!(h.has_text("Body"));

    h.click(&l(icons::FLOPPY_DISK, "Save"));
    h.frame();
    assert_eq!(h.outstanding::<WriteFile>(), 1);
    h.frame();
    assert!(h.has_label("Saving…"));
    h.reply::<WriteFile>(&());
    h.frame();
    assert!(!h.app.dirty());
    assert_eq!(h.app.status.as_deref(), Some("Saved"));

    // Done goes back to reading, which re-reads the saved document.
    h.click(&l(icons::CHECK, "Done"));
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/Guide.md".into()));
    h.frame();
    assert_eq!(h.outstanding::<ReadFile>(), 1);
}

#[test]
fn a_failed_save_says_why_and_keeps_the_changes() {
    let mut h = started(true);
    open_doc(&mut h, &l(icons::FILE_TEXT, "Guide"), "text", true);
    h.click(&l(icons::PENCIL_SIMPLE, "Edit"));
    h.frame();
    h.frame();
    h.type_text(1, "changed");
    h.frame();
    h.click(&l(icons::FLOPPY_DISK, "Save"));
    h.frame();
    h.fail::<WriteFile>(rpc_error::DENIED);
    h.frame();
    assert_eq!(
        h.app.status.as_deref(),
        Some("Could not save: permission denied")
    );
    assert!(h.app.dirty());
    assert_eq!(h.app.editor.as_str(), "changed");
}

#[test]
fn formatting_buttons_add_markdown_and_preview_toggles() {
    let mut h = started(true);
    open_doc(&mut h, &l(icons::FILE_TEXT, "Guide"), "x", true);
    h.click(&l(icons::PENCIL_SIMPLE, "Edit"));
    h.frame();
    h.frame();
    for button in [
        icons::TEXT_H,
        icons::TEXT_B,
        icons::TEXT_ITALIC,
        icons::LINK,
        icons::LIST_BULLETS,
        icons::LIST_NUMBERS,
        icons::CHECK_SQUARE,
        icons::CODE_BLOCK,
    ] {
        h.click(button);
        h.frame();
    }
    assert_eq!(
        h.app.editor.as_str(),
        "x\n## Heading\n**bold***italic*[text](other-document.md)\n- item\n\n1. item\n\n- [ ] task\n\n```\ncode\n```\n"
    );
    h.frame();
    assert_eq!(
        h.text_fields()[1],
        h.app.editor.as_str(),
        "the shell got it too"
    );

    assert!(h.has_text("Preview"));
    h.click(&l(icons::EYE_SLASH, "Hide preview"));
    h.frame();
    h.frame();
    assert!(!h.app.preview);
    assert!(!h.has_text("Preview"));
    h.click(&l(icons::EYE, "Show preview"));
    h.frame();
    assert!(h.app.preview);
}

#[test]
fn a_preview_link_is_followed_only_when_nothing_is_unsaved() {
    let mut h = started(true);
    open_doc(
        &mut h,
        &l(icons::FILE_TEXT, "Guide"),
        "[next](next.md)",
        true,
    );
    h.click(&l(icons::PENCIL_SIMPLE, "Edit"));
    h.frame();
    h.frame();
    h.click_link("next");
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/next.md".into()));

    // Back to the editor, with an edit that has not been saved.
    h.click(&l(icons::ARROW_LEFT, "Back"));
    h.frame();
    h.reply::<ReadFile>(&file("[next](next.md)", true));
    h.frame();
    h.click(&l(icons::PENCIL_SIMPLE, "Edit"));
    h.frame();
    h.frame();
    h.type_text(1, "[next](next.md) plus");
    h.frame();
    h.click_link("next");
    h.frame();
    assert_eq!(h.app.view, View::Edit("/Docs/Guide.md".into()));
}

#[test]
fn the_editor_reports_a_document_it_cannot_load() {
    let mut h = started(true);
    h.app.view = View::Edit("/Docs/gone.md".into());
    h.frame();
    assert!(h.has_label("Loading…"));
    h.fail::<ReadFile>(rpc_error::SERVER);
    h.frame();
    assert!(h.has_label("server error"));
    h.click("Close");
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/gone.md".into()));
}

#[test]
fn creating_a_document_opens_it_in_the_editor() {
    let mut h = started(true);
    assert!(h.has_button(&l(icons::FILE_PLUS, "New document")));

    // A name is required.
    h.click(&l(icons::FILE_PLUS, "New document"));
    h.frame();
    assert!(
        h.app
            .status
            .as_deref()
            .unwrap()
            .starts_with("Type a name first")
    );

    // One that is already there is refused before asking the server.
    h.type_text(1, "Guide");
    h.frame();
    h.click(&l(icons::FILE_PLUS, "New document"));
    h.frame();
    assert_eq!(
        h.app.status.as_deref(),
        Some("“Guide.md” already exists here.")
    );
    assert_eq!(h.outstanding::<WriteFile>(), 0);

    h.type_text(1, "Plan");
    h.frame();
    h.click(&l(icons::FILE_PLUS, "New document"));
    h.frame();
    assert_eq!(h.outstanding::<WriteFile>(), 1);
    h.reply::<WriteFile>(&());
    h.frame();
    assert_eq!(h.app.view, View::Edit("/Docs/Plan.md".into()));
    assert_eq!(h.app.new_name.as_str(), "", "the name field is cleared");
    // The new document is read like any other and starts with its title.
    h.reply::<ReadFile>(&file("# Plan\n\n", true));
    h.frame();
    assert_eq!(h.app.editor.as_str(), "# Plan\n\n");
}

#[test]
fn creating_a_folder_goes_into_it_and_failures_are_reported() {
    let mut h = started(true);
    h.type_text(1, "Team");
    h.frame();
    h.click(&l(icons::FOLDER_PLUS, "New folder"));
    h.frame();
    h.fail::<CreateDir>(rpc_error::DENIED);
    h.frame();
    assert_eq!(
        h.app.status.as_deref(),
        Some("Could not create the folder: permission denied")
    );

    h.click(&l(icons::FOLDER_PLUS, "New folder"));
    h.frame();
    h.reply::<CreateDir>(&());
    h.frame();
    assert_eq!(h.app.view, View::Browse("/Docs/Team".into()));

    // And a failed document create.
    grant(&mut h, true, Some("alice"));
    h.reply::<ListDir>(&listing(&[]));
    h.frame();
    h.type_text(1, "Notes");
    h.frame();
    h.click(&l(icons::FILE_PLUS, "New document"));
    h.frame();
    h.fail::<WriteFile>(rpc_error::SERVER);
    h.frame();
    assert_eq!(
        h.app.status.as_deref(),
        Some("Could not create the document: server error")
    );
}

#[test]
fn search_lists_matches_and_opens_them() {
    let mut h = started(false);
    // An empty query does nothing.
    h.click(icons::MAGNIFYING_GLASS);
    h.frame();
    assert_eq!(h.app.view, View::Browse(DOCS_ROOT.into()));

    h.type_text(0, "clock");
    h.frame();
    h.click(icons::MAGNIFYING_GLASS);
    h.frame();
    assert_eq!(h.app.view, View::Search("clock".into()));
    h.frame();
    assert!(h.has_label("Searching…"));
    h.reply::<Search>(&SearchReply {
        hits: vec![
            SearchHit {
                path: "/Docs/Apps/clock.md".into(),
                line: "The clock shows the time.".into(),
            },
            SearchHit {
                path: "/Docs/Apps/clock-2.md".into(),
                line: String::new(),
            },
        ],
        truncated: true,
    });
    h.frame();
    assert!(h.has_text("clock"));
    assert!(h.has_text("The clock shows the time."));
    assert!(h.has_text("/Docs/Apps"));
    assert!(h.has_text("(more matches than shown; try a longer search)"));
    h.click(&l(icons::FILE_TEXT, "clock"));
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/Apps/clock.md".into()));
}

#[test]
fn search_with_no_results_or_a_failure() {
    let mut h = started(false);
    h.type_text(0, "zzz");
    h.frame();
    h.click(icons::MAGNIFYING_GLASS);
    h.frame();
    h.frame();
    h.reply::<Search>(&SearchReply {
        hits: Vec::new(),
        truncated: false,
    });
    h.frame();
    assert!(h.has_text("No documents match."));

    h.type_text(0, "yyy");
    h.frame();
    h.click(icons::MAGNIFYING_GLASS);
    h.frame();
    h.frame();
    h.fail::<Search>(rpc_error::TIMEOUT);
    h.frame();
    assert!(h.has_label("timed out"));
}

#[test]
fn a_failed_listing_can_be_retried_and_truncation_is_shown() {
    let mut h = Harness::new(Docs::default());
    h.frame();
    grant(&mut h, false, None);
    h.fail::<ListDir>(rpc_error::TRANSPORT);
    h.frame();
    assert!(h.has_label("connection lost"));
    h.click(&l(icons::ARROW_CLOCKWISE, "Retry"));
    h.frame();
    h.frame();
    h.reply::<ListDir>(&DirListing {
        entries: vec![entry("a.md", EntryKind::File)],
        truncated: true,
    });
    h.frame();
    assert!(h.has_text("(more not shown)"), "in the sidebar");
}

#[test]
fn unsaved_changes_cannot_be_lost_by_leaving_the_editor() {
    let mut h = started(true);
    open_doc(&mut h, &l(icons::FILE_TEXT, "Guide"), "saved", true);
    h.click(&l(icons::PENCIL_SIMPLE, "Edit"));
    h.frame();
    h.frame();
    assert!(!h.has_button(&l(icons::TRASH, "Discard changes")));
    h.type_text(1, "edited");
    h.frame();
    h.frame();

    // Done, Back and the places all stay put, and say why.
    for button in [
        &l(icons::CHECK, "Done"),
        &l(icons::ARROW_LEFT, "Back"),
        &l(icons::USERS, "Shared"),
    ] {
        h.click(button);
        h.frame();
        assert_eq!(h.app.view, View::Edit("/Docs/Guide.md".into()), "{button}");
        assert_eq!(
            h.app.status.as_deref(),
            Some("You have unsaved changes. Save them, or discard them to leave.")
        );
    }
    assert_eq!(h.app.editor.as_str(), "edited");

    // Discard goes back to reading the saved document.
    h.click(&l(icons::TRASH, "Discard changes"));
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/Guide.md".into()));
    assert!(h.app.status.is_none());
    h.frame();
    assert!(h.has_text("saved"));

    // Editing again starts from the saved text, not the discarded edit.
    h.click(&l(icons::PENCIL_SIMPLE, "Edit"));
    h.frame();
    h.frame();
    assert_eq!(h.app.editor.as_str(), "saved");
}

#[test]
fn the_sidebar_tree_opens_folders_and_marks_where_you_are() {
    let mut h = started(false);
    let docs = l(icons::BOOK_OPEN, "Documentation");
    // The place you are in is the highlighted row; the tree below is that place's folder.
    assert!(h.selectables().contains(&(docs.clone(), true)));
    assert!(
        h.selectables()
            .contains(&(l(icons::USERS, "Shared"), false))
    );
    assert!(row(&h, &l(icons::FOLDER, "Apps")));

    // Clicking a folder opens it: the page shows it, and the tree lists it underneath.
    h.click(&l(icons::FOLDER, "Apps"));
    h.frame();
    assert_eq!(h.app.view, View::Browse("/Docs/Apps".into()));
    assert!(h.app.expanded.contains("/Docs/Apps"));
    h.frame();
    assert!(h.has_text("Loading…"));
    h.reply::<ListDir>(&listing(&[
        ("files.md", EntryKind::File),
        ("clock.md", EntryKind::File),
    ]));
    h.frame();
    assert!(
        h.selectables()
            .contains(&(l(icons::FOLDER_OPEN, "Apps"), true)),
        "an open folder shows it, and it is where you are"
    );

    // A document in the tree opens it, and becomes the highlighted row.
    h.click(&l(icons::FILE_TEXT, "clock"));
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/Apps/clock.md".into()));
    h.frame();
    h.reply::<ReadFile>(&file("# Clock", false));
    h.frame();
    assert!(
        h.selectables()
            .contains(&(l(icons::FILE_TEXT, "clock"), true))
    );
    assert!(
        h.selectables()
            .contains(&(l(icons::FOLDER_OPEN, "Apps"), false))
    );

    // Back in the folder, a second click on it closes it.
    h.click(&l(icons::FOLDER_OPEN, "Apps"));
    h.frame();
    h.frame();
    h.click(&l(icons::FOLDER_OPEN, "Apps"));
    h.frame();
    h.frame();
    assert!(!h.app.expanded.contains("/Docs/Apps"));
    assert!(!row(&h, &l(icons::FILE_TEXT, "clock")));
    assert!(row(&h, &l(icons::FOLDER, "Apps")));
}

#[test]
fn the_shared_tree_leaves_out_the_places_that_have_their_own_row() {
    let mut h = started(true);
    h.click(&l(icons::USERS, "Shared"));
    h.frame();
    h.frame();
    h.reply::<ListDir>(&listing(&[
        ("Docs", EntryKind::Dir),
        ("home", EntryKind::Dir),
        ("Projects", EntryKind::Dir),
    ]));
    h.frame();
    assert!(row(&h, &l(icons::FOLDER, "Projects")));
    assert!(!row(&h, &l(icons::FOLDER, "Docs")));
    assert!(!row(&h, &l(icons::FOLDER, "home")));
}

#[test]
fn an_empty_or_truncated_folder_says_so_in_the_tree() {
    let mut h = Harness::new(Docs::default());
    h.frame();
    grant(&mut h, false, None);
    h.reply::<ListDir>(&DirListing {
        entries: vec![entry("logo.png", EntryKind::File)],
        truncated: true,
    });
    h.frame();
    assert!(h.has_text("Empty"));
    assert!(h.has_text("(more not shown)"));
}

#[test]
fn the_places_switch_the_trees_section() {
    let mut h = started(true);
    h.click(&l(icons::HOUSE, "My documents"));
    h.frame();
    h.frame();
    assert!(
        h.selectables()
            .contains(&(l(icons::HOUSE, "My documents"), true)),
        "now the current place"
    );
    assert!(
        h.selectables()
            .contains(&(l(icons::BOOK_OPEN, "Documentation"), false))
    );

    assert_eq!(h.app.view, View::Browse("/home/alice".into()));
    h.click(&l(icons::BOOK_OPEN, "Documentation"));
    h.frame();
    assert_eq!(h.app.view, View::Browse(DOCS_ROOT.into()));
}

#[test]
fn the_section_is_the_place_a_path_is_in() {
    use crate::section_root;
    assert_eq!(section_root("/Docs/Apps/files.md", Some("alice")), "/Docs");
    assert_eq!(section_root("/Docs", None), "/Docs");
    assert_eq!(section_root("/Docsy/x.md", None), "/");
    assert_eq!(
        section_root("/home/alice/a.md", Some("alice")),
        "/home/alice"
    );
    assert_eq!(section_root("/home/alice/a.md", None), "/");
    assert_eq!(section_root("/Shared/a.md", Some("alice")), "/");
}

#[test]
fn following_a_link_opens_the_tree_down_to_the_document() {
    let mut h = started(false);
    open_doc(
        &mut h,
        &l(icons::FILE_TEXT, "Guide"),
        "[Files](Apps/files.md)",
        false,
    );
    assert!(!h.app.expanded.contains("/Docs/Apps"));
    h.click_link("Files");
    h.frame();
    assert!(h.app.expanded.contains("/Docs/Apps"));
    assert!(h.app.expanded.contains("/Docs"));
    // Back reveals too: the tree always shows where you are.
    h.app.expanded.clear();
    h.click(&l(icons::ARROW_LEFT, "Back"));
    h.frame();
    assert!(h.app.expanded.contains("/Docs"));
}

#[test]
fn a_folder_shows_its_readme_and_not_a_second_listing() {
    let mut h = Harness::new(Docs::default());
    h.frame();
    grant(&mut h, false, None);
    h.reply::<ListDir>(&listing(&[
        ("Apps", EntryKind::Dir),
        ("README.md", EntryKind::File),
    ]));
    h.frame();
    // Each entry once: in the sidebar tree, not again in the page.
    let readme = l(icons::FILE_TEXT, "README");
    let apps = l(icons::FOLDER, "Apps");
    assert_eq!(
        h.selectables().iter().filter(|(t, _)| *t == readme).count(),
        1
    );
    assert_eq!(
        h.selectables().iter().filter(|(t, _)| *t == apps).count(),
        1
    );
    assert!(!h.buttons().contains(&readme) && !h.buttons().contains(&apps));

    h.reply::<ReadFile>(&file("# Welcome\n\nSee [Files](Apps/files.md).", false));
    h.frame();
    assert!(h.has_text("Welcome"));
    h.click_link("Files");
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/Apps/files.md".into()));
}

#[test]
fn a_folder_without_a_readme_points_at_the_sidebar() {
    let mut h = Harness::new(Docs::default());
    h.frame();
    grant(&mut h, false, None);
    h.reply::<ListDir>(&listing(&[("a.md", EntryKind::File)]));
    h.frame();
    assert!(h.has_text("Choose a document in the sidebar."));

    h.click(&l(icons::USERS, "Shared"));
    h.frame();
    h.frame();
    grant(&mut h, true, None);
    h.reply::<ListDir>(&listing(&[("README.md", EntryKind::File)]));
    h.frame();
    h.fail::<ReadFile>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
}
