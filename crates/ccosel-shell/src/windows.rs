//! The desktop's window list: what a finished launch does to it, which window a drop lands on,
//! what a finished upload tells its window, and how a window's calls reach the transport and
//! are let go when it closes.
//!
//! Split out of `desktop.rs` so it can be tested. Everything here is generic over the app
//! instance and touches neither `fetch` nor the DOM, so the tests drive it with a fake app
//! rather than a real module in a browser.

use ccosel_host::AppInstance;
use ccosel_transport::{PendingKey, Transport};

use crate::app_window::AppWindow;
use crate::upload::{DropTarget, Upload};

/// A launch's outcome. Launching is async (fetch + compile); the render loop is not, so
/// outcomes wait in an inbox and the next frame picks them up with [`accept`].
///
/// `restored` is the window's place in a remembered desktop, back to front, for one being
/// reopened from it.
pub enum Launch<I: AppInstance> {
    Ready {
        window: Box<AppWindow<I>>,
        restored: Option<usize>,
    },
    Failed {
        name: String,
        error: String,
        restored: Option<usize>,
    },
}

/// The windows of a remembered desktop that are still opening, and the stacking place of each
/// one that has.
#[derive(Default)]
pub struct Restoring {
    pending: usize,
    order: Vec<(usize, u64)>,
}

impl Restoring {
    /// One more remembered window has started opening.
    pub fn expect(&mut self) {
        self.pending += 1;
    }

    pub fn in_progress(&self) -> bool {
        self.pending > 0
    }
}

/// The egui id of a window, from its instance: distinct per instance, so two windows of one
/// app never share egui state.
pub fn window_id(instance_id: u64) -> egui::Id {
    egui::Id::new(("app-window", instance_id))
}

/// Apply a finished launch: its window joins `windows`, or why it failed joins `errors`.
///
/// Once the last remembered window has opened, returns their instance ids back to front, so
/// the desktop can stack them as they were: they finish opening in whatever order their apps
/// load.
pub fn accept<I: AppInstance>(
    windows: &mut Vec<AppWindow<I>>,
    errors: &mut Vec<String>,
    restoring: &mut Restoring,
    launch: Launch<I>,
) -> Option<Vec<u64>> {
    let restored = match &launch {
        Launch::Ready { restored, .. } | Launch::Failed { restored, .. } => *restored,
    };
    match launch {
        Launch::Ready { window: mut w, .. } => {
            if let Some(i) = restored {
                restoring.order.push((i, w.instance_id));
            }
            // Two windows of the same app need distinguishable taskbar entries, or the
            // taskbar stops being a way to find a particular window.
            let n = windows.iter().filter(|x| x.app_id == w.app_id).count();
            if n > 0 {
                w.title = format!("{} {}", w.title, n + 1);
            }
            windows.push(*w);
        }
        Launch::Failed { name, error, .. } => errors.push(format!("{name}: {error}")),
    }
    if restored.is_none() || restoring.pending == 0 {
        return None;
    }
    restoring.pending -= 1;
    if restoring.pending > 0 {
        return None;
    }
    let mut order = std::mem::take(&mut restoring.order);
    order.sort_unstable();
    Some(order.into_iter().map(|(_, id)| id).collect())
}

/// The window a drop on egui layer `layer` landed on, and where it uploads to, or what to tell
/// the user instead. On an app's own page (`solo`) the app is the whole page, so anywhere
/// counts as on it. A minimised window is never a target.
pub fn drop_destination<I: AppInstance>(
    windows: &[AppWindow<I>],
    solo: bool,
    layer: Option<egui::Id>,
) -> Result<(u64, DropTarget), String> {
    let window = windows
        .iter()
        .find(|w| !w.placement.minimized && (solo || Some(window_id(w.instance_id)) == layer))
        .ok_or("upload: drop files onto an app window to upload them")?;
    window
        .drop_target()
        .map(|target| (window.instance_id, target))
        .ok_or_else(|| format!("upload: {} doesn't take uploads", window.title))
}

/// Tell the window an upload came from that it has finished, so its app can re-list the
/// folder or open the project. A window closed since is told nothing.
pub fn upload_finished<I: AppInstance>(windows: &mut [AppWindow<I>], done: &Upload) {
    let Some(w) = windows.iter_mut().find(|w| w.instance_id == done.instance) else {
        return;
    };
    // A project upload that failed before it had a folder tells the app nothing: there's
    // nothing to build, and the error list says why.
    match (done.project, done.scratch) {
        (true, Some(id)) => w.project_uploaded(done.widget, id),
        (true, None) => {}
        (false, _) => w.upload_finished(done.widget),
    }
}

/// Hand the calls `window`'s app made, and the ones it abandoned, during its last frame to the
/// transport. Replies go to the window's own sink.
pub fn forward_calls<I: AppInstance>(
    window: &mut AppWindow<I>,
    transport: &mut Transport,
    now_ms: f64,
) {
    let sink = window.sink();
    for call in window.take_outbox() {
        transport.enqueue(
            PendingKey {
                instance: window.instance_id,
                call: call.call_id,
            },
            call.method as u16,
            call.args,
            sink.clone(),
            now_ms,
        );
    }
    for call_id in window.take_cancels() {
        transport.cancel(PendingKey {
            instance: window.instance_id,
            call: call_id,
        });
    }
}

/// Drop the windows that closed this frame, and every call they were still waiting on.
///
/// Dropping the window drops its instance, which is the only way to reclaim a guest's memory:
/// wasm linear memory cannot shrink, so a live instance holds its high-water mark forever.
pub fn reap_closed<I: AppInstance>(windows: &mut Vec<AppWindow<I>>, transport: &mut Transport) {
    for window in windows.iter().filter(|w| !w.open) {
        transport.forget_instance(window.instance_id);
    }
    windows.retain(|w| w.open);
}

#[cfg(test)]
mod tests;
