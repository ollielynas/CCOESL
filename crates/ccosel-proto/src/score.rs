//! The Music Score app's method: engrave LilyPond source into sheet music on the server.
//!
//! A job like `Compile`, polled the same way: the first `Engrave` for a source starts LilyPond,
//! every later one reports how far it has got. The job is keyed by a hash of the source rather
//! than a generation number, so engraving text that hasn't changed finds the finished job and
//! costs nothing.
//!
//! The pictures, PDF and MIDI it makes are not in the reply. The server serves them under
//! [`ENGRAVED`], at the URLs [`page_url`], [`pdf_url`] and [`midi_url`] build from the result's
//! `key`, so the shell fetches the pages as images and they never pass through the app.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Effect, Method, Query, Rpc};

/// Where engraved results are served: `/engraved/<key>/<file>`.
pub const ENGRAVED: &str = "/engraved";

#[derive(Debug, Serialize, Deserialize)]
pub struct EngraveReq<'a> {
    /// The whole `.ly` source. At most `fs::MAX_TEXT_BYTES`, the same as a file the app could
    /// open or save.
    #[serde(borrow)]
    pub source: &'a str,
}

/// Something LilyPond said about a line of the source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreMessage {
    /// 1-based. 0 when the message is about the score as a whole, not a line of it.
    pub line: u32,
    /// 1-based, or 0 when LilyPond didn't give one.
    pub column: u32,
    /// An error rather than a warning.
    pub error: bool,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngraveResult {
    /// LilyPond finished without errors and made at least one page.
    pub success: bool,
    /// Names this result in the URLs its files are served at.
    pub key: String,
    /// How many page pictures there are; see [`page_url`].
    pub pages: u32,
    pub pdf: bool,
    /// Only when the score asks for one, with `\midi`.
    pub midi: bool,
    /// Errors and warnings about the source, in the order LilyPond gave them.
    pub messages: Vec<ScoreMessage>,
    /// LilyPond's own log, tail-capped like the Compiler's: see `output_truncated`.
    pub output: String,
    pub output_truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngraveStatus {
    pub finished: bool,
    pub elapsed_ms: u64,
    /// Present exactly when `finished`.
    pub result: Option<EngraveResult>,
}

/// Page `page` (1-based) of an engraved result, as a PNG.
pub fn page_url(key: &str, page: u32) -> String {
    format!("{ENGRAVED}/{key}/page-{page}.png")
}

pub fn pdf_url(key: &str) -> String {
    format!("{ENGRAVED}/{key}/score.pdf")
}

pub fn midi_url(key: &str) -> String {
    format!("{ENGRAVED}/{key}/score.midi")
}

pub struct Engrave;

impl Rpc for Engrave {
    const METHOD: Method = Method::Engrave;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    // The same source always names the same job, so repeating it starts nothing new.
    const EFFECT: Effect = Effect::Idempotent;
    // A poll: the server answers out of its job table without waiting for LilyPond.
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = EngraveReq<'a>;
    type Reply = EngraveStatus;
}

impl Query for Engrave {}
