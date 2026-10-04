//! Remembering someone's desktop between visits: which apps were open, and where.
//!
//! The desktop asks the server for the saved layout when it starts, and opens those windows
//! where they were. After that it compares the windows on screen with what it last saved, and
//! once they have differed for [`SAVE_DELAY_MS`] it saves again. Not on every change: dragging
//! a window changes its position every frame, and one save after it settles says the same.
//!
//! These are the shell's own calls, not an app's, so they go through the transport under
//! [`SHELL_INSTANCE`], an instance id no window ever has.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use ccosel_proto::desktop::{DesktopLayout, MAX_WINDOWS, SavedWindow};
use ccosel_transport::EventSink;

/// How long the windows must have changed before they are saved.
pub const SAVE_DELAY_MS: f64 = 5000.0;

/// The transport instance the shell's own calls are made under. Windows are numbered from 2.
pub const SHELL_INSTANCE: u64 = 0;

/// Where replies to the shell's own calls land, until the next frame reads them.
#[derive(Clone, Default)]
pub struct ShellSink {
    pub queue: Rc<RefCell<VecDeque<Vec<u8>>>>,
}

impl EventSink for ShellSink {
    fn deliver(&self, batch: Vec<u8>) {
        self.queue.borrow_mut().push_back(batch);
    }
    fn alive(&self) -> bool {
        true
    }
}

/// One window on screen, as much of it as is saved.
pub struct OnScreen<'a> {
    pub app: &'a str,
    pub arg: Option<&'a str>,
    /// Where it is, or for a maximised window where it goes back to. `None` for a window that
    /// hasn't been drawn yet, which has no place to remember.
    pub rect: Option<egui::Rect>,
    pub minimized: bool,
    pub maximized: bool,
}

/// The layout to save for `windows`, back to front. At most [`MAX_WINDOWS`], the frontmost
/// ones: the server keeps no more.
pub fn layout<'a>(windows: impl IntoIterator<Item = OnScreen<'a>>) -> DesktopLayout {
    let mut saved: Vec<SavedWindow> = windows
        .into_iter()
        .filter_map(|w| {
            let r = w.rect?;
            Some(SavedWindow {
                app: w.app.to_owned(),
                arg: w.arg.map(str::to_owned),
                rect: [r.min.x, r.min.y, r.max.x, r.max.y],
                minimized: w.minimized,
                maximized: w.maximized,
            })
        })
        .collect();
    let extra = saved.len().saturating_sub(MAX_WINDOWS);
    saved.drain(..extra);
    DesktopLayout { windows: saved }
}

/// The saved rect as an egui one, if it is a real one: a damaged or hand-edited layout must
/// not put a window at NaN or make it inside out.
pub fn rect_of(saved: &SavedWindow) -> Option<egui::Rect> {
    let [l, t, r, b] = saved.rect;
    let rect = egui::Rect::from_min_max(egui::pos2(l, t), egui::pos2(r, b));
    (rect.is_finite() && rect.is_positive()).then_some(rect)
}

/// What the desktop should do about saving this frame.
#[derive(Debug, PartialEq)]
pub enum Save {
    /// Nothing has changed since the last save.
    Nothing,
    /// Something has; save it if nothing else moves by then (a frame is needed at that time).
    At(f64),
    /// Save this now.
    Now(DesktopLayout),
}

/// Decides when to save: once the windows have differed from the last save for
/// [`SAVE_DELAY_MS`].
#[derive(Default)]
pub struct Autosave {
    /// What was last saved, or loaded. `None` until the saved layout has been loaded: nothing
    /// is saved before then, or a slow load would be overwritten with an empty desktop.
    saved: Option<DesktopLayout>,
    /// When the current difference was first seen.
    since: Option<f64>,
}

impl Autosave {
    /// The layout as loaded from the server, which counts as saved.
    pub fn loaded(&mut self, layout: DesktopLayout) {
        self.saved = Some(layout);
        self.since = None;
    }

