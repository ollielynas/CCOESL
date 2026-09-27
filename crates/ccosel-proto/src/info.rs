//! Server-info method.
//!
//! Which server a dashboard is talking to, and a snapshot of how that machine is doing right
//! now. A dashboard polls it and keeps the history itself, so the server stays stateless about
//! who is watching.

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
    /// How long this server process has been running.
    pub uptime_ms: u64,
    /// RPC calls answered since start, this one included. Its rate is the server's load.
    pub rpc_calls: u64,
    /// Logical CPUs on the host.
    pub cpus: u32,
    /// 1-minute load average ×1000, so it crosses the wire without a float. `None` where the
    /// OS doesn't expose one.
    pub load_milli: Option<u32>,
    /// Memory in use and in total, in KiB. `None` where the OS doesn't expose it.
    pub mem_used_kib: Option<u64>,
    pub mem_total_kib: Option<u64>,
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
