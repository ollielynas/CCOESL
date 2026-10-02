//! The Octave app: GNU Octave, running on the server, driven from a window.
//!
//! The server keeps one Octave session per user, so what this app shows is a view of that
//! session: the command window's output, the workspace, the figures. The app itself keeps
//! only what a session doesn't: the log it has drawn, the command history, the editor's text.
//!
//! Running anything is a job (`ccosel_proto::octave`): [`OctaveRun`] sends the input once,
//! then [`OctavePoll`] follows it at a few hertz, asking only for output it hasn't had. One job
//! runs at a time; the finished job's report replaces the workspace and figures wholesale.
//!
//! Layout, like Octave's own: a toolbar (working folder, restart), the current folder's files,
//! the workspace and the history down the side, and tabs for the command window, editor,
//! figures and the variable view. The command window is a terminal: its output scrolls above a
//! prompt that stays at the bottom, and Enter runs what is typed.

use std::collections::BTreeSet;

use ccosel_proto::fs::{
    EntryKind, ListDir, ListDirReq, PathReq, ReadFile, WriteFile, WriteFileReq,
};
use ccosel_proto::octave::{
    Axes, DEFAULT_LIMIT_MS, Figure, MessageIcon, OctaveAction, OctaveAnswer, OctaveAnswerReq,
    OctaveControl, OctaveControlReq, OctaveInput, OctavePoll, OctavePollReq, OctavePrompt,
    OctaveResult, OctaveRun, OctaveRunReq, OctaveStatus, PromptAnswer, PromptKind, figure_url,
};
use ccosel_sdk::{App, CallId, CodeLang, Poll, Text, TextStyle, Ui, Vec2, icons};

/// 4 Hz while a job runs, the rate `ARCHITECTURE.md` sets for job progress.
const POLL_MS: u32 = 250;

/// Lines the command window keeps. Older ones scroll away for good.
const MAX_LOG: usize = 1000;

/// History entries the side panel lists at once, newest first.
const HISTORY_SHOWN: usize = 50;

/// How deep the current folder's tree opens. Bounds the listings one frame can ask for.
const TREE_DEPTH: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Command,
    Editor,
    Figures,
    Variable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// What was typed, shown after a `>>` prompt.
    Input,
    Output,
    Error,
    /// A figure the command before it drew, where desktop Octave would have popped up a
    /// window: the line's text is its image's URL, this its number and pixel size.
    Figure {
        number: u32,
        width: u32,
        height: u32,
    },
}

/// Where a job's output goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Console,
    /// Displaying one variable for the variable view: its output is not the log's business.
    Inspect(String),
    /// Drawing the figures as images for the Figures tab. Its output is gnuplot's chatter.
    Render,
    /// Running nothing, to learn the session's state when the window opens.
    Sync,
}

/// What to run, owned, until it is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Run {
    Code(String),
    File(String),
    Restart,
    Render,
}

pub struct Job {
    pub seq: u32,
    pub target: Target,
    /// The `OctaveRun` call, until the server has taken the job.
    sent: Option<CallId>,
    /// How many bytes of output have arrived.
    received: u32,
    /// Output not yet ending in a newline, held until the rest of the line arrives.
    partial: String,
    /// How long it has run, and when the server will stop it, as the last poll said.
    pub elapsed_ms: u64,
    pub limit_ms: u64,
    /// Stop was pressed: the server is interrupting it.
    pub stopping: bool,
    /// How many of its prompts (dialogs) the app has already shown.
    prompts_seen: usize,
    /// Run from the editor: its file's path, or empty for its unsaved text.
    from_editor: Option<String>,
}

/// An error the last run left in the editor's text, marked there until it is edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorError {
    pub line: u32,
    /// 0 when Octave gave only the line.
    pub column: u32,
    pub message: String,
    /// The editor text's version when it was marked: any edit since clears it.
    version: u32,
}

/// A message a job showed with `msgbox` and kin, until OK is pressed.
pub struct Message {
    /// Unique among the messages shown, so two with the same title are two windows.
    pub id: u32,
    pub icon: MessageIcon,
    pub title: String,
    pub text: String,
}

/// An `input` (or a `keyboard` line) a job is waiting on, answered at the prompt.
pub struct Asking {
    pub seq: u32,
    pub prompt: u32,
    /// What it asked, which may run over several lines: the last is shown at the prompt.
    pub text: String,
    /// The answer on its way, and what it was.
    sending: Option<(CallId, String)>,
}

impl Asking {
    /// The line shown in place of `>>`.
    fn label(&self) -> &str {
        self.text.rsplit('\n').next().unwrap_or_default()
    }
}

/// The file picker a job's `uigetfile` is waiting on.
pub struct Picker {
    pub seq: u32,
    pub prompt: u32,
    pub title: String,
    /// Glob patterns a file must match one of to be listed, such as `*.m`.
    pub patterns: Vec<String>,
    /// The folder shown, a jail path.
    pub dir: String,
    pub selected: Option<String>,
    /// The answer on its way to the server.
    sending: Option<CallId>,
    /// Why the last answer was refused, if it was.
    pub status: String,
}

/// An editor save in flight, and whether to run the file once it is saved.
struct Save {
    call: CallId,
    run_after: bool,
}

pub struct Octave {
    /// This window's name for its jobs, picked on the first frame. See `ccosel_proto::octave`.
    client: Option<u32>,
    seq: u32,
    pub tab: Tab,
    pub input: Text,
    pub log: Vec<(LineKind, String)>,
    pub history: Vec<String>,
    /// Which history entry ▲/▼ last put in the prompt, counting back from the newest.
    recall: Option<usize>,
    pub history_filter: Text,
    pub job: Option<Job>,
    /// The session as the last finished job left it.
    pub session: OctaveResult,
    /// The server has no Octave.
    pub unavailable: bool,
    pub editor_path: Text,
    pub editor: Text,
    pub editor_status: String,
    loading: Option<String>,
    save: Option<Save>,
    /// The variable the variable view shows, and what displaying it printed.
    pub inspected: Option<String>,
    pub inspect_output: Vec<String>,
    /// Folders open in the current folder's tree.
    pub expanded: BTreeSet<String>,
    /// A job or a save may have changed the files: list the tree's folders again.
    refresh_tree: bool,
    /// Stop and extend requests not yet answered.
    controls: Vec<CallId>,
    /// The render job whose images the session's figures have, if they have any.
    pub rendered: Option<u32>,
    /// The figures changed since they were last drawn as images.
    pub figures_stale: bool,
    /// Figures the last command drew, to show in the command window once they are rendered.
    pub inline: Vec<u32>,
    /// Messages from `msgbox` and kin, shown until OK.
    pub messages: Vec<Message>,
    messages_shown: u32,
    /// The file picker a running `uigetfile` waits on.
    pub picker: Option<Picker>,
    /// The `input` a running job waits on, answered at the Command Window's prompt.
    pub asking: Option<Asking>,
    /// Where the last run's error is in the editor's text, if it is in it.
    pub editor_error: Option<EditorError>,
    /// A job has reported the session's state at least once. Until then the app doesn't know
    /// the working folder or the variables, which isn't the same as there being none.
    pub synced: bool,
}

