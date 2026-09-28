//! Per-user desktop settings, and the list of installed apps the Settings app offers for the
//! dock.
//!
//! `GetSettings` and `SetSettings` are answered by the server, which keeps one file per user
//! in their home folder, so settings follow the account rather than the browser. The shell
//! also *watches* `SetSettings` go past on its way out and applies it the same frame, so a
//! change shows before the server has even replied.
//!
//! `ListApps` is different: the shell answers it itself, from its own catalog, and never puts
//! it on the wire. The server has no idea which apps the shell ships.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Command, Effect, Method, Query, Rpc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    /// Follow the browser's light/dark preference.
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundChoice {
    /// The photo when the connection can afford it, drawn shapes otherwise.
    #[default]
    Auto,
    /// Always the photo.
    Photo,
    /// Always the drawn shapes. Costs nothing on the wire.
    Plain,
    /// An image from the user's own files, at [`Settings::custom_image`].
    Custom,
}

/// Everything the Settings app can change. The default is how the desktop behaves with no
/// settings saved at all.
///
/// `#[serde(default)]` matters for the server's stored copy, which is JSON: a file written
/// before a field existed still loads, with that field at its default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub background: BackgroundChoice,
    /// A path in the server's files, such as `/home/alice/beach.jpg`. Only used when
    /// `background` is [`BackgroundChoice::Custom`].
    pub custom_image: String,
    /// Turn off animations, and tell apps to (`REDUCED_MOTION`).
    pub reduce_motion: bool,
    /// Never fetch the photo background, and tell apps to fetch less (`LOW_BANDWIDTH`).
    pub save_data: bool,
    /// App ids kept on the dock, in dock order.
    pub pinned: Vec<String>,
}

pub struct GetSettings;

impl Rpc for GetSettings {
    const METHOD: Method = Method::GetSettings;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 4_000;
    type Req<'a> = ();
    type Reply = Settings;
}

impl Query for GetSettings {}

/// Replace the signed-in user's settings with these. Refused with
/// [`crate::server_error::NO_USER`] when nobody is signed in, since there is no home folder to
/// keep them in; the shell then keeps them in the browser instead.
pub struct SetSettings;

impl Rpc for SetSettings {
    const METHOD: Method = Method::SetSettings;
    const COALESCE: Coalesce = Coalesce::None;
    const EFFECT: Effect = Effect::Effectful;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = Settings;
    type Reply = ();
}

impl Command for SetSettings {}

/// One installed app, as the dock shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    /// The glyph the dock draws on its badge.
    pub icon: String,
}

/// Every installed app. Answered by the shell, never the server.
pub struct ListApps;

impl Rpc for ListApps {
    const METHOD: Method = Method::ListApps;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 4_000;
    type Req<'a> = ();
    type Reply = Vec<AppInfo>;
}

impl Query for ListApps {}
