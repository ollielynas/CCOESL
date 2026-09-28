//! The app, driven the way a user and the server would.

use ccosel_proto::fs::{
    Access, AccessReply, CreateDir, DirEntry, DirListing, EntryKind, FileText, ListDir, ReadFile,
    Search, SearchHit, SearchReply, WriteFile,
};
use ccosel_sdk::TextStyle;
use ccosel_sdk::testing::{Harness, rpc_error};

use crate::{DOCS_ROOT, Docs, View};

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

    assert!(h.has_button("📁 Apps"));
    assert!(h.has_button("📄 Guide"));
    assert!(!h.buttons().iter().any(|b| b.contains("logo")));
    // Anonymous, and the folder is read-only: no private folder, nothing to create.
    assert!(h.has_text("Sign in for a private folder"));
    assert!(!h.has_button("🏠 My documents"));
    assert!(h.has_text("🔒 Read-only folder"));
    assert!(!h.has_button("📄 New document"));
}

#[test]
fn places_lead_to_the_shared_root_and_the_users_home() {
    let mut h = started(true);
    h.click("🏠 My documents");
    h.frame();
    assert_eq!(h.app.view, View::Browse("/home/alice".into()));

    h.click("🗂 Shared");
    h.frame();
    assert_eq!(h.app.view, View::Browse("/".into()));

    h.click("📘 Documentation");
    h.frame();
    assert_eq!(h.app.view, View::Browse(DOCS_ROOT.into()));

    // Back retraces the way.
    h.click("← Back");
    h.frame();
    assert_eq!(h.app.view, View::Browse("/".into()));
}

#[test]
fn folders_and_breadcrumbs_navigate() {
    let mut h = started(false);
    h.click("📁 Apps");
    h.frame();
    assert_eq!(h.app.view, View::Browse("/Docs/Apps".into()));
    h.frame();
    grant(&mut h, false, Some("alice"));
    h.reply::<ListDir>(&listing(&[]));
    h.frame();
    assert!(h.has_text("No documents here yet."));
    // The trail: root, then Docs, then this folder as plain text.
    assert!(h.has_button("/"));
    assert!(h.has_text("Apps"));
    h.click("Docs");
    h.frame();
    assert_eq!(h.app.view, View::Browse("/Docs".into()));
    h.click("/");
    h.frame();
    assert_eq!(h.app.view, View::Browse("/".into()));
}