impl Default for Octave {
    fn default() -> Self {
        Self {
            client: None,
            seq: 0,
            tab: Tab::Command,
            input: Text::new(""),
            log: Vec::new(),
            history: Vec::new(),
            recall: None,
            history_filter: Text::new(""),
            job: None,
            session: OctaveResult::default(),
            unavailable: false,
            editor_path: Text::new(""),
            editor: Text::new(""),
            editor_status: String::new(),
            loading: None,
            save: None,
            inspected: None,
            inspect_output: Vec::new(),
            expanded: BTreeSet::new(),
            refresh_tree: false,
            controls: Vec::new(),
            rendered: None,
            figures_stale: false,
            inline: Vec::new(),
            messages: Vec::new(),
            messages_shown: 0,
            picker: None,
            asking: None,
            editor_error: None,
            synced: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Tab(Tab),
    Submit,
    Older,
    Newer,
    ClearLog,
    Restart,
    Recall(usize),
    Rerun(usize),
    Script(Vec<usize>),
    Inspect(String),
    Open,
    Save {
        run_after: bool,
    },
    RunEditor,
    NewFile,
    SaveFigure(u32),
    /// Open or close a folder in the tree.
    Toggle(String),
    /// Open a file from the tree in the editor.
    Edit(String),
    /// Give the running job more time.
    Extend(u32),
    /// Interrupt the running job.
    Stop,
    /// OK on message `id`.
    Dismiss(u32),
    /// In the file picker: show this folder; select this file; answer with the file, or
    /// with Cancel.
    PickerDir(String),
    PickerSelect(String),
    PickerAnswer(Option<String>),
    /// Change Octave's working folder, to a path relative to the current one: the session's
    /// `cwd` is a jail path, which means nothing to Octave itself.
    Cd(String),
}

/// Hand-rolled, as in the other apps: float `Display` would cost every user tens of KB.
fn itoa(mut n: u64) -> String {
    if n == 0 {
        return "0".to_owned();
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    String::from_utf8_lossy(&buf[i..]).into_owned()
}

/// A Phosphor icon and a word, for a button or a row. The shell's fonts lack most emoji and
/// draw them as empty boxes; these always render.
pub fn label(icon: &str, text: &str) -> String {
    let mut s = String::from(icon);
    s.push(' ');
    s.push_str(text);
    s
}

/// Says what the time limit is, before anything runs.
const LIMIT_TIP: &str = "Run it. A command gets 10 minutes before the server stops it; while it \
                         runs you can give it longer, or stop it.";

/// What the command window shows for running the editor's text when it has no file.
const UNSAVED_RUN: &str = "run (unsaved script)";

/// `ms` as a clock: `1:05`, or `1:02:05` past an hour.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    let mut out = String::new();
    let two = |out: &mut String, n: u64| {
        if n < 10 {
            out.push('0');
        }
        out.push_str(&itoa(n));
    };
    if h > 0 {
        out.push_str(&itoa(h));
        out.push(':');
        two(&mut out, m);
    } else {
        out.push_str(&itoa(m));
    }
    out.push(':');
    two(&mut out, s);
    out
}

/// `name` inside the folder `dir`, both jail paths.
fn join(dir: &str, name: &str) -> String {
    let mut path = String::from(dir.trim_end_matches('/'));
    path.push('/');
    path.push_str(name);
    path
}

/// `s` as an Octave single-quoted string, quotes doubled.
fn quoted(s: &str) -> String {
    let mut out = String::from("'");
    out.push_str(&s.replace('\'', "''"));
    out.push('\'');
    out
}

/// Whether the tree opens this file in the editor: Octave's own scripts and functions.
fn is_script(name: &str) -> bool {
    name.ends_with(".m")
}

/// `name` if it is an Octave identifier: only a real variable name is ever spliced into code
/// the app sends.
fn identifier(name: &str) -> Option<&str> {
    let mut chars = name.chars();
    let first = chars.next()?;
    let ok = (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then_some(name)
}

impl Octave {
    /// Start a job, unless one is already running. Returns whether it started.
    fn start(&mut self, ui: &mut Ui<'_>, run: Run, target: Target) -> bool {
        if self.job.is_some() || self.unavailable {
            return false;
        }
        self.seq = self.seq.wrapping_add(1);
        let input = match &run {
            Run::Code(code) => OctaveInput::Code(code),
            Run::File(path) => OctaveInput::File(path),
            Run::Restart => OctaveInput::Restart,
            Run::Render => OctaveInput::Render,
        };
        let call = ui.rpc().send::<OctaveRun>(&OctaveRunReq {
            client: self.client.unwrap_or(1),
            seq: self.seq,
            input,
        });
        self.job = Some(Job {
            seq: self.seq,
            target,
            sent: Some(call),
            received: 0,
            partial: String::new(),
            elapsed_ms: 0,
            limit_ms: DEFAULT_LIMIT_MS,
            stopping: false,
            prompts_seen: 0,
            from_editor: None,
        });
        true
    }

    /// Run code in the command window, and remember it in the history.
    fn submit(&mut self, ui: &mut Ui<'_>, code: &str) {
        let code = code.trim_end();
        if code.trim().is_empty() {
            return;
        }
        if !self.start(ui, Run::Code(code.to_owned()), Target::Console) {
            return;
        }
        for line in code.lines() {
            self.push_log(LineKind::Input, line);
        }
        // Like a shell: running the same thing twice in a row is one history entry.
        if self.history.last().map(String::as_str) != Some(code) {
            self.history.push(code.to_owned());
        }
        self.recall = None;
        self.input.set("");
    }

    fn push_log(&mut self, kind: LineKind, line: &str) {
        self.log.push((kind, line.to_owned()));
        if self.log.len() > MAX_LOG {
            let excess = self.log.len() - MAX_LOG;
            self.log.drain(..excess);
        }
    }

    /// Output that arrived for the running job: whole lines go wherever the job's output goes.
    fn take_output(&mut self, text: &str, finished: bool) {
        let Some(job) = &mut self.job else {
            return;
        };
        job.partial.push_str(text);
        let mut lines: Vec<String> = Vec::new();
        while let Some(i) = job.partial.find('\n') {
            lines.push(job.partial[..i].to_owned());
            job.partial.drain(..=i);
        }
        if finished && !job.partial.is_empty() {
            lines.push(core::mem::take(&mut job.partial));
        }
        let target = job.target.clone();
        for line in lines {
            match &target {
                Target::Console => {
                    let kind = if line.starts_with("error: ") {
                        LineKind::Error
                    } else {
                        LineKind::Output
                    };
                    self.push_log(kind, &line);
                }
                Target::Inspect(_) => self.inspect_output.push(line),
                Target::Render | Target::Sync => {}
            }
        }
    }

    /// The job is over: its output is already in (see `follow`).
    fn finish(&mut self, status: &OctaveStatus) {
        if status.output_truncated {
            self.push_log(
                LineKind::Error,
                "error: output past the server's limit was dropped",
            );
        }
        if let Some(result) = &status.result {
            self.session = result.clone();
            self.synced = true;
        }
        // A render's report is the one whose figures carry images; any other job's replaces
        // it with figures that may have changed, so the next look at them draws them again.
        let Some(job) = self.job.take() else {
            return;
        };
        // Nothing waits on the picker, or for an answer, any more.
        self.picker = None;
        self.asking = None;
        self.refresh_tree = true;
        if job.target == Target::Render {
            self.rendered = Some(job.seq);
            self.figures_stale = false;
            self.show_inline(job.seq);
            return;
        }
        self.rendered = None;
        self.figures_stale = !self.session.figures.is_empty();
        if job.target == Target::Console {
            self.place_error(job.from_editor.as_deref());
        }
        if job.target == Target::Console {
            // Where a desktop would pop up a window, draw the figure into the command window.
            self.inline = (self.session.figures.iter())
                .filter(|f| f.changed)
                .map(|f| f.number)
                .collect();
        }
    }

    /// If the job that just ended failed in the editor's text, mark the place there. That is its
    /// unsaved text, if it was run from the editor; or its file, however it was run. A run from
    /// the editor that failed there goes back to it, to show where.
    fn place_error(&mut self, from_editor: Option<&str>) {
        let open = self.editor_path.as_str().trim();
        let here = self
            .session
            .error_at
            .as_ref()
            .filter(|at| match at.path.as_str() {
                "" => from_editor == Some(""),
                path => path == open,
            });
        match here {
            Some(at) => {
                self.editor_error = Some(EditorError {
                    line: at.line,
                    column: at.column,
                    message: at.message.clone(),
                    version: self.editor.version(),
                });
                if from_editor.is_some() {
                    self.tab = Tab::Editor;
                }
            }
            // A run from the editor that got past its error, or failed somewhere else.
            None if from_editor.is_some() => self.editor_error = None,
            None => {}
        }
    }

    /// Put figure `number`'s image, at `url`, in the command window. If the window ends in
    /// figures, with nothing printed since, and this figure is one of them, its image there is
    /// updated instead, as its window would be: a drawing loop redraws in place rather than
    /// filling the window with frames.
    fn show_figure(&mut self, number: u32, width: u32, height: u32, url: String) {
        let kind = LineKind::Figure {
            number,
            width,
            height,
        };
        let trailing = (self.log.iter_mut().rev())
            .take_while(|(k, _)| matches!(k, LineKind::Figure { .. }))
            .find(|(k, _)| matches!(k, LineKind::Figure { number: n, .. } if *n == number));
        match trailing {
            Some(entry) => *entry = (kind, url),
            None => self.push_log(kind, &url),
        }
    }

    /// A poll's output, which starts `start` bytes into the job's, and the prompts the job has
    /// made since the last one, each put in its place among the output: a figure drawn before
    /// a line was printed goes above it. A prompt made after this poll's output ended waits for
    /// the next poll, which will have the output before it.
    fn take_output_and_prompts(&mut self, start: u32, status: &OctaveStatus) {
        let text = status.output.as_str();
        let seen = self.job.as_ref().map_or(0, |j| j.prompts_seen);
        let mut done = 0;
        let mut taken = seen;
        for prompt in status.prompts.get(seen..).unwrap_or_default() {
            let upto = prompt.at.saturating_sub(start) as usize;
            if upto > text.len() && !status.finished {
                break;
            }
            let upto = upto.clamp(done, text.len());
            if text.is_char_boundary(upto) {
                self.take_output(&text[done..upto], false);
                done = upto;
            }
            self.take_prompt(prompt);
            taken += 1;
        }
        if let Some(job) = &mut self.job {
            job.prompts_seen = taken;
        }
        self.take_output(&text[done..], status.finished);
    }

    /// Show what a prompt asks for: a message, a picker, a question, or a figure drawn.
    fn take_prompt(&mut self, prompt: &OctavePrompt) {
        let Some(job) = &self.job else {
            return;
        };
        let seq = job.seq;
        {
            match &prompt.kind {
                PromptKind::Message { icon, title, text } => {
                    self.messages_shown += 1;
                    self.messages.push(Message {
                        id: self.messages_shown,
                        icon: *icon,
                        title: title.clone(),
                        text: text.clone(),
                    });
                }
                PromptKind::Figure {
                    number,
                    image,
                    width,
                    height,
                } => {
                    // Drawn partway through, where a desktop window would be showing it.
                    let url = figure_url(self.client.unwrap_or(1), seq, *image);
                    self.show_figure(*number, *width, *height, url);
                }
                PromptKind::Input { prompt: text } => {
                    // Like a terminal: every line of the question but the last goes in the
                    // command window; the last is shown at the prompt, where the answer goes.
                    let lines: Vec<&str> = text.split('\n').collect();
                    let earlier: Vec<String> = (lines[..lines.len() - 1].iter())
                        .map(|l| (*l).to_owned())
                        .collect();
                    for line in earlier {
                        self.push_log(LineKind::Output, &line);
                    }
                    self.asking = Some(Asking {
                        seq,
                        prompt: prompt.id,
                        text: text.clone(),
                        sending: None,
                    });
                    self.tab = Tab::Command;
                }
                PromptKind::OpenFile {
                    title,
                    filter,
                    start,
                } => {
                    let dir = match (start.as_str(), self.session.cwd.as_str()) {
                        ("", "") => "/",
                        ("", cwd) => cwd,
                        (start, _) => start,
                    };
                    self.picker = Some(Picker {
                        seq,
                        prompt: prompt.id,
                        title: if title.is_empty() {
                            String::from("Select a file")
                        } else {
                            title.clone()
                        },
                        patterns: filter
                            .split(';')
                            .map(str::trim)
                            .filter(|p| !p.is_empty())
                            .map(str::to_owned)
                            .collect(),
                        dir: dir.to_owned(),
                        selected: None,
                        sending: None,
                        status: String::new(),
                    });
                }
            }
        }
    }

    /// Send what is at the prompt as the answer to the `input` waiting on it.
    fn answer(&mut self, ui: &mut Ui<'_>) {
        let client = self.client.unwrap_or(1);
        let Some(asking) = &mut self.asking else {
            return;
        };
        if asking.sending.is_some() {
            return;
        }
        let text = self.input.as_str().to_owned();
        let call = ui.rpc().send::<OctaveAnswer>(&OctaveAnswerReq {
            client,
            seq: asking.seq,
            prompt: asking.prompt,
            answer: PromptAnswer::Text(&text),
        });
        asking.sending = Some((call, text));
        self.input.set("");
    }

    /// See whether an answer to `input` was taken: the question and answer go in the command
    /// window, as a terminal would leave them.
    fn input_answered(&mut self, ui: &mut Ui<'_>) {
        let Some(asking) = &mut self.asking else {
            return;
        };
        let Some((call, _)) = &asking.sending else {
            return;
        };
        match ui.rpc().outcome::<OctaveAnswer>(*call) {
            Poll::Pending => {}
            Poll::Ready(_) => {
                let mut line = asking.label().to_owned();
                if let Some((_, text)) = &asking.sending {
                    line.push_str(text);
                }
                self.asking = None;
                self.push_log(LineKind::Output, &line);
            }
            Poll::Failed(err) => {
                asking.sending = None;
                let mut msg = String::from("error: could not answer: ");
                msg.push_str(err.message());
                self.push_log(LineKind::Error, &msg);
            }
        }
    }

    /// See whether the picker's answer was taken: it closes, or says why not and stays open.
    fn answered(&mut self, ui: &mut Ui<'_>) {
        let Some(picker) = &mut self.picker else {
            return;
        };
        let Some(call) = picker.sending else {
            return;
        };
        match ui.rpc().outcome::<OctaveAnswer>(call) {
            Poll::Pending => {}
            Poll::Ready(_) => self.picker = None,
            Poll::Failed(err) => {
                picker.sending = None;
                picker.status = String::from("Couldn't open that: ");
                picker.status.push_str(err.message());
            }
        }
    }

    /// Messages, and the file picker, as windows over the app.
    fn dialogs(&self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        for m in &self.messages {
            ui.push_id(&itoa(u64::from(m.id)), |ui| message(ui, m, act));
        }
        if let Some(picker) = &self.picker {
            ui.push_id("picker", |ui| picker_window(ui, picker, act));
        }
    }

    /// Put the figures the last command drew into the command window, now render `seq` has
    /// drawn them.
    fn show_inline(&mut self, seq: u32) {
        let client = self.client.unwrap_or(1);
        for number in core::mem::take(&mut self.inline) {
            let fig = self.session.figures.iter().find(|f| f.number == number);
            if let Some((width, height)) = fig.and_then(|f| f.image) {
                self.show_figure(number, width, height, figure_url(client, seq, number));
            }
        }
    }

    /// Draw the figures as Octave prints them, if they are on screen and have changed since.
    fn render(&mut self, ui: &mut Ui<'_>) {
        let wanted = (self.tab == Tab::Figures && self.figures_stale) || !self.inline.is_empty();
        if wanted && self.job.is_none() {
            // Cleared first, so a render that fails isn't tried again and again: the line
            // drawings stay until the figures next change.
            self.figures_stale = false;
            if !self.start(ui, Run::Render, Target::Render) {
                self.inline.clear();
            }
        }
    }

    fn fail(&mut self, what: &str, err: &str) {
        if self.job.take().is_some_and(|j| j.target == Target::Render) {
            // Not drawn, so not shown: the Figures tab still has the lines.
            self.inline.clear();
        }
        let mut msg = String::from("error: ");
        msg.push_str(what);
        msg.push_str(err);
        self.push_log(LineKind::Error, &msg);
    }

    /// Ask the server to stop the running job, or give it more time.
    fn control(&mut self, ui: &mut Ui<'_>, action: OctaveAction) {
        let Some(job) = &mut self.job else {
            return;
        };
        let call = ui.rpc().send::<OctaveControl>(&OctaveControlReq {
            client: self.client.unwrap_or(1),
            seq: job.seq,
            action,
        });
        if action == OctaveAction::Stop {
            job.stopping = true;
        }
        self.controls.push(call);
    }

    /// See how stop and extend requests went: an extension's reply is the job's new limit.
    fn controlled(&mut self, ui: &mut Ui<'_>) {
        for call in core::mem::take(&mut self.controls) {
            match ui.rpc().outcome::<OctaveControl>(call) {
                Poll::Pending => self.controls.push(call),
                Poll::Ready(limit) => {
                    if let Some(job) = &mut self.job {
                        job.limit_ms = *limit;
                    }
                }
                Poll::Failed(err) => {
                    if let Some(job) = &mut self.job {
                        job.stopping = false;
                    }
                    let mut msg = String::from("error: could not change the running job: ");
                    msg.push_str(err.message());
                    self.push_log(LineKind::Error, &msg);
                }
            }
        }
    }

    /// Follow the running job: see whether the server took it, then poll it.
    fn follow(&mut self, ui: &mut Ui<'_>) {
        let Some(job) = &mut self.job else {
            return;
        };
        if let Some(call) = job.sent {
            match ui.rpc().outcome::<OctaveRun>(call) {
                Poll::Pending => return,
                Poll::Ready(started) => {
                    job.sent = None;
                    if !*started {
                        self.unavailable = true;
                        self.job = None;
                        return;
                    }
                }
                Poll::Failed(err) => return self.fail("could not run it: ", err.message()),
            }
        }
        let Some(job) = &self.job else {
            return;
        };
        let req = OctavePollReq {
            client: self.client.unwrap_or(1),
            seq: job.seq,
            from: job.received,
        };
        match ui.rpc().get::<OctavePoll>(&req) {
            Poll::Pending => {}
            Poll::Ready(status) => {
                // A poll with no new output repeats the last one's key: drop the answer so the
                // next frame really asks again rather than reading this copy.
                ui.rpc().invalidate::<OctavePoll>(&req);
                let mut start = 0;
                if let Some(job) = &mut self.job {
                    start = job.received;
                    job.received = status.next;
                    job.elapsed_ms = status.elapsed_ms;
                    job.limit_ms = status.limit_ms;
                }
                self.take_output_and_prompts(start, &status);
                if status.finished {
                    self.finish(&status);
                }
            }
            Poll::Failed(err) => {
                ui.rpc().invalidate::<OctavePoll>(&req);
                self.fail("lost track of it: ", err.message());
            }
        }
    }

    fn toolbar(&self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.label(icons::FOLDER);
            let cwd = match (self.session.cwd.as_str(), self.synced) {
                ("", false) => "…",
                ("", true) => "(outside the shared files)",
                (cwd, _) => cwd,
            };
            ui.label(cwd);
            ui.tooltip("Octave's working folder. Change it with cd at the prompt.");
            let idle = self.job.is_none();
            ui.enabled(idle, |ui| {
                if ui
                    .button(&label(icons::ARROW_CLOCKWISE, "Restart"))
                    .clicked()
                {
                    act.push(Action::Restart);
                }
                ui.tooltip("Start a fresh Octave session: every variable and figure is cleared");
            });
            if let Some(job) = &self.job {
                self.running(ui, job, act);
            }
        });
        ui.horizontal(|ui| {
            let mut figures = String::from("Figures");
            if !self.session.figures.is_empty() {
                figures.push_str(" (");
                figures.push_str(&itoa(self.session.figures.len() as u64));
                figures.push(')');
            }
            for (tab, label) in [
                (Tab::Command, "Command Window"),
                (Tab::Editor, "Editor"),
                (Tab::Figures, figures.as_str()),
                (Tab::Variable, "Variable"),
            ] {
                if ui.selectable(self.tab == tab, label).clicked() {
                    act.push(Action::Tab(tab));
                }
            }
        });
        ui.separator();
    }

    /// The running job: how long it has had, and the controls to give it longer or stop it.
    fn running(&self, ui: &mut Ui<'_>, job: &Job, act: &mut Vec<Action>) {
        if job.stopping {
            ui.label(&label(icons::HOURGLASS, "Stopping…"));
            return;
        }
        if job.target == Target::Render {
            ui.label(&label(icons::HOURGLASS, "Drawing figures…"));
            return;
        }
        if job.target == Target::Sync {
            ui.label(&label(icons::HOURGLASS, "Starting Octave…"));
            return;
        }
        if self.asking.is_some() {
            ui.label(&label(icons::CHAT_TEXT, "Waiting for your answer"));
            ui.tooltip("The script asked a question: answer it at the Command Window's prompt");
        } else {
            ui.label(&label(icons::HOURGLASS, "Running…"));
        }
        let mut time = clock(job.elapsed_ms);
        time.push_str(" of ");
        time.push_str(&clock(job.limit_ms));
        ui.label(&time);
        ui.tooltip(
            "How long it has run, and its time limit. At the limit the server stops it and \
             restarts Octave, which clears your variables.",
        );
        // Until the server has taken the job there is nothing there to change.
        ui.enabled(job.sent.is_none(), |ui| {
            if ui.button(&label(icons::TIMER, "+10 min")).clicked() {
                act.push(Action::Extend(10));
            }
            ui.tooltip("Give it 10 more minutes before the time limit");
            if ui.button("+1 h").clicked() {
                act.push(Action::Extend(60));
            }
            ui.tooltip("Give it an hour more before the time limit");
            if ui.button(&label(icons::STOP, "Stop")).clicked() {
                act.push(Action::Stop);
            }
            ui.tooltip("Interrupt it, like Ctrl-C. Your variables are kept.");
        });
    }

    fn sidebar(&mut self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        // One scroll for the whole column, so a long tree or history never pushes the
        // command window's prompt off the window.
        ui.scroll(|ui| {
            self.folder(ui, act);
            ui.separator();
            self.workspace(ui, act);
            ui.separator();
            self.history_list(ui, act);
        });
    }

    /// The current folder's files, as a tree. Scripts open in the editor.
    fn folder(&self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        ui.push_id("folder", |ui| {
            ui.horizontal(|ui| {
                ui.styled("Current Folder", TextStyle::heading(3));
                let cwd = self.session.cwd.as_str();
                if !cwd.is_empty() && cwd != "/" {
                    ui.enabled(self.job.is_none(), |ui| {
                        if ui.button(icons::ARROW_UP).clicked() {
                            act.push(Action::Cd("..".to_owned()));
                        }
                        ui.tooltip("Go up to the folder that holds this one");
                    });
                }
            });
            if !self.synced {
                ui.styled("Loading…", TextStyle::WEAK);
                return;
            }
            if self.session.cwd.is_empty() {
                ui.styled(
                    "Octave's working folder is outside the shared files.",
                    TextStyle::WEAK,
                );
                return;
            }
            let root = self.session.cwd.clone();
            self.tree(ui, &root, 0, act);
        });
    }

    fn tree(&self, ui: &mut Ui<'_>, dir: &str, depth: usize, act: &mut Vec<Action>) {
        let listing = match ui.rpc().get::<ListDir>(&ListDirReq { path: dir }) {
            Poll::Ready(listing) => listing,
            Poll::Pending => {
                ui.styled("Loading…", TextStyle::WEAK);
                return;
            }
            Poll::Failed(err) => {
                ui.styled(err.message(), TextStyle::WEAK | TextStyle::ITALIC);
                return;
            }
        };
        // Folders first, then files, each by name, as Octave's own file browser lists them.
        let mut entries: Vec<_> = listing
            .entries
            .iter()
            .filter(|e| !e.name.starts_with('.'))
            .collect();
        entries.sort_by(|a, b| {
            (a.kind != EntryKind::Dir)
                .cmp(&(b.kind != EntryKind::Dir))
                .then_with(|| a.name.cmp(&b.name))
        });
        if entries.is_empty() {
            ui.styled("Empty", TextStyle::WEAK | TextStyle::ITALIC);
        }
        let editing = self.editor_path.as_str().trim();
        for entry in entries {
            let path = join(dir, &entry.name);
            ui.push_id(&entry.name, |ui| {
                if entry.kind == EntryKind::Dir {
                    let open = self.expanded.contains(&path);
                    let icon = if open {
                        icons::FOLDER_OPEN
                    } else {
                        icons::FOLDER
                    };
                    let text = label(icon, &entry.name);
                    ui.horizontal(|ui| {
                        if ui.selectable(false, &text).clicked() {
                            act.push(Action::Toggle(path.clone()));
                        }
                        ui.tooltip("Open or close this folder");
                        ui.enabled(self.job.is_none(), |ui| {
                            if ui.button("cd").clicked() {
                                let rel = path.strip_prefix(&self.session.cwd).unwrap_or(&path);
                                act.push(Action::Cd(rel.trim_start_matches('/').to_owned()));
                            }
                            ui.tooltip("Make this Octave's working folder");
                        });
                    });
                    if open && depth + 1 < TREE_DEPTH {
                        ui.indent(|ui| self.tree(ui, &path, depth + 1, act));
                    }
                } else if is_script(&entry.name) {
                    let text = label(icons::FILE_CODE, &entry.name);
                    if ui.selectable(editing == path, &text).clicked() {
                        act.push(Action::Edit(path.clone()));
                    }
                    ui.tooltip("Open it in the editor");
                } else {
                    let text = label(icons::FILE, &entry.name);
                    ui.styled(&text, TextStyle::WEAK);
                }
            });
        }
        if listing.truncated {
            ui.styled("(more not shown)", TextStyle::WEAK);
        }
    }

    /// List the tree's folders again, the next time they are drawn.
    fn invalidate_tree(&mut self, ui: &mut Ui<'_>) {
        if !core::mem::take(&mut self.refresh_tree) || self.session.cwd.is_empty() {
            return;
        }
        let rpc = ui.rpc();
        rpc.invalidate::<ListDir>(&ListDirReq {
            path: &self.session.cwd,
        });
        for dir in &self.expanded {
            rpc.invalidate::<ListDir>(&ListDirReq { path: dir });
        }
    }

    fn workspace(&self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        ui.styled("Workspace", TextStyle::heading(3));
        if self.session.variables.is_empty() {
            ui.styled("No variables yet", TextStyle::WEAK);
        }
        // Showing a variable is a job, so it waits for the running one, like Run.
        let idle = self.job.is_none();
        for var in &self.session.variables {
            ui.push_id(&var.name, |ui| {
                let mut row = var.name.clone();
                row.push_str("  ");
                row.push_str(&var.size);
                row.push(' ');
                row.push_str(&var.class);
                let selected = self.inspected.as_deref() == Some(var.name.as_str());
                ui.enabled(idle, |ui| {
                    if ui.selectable(selected, &row).clicked() {
                        act.push(Action::Inspect(var.name.clone()));
                    }
                    if !var.attributes.is_empty() {
                        ui.tooltip(&var.attributes);
                    }
                });
                if !var.value.is_empty() {
                    ui.indent(|ui| ui.styled(&var.value, TextStyle::CODE));
                }
            });
        }
        if self.session.variables_truncated {
            ui.styled("(more variables than shown)", TextStyle::WEAK);
        }
    }

    fn history_list(&mut self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        ui.styled("Command History", TextStyle::heading(3));
        ui.text_edit(&mut self.history_filter);
        ui.tooltip("Search the history");
        let filter = self.history_filter.as_str().to_lowercase();
        let matching: Vec<usize> = (0..self.history.len())
            .rev()
            .filter(|&i| filter.is_empty() || self.history[i].to_lowercase().contains(&filter))
            .take(HISTORY_SHOWN)
            .collect();
        if matching.is_empty() {
            ui.styled("Nothing yet", TextStyle::WEAK);
            return;
        }
        if ui
            .button(&label(icons::NOTE_PENCIL, "Create script"))
            .clicked()
        {
            act.push(Action::Script(matching.iter().rev().copied().collect()));
        }
        ui.tooltip("Open the commands listed here in the editor, oldest first");
        for &i in &matching {
            ui.push_id(&itoa(i as u64), |ui| {
                ui.horizontal(|ui| {
                    ui.enabled(self.job.is_none(), |ui| {
                        if ui.button(icons::PLAY).clicked() {
                            act.push(Action::Rerun(i));
                        }
                        ui.tooltip("Run it again");
                    });
                    let first = self.history[i].lines().next().unwrap_or_default();
                    if ui.selectable(false, first).clicked() {
                        act.push(Action::Recall(i));
                    }
                    // The row may be cut short, and shows only the first line: the whole
                    // command is here.
                    let mut tip = self.history[i].clone();
                    tip.push_str("\n\nClick to put it back in the prompt.");
                    ui.tooltip(&tip);
                });
            });
        }
    }

    fn command_window(&mut self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        let log = &self.log;
        let input = &mut self.input;
        // An `input` waiting for an answer takes the prompt over, as it would in a terminal.
        let asking = self.asking.as_ref();
        let idle = self.job.is_none() || asking.is_some_and(|a| a.sending.is_none());
        let mut open_figures = false;
        ui.scroll_with_footer(
            |ui| Self::console(ui, log, &mut open_figures),
            |ui| {
                ui.label(asking.map_or(">>", Asking::label));
                if ui.text_edit(input).submitted() {
                    act.push(Action::Submit);
                }
                ui.tooltip(if asking.is_some() {
                    "The script is waiting for your answer: type it here and press Enter"
                } else {
                    // Typing ahead is fine while a job runs; running it waits until it ends.
                    "Type Octave code here and press Enter to run it"
                });
                ui.enabled(idle, |ui| {
                    if ui.button(&label(icons::PLAY, "Run")).clicked() {
                        act.push(Action::Submit);
                    }
                    ui.tooltip(if asking.is_some() {
                        "Send your answer"
                    } else {
                        LIMIT_TIP
                    });
                });
                if ui.button(icons::CARET_UP).clicked() {
                    act.push(Action::Older);
                }
                ui.tooltip("The previous command");
                if ui.button(icons::CARET_DOWN).clicked() {
                    act.push(Action::Newer);
                }
                ui.tooltip("The next command");
                if ui.button(&label(icons::BROOM, "Clear")).clicked() {
                    act.push(Action::ClearLog);
                }
                ui.tooltip("Clear the command window. Variables are kept.");
            },
        );
        if open_figures {
            act.push(Action::Tab(Tab::Figures));
        }
    }

    /// The command window's output, oldest first. `open_figures` is set if a figure's link to
    /// the Figures tab was clicked.
    fn console(ui: &mut Ui<'_>, log: &[(LineKind, String)], open_figures: &mut bool) {
        if log.is_empty() {
            ui.styled(
                "Type Octave code at the >> prompt below and press Enter to run it. \
                 Variables are kept between commands.",
                TextStyle::WEAK,
            );
        }
        for (kind, line) in log {
            match kind {
                LineKind::Input => {
                    let mut s = String::from(">> ");
                    s.push_str(line);
                    ui.styled(&s, TextStyle::CODE | TextStyle::STRONG);
                }
                LineKind::Output => {
                    ui.styled(line, TextStyle::CODE);
                }
                LineKind::Error => {
                    ui.styled(line, TextStyle::CODE | TextStyle::ITALIC);
                }
                LineKind::Figure {
                    number,
                    width,
                    height,
                } => {
                    let (w, h) = fit(*width, *height, MAX_INLINE_WIDTH);
                    ui.image(line, Vec2::new(w as f32, h as f32));
                    let mut caption = String::from("Figure ");
                    caption.push_str(&itoa(u64::from(*number)));
                    caption.push_str(" · open in the Figures tab");
                    if ui.styled(&caption, TextStyle::LINK).clicked() {
                        *open_figures = true;
                    }
                }
            }
        }
    }

    fn editor_tab(&mut self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.label("File:");
            ui.text_edit(&mut self.editor_path);
            ui.tooltip("A path in the shared files, such as /home/you/script.m");
            if ui.button(&label(icons::FOLDER_OPEN, "Open")).clicked() {
                act.push(Action::Open);
            }
            if ui.button(&label(icons::FLOPPY_DISK, "Save")).clicked() {
                act.push(Action::Save { run_after: false });
            }
            ui.enabled(self.job.is_none(), |ui| {
                if ui.button(&label(icons::PLAY, "Run")).clicked() {
                    act.push(Action::RunEditor);
                }
                ui.tooltip(
                    "Save the file and run it. Without a file, run the text as a script, \
                     from a temporary copy.",
                );
            });
            if ui.button("New").clicked() {
                act.push(Action::NewFile);
            }
        });
        if !self.editor_status.is_empty() {
            ui.styled(&self.editor_status, TextStyle::WEAK);
        }
        // The file scrolls by itself, under its toolbar, rather than the whole window.
        // An edit since the error was marked may have moved it, or fixed it.
        if self
            .editor_error
            .as_ref()
            .is_some_and(|e| e.version != self.editor.version())
        {
            self.editor_error = None;
        }
        if let Some(e) = &self.editor_error {
            let mut at = String::from("Line ");
            at.push_str(&itoa(u64::from(e.line)));
            if e.column > 0 {
                at.push_str(", column ");
                at.push_str(&itoa(u64::from(e.column)));
            }
            at.push_str(": ");
            at.push_str(&e.message);
            ui.styled(&label(icons::X_CIRCLE, &at), TextStyle::STRONG);
        }
        let mark = self.editor_error.as_ref().map(|e| (e.line, e.column));
        ui.scroll(|ui| match mark {
            Some((line, column)) => {
                ui.code_editor_marked(&mut self.editor, CodeLang::Octave, line, column)
            }
            None => ui.code_editor(&mut self.editor, CodeLang::Octave),
        });
    }

    fn figures_tab(&self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        if self.session.figures.is_empty() {
            ui.styled(
                "No figures. Draw one with plot, for example plot(sin(0:0.1:6)).",
                TextStyle::WEAK,
            );
            return;
        }
        let idle = self.job.is_none();
        let client = self.client.unwrap_or(1);
        ui.scroll(|ui| {
            for fig in &self.session.figures {
                // Octave's own drawing, when there is one for this figure.
                let image = self
                    .rendered
                    .zip(fig.image)
                    .map(|(seq, size)| (figure_url(client, seq, fig.number), size));
                ui.push_id(&itoa(u64::from(fig.number)), |ui| {
                    figure(ui, fig, image, idle, act)
                });
            }
        });
    }

    fn variable_tab(&self, ui: &mut Ui<'_>) {
        let Some(name) = &self.inspected else {
            ui.styled(
                "Pick a variable in the workspace to see all of it here.",
                TextStyle::WEAK,
            );
            return;
        };
        ui.styled(name, TextStyle::heading(2));
        match self.session.variables.iter().find(|v| &v.name == name) {
            Some(var) => {
                let mut about = var.size.clone();
                about.push(' ');
                about.push_str(&var.class);
                if !var.attributes.is_empty() {
                    about.push_str(", ");
                    about.push_str(&var.attributes);
                }
                ui.label(&about);
            }
            None => {
                ui.styled("(no longer in the workspace)", TextStyle::WEAK);
            }
        }
        ui.separator();
        ui.scroll(|ui| {
            for line in &self.inspect_output {
                ui.styled(line, TextStyle::CODE);
            }
        });
    }

    /// Editor: fill it with the file at `loading` once its text arrives.
    fn load(&mut self, ui: &mut Ui<'_>) {
        let Some(path) = self.loading.clone() else {
            return;
        };
        let req = PathReq { path: &path };
        let status = match ui.rpc().get::<ReadFile>(&req) {
            Poll::Pending => return,
            Poll::Ready(file) => {
                self.editor.set(&file.text);
                if file.writable {
                    String::from("Opened.")
                } else {
                    String::from("Opened, read-only: you can run it but not save over it.")
                }
            }
            Poll::Failed(err) => {
                let mut s = String::from("Could not open it: ");
                s.push_str(err.message());
                s
            }
        };
        self.editor_status = status;
        self.loading = None;
        // Opening it again later should read it again, not this copy.
        ui.rpc().invalidate::<ReadFile>(&req);
    }

    /// Editor: see how the save went, and run the file if that was asked for.
    fn saved(&mut self, ui: &mut Ui<'_>) {
        let Some(save) = &self.save else {
            return;
        };
        match ui.rpc().outcome::<WriteFile>(save.call) {
            Poll::Pending => {}
            Poll::Ready(_) => {
                let run_after = save.run_after;
                self.save = None;
                self.editor_status = String::from("Saved.");
                self.refresh_tree = true;
                if run_after {
                    let path = self.editor_path.as_str().trim().to_owned();
                    self.run_file(ui, path);
                }
            }
            Poll::Failed(err) => {
                self.save = None;
                self.editor_status = String::from("Could not save: ");
                self.editor_status.push_str(err.message());
            }
        }
    }

    fn run_file(&mut self, ui: &mut Ui<'_>, path: String) {
        let mut shown = String::from("run ");
        shown.push_str(&path);
        if self.start(ui, Run::File(path.clone()), Target::Console) {
            self.push_log(LineKind::Input, &shown);
            self.tab = Tab::Command;
            self.ran_from_editor(path);
        }
    }

    /// The job just started is the editor's file at `path`, or its unsaved text if empty.
    fn ran_from_editor(&mut self, path: String) {
        if let Some(job) = &mut self.job {
            job.from_editor = Some(path);
        }
        self.editor_error = None;
    }

    fn recall(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let back = match (older, self.recall) {
            (true, None) => 0,
            (true, Some(n)) => (n + 1).min(self.history.len() - 1),
            (false, None | Some(0)) => {
                self.recall = None;
                self.input.set("");
                return;
            }
            (false, Some(n)) => n - 1,
        };
        self.recall = Some(back);
        let entry = self.history[self.history.len() - 1 - back].clone();
        self.input.set(&entry);
    }

    fn act(&mut self, ui: &mut Ui<'_>, action: Action) {
        match action {
            Action::Tab(tab) => self.tab = tab,
            Action::Submit if self.asking.is_some() => self.answer(ui),
            Action::Submit => {
                let code = self.input.as_str().to_owned();
                self.submit(ui, &code);
            }
            Action::Older => self.recall(true),
            Action::Newer => self.recall(false),
            Action::ClearLog => self.log.clear(),
            Action::Restart => {
                if self.start(ui, Run::Restart, Target::Console) {
                    self.push_log(LineKind::Input, "(restart)");
                    self.inspected = None;
                    self.inspect_output.clear();
                }
            }
            Action::Recall(i) => {
                if let Some(entry) = self.history.get(i).cloned() {
                    self.input.set(&entry);
                    self.tab = Tab::Command;
                }
            }
            Action::Rerun(i) => {
                if let Some(entry) = self.history.get(i).cloned() {
                    self.submit(ui, &entry);
                    self.tab = Tab::Command;
                }
            }
            Action::Script(entries) => {
                let mut script = String::new();
                for i in entries {
                    if let Some(entry) = self.history.get(i) {
                        script.push_str(entry);
                        script.push('\n');
                    }
                }
                self.editor.set(&script);
                self.editor_path.set("");
                self.editor_status =
                    String::from("A new script from the history. Save it to keep it.");
                self.tab = Tab::Editor;
            }
            Action::Inspect(name) => {
                let Some(id) = identifier(&name) else {
                    return;
                };
                let mut code = String::from("disp(");
                code.push_str(id);
                code.push(')');
                if self.start(ui, Run::Code(code), Target::Inspect(name.clone())) {
                    self.inspected = Some(name);
                    self.inspect_output.clear();
                    self.tab = Tab::Variable;
                }
            }
            Action::Open => {
                let path = self.editor_path.as_str().trim().to_owned();
                if path.is_empty() {
                    self.editor_status = String::from("Type the file's path first.");
                } else {
                    self.editor_status = String::from("Opening…");
                    self.loading = Some(path);
                }
            }
            Action::Save { run_after } => {
                let path = self.editor_path.as_str().trim().to_owned();
                if path.is_empty() {
                    self.editor_status = String::from("Type a path to save to first.");
                    return;
                }
                let call = ui.rpc().send::<WriteFile>(&WriteFileReq {
                    path: &path,
                    text: self.editor.as_str(),
                    create_only: false,
                });
                self.save = Some(Save { call, run_after });
                self.editor_status = String::from("Saving…");
            }
            Action::RunEditor => {
                if self.editor_path.as_str().trim().is_empty() {
                    // One job for the whole script: the server saves it to a temporary file
                    // and runs that, as it would a saved one. It is not a command typed at the
                    // prompt, so it doesn't go in the history line by line.
                    let code = self.editor.as_str().to_owned();
                    if code.trim().is_empty() {
                        return;
                    }
                    if self.start(ui, Run::Code(code), Target::Console) {
                        self.push_log(LineKind::Input, UNSAVED_RUN);
                        self.tab = Tab::Command;
                        self.ran_from_editor(String::new());
                    }
                } else {
                    self.act(ui, Action::Save { run_after: true });
                }
            }
            Action::NewFile => {
                self.editor.set("");
                self.editor_path.set("");
                self.editor_status.clear();
            }
            Action::Dismiss(id) => self.messages.retain(|m| m.id != id),
            Action::PickerDir(dir) => {
                if let Some(picker) = &mut self.picker {
                    picker.dir = dir;
                    picker.selected = None;
                }
            }
            Action::PickerSelect(path) => {
                if let Some(picker) = &mut self.picker {
                    picker.selected = Some(path);
                }
            }
            Action::PickerAnswer(path) => {
                let client = self.client.unwrap_or(1);
                if let Some(picker) = &mut self.picker
                    && picker.sending.is_none()
                {
                    picker.status.clear();
                    picker.sending = Some(ui.rpc().send::<OctaveAnswer>(&OctaveAnswerReq {
                        client,
                        seq: picker.seq,
                        prompt: picker.prompt,
                        answer: PromptAnswer::File(path.as_deref()),
                    }));
                }
            }
            Action::Extend(minutes) => self.control(ui, OctaveAction::Extend { minutes }),
            Action::Stop => self.control(ui, OctaveAction::Stop),
            Action::Toggle(path) => {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
            }
            Action::Edit(path) => {
                self.editor_path.set(&path);
                self.editor_status = String::from("Opening…");
                self.loading = Some(path);
                self.tab = Tab::Editor;
            }
            Action::Cd(path) => {
                let mut code = String::from("cd ");
                code.push_str(&quoted(&path));
                self.submit(ui, &code);
            }
            Action::SaveFigure(n) => {
                let n = itoa(u64::from(n));
                let mut code = String::from("print(");
                code.push_str(&n);
                code.push_str(", \"figure");
                code.push_str(&n);
                code.push_str(".png\"); printf(\"Saved figure");
                code.push_str(&n);
                code.push_str(".png in %s\\n\", pwd())");
                self.submit(ui, &code);
            }
        }
    }
}

