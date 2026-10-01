//! Who is signed in, signing out, and app passwords.
//!
//! The first two are queries rather than commands, and honestly so: asking twice is the same as
//! asking once, and so is signing out twice — ending an already-ended session changes nothing.
//! That makes them safe to retry and to cache, which is the whole contract of [`Query`].
//! Making and revoking an app password are commands: making two is not the same as making one.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Command, Effect, Method, Query, Rpc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Whether the server has login turned on at all. Off, every visitor is let in and there
    /// is no one to sign out.
    pub login_enabled: bool,
    /// The signed-in account name. Always `Some` when `login_enabled`, since the server
    /// refuses `/rpc` to anyone without a session.
    pub name: Option<String>,
    /// The identity provider's page for managing this account (profile, password, sessions).
    /// `None` when login is off.
    pub account_url: Option<String>,
}

pub struct WhoAmI;

impl Rpc for WhoAmI {
    const METHOD: Method = Method::WhoAmI;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 4_000;
    type Req<'a> = ();
    type Reply = Account;
}

impl Query for WhoAmI {}

/// Ends this browser's session, here and at the identity provider. The next `/rpc` call is
/// then refused, which the shell answers by going back to the sign-in page.
pub struct SignOut;

impl Rpc for SignOut {
    const METHOD: Method = Method::SignOut;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = ();
    type Reply = ();
}

impl Query for SignOut {}

/// The most app passwords one account may have at a time.
pub const MAX_APP_PASSWORDS: usize = 20;

/// The longest name an app password may have, in bytes.
pub const MAX_APP_PASSWORD_NAME: usize = 64;

/// One of the caller's app passwords: a password for a single device or program (a WebDAV
/// client, say) that signs in as them without ever seeing their real password. The password
/// itself is shown once, when it is made, and never again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPasswordInfo {
    pub id: String,
    /// What the user called it, to tell their devices apart.
    pub name: String,
    /// Seconds since the Unix epoch.
    pub created_s: i64,
    /// When it last signed something in, to the nearest minute or so. `None` if never.
    pub last_used_s: Option<i64>,
}

/// The caller's app passwords, oldest first. Refused with `DENIED` when nobody is signed in.
pub struct ListAppPasswords;

impl Rpc for ListAppPasswords {
    const METHOD: Method = Method::ListAppPasswords;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 4_000;
    type Req<'a> = ();
    type Reply = Vec<AppPasswordInfo>;
}

impl Query for ListAppPasswords {}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateAppPasswordReq<'a> {
    #[serde(borrow)]
    pub name: &'a str,
}

/// A new app password, with the password itself: the only time it is ever sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAppPassword {
    pub info: AppPasswordInfo,
    pub password: String,
}

/// Makes an app password for the caller. Refused with `MALFORMED` for an empty or overlong
/// name, and `LIMIT` once they have [`MAX_APP_PASSWORDS`].
pub struct CreateAppPassword;

impl Rpc for CreateAppPassword {
    const METHOD: Method = Method::CreateAppPassword;
    const COALESCE: Coalesce = Coalesce::None;
    const EFFECT: Effect = Effect::Effectful;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = CreateAppPasswordReq<'a>;
    type Reply = NewAppPassword;
}

impl Command for CreateAppPassword {}

#[derive(Debug, Serialize, Deserialize)]
pub struct RevokeAppPasswordReq<'a> {
    #[serde(borrow)]
    pub id: &'a str,
}

/// Revokes one of the caller's app passwords; it stops working on the next request.
/// `NOT_FOUND` if they have none with that id.
pub struct RevokeAppPassword;

impl Rpc for RevokeAppPassword {
    const METHOD: Method = Method::RevokeAppPassword;
    const COALESCE: Coalesce = Coalesce::None;
    const EFFECT: Effect = Effect::Effectful;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = RevokeAppPasswordReq<'a>;
    type Reply = ();
}

impl Command for RevokeAppPassword {}
