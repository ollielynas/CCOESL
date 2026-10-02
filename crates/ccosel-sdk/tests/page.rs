//! What the shell says about the page an app runs in: its origin, through `Ui::page`.

use ccosel_proto::fs::{DirListing, ListDir, ListDirReq};
use ccosel_sdk::testing::Harness;
use ccosel_sdk::{App, Poll, Ui};

/// Shows the WebDAV address it would give someone, the way the Account app will.
#[derive(Default)]
struct Linker;

impl App for Linker {
    fn update(&mut self, ui: &mut Ui<'_>) {
        match ui.page().origin() {
            Some(origin) => ui.label(&format!("{origin}/dav/")),
            None => ui.label("address unknown"),
        }
        // Deep inside nested scopes, the page is the same page.
        ui.horizontal(|ui| {
            ui.push_id("inner", |ui| {
                if let Some(origin) = ui.page().origin() {
                    ui.label(&format!("inner {origin}"));
                }
            });
        });
        if let Poll::Ready(list) = ui.rpc().get::<ListDir>(&ListDirReq { path: "/" }) {
            ui.label(&format!("{} entries", list.entries.len()));
        }
    }
}

#[test]
fn an_app_knows_nothing_about_the_page_until_the_shell_says() {
    let mut h = Harness::new(Linker);
    h.frame();
    assert!(h.has_label("address unknown"));
}

#[test]
fn the_origin_reaches_every_scope_of_the_app() {
    let mut h = Harness::new(Linker);
    h.set_origin("https://ccosel.example.com");
    h.frame();
    assert!(h.has_label("https://ccosel.example.com/dav/"));
    assert!(h.has_label("inner https://ccosel.example.com"));
    // It stays: it is about the page, not about one frame.
    h.frame();
    assert!(h.has_label("https://ccosel.example.com/dav/"));
}

#[test]
fn an_empty_origin_is_no_origin() {
    let mut h = Harness::new(Linker);
    h.set_origin("");
    h.frame();
    assert!(h.has_label("address unknown"));
}

#[test]
fn page_info_does_not_disturb_a_call_in_flight() {
    let mut h = Harness::new(Linker);
    h.frame();
    assert_eq!(h.outstanding::<ListDir>(), 1);
    // Arriving between a call and its reply, as it could from a shell.
    h.set_origin("http://192.168.1.20:8777");
    assert_eq!(h.outstanding::<ListDir>(), 1);
    h.reply::<ListDir>(&DirListing {
        entries: Vec::new(),
        truncated: false,
    });
    h.frame();
    assert!(h.has_label("0 entries"));
    assert!(h.has_label("http://192.168.1.20:8777/dav/"));
}