/// Whether `name` matches one of `patterns`: globs with `*` and `?`, ignoring case. No patterns,
/// or `*`, is any file.
pub fn matches(patterns: &[String], name: &str) -> bool {
    fn glob(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => glob(&p[1..], s) || (!s.is_empty() && glob(p, &s[1..])),
            (Some(b'?'), Some(_)) => glob(&p[1..], &s[1..]),
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && glob(&p[1..], &s[1..]),
            _ => false,
        }
    }
    patterns.is_empty()
        || patterns
            .iter()
            .any(|p| p == "*.*" || glob(p.as_bytes(), name.as_bytes()))
}

fn message(ui: &mut Ui<'_>, m: &Message, act: &mut Vec<Action>) {
    let (icon, title) = match m.icon {
        MessageIcon::None => (icons::CHAT_TEXT, "Message"),
        MessageIcon::Error => (icons::X_CIRCLE, "Error"),
        MessageIcon::Warning => (icons::WARNING, "Warning"),
        MessageIcon::Help => (icons::INFO, "Help"),
    };
    let title = if m.title.is_empty() { title } else { &m.title };
    ui.window(title, |ui| {
        ui.horizontal_top(|ui| {
            ui.styled(icon, TextStyle::heading(2));
            ui.vertical(|ui| {
                for line in m.text.lines() {
                    ui.label(line);
                }
            });
        });
        if ui.button("OK").clicked() {
            act.push(Action::Dismiss(m.id));
        }
    });
}

