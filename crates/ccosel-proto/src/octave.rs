//! The Octave app's methods: run code in the caller's GNU Octave session, and follow it.
//!
//! Each signed-in user has one long-lived Octave process on the server, so variables survive
//! from one command to the next the way they do at a real Octave prompt. Code can run for
//! minutes, so this is a **job**, like `build::Compile`, split in two:
//!
//! - [`OctaveRun`] sends the input once. It is a command: the code travels exactly one time,
//!   however long the job then runs.
//! - [`OctavePoll`] asks how the job is going. It carries only `(client, seq, from)`, so polling
//!   at a few hertz costs a few bytes each way, and replies carry only the output produced
//!   since `from`.
//!
//! `(client, seq)` names a job. `client` is a number a window picks for itself so two windows
//! of the same user, sharing one session, do not collide; `seq` goes up by one per run. Running
//! the same `(client, seq)` twice starts nothing new, so a retried send is harmless.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Command, Effect, Method, Query, Rpc};

/// The most output one job keeps. Later output is dropped and the job reports it was.
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// What to run.
#[derive(Debug, Serialize, Deserialize)]
pub enum OctaveInput<'a> {
    /// Code typed at the prompt, or an editor's contents.
    Code(#[serde(borrow)] &'a str),
    /// A script file in the jail, e.g. `/home/alice/fit.m`, run as Octave's `run` would.
    File(#[serde(borrow)] &'a str),
    /// Stop the session and start a fresh one: every variable and figure is gone.
    Restart,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OctaveRunReq<'a> {
    pub client: u32,
    pub seq: u32,
    #[serde(borrow)]
    pub input: OctaveInput<'a>,
}

/// Start a job. Answers once the job is queued, not when it is done: follow it with
/// [`OctavePoll`].
///
/// The reply is `false` when the server has no Octave to run it with. A reply rather than an
/// error, because the shell folds every server error code into one "server error", and the
/// app needs to tell this case apart to say what is wrong.
pub struct OctaveRun;

impl Rpc for OctaveRun {
    const METHOD: Method = Method::OctaveRun;
    const COALESCE: Coalesce = Coalesce::None;
    const EFFECT: Effect = Effect::Effectful;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = OctaveRunReq<'a>;
    type Reply = bool;
}

impl Command for OctaveRun {}

#[derive(Debug, Serialize, Deserialize)]
pub struct OctavePollReq {
    pub client: u32,
    pub seq: u32,
    /// How many bytes of this job's output the app already has.
    pub from: u32,
}

/// One variable in the workspace, already formatted for display.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    /// `double`, `char`, `cell`, `struct`, ...
    pub class: String,
    /// `3x3`.
    pub size: String,
    /// A short rendering (`[1 2;3 4]`, `"text"`), or empty when the value is too big to show
    /// in a line. The app asks for the full value separately.
    pub value: String,
    /// `global`, `persistent`, `complex`, `sparse`, comma-separated, or empty.
    pub attributes: String,
}

/// One line in a plot, resampled to evenly spaced points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Series {
    /// The legend entry, or empty.
    pub label: String,
    /// Each point scaled to `0..=255` between its axes' `y_min` and `y_max`, which is what
    /// `Ui::plot` takes. One byte a point is what keeps a figure cheap to send every run.
    pub samples: Vec<u8>,
}

/// One set of axes in a figure. The numbers are formatted by the server: formatting floats in
/// the app would pull float `Display` into a module every user downloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Axes {
    pub title: String,
    pub x_min: String,
    pub x_max: String,
    pub y_min: String,
    pub y_max: String,
    pub series: Vec<Series>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Figure {
    /// Octave's figure number, as in `figure(2)`.
    pub number: u32,
    pub name: String,
    pub axes: Vec<Axes>,
}

/// The session's state after a job, so one poll brings the whole UI up to date.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OctaveResult {
    /// The job ended in an error (the last output line says which).
    pub error: bool,
    /// The session ended during the job: Octave exited, crashed, or ran past the time limit.
    /// The next run starts a fresh session, with no variables.
    pub ended: bool,
    /// Octave's working directory, as a jail path (`/home/alice`) when it is inside the jail,
    /// or empty when it is not.
    pub cwd: String,
    pub variables: Vec<Variable>,
    /// More variables existed than a reply carries.
    pub variables_truncated: bool,
    pub figures: Vec<Figure>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OctaveStatus {
    pub finished: bool,
    pub elapsed_ms: u64,
    /// Output from byte `from` onwards.
    pub output: String,
    /// The output offset to ask from next time.
    pub next: u32,
    /// The job produced more than [`MAX_OUTPUT_BYTES`], and the rest was dropped.
    pub output_truncated: bool,
    /// Present exactly when `finished`.
    pub result: Option<OctaveResult>,
}

/// How a job is going. Idempotent: a job is only read, never started, by this.
pub struct OctavePoll;

impl Rpc for OctavePoll {
    const METHOD: Method = Method::OctavePoll;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = OctavePollReq;
    type Reply = OctaveStatus;
}

impl Query for OctavePoll {}