    pub fn tick(&mut self, now_ms: f64, current: DesktopLayout) -> Save {
        let Some(saved) = &self.saved else {
            return Save::Nothing;
        };
        if *saved == current {
            self.since = None;
            return Save::Nothing;
        }
        let since = *self.since.get_or_insert(now_ms);
        if now_ms - since < SAVE_DELAY_MS {
            return Save::At(since + SAVE_DELAY_MS);
        }
        self.saved = Some(current.clone());
        self.since = None;
        Save::Now(current)
    }
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    fn rect(x: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, 40.0), egui::vec2(400.0, 300.0))
    }

    fn on_screen(app: &str, x: f32) -> OnScreen<'_> {
        OnScreen {
            app,
            arg: None,
            rect: Some(rect(x)),
            minimized: false,
            maximized: false,
        }
    }

    #[wasm_bindgen_test]
    fn the_layout_is_every_drawn_window_back_to_front() {
        let l = layout([
            on_screen("clock", 10.0),
            OnScreen {
                arg: Some("/a.png"),
                minimized: true,
                ..on_screen("viewer", 20.0)
            },
            OnScreen {
                rect: None,
                ..on_screen("docs", 0.0)
            },
        ]);
        assert_eq!(l.windows.len(), 2, "a window never drawn has no place yet");
        assert_eq!(l.windows[0].app, "clock");
        assert_eq!(l.windows[0].rect, [10.0, 40.0, 410.0, 340.0]);
        assert_eq!(l.windows[1].arg.as_deref(), Some("/a.png"));
        assert!(l.windows[1].minimized);
    }

    #[wasm_bindgen_test]
    fn only_the_frontmost_windows_are_kept_past_the_limit() {
        let names: Vec<String> = (0..MAX_WINDOWS + 3).map(|i| i.to_string()).collect();
        let l = layout(names.iter().map(|n| on_screen(n, 0.0)));
        assert_eq!(l.windows.len(), MAX_WINDOWS);
        assert_eq!(l.windows[0].app, "3");
        assert_eq!(l.windows.last().unwrap().app, (MAX_WINDOWS + 2).to_string());
    }

    #[wasm_bindgen_test]
    fn a_saved_rect_must_be_a_real_one() {
        let w = |r: [f32; 4]| SavedWindow {
            app: "clock".to_owned(),
            arg: None,
            rect: r,
            minimized: false,
            maximized: false,
        };
        assert_eq!(rect_of(&w([10.0, 40.0, 410.0, 340.0])), Some(rect(10.0)));
        assert_eq!(rect_of(&w([10.0, 40.0, 5.0, 340.0])), None, "inside out");
        assert_eq!(rect_of(&w([f32::NAN, 0.0, 1.0, 1.0])), None);
        assert_eq!(rect_of(&w([0.0, 0.0, f32::INFINITY, 1.0])), None);
    }

    fn one(x: f32) -> DesktopLayout {
        layout([on_screen("clock", x)])
    }

    #[wasm_bindgen_test]
    fn nothing_is_saved_before_the_saved_desktop_has_loaded() {
        let mut a = Autosave::default();
        assert_eq!(a.tick(0.0, one(1.0)), Save::Nothing);
        assert_eq!(a.tick(60_000.0, one(1.0)), Save::Nothing);
    }

    #[wasm_bindgen_test]
    fn a_change_is_saved_once_it_has_settled() {
        let mut a = Autosave::default();
        a.loaded(one(0.0));
        assert_eq!(a.tick(100.0, one(0.0)), Save::Nothing, "as loaded");

        // Dragging: it keeps changing, and the wait counts from the first change.
        assert_eq!(a.tick(1000.0, one(5.0)), Save::At(6000.0));
        assert_eq!(a.tick(3000.0, one(9.0)), Save::At(6000.0));
        assert_eq!(a.tick(6000.0, one(9.0)), Save::Now(one(9.0)));
        assert_eq!(a.tick(6100.0, one(9.0)), Save::Nothing, "saved once");
    }

    #[wasm_bindgen_test]
    fn a_change_undone_before_it_is_saved_is_not_saved() {
        let mut a = Autosave::default();
        a.loaded(one(0.0));
        assert_eq!(a.tick(0.0, one(5.0)), Save::At(5000.0));
        assert_eq!(a.tick(1000.0, one(0.0)), Save::Nothing);
        assert_eq!(a.tick(2000.0, one(5.0)), Save::At(7000.0), "a fresh wait");
    }
}
