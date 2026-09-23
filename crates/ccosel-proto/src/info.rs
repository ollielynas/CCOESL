//! Server-info method.
//!
//! What a management dashboard needs first: which server it is talking to, and what it is
//! serving. Everything else the server manages (the shared file jail's contents) is already
//! reachable through [`crate::fs::ListDir`]; this method exists for the state that has no
//! other RPC to ride along on.

use alloc::string::String;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Effect, Method, Query, Rpc};

/// No fields: this call carries no arguments, only the reply does.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ServerInfoReq;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerInfoReply {
    /// [`crate::PROTO_VERSION`] as the server negotiates it, so a dashboard can flag a mismatch
    /// rather than guess at a decode failure.
    pub proto_version: u32,
    /// The jail root the server is serving files from, as an absolute path. Not a capability —
    /// it is already implied by every successful `ListDir` reply — but worth naming plainly on
    /// a dashboard instead of leaving an operator to infer it from `--root`.
    pub root: String,
}

pub struct ServerInfo;

impl Rpc for ServerInfo {
    const METHOD: Method = Method::ServerInfo;
    // There is only ever one distinct call (no args), so every caller shares it.
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 4_000;
    type Req<'a> = ServerInfoReq;
    type Reply = ServerInfoReply;
}

impl Query for ServerInfo {}
