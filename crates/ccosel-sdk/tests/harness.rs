//! The harness is what every app's own tests stand on, so it is tested here against the
//! behaviours apps rely on: a click arrives one frame late, an RPC stays outstanding until the
//! test answers it, and a failed call renders without a decoder.

use ccosel_abi::event::rpc_error;
use ccosel_proto::fs::{DirEntry, DirListing, EntryKind, ListDir, ListDirReq};
use ccosel_sdk::testing::Harness;
use ccosel_sdk::{App, Poll, Ui};

#[derive(Default)]
struct Lister {
    clicks: u32,
}

impl App for Lister {
    fn update(&mut self, ui: &mut Ui<'_>) {
        match ui.rpc().get::<ListDir>(&ListDirReq { path: "/" }) {
            Poll::Pending => {
                ui.label("loading");
            }
            Poll::Failed(e) => {
                ui.label(e.message());
            }
            Poll::Ready(list) => {
                for entry in &list.entries {
                    ui.label(entry.name.as_str());
                }
            }
        }
        if ui.button("Go").clicked() {
            self.clicks += 1;
        }
    }
}

fn listing(names: &[&str]) -> DirListing {
    DirListing {
        entries: names
            .iter()
            .map(|n| DirEntry {
                name: (*n).to_owned(),
                kind: EntryKind::File,
                size: 1,
                mtime_s: 0,
                writable: true,
            })
            .collect(),
        truncated: false,
    }
}

#[test]
fn a_call_stays_outstanding_until_answered_then_renders() {
    let mut h = Harness::new(Lister::default());

    h.frame();
    assert!(h.has_label("loading"));
    assert_eq!(h.outstanding::<ListDir>(), 1);

    h.reply::<ListDir>(&listing(&["a.txt", "b.txt"]));
    assert_eq!(h.outstanding::<ListDir>(), 0);

    h.frame();
    assert_eq!(h.labels(), ["a.txt", "b.txt"]);
    assert_eq!(
        h.outstanding::<ListDir>(),
        0,
        "the answer must not be re-requested"
    );
}

#[test]
fn a_failed_call_renders_its_message() {
    let mut h = Harness::new(Lister::default());
    h.frame();
    h.fail::<ListDir>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
}

#[test]
fn a_click_is_observed_one_frame_later_and_does_not_repeat() {
    let mut h = Harness::new(Lister::default());
    h.frame();
    assert!(h.has_button("Go"));

    h.click("Go");
    assert_eq!(h.app.clicks, 0, "not before the next frame");
    h.frame();
    assert_eq!(h.app.clicks, 1);
    h.frame();
    assert_eq!(h.app.clicks, 1, "responses are not sticky");
}

#[test]
#[should_panic(expected = "no button labelled")]
fn clicking_a_button_that_was_not_drawn_names_what_was() {
    let mut h = Harness::new(Lister::default());
    h.frame();
    h.click("Nope");
}

#[test]
#[should_panic(expected = "no outstanding call")]
fn answering_a_call_that_was_never_made_panics() {
    let mut h = Harness::new(Lister::default());
    h.reply::<ListDir>(&listing(&[]));
}

/// An editor: loads text once, lets the user type, and saves with a command.
#[derive(Default)]
struct Editor {
    body: ccosel_sdk::Text,
    loaded: bool,
    saving: Option<ccosel_sdk::CallId>,
    saved: u32,
    save_failed: bool,
    link_clicks: u32,
}

impl App for Editor {
    fn update(&mut self, ui: &mut Ui<'_>) {
        use ccosel_proto::fs::{WriteFile, WriteFileReq};
        use ccosel_sdk::TextStyle;

        if !self.loaded {
            self.body.set("hello");
            self.loaded = true;
        }
        ui.wrapped(|ui| {
            ui.styled("Title", TextStyle::heading(1));
            if ui.styled("a link", TextStyle::LINK).clicked() {
                self.link_clicks += 1;
            }
        });
        ui.text_edit_multiline(&mut self.body);
        if ui.button("Save").clicked() {
            self.saving = Some(ui.rpc().send::<WriteFile>(&WriteFileReq {
                path: "/a.md",
                text: self.body.as_str(),
                create_only: false,
            }));
        }
        if let Some(id) = self.saving {
            match ui.rpc().outcome::<WriteFile>(id) {
                Poll::Pending => {}
                Poll::Ready(_) => {
                    self.saving = None;
                    self.saved += 1;
                }
                Poll::Failed(_) => {
                    self.saving = None;
                    self.save_failed = true;
                }
            }
        }
    }
}

#[test]
fn typed_text_reaches_the_app_and_a_set_reaches_the_shell() {
    let mut h = Harness::new(Editor::default());
    h.frame();
    assert_eq!(h.text_fields(), vec!["hello"]);

    h.type_text(0, "hello world");
    // Not yet: the app applies the edit when it next draws the field.
    assert_eq!(h.app.body.as_str(), "hello");
    h.frame();
    assert_eq!(h.app.body.as_str(), "hello world");
    assert_eq!(h.text_fields(), vec!["hello world"]);

    // A `set` in the same frame as a pending edit wins, as it does in the real shell.
    h.type_text(0, "typed");
    h.app.body.set("from the app");
    h.frame();
    assert_eq!(h.app.body.as_str(), "from the app");
    assert_eq!(h.text_fields(), vec!["from the app"]);
}