fn picker_window(ui: &mut Ui<'_>, picker: &Picker, act: &mut Vec<Action>) {
    ui.window(&picker.title, |ui| {
        ui.horizontal(|ui| {
            if picker.dir != "/" {
                if ui.button(icons::ARROW_UP).clicked() {
                    let up = match picker.dir.trim_end_matches('/').rfind('/') {
                        Some(0) | None => "/",
                        Some(i) => &picker.dir[..i],
                    };
                    act.push(Action::PickerDir(up.to_owned()));
                }
                ui.tooltip("The folder that holds this one");
            }
            ui.label(&label(icons::FOLDER, &picker.dir));
        });
        ui.separator();
        ui.scroll(
            |ui| match ui.rpc().get::<ListDir>(&ListDirReq { path: &picker.dir }) {
                Poll::Pending => {
                    ui.styled("Loading…", TextStyle::WEAK);
                }
                Poll::Failed(err) => {
                    ui.styled(err.message(), TextStyle::WEAK | TextStyle::ITALIC);
                }
                Poll::Ready(listing) => {
                    let mut entries: Vec<_> = (listing.entries.iter())
                        .filter(|e| !e.name.starts_with('.'))
                        .filter(|e| e.kind == EntryKind::Dir || matches(&picker.patterns, &e.name))
                        .collect();
                    entries.sort_by(|a, b| {
                        (a.kind != EntryKind::Dir)
                            .cmp(&(b.kind != EntryKind::Dir))
                            .then_with(|| a.name.cmp(&b.name))
                    });
                    if entries.is_empty() {
                        ui.styled("Nothing here matches", TextStyle::WEAK | TextStyle::ITALIC);
                    }
                    for e in entries {
                        let path = join(&picker.dir, &e.name);
                        ui.push_id(&e.name, |ui| {
                            if e.kind == EntryKind::Dir {
                                if ui
                                    .selectable(false, &label(icons::FOLDER, &e.name))
                                    .clicked()
                                {
                                    act.push(Action::PickerDir(path));
                                }
                            } else {
                                let here = picker.selected.as_deref() == Some(path.as_str());
                                if ui.selectable(here, &label(icons::FILE, &e.name)).clicked() {
                                    act.push(Action::PickerSelect(path));
                                }
                            }
                        });
                    }
                }
            },
        );
        if !picker.status.is_empty() {
            ui.styled(&picker.status, TextStyle::WEAK);
        }
        ui.separator();
        ui.horizontal(|ui| {
            if !picker.patterns.is_empty() {
                let mut patterns = String::from("Showing ");
                patterns.push_str(&picker.patterns.join(", "));
                ui.styled(&patterns, TextStyle::WEAK);
            }
            let idle = picker.sending.is_none();
            ui.enabled(idle && picker.selected.is_some(), |ui| {
                if ui.button(&label(icons::FOLDER_OPEN, "Open")).clicked() {
                    act.push(Action::PickerAnswer(picker.selected.clone()));
                }
            });
            ui.enabled(idle, |ui| {
                if ui.button("Cancel").clicked() {
                    act.push(Action::PickerAnswer(None));
                }
            });
        });
    });
}

