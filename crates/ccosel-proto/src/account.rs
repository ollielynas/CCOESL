//! Who is signed in, and signing out.
//!
//! Both are queries rather than commands, and honestly so: asking twice is the same as asking
//! once, and so is signing out twice — ending an already-ended session changes nothing. That
//! makes them safe to retry and to cache, which is the whole contract of [`Query`].

use alloc::string::String;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Effect, Method, Query, Rpc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Whether the server has login turned on at all. Off, every visitor is let in and there
    /// is no one to sign out.
    pub login_enabled: bool,
    /// The signed-in account name. Always `Some` when `login_enabled`, since the server
    /// refuses `/rpc` to anyone without a session.
    pub name: Option<String>,
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