#[test]
fn a_command_goes_out_once_and_its_outcome_is_collected_once() {
    use ccosel_proto::fs::WriteFile;

    let mut h = Harness::new(Editor::default());
    h.frame();
    h.click("Save");
    h.frame();
    assert_eq!(h.outstanding::<WriteFile>(), 1);
    // Asking every frame while it is in flight sends nothing more.
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<WriteFile>(), 1);
    h.reply::<WriteFile>(&());
    h.frame();
    assert_eq!(h.app.saved, 1);
    assert!(h.app.saving.is_none());

    // Two saves are two calls: commands are never merged.
    h.click("Save");
    h.frame();
    h.fail::<WriteFile>(rpc_error::DENIED);
    h.frame();
    assert!(h.app.save_failed);
}

#[test]
fn an_unknown_or_collected_command_reads_as_cancelled() {
    use ccosel_proto::fs::{WriteFile, WriteFileReq};
    let rpc = ccosel_sdk::RpcCtx::new();
    let id = rpc.send::<WriteFile>(&WriteFileReq {
        path: "/x",
        text: "",
        create_only: true,
    });
    assert!(matches!(rpc.outcome::<WriteFile>(id), Poll::Pending));
    let other = ccosel_sdk::RpcCtx::new();
    match other.outcome::<WriteFile>(id) {
        Poll::Failed(e) => assert_eq!(e.code, rpc_error::CANCELLED),
        _ => panic!("an id this context never issued is not pending"),
    }
}

#[test]
fn styled_links_are_clickable_and_listed() {
    let mut h = Harness::new(Editor::default());
    h.frame();
    assert!(h.has_text("Title"));
    assert!(!h.has_text("nope"));
    assert_eq!(h.styled().len(), 2);
    h.click_link("a link");
    h.frame();
    assert_eq!(h.app.link_clicks, 1);
}

#[test]
#[should_panic(expected = "no link reading")]
fn clicking_a_missing_link_names_the_links() {
    let mut h = Harness::new(Editor::default());
    h.frame();
    h.click_link("Title");
}

#[test]
#[should_panic(expected = "no text field 3")]
fn typing_into_a_missing_field_panics() {
    let mut h = Harness::new(Editor::default());
    h.frame();
    h.type_text(3, "x");
}

#[derive(Default)]
struct Picker {
    picked: Option<&'static str>,
}

impl App for Picker {
    fn update(&mut self, ui: &mut Ui<'_>) {
        for name in ["a", "b"] {
            let selected = self.picked == Some(name);
            if ui.selectable(selected, name).clicked() {
                self.picked = Some(name);
            }
        }
        ui.indent(|ui| ui.label("nested"));
    }
}

#[test]
fn selectable_rows_are_listed_and_clicked_like_buttons() {
    let mut h = Harness::new(Picker::default());
    h.frame();
    assert_eq!(
        h.selectables(),
        vec![("a".into(), false), ("b".into(), false)]
    );
    h.click("b");
    h.frame();
    h.frame();
    assert_eq!(
        h.selectables(),
        vec![("a".into(), false), ("b".into(), true)]
    );
    assert!(h.has_label("nested"));
}

/// Tabs in a top-level `horizontal`, then a body below it: the shape that made one click reach
/// two widgets when an app's first scope shared the root id (#44).
#[derive(Default)]
struct Tabs {
    tab_clicks: [u32; 2],
    body_clicks: [u32; 2],
}

impl App for Tabs {
    fn update(&mut self, ui: &mut Ui<'_>) {
        ui.horizontal(|ui| {
            if ui.button("Tab A").clicked() {
                self.tab_clicks[0] += 1;
            }
            if ui.button("Tab B").clicked() {
                self.tab_clicks[1] += 1;
            }
        });
        if ui.button("Body A").clicked() {
            self.body_clicks[0] += 1;
        }
        if ui.button("Body B").clicked() {
            self.body_clicks[1] += 1;
        }
    }
}

#[test]
fn a_click_in_the_first_top_level_scope_reaches_exactly_one_widget() {
    for (label, tabs, body) in [
        ("Tab A", [1, 0], [0, 0]),
        ("Tab B", [0, 1], [0, 0]),
        ("Body A", [0, 0], [1, 0]),
        ("Body B", [0, 0], [0, 1]),
    ] {
        let mut h = Harness::new(Tabs::default());
        h.frame();
        h.click(label);
        h.frame();
        assert_eq!(h.app.tab_clicks, tabs, "clicked {label}");
        assert_eq!(h.app.body_clicks, body, "clicked {label}");
    }
}
