//! Opening an app on something, and the widgets that hand things to other apps and the browser:
//! right-click menus, `open_app`, `copy_link`, `media` and the read-only `text_view`.

use ccosel_sdk::testing::Harness;
use ccosel_sdk::{App, MediaKind, Text, Ui, Vec2, url};

#[derive(Default)]
struct Opener {
    opened: Vec<String>,
    view: Text,
    copied: u32,
}

impl App for Opener {
    fn open(&mut self, arg: &str) {
        self.opened.push(arg.to_owned());
        self.view.set(arg);
    }

    fn update(&mut self, ui: &mut Ui<'_>) {
        ui.button("notes.md");
        ui.context_menu(|ui| {
            ui.open_app("Open with Viewer", "viewer", "/Docs/notes.md");
            if ui
                .copy_link("Share", &url::app_link("viewer", "/Docs/notes.md"))
                .clicked()
            {
                self.copied += 1;
            }
            ui.horizontal(|ui| {
                ui.open_url("Download", &url::file_url("/Docs/notes.md"));
            });
        });
        ui.button("Outside the menu");
        ui.media(
            "/files/a.mp4?inline=1",
            MediaKind::Video,
            Vec2::new(0.0, 0.0),
        );
        ui.text_view(&mut self.view);
    }
}

#[test]
fn a_launch_argument_reaches_open_before_the_first_frame() {
    let mut h = Harness::new(Opener::default());
    h.launch("/Docs/notes.md");
    assert_eq!(h.app.opened, ["/Docs/notes.md"]);
    h.frame();
    assert_eq!(h.text_views(), ["/Docs/notes.md"]);
}

#[test]
fn an_app_opened_without_an_argument_is_not_told_anything() {
    let mut h = Harness::new(Opener::default());
    h.frame();
    assert!(h.app.opened.is_empty());
    assert_eq!(h.text_views(), [""]);
}

#[test]
fn a_context_menu_holds_only_its_own_entries() {
    let mut h = Harness::new(Opener::default());
    h.frame();
    assert_eq!(
        h.context_menu_items(),
        ["Open with Viewer", "Share", "Download"],
        "nested scopes count, the button after the menu doesn't"
    );
    assert_eq!(
        h.open_apps(),
        [(
            "Open with Viewer".to_owned(),
            "viewer".to_owned(),
            "/Docs/notes.md".to_owned()
        )]
    );
    assert_eq!(
        h.copy_links(),
        [(
            "Share".to_owned(),
            "/app/viewer?open=%2FDocs%2Fnotes.md".to_owned()
        )]
    );
    assert_eq!(
        h.media(),
        [("/files/a.mp4?inline=1".to_owned(), MediaKind::Video)]
    );
}

#[test]
fn menu_entries_can_be_clicked() {
    let mut h = Harness::new(Opener::default());
    h.frame();
    h.click("Share");
    h.frame();
    assert_eq!(h.app.copied, 1);
}

#[test]
fn urls_are_encoded_for_where_they_go() {
    assert_eq!(
        url::file_url("/Docs/notes #1.md"),
        "/files/Docs/notes%20%231.md"
    );
    assert_eq!(url::file_url("a/b"), "/files/a/b");
    assert_eq!(
        url::inline_file_url("/v/clip.mov"),
        "/files/v/clip.mov?inline=1"
    );
    assert_eq!(url::encode_component("/a b/ü"), "%2Fa%20b%2F%C3%BC");
    assert_eq!(
        url::app_link("viewer", "/x?y=1&z"),
        "/app/viewer?open=%2Fx%3Fy%3D1%26z"
    );
}
