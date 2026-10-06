//! The Chess app's method: ask GNU Chess, on the server, for its move.
//!
//! The request is the whole game so far, as the coordinate moves from the starting position,
//! and the level. So it names a position exactly, and the request cache works unchanged: the
//! same position is asked about once, and the next move simply makes a new request. The server
//! keeps nothing between calls: each one starts GNU Chess, sets up the position, and stops it
//! once it has answered.
//!
//! Every level thinks for at most [`MAX_THINK_SECONDS`], so this is an ordinary call rather
//! than a job.

use alloc::string::String;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Effect, Method, Query, Rpc};

/// The levels, from 1 (weakest) to this.
pub const MAX_LEVEL: u8 = 5;

/// The longest any level thinks about a move.
pub const MAX_THINK_SECONDS: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
pub struct EngineMoveReq<'a> {
    /// The game so far: coordinate moves (`e2e4 e7e5 g1f3`, `e7e8q` for a promotion),
    /// separated by spaces, from the usual starting position.
    #[serde(borrow)]
    pub moves: &'a str,
    /// 1 to [`MAX_LEVEL`].
    pub level: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineReply {
    /// Its move, in coordinate notation. The server has checked it is legal.
    Move(String),
    /// It gives up.
    Resign,
    /// It couldn't answer: it crashed, took too long, or said something that isn't a legal
    /// move. Why, for the app to show.
    Failed(String),
}

pub struct EngineMove;

impl Rpc for EngineMove {
    const METHOD: Method = Method::EngineMove;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    // The same position at the same level is the same question, so it is asked once.
    const EFFECT: Effect = Effect::Idempotent;
    // Thinking time, plus starting the engine, plus the network.
    const DEADLINE_MS: u32 = 10_000;
    type Req<'a> = EngineMoveReq<'a>;
    type Reply = EngineReply;
}

impl Query for EngineMove {}
