//! Remembering each person's desktop: which apps they had open, and where.
//!
//! The shell saves the layout every few seconds while it changes, and loads it when someone opens
//! the desktop again, from any computer. Only a signed-in person has a desktop to remember: with
//! login off everyone is the same anonymous visitor, so nothing is saved and nothing restored.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Effect, Method, Query, Rpc};

/// The most windows a saved desktop holds. More are left off: a layout is a convenience, and an
/// unbounded one would let one person fill the server's disk.
pub const MAX_WINDOWS: usize = 64;

/// One open window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    /// The app's id in the shell's catalog, such as `"file-browser"`.
    pub app: String,
    /// What it was opened on, such as the file a Viewer window shows.
    pub arg: Option<String>,
    /// Where it was, in points: `[left, top, right, bottom]`. For a maximised window, where it
    /// goes back to when un-maximised.
    pub rect: [f32; 4],
    pub minimized: bool,
    pub maximized: bool,
}

/// Every open window, back to front.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DesktopLayout {
    pub windows: Vec<SavedWindow>,
}

/// The caller's saved desktop. Empty for someone who has none yet, or isn't signed in.
pub struct LoadDesktop;

impl Rpc for LoadDesktop {
    const METHOD: Method = Method::LoadDesktop;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = ();
    type Reply = DesktopLayout;
}

impl Query for LoadDesktop {}

/// Replace the caller's saved desktop. Does nothing for someone who isn't signed in.
pub struct SaveDesktop;

impl Rpc for SaveDesktop {
    const METHOD: Method = Method::SaveDesktop;
    const COALESCE: Coalesce = Coalesce::None;
    const EFFECT: Effect = Effect::Effectful;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = DesktopLayout;
    type Reply = ();
}