/// The widest a figure's image is shown. Octave's default figure prints narrower than this.
const MAX_IMAGE_WIDTH: u32 = 640;

/// The widest a figure is shown in the command window, where it sits among lines of text.
const MAX_INLINE_WIDTH: u32 = 480;

/// `w` by `h`, scaled down to `max` wide if it is wider, keeping its shape.
fn fit(w: u32, h: u32, max: u32) -> (u32, u32) {
    if w > max { (max, h * max / w) } else { (w, h) }
}

/// One figure: Octave's `image` of it (its URL and pixel size) when there is one, else its
/// lines drawn here. Its Save PNG is a job, so it is offered only when `idle`.
fn figure(
    ui: &mut Ui<'_>,
    fig: &Figure,
    image: Option<(String, (u32, u32))>,
    idle: bool,
    act: &mut Vec<Action>,
) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            let mut title = String::from("Figure ");
            title.push_str(&itoa(u64::from(fig.number)));
            if !fig.name.is_empty() {
                title.push_str(": ");
                title.push_str(&fig.name);
            }
            ui.styled(&title, TextStyle::heading(3));
            ui.enabled(idle, |ui| {
                if ui.button(&label(icons::FLOPPY_DISK, "Save PNG")).clicked() {
                    act.push(Action::SaveFigure(fig.number));
                }
                ui.tooltip("Save it as an image in Octave's working folder");
            });
        });
        if let Some((url, (w, h))) = image {
            let (w, h) = fit(w, h, MAX_IMAGE_WIDTH);
            ui.image(&url, Vec2::new(w as f32, h as f32));
            return;
        }
        if fig.axes.is_empty() {
            ui.styled("(nothing plotted)", TextStyle::WEAK);
        }
        for (i, axes) in fig.axes.iter().enumerate() {
            ui.push_id(&itoa(i as u64), |ui| draw_axes(ui, axes));
        }
    });
}