#[test]
fn a_document_is_rendered_and_its_links_followed() {
    let mut h = started(false);
    open_doc(
        &mut h,
        "📄 Guide",
        "# Welcome\n\nRead about [Files](Apps/files.md), fetch [the logo](logo.png) or visit <https://example.com>.\n\n- one\n1. two\n- [x] done\n\n> note\n\n```\ncode\n\nmore\n```\n\n---\n\n| A | B |\n|---|---|\n| [c](c.md) | d |\n\n[top](#top)",
        false,
    );
    assert_eq!(h.app.view, View::Read("/Docs/Guide.md".into()));
    assert!(
        h.styled()
            .contains(&("Welcome".into(), TextStyle::heading(1)))
    );
    assert!(h.has_text("🔒 Read-only"));
    assert!(!h.has_button("✏ Edit"));
    assert!(h.has_text("    1. "));
    assert!(h.has_text("code"));
    assert!(h.has_text("top"), "an anchor-only link is shown as text");

    let urls = h.open_urls();
    assert!(urls.contains(&("⬇ Download".into(), "/files/Docs/Guide.md".into())));
    assert!(urls.contains(&("📎 the logo".into(), "/files/Docs/logo.png".into())));
    assert!(urls.contains(&(
        "🔗 https://example.com".into(),
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
    h.click("← Back");
    h.frame();
    h.frame(); // the README is still cached, so it draws at once
    h.click_link("c");
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/c.md".into()));

    h.click("📂 Folder");
    h.frame();
    assert_eq!(h.app.view, View::Browse("/Docs".into()));
}

#[test]
fn a_failed_read_can_be_retried() {
    let mut h = started(false);
    h.click("📄 Guide");
    h.frame();
    h.frame();
    h.fail::<ReadFile>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
    h.click("Retry");
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ReadFile>(), 1, "asked again");
}

#[test]
fn editing_saving_and_the_unsaved_marker() {
    let mut h = started(true);
    open_doc(&mut h, "📄 Guide", "# Old\n", true);
    h.click("✏ Edit");
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
    assert!(h.has_text("● unsaved"));
    assert!(h.has_text("New"));
    assert!(h.has_text("Body"));

    h.click("💾 Save");
    h.frame();
    assert_eq!(h.outstanding::<WriteFile>(), 1);
    h.frame();
    assert!(h.has_label("Saving…"));
    h.reply::<WriteFile>(&());
    h.frame();
    assert!(!h.app.dirty());
    assert_eq!(h.app.status.as_deref(), Some("Saved"));

    // Done goes back to reading, which re-reads the saved document.
    h.click("✔ Done");
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/Guide.md".into()));
    h.frame();
    assert_eq!(h.outstanding::<ReadFile>(), 1);
}

#[test]
fn a_failed_save_says_why_and_keeps_the_changes() {
    let mut h = started(true);
    open_doc(&mut h, "📄 Guide", "text", true);
    h.click("✏ Edit");
    h.frame();
    h.frame();
    h.type_text(1, "changed");
    h.frame();
    h.click("💾 Save");
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
    open_doc(&mut h, "📄 Guide", "x", true);
    h.click("✏ Edit");
    h.frame();
    h.frame();
    for button in ["H", "B", "I", "🔗", "•", "1.", "☐", "{}"] {
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
    h.click("Hide preview");
    h.frame();
    h.frame();
    assert!(!h.app.preview);
    assert!(!h.has_text("Preview"));
    h.click("Show preview");
    h.frame();
    assert!(h.app.preview);
}

#[test]
fn a_preview_link_is_followed_only_when_nothing_is_unsaved() {
    let mut h = started(true);
    open_doc(&mut h, "📄 Guide", "[next](next.md)", true);
    h.click("✏ Edit");
    h.frame();
    h.frame();
    h.click_link("next");
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/next.md".into()));

    // Back to the editor, with an edit that has not been saved.
    h.click("← Back");
    h.frame();
    h.reply::<ReadFile>(&file("[next](next.md)", true));
    h.frame();
    h.click("✏ Edit");
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
    assert!(h.has_button("📄 New document"));

    // A name is required.
    h.click("📄 New document");
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
    h.click("📄 New document");
    h.frame();
    assert_eq!(
        h.app.status.as_deref(),
        Some("“Guide.md” already exists here.")
    );
    assert_eq!(h.outstanding::<WriteFile>(), 0);

    h.type_text(1, "Plan");
    h.frame();
    h.click("📄 New document");
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
    h.click("📁 New folder");
    h.frame();
    h.fail::<CreateDir>(rpc_error::DENIED);
    h.frame();
    assert_eq!(
        h.app.status.as_deref(),
        Some("Could not create the folder: permission denied")
    );

    h.click("📁 New folder");
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
    h.click("📄 New document");
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
    h.click("🔍");
    h.frame();
    assert_eq!(h.app.view, View::Browse(DOCS_ROOT.into()));

    h.type_text(0, "clock");
    h.frame();
    h.click("🔍");
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
    h.click("📄 clock");
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/Apps/clock.md".into()));
}

#[test]
fn search_with_no_results_or_a_failure() {
    let mut h = started(false);
    h.type_text(0, "zzz");
    h.frame();
    h.click("🔍");
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
    h.click("🔍");
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
    h.click("Retry");
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
    open_doc(&mut h, "📄 Guide", "saved", true);
    h.click("✏ Edit");
    h.frame();
    h.frame();
    assert!(!h.has_button("🗑 Discard changes"));
    h.type_text(1, "edited");
    h.frame();
    h.frame();

    // Done, Back and the places all stay put, and say why.
    for button in ["✔ Done", "← Back", "🗂 Shared"] {
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
    h.click("🗑 Discard changes");
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/Guide.md".into()));
    assert!(h.app.status.is_none());
    h.frame();
    assert!(h.has_text("saved"));

    // Editing again starts from the saved text, not the discarded edit.
    h.click("✏ Edit");
    h.frame();
    h.frame();
    assert_eq!(h.app.editor.as_str(), "saved");
}

#[test]
fn the_sidebar_tree_opens_folders_and_marks_where_you_are() {
    let mut h = started(false);
    // The tree shows the Documentation section, sharing the main column's listing.
    assert!(
        h.has_text("📘 Documentation"),
        "the current place is text, not a button"
    );
    assert!(!h.has_button("📘 Documentation"));
    assert!(h.has_button("▸"));

    // Opening a folder in the tree lists it underneath, without leaving the page.
    h.click("▸");
    h.frame();
    assert!(h.app.expanded.contains("/Docs/Apps"));
    assert_eq!(h.app.view, View::Browse(DOCS_ROOT.into()));
    h.frame();
    assert!(h.has_text("  …"), "loading, indented under its folder");
    h.reply::<ListDir>(&listing(&[
        ("files.md", EntryKind::File),
        ("clock.md", EntryKind::File),
    ]));
    h.frame();
    assert!(h.has_button("▾"));

    // A document in the tree opens it, and is then shown in bold as where you are.
    h.click("📄 clock");
    h.frame();
    assert_eq!(h.app.view, View::Read("/Docs/Apps/clock.md".into()));
    h.frame();
    h.reply::<ReadFile>(&file("# Clock", false));
    h.frame();
    assert!(h.styled().contains(&("📄 clock".into(), TextStyle::STRONG)));
    assert!(!h.has_button("📄 clock"));

    // Closing the folder hides its documents.
    h.click("▾");
    h.frame();
    h.frame();
    assert!(!h.app.expanded.contains("/Docs/Apps"));
    assert!(!h.has_text("📄 clock"));
}

#[test]
fn the_places_switch_the_trees_section() {
    let mut h = started(true);
    h.click("🏠 My documents");
    h.frame();
    h.frame();
    assert!(h.has_text("🏠 My documents"), "now the current place");
    assert!(h.has_button("📘 Documentation"));

    assert_eq!(h.app.view, View::Browse("/home/alice".into()));
    h.click("📘 Documentation");
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
    open_doc(&mut h, "📄 Guide", "[Files](Apps/files.md)", false);
    assert!(!h.app.expanded.contains("/Docs/Apps"));
    h.click_link("Files");
    h.frame();
    assert!(h.app.expanded.contains("/Docs/Apps"));
    assert!(h.app.expanded.contains("/Docs"));
    // Back reveals too: the tree always shows where you are.
    h.app.expanded.clear();
    h.click("← Back");
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
    assert_eq!(h.buttons().iter().filter(|b| *b == "📄 README").count(), 1);
    assert_eq!(h.buttons().iter().filter(|b| *b == "📁 Apps").count(), 1);

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

    h.click("🗂 Shared");
    h.frame();
    h.frame();
    grant(&mut h, true, None);
    h.reply::<ListDir>(&listing(&[("README.md", EntryKind::File)]));
    h.frame();
    h.fail::<ReadFile>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
}
