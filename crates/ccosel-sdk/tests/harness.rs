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