fn draw_axes(ui: &mut Ui<'_>, axes: &Axes) {
    if !axes.title.is_empty() {
        ui.styled(&axes.title, TextStyle::STRONG);
    }
    if axes.series.is_empty() {
        ui.styled(
            "(no lines: only line plots are drawn here; Save PNG shows everything)",
            TextStyle::WEAK,
        );
        return;
    }
    let mut top = String::from("y ");
    top.push_str(&axes.y_max);
    ui.styled(&top, TextStyle::WEAK);
    for (i, series) in axes.series.iter().enumerate() {
        ui.push_id(&itoa(i as u64), |ui| {
            // One graph per line, since `Ui::plot` draws a single series. They share the axes'
            // range, so they still compare by eye.
            ui.plot(&series.samples, Vec2::new(0.0, 90.0));
            if !series.label.is_empty() {
                ui.styled(&series.label, TextStyle::ITALIC);
            }
        });
    }
    let mut bottom = String::from("y ");
    bottom.push_str(&axes.y_min);
    bottom.push_str("    x ");
    bottom.push_str(&axes.x_min);
    bottom.push_str(" … ");
    bottom.push_str(&axes.x_max);
    ui.styled(&bottom, TextStyle::WEAK);
}

impl App for Octave {
    fn update(&mut self, ui: &mut Ui<'_>) {
        if self.client.is_none() {
            // Any number another window is unlikely to have picked. The clock will do.
            self.client = Some((ui.ctx().time_ms as u64 as u32) | 1);
            // The session may well exist already (another window, a reload), and even a new
            // one has a folder: ask, rather than show nothing until the first command. It
            // also finds out at once whether the server has Octave at all.
            if !self.synced {
                self.start(ui, Run::Code(String::new()), Target::Sync);
            }
        }
        self.controlled(ui);
        self.answered(ui);
        self.input_answered(ui);
        self.follow(ui);
        self.load(ui);
        self.saved(ui);
        self.invalidate_tree(ui);
        self.render(ui);

        if self.unavailable {
            ui.styled("Octave isn't installed on this server.", TextStyle::STRONG);
            ui.label(
                "Ask whoever runs the server to install GNU Octave (octave-cli), or to run \
                 the Docker image built with Octave included.",
            );
            return;
        }
        let mut act = Vec::new();
        self.toolbar(ui, &mut act);
        ui.horizontal_top(|ui| {
            ui.side_column(|ui| self.sidebar(ui, &mut act));
            ui.vertical(|ui| match self.tab {
                Tab::Command => self.command_window(ui, &mut act),
                Tab::Editor => self.editor_tab(ui, &mut act),
                Tab::Figures => self.figures_tab(ui, &mut act),
                Tab::Variable => self.variable_tab(ui),
            });
        });
        self.dialogs(ui, &mut act);
        for a in act {
            self.act(ui, a);
        }
    }

    fn wants_repaint_after_ms(&self) -> u32 {
        if self.job.is_some() {
            POLL_MS
        } else {
            ccosel_sdk::REPAINT_ON_INPUT_ONLY
        }
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(Octave);
