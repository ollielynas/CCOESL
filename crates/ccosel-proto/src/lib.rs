//! The client↔server protocol, shared verbatim by guests, the shell and the server.
//!
//! Defined in one place so the three cannot drift. `no_std` + alloc, because guests link it
//! and every byte here ships to every client over a bad LAN. postcard on the wire — never
//! JSON, and never `bincode` (which defaults to fixed-width integers).
//!
//! **This crate is deliberately not a dependency of `ccosel-abi`.** The guest↔shell ABI
//! carries RPC payloads as opaque bytes, so adding a method here never changes `ABI_VERSION`
//! and therefore never invalidates a module a client has already cached. That separation is
//! the single most valuable property of the envelope design; don't collapse it.

#![no_std]

extern crate alloc;

pub mod fs;

use serde::{Deserialize, Serialize};

/// Bumped when a request or reply type changes shape incompatibly. Negotiated separately from
/// `ccosel_abi::ABI_VERSION`, which governs the guest↔shell boundary.
pub const PROTO_VERSION: u32 = 1;

/// Numeric method ids are wire ABI: append, never renumber.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
pub enum Method {
    ListDir = 1,
    Stat = 2,
}

impl Method {
    pub const fn from_u16(v: u16) -> Option<Self> {
        match v {
            1 => Some(Self::ListDir),
            2 => Some(Self::Stat),
            _ => None,
        }
    }
}

/// How the shell may collapse duplicate in-flight calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coalesce {
    /// Never collapse.
    None,
    /// Identical `(method, args)` share one wire request and one reply, fanned out to every
    /// caller. Two Files windows showing `/shared` cost one request.
    ByArgs,
    /// A newer call supersedes an older one; the older is retired with `RPC_ERR{SHED}`.
    Latest,
}

/// Whether a call may be retried without the user noticing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Safe to auto-retry after a dropped link, and safe to cache.
    Idempotent,
    /// Has consequences. Never cached, never auto-retried.
    Effectful,
}

/// One request/reply pair.
///
/// The policy constants live here rather than on the wire so a guest cannot claim an effectful
/// call is idempotent and get it auto-retried. Policy belongs somewhere it is enforceable.
pub trait Rpc {
    const METHOD: Method;
    const COALESCE: Coalesce;
    const EFFECT: Effect;
    /// After this long the shell gives up and synthesises an error, so no app ever has to
    /// handle a "waiting forever" state.
    const DEADLINE_MS: u32;

    type Req<'a>: Serialize;
    type Reply: for<'de> Deserialize<'de> + 'static;
}

/// A read. Cacheable, idempotent, auto-retryable. Only these reach `RpcCtx::get`.
pub trait Query: Rpc {}

/// A write. Never cached, never auto-retried.
pub trait Command: Rpc {}

/// One call as it crosses the network.
///
/// `POST /rpc` carries a *batch* of these. Batching is what makes coalescing meaningful over
/// HTTP — N logical calls share one set of headers — and it keeps the envelope identical if
/// this ever moves onto a WebSocket.
#[derive(Debug, Serialize, Deserialize)]
pub struct WireRequest<'a> {
    /// Echoed back so replies can be matched without relying on order.
    pub seq: u32,
    pub method: u16,
    #[serde(borrow)]
    pub args: &'a [u8],
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WireReply<'a> {
    pub seq: u32,
    #[serde(borrow)]
    pub result: WireResult<'a>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum WireResult<'a> {
    Ok(#[serde(borrow)] &'a [u8]),
    Err {
        code: u32,
        #[serde(borrow)]
        detail: &'a str,
    },
}

/// Server-side failure codes. Distinct from `ccosel_abi::rpc_error`, which also covers
/// failures that never reach the server (timeout, cancellation, shedding).
pub mod server_error {
    /// Path outside the jail, or no capability for it.
    pub const DENIED: u32 = 1;
    pub const NOT_FOUND: u32 = 2;
    pub const NOT_A_DIRECTORY: u32 = 3;
    pub const IO: u32 = 4;
    pub const UNKNOWN_METHOD: u32 = 5;
    pub const MALFORMED: u32 = 6;
}
