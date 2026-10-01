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
    Axes, Figure, OctaveInput, OctavePoll, OctavePollReq, OctaveResult, OctaveRun, OctaveRunReq,
    OctaveStatus,
};
use ccosel_sdk::{App, CallId, Poll, Text, TextStyle, Ui, Vec2};

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
}

/// Where a job's output goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Console,
    /// Displaying one variable for the variable view: its output is not the log's business.
    Inspect(String),
}

/// What to run, owned, until it is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Run {
    Code(String),
    File(String),
    Restart,
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
            }
        }
    }

    fn finish(&mut self, status: &OctaveStatus) {
        self.take_output(&status.output, true);
        if status.output_truncated {
            self.push_log(
                LineKind::Error,
                "error: output past the server's limit was dropped",
            );
        }
        if let Some(result) = &status.result {
            self.session = result.clone();
        }
        self.job = None;
        self.refresh_tree = true;
    }

    fn fail(&mut self, what: &str, err: &str) {
        self.job = None;
        let mut msg = String::from("error: ");
        msg.push_str(what);
        msg.push_str(err);
        self.push_log(LineKind::Error, &msg);
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
                if let Some(job) = &mut self.job {
                    job.received = status.next;
                }
                if status.finished {
                    self.finish(&status);
                } else {
                    self.take_output(&status.output, false);
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
            ui.label("📁");
            let cwd = if self.session.cwd.is_empty() {
                "(outside the shared files)"
            } else {
                self.session.cwd.as_str()
            };
            ui.label(cwd);
            ui.tooltip("Octave's working folder. Change it with cd at the prompt.");
            if ui.button("⟳ Restart").clicked() {
                act.push(Action::Restart);
            }
            ui.tooltip("Start a fresh Octave session: every variable and figure is cleared");
            if self.job.is_some() {
                ui.label("⏳ Running…");
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
                if !cwd.is_empty() && cwd != "/" && ui.button("⬆").clicked() {
                    act.push(Action::Cd("..".to_owned()));
                }
                ui.tooltip("Go up to the folder that holds this one");
            });
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
                    let mut text = String::from(if open { "📂 " } else { "📁 " });
                    text.push_str(&entry.name);
                    ui.horizontal(|ui| {
                        if ui.selectable(false, &text).clicked() {
                            act.push(Action::Toggle(path.clone()));
                        }
                        ui.tooltip("Open or close this folder");
                        if ui.button("cd").clicked() {
                            let rel = path.strip_prefix(&self.session.cwd).unwrap_or(&path);
                            act.push(Action::Cd(rel.trim_start_matches('/').to_owned()));
                        }
                        ui.tooltip("Make this Octave's working folder");
                    });
                    if open && depth + 1 < TREE_DEPTH {
                        ui.indent(|ui| self.tree(ui, &path, depth + 1, act));
                    }
                } else if is_script(&entry.name) {
                    let mut text = String::from("📜 ");
                    text.push_str(&entry.name);
                    if ui.selectable(editing == path, &text).clicked() {
                        act.push(Action::Edit(path.clone()));
                    }
                    ui.tooltip("Open it in the editor");
                } else {
                    let mut text = String::from("📄 ");
                    text.push_str(&entry.name);
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
        for var in &self.session.variables {
            ui.push_id(&var.name, |ui| {
                let mut row = var.name.clone();
                row.push_str("  ");
                row.push_str(&var.size);
                row.push(' ');
                row.push_str(&var.class);
                let selected = self.inspected.as_deref() == Some(var.name.as_str());
                if ui.selectable(selected, &row).clicked() {
                    act.push(Action::Inspect(var.name.clone()));
                }
                if !var.attributes.is_empty() {
                    ui.tooltip(&var.attributes);
                }
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
        if ui.button("📝 Create script").clicked() {
            act.push(Action::Script(matching.iter().rev().copied().collect()));
        }
        ui.tooltip("Open the commands listed here in the editor, oldest first");
        for &i in &matching {
            ui.push_id(&itoa(i as u64), |ui| {
                ui.horizontal(|ui| {
                    if ui.button("▶").clicked() {
                        act.push(Action::Rerun(i));
                    }
                    ui.tooltip("Run it again");
                    let first = self.history[i].lines().next().unwrap_or_default();
                    if ui.selectable(false, first).clicked() {
                        act.push(Action::Recall(i));
                    }
                    ui.tooltip("Put it back in the prompt");
                });
            });
        }
    }

    fn command_window(&mut self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        let log = &self.log;
        let input = &mut self.input;
        ui.scroll_with_footer(
            |ui| Self::console(ui, log),
            |ui| {
                ui.label(">>");
                if ui.text_edit(input).submitted() {
                    act.push(Action::Submit);
                }
                ui.tooltip("Type Octave code here and press Enter to run it");
                if ui.button("⏎ Run").clicked() {
                    act.push(Action::Submit);
                }
                if ui.button("▲").clicked() {
                    act.push(Action::Older);
                }
                ui.tooltip("The previous command");
                if ui.button("▼").clicked() {
                    act.push(Action::Newer);
                }
                ui.tooltip("The next command");
                if ui.button("Clear").clicked() {
                    act.push(Action::ClearLog);
                }
                ui.tooltip("Clear the command window. Variables are kept.");
            },
        );
    }

    /// The command window's output, oldest first.
    fn console(ui: &mut Ui<'_>, log: &[(LineKind, String)]) {
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
            }
        }
    }

    fn editor_tab(&mut self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.label("File:");
            ui.text_edit(&mut self.editor_path);
            ui.tooltip("A path in the shared files, such as /home/you/script.m");
            if ui.button("📂 Open").clicked() {
                act.push(Action::Open);
            }
            if ui.button("💾 Save").clicked() {
                act.push(Action::Save { run_after: false });
            }
            if ui.button("▶ Run").clicked() {
                act.push(Action::RunEditor);
            }
            ui.tooltip("Save the file and run it, or just run the code if it has no file");
            if ui.button("New").clicked() {
                act.push(Action::NewFile);
            }
        });
        if !self.editor_status.is_empty() {
            ui.styled(&self.editor_status, TextStyle::WEAK);
        }
        // The file scrolls by itself, under its toolbar, rather than the whole window.
        ui.scroll(|ui| ui.text_edit_multiline(&mut self.editor));
    }

    fn figures_tab(&self, ui: &mut Ui<'_>, act: &mut Vec<Action>) {
        if self.session.figures.is_empty() {
            ui.styled(
                "No figures. Draw one with plot, for example plot(sin(0:0.1:6)).",
                TextStyle::WEAK,
            );
            return;
        }
        ui.scroll(|ui| {
            for fig in &self.session.figures {
                ui.push_id(&itoa(u64::from(fig.number)), |ui| figure(ui, fig, act));
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
        if self.start(ui, Run::File(path), Target::Console) {
            self.push_log(LineKind::Input, &shown);
            self.tab = Tab::Command;
        }
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
                    let code = self.editor.as_str().to_owned();
                    self.submit(ui, &code);
                    self.tab = Tab::Command;
                } else {
                    self.act(ui, Action::Save { run_after: true });
                }
            }
            Action::NewFile => {
                self.editor.set("");
                self.editor_path.set("");
                self.editor_status.clear();
            }
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

fn figure(ui: &mut Ui<'_>, fig: &Figure, act: &mut Vec<Action>) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            let mut title = String::from("Figure ");
            title.push_str(&itoa(u64::from(fig.number)));
            if !fig.name.is_empty() {
                title.push_str(": ");
                title.push_str(&fig.name);
            }
            ui.styled(&title, TextStyle::heading(3));
            if ui.button("💾 Save PNG").clicked() {
                act.push(Action::SaveFigure(fig.number));
            }
            ui.tooltip("Save it as an image in Octave's working folder");
        });
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
        }
        self.follow(ui);
        self.load(ui);
        self.saved(ui);
        self.invalidate_tree(ui);

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
