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
//! - [`OctaveControl`] changes a running job: moves its time limit, or stops it.
//! - [`OctaveAnswer`] answers a job's [`OctavePrompt`]: the file a `uigetfile` asked for, or
//!   what was typed for an `input`.
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

/// How long a job may run before the server stops it, unless [`OctaveControl`] extends it.
pub const DEFAULT_LIMIT_MS: u64 = 10 * 60 * 1000;

/// The furthest a job's limit can be extended to, from its start.
pub const MAX_LIMIT_MS: u64 = 8 * 60 * 60 * 1000;

/// The most prompts one job keeps. Later ones are dropped, and one waiting for an answer
/// waits until Stop or the time limit.
pub const MAX_PROMPTS: usize = 32;

/// What to run.
#[derive(Debug, Serialize, Deserialize)]
pub enum OctaveInput<'a> {
    /// Code typed at the prompt, or an editor's contents.
    Code(#[serde(borrow)] &'a str),
    /// A script file in the jail, e.g. `/home/alice/fit.m`, run as Octave's `run` would.
    File(#[serde(borrow)] &'a str),
    /// Stop the session and start a fresh one: every variable and figure is gone.
    Restart,
    /// Draw every figure the way Octave prints it, to PNG, for [`figure_url`]. Costs Octave a
    /// fraction of a second a figure, so an app asks only when the figures are on screen.
    Render,
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
    /// The job created this figure or drew into it, where desktop Octave would have popped up
    /// its window.
    pub changed: bool,
    /// After an [`OctaveInput::Render`] job: the PNG's width and height in pixels, at
    /// [`figure_url`] for that job. `None` otherwise, or if Octave couldn't print it.
    pub image: Option<(u32, u32)>,
}

/// Where the PNG of figure `number`, drawn by render job `(client, seq)`, is served. Only to
/// the user whose job it was, and only while the server remembers the job.
pub fn figure_url(client: u32, seq: u32, number: u32) -> String {
    // By hand rather than `format!`, which would pull the formatting machinery into every app
    // that links this.
    fn push(s: &mut String, mut n: u32) {
        let mut digits = [0u8; 10];
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        s.extend(digits[i..].iter().map(|&d| char::from(d)));
    }
    let mut url = String::from("/octave/figure/");
    push(&mut url, client);
    url.push('/');
    push(&mut url, seq);
    url.push('/');
    push(&mut url, number);
    url.push_str(".png");
    url
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
    /// When, counting from the job's start, the server will stop it.
    pub limit_ms: u64,
    /// Output from byte `from` onwards.
    pub output: String,
    /// The output offset to ask from next time.
    pub next: u32,
    /// The job produced more than [`MAX_OUTPUT_BYTES`], and the rest was dropped.
    pub output_truncated: bool,
    /// Present exactly when `finished`.
    pub result: Option<OctaveResult>,
    /// Every dialog the job has asked the app for so far, oldest first.
    pub prompts: Vec<OctavePrompt>,
}

/// A dialog a running job wants shown, from `msgbox`, `uigetfile` and the like, which have no
/// window of their own in a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OctavePrompt {
    /// Its number within the job, for [`OctaveAnswer`].
    pub id: u32,
    pub kind: PromptKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PromptKind {
    /// `msgbox` and kin: show it. The job carries on meanwhile, and wants no answer.
    Message {
        icon: MessageIcon,
        title: String,
        text: String,
    },
    /// `input`, or a line at `keyboard`'s `K>>`: type an answer at the Command Window's prompt.
    /// The job waits for an [`OctaveAnswer`] with [`PromptAnswer::Text`].
    Input {
        /// What `input` was given to ask, which may run over several lines.
        prompt: String,
    },
    /// `uigetfile`: pick a file. The job waits for an [`OctaveAnswer`] with
    /// [`PromptAnswer::File`].
    OpenFile {
        title: String,
        /// Patterns such as `*.m`, joined by `;`. `*` for any file.
        filter: String,
        /// The folder to start in, as a jail path, or empty for Octave's working folder.
        start: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageIcon {
    /// `msgbox`.
    None,
    /// `errordlg`.
    Error,
    /// `warndlg`.
    Warning,
    /// `helpdlg`.
    Help,
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

/// What to do to a running job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OctaveAction {
    /// Give it this many more minutes before the time limit, up to [`MAX_LIMIT_MS`] in all.
    Extend { minutes: u32 },
    /// Interrupt it, as Ctrl-C would at Octave's prompt. The session and its variables stay;
    /// the job finishes with an error saying it was stopped.
    Stop,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OctaveControlReq {
    pub client: u32,
    pub seq: u32,
    pub action: OctaveAction,
}

/// Change job `(client, seq)`. Replies with its time limit afterwards, as in
/// [`OctaveStatus::limit_ms`]. A job that has already finished is left as it is.
pub struct OctaveControl;

impl Rpc for OctaveControl {
    const METHOD: Method = Method::OctaveControl;
    const COALESCE: Coalesce = Coalesce::None;
    const EFFECT: Effect = Effect::Effectful;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = OctaveControlReq;
    type Reply = u64;
}

impl Command for OctaveControl {}

#[derive(Debug, Serialize, Deserialize)]
pub struct OctaveAnswerReq<'a> {
    pub client: u32,
    pub seq: u32,
    /// [`OctavePrompt::id`].
    pub prompt: u32,
    #[serde(borrow)]
    pub answer: PromptAnswer<'a>,
}

/// An answer to a prompt, of the kind it asked for.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptAnswer<'a> {
    /// For [`PromptKind::OpenFile`]: the file picked, as a jail path, or `None` for Cancel.
    File(#[serde(borrow)] Option<&'a str>),
    /// For [`PromptKind::Input`]: what was typed.
    Text(#[serde(borrow)] &'a str),
}

/// Answer prompt `prompt` of job `(client, seq)`, which is waiting for it. For a file, the server
/// checks the caller may read it, and hands Octave its real path.
pub struct OctaveAnswer;

impl Rpc for OctaveAnswer {
    const METHOD: Method = Method::OctaveAnswer;
    const COALESCE: Coalesce = Coalesce::None;
    const EFFECT: Effect = Effect::Effectful;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = OctaveAnswerReq<'a>;
    type Reply = ();
}

impl Command for OctaveAnswer {}
