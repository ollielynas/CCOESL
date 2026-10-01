//! The Octave app's back end: one long-lived GNU Octave process per user, running jobs.
//!
//! A session is an `octave-cli` reading commands on its stdin, so variables and figures survive
//! from one command to the next the way they do at a real prompt. Each job the app sends is
//! written to a script file and run through `__ccosel_run__` (in `octave/`), which runs it in
//! the base workspace inside a `try`, then prints a report of the session's state: working
//! folder, variables, figures. Report lines start with a tag made fresh for each job, which is
//! how the server tells them apart from what the code printed, and knows the job is over.
//!
//! Like `build_api`, a job outlives the request that started it: `run` starts it and returns
//! at once, `poll` reports how far it has got.
//!
//! **Running Octave code is running arbitrary code as the server's user**, with its files and
//! network, exactly like building a project in the Compiler (see `ARCHITECTURE.md`). Octave
//! is optional for that reason as much as for its size: without it, `run` answers `false`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ccosel_proto::octave::{
    Axes, DEFAULT_LIMIT_MS, Figure, MAX_LIMIT_MS, MAX_OUTPUT_BYTES, OctaveAction, OctaveControlReq,
    OctaveInput, OctavePollReq, OctaveResult, OctaveRunReq, OctaveStatus, Series, Variable,
};
use ccosel_proto::server_error;

use crate::access;
use crate::fs_api::{Jail, Need};

/// The helper every session loads. Written into the support folder when the server starts, so
/// the server binary is all that needs deploying.
const RUNNER: &str = include_str!("octave/__ccosel_run__.m");

/// A job still running after this is not coming back, unless its user said it would take
/// longer (`OctaveControl`). The session is killed to stop it, which loses its variables; the
/// app says so.
const JOB_TIMEOUT: Duration = Duration::from_millis(DEFAULT_LIMIT_MS);

/// How long Octave gets to come back to its prompt after being interrupted, before it is
/// killed instead. Octave notices an interrupt between operations, so this is usually instant;
/// one long builtin call can hold it up.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// How often a running job's wait looks up from Octave's output to see whether its limit moved
/// or it was asked to stop.
const TICK: Duration = Duration::from_millis(100);

/// A session nobody has used for this long is stopped, so an abandoned window doesn't keep an
/// Octave process (and whatever it allocated) alive forever.
const SESSION_IDLE: Duration = Duration::from_secs(30 * 60);

/// How long a new Octave may take to be ready for its first job.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// Finished jobs older than this are forgotten. Long enough for an app that stopped polling
/// (occluded window, dropped link) to come back for its result.
const JOB_RETENTION: Duration = Duration::from_secs(10 * 60);

/// Points per plotted line. `Ui::plot` draws one byte a point, so this is the cost of a line.
const SAMPLES: usize = 120;

const MAX_VARIABLES: usize = 200;
const MAX_FIGURES: usize = 8;
const MAX_AXES: usize = 8;
const MAX_SERIES: usize = 16;

/// Runs Octave jobs for every user of this server.
pub struct Octave {
    /// The Octave program and its arguments, or `None` when this server has no Octave.
    program: Option<Vec<String>>,
    /// Holds `__ccosel_run__.m`, and a folder per session for the scripts it runs.
    support: PathBuf,
    timeout: Duration,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    jobs: Mutex<HashMap<JobKey, Arc<Job>>>,
}

/// `(user, client, seq)`: the user, so one person can't read another's output by guessing
/// numbers, and then the app's own name for the job.
type JobKey = (String, u32, u32);

impl Octave {
    /// Finds Octave: `$CCOSEL_OCTAVE` if set (empty turns Octave off), else `octave-cli` on the
    /// `PATH`. `support` is a folder the server may fill and empty as it likes.
    pub fn detect(support: PathBuf) -> Self {
        let program = match std::env::var("CCOSEL_OCTAVE") {
            Ok(p) if p.is_empty() => None,
            Ok(p) => Some(p),
            Err(_) => on_path("octave-cli").map(|p| p.display().to_string()),
        };
        let program = program.map(|p| {
            [
                p.as_str(),
                "--quiet",
                "--norc",
                "--no-history",
                "--no-window-system",
            ]
            .map(str::to_owned)
            .to_vec()
        });
        Self::with_program(program, support, JOB_TIMEOUT)
    }

    /// Octave as `program` (the command and its arguments), or none at all. For tests, which
    /// stand in a recording for Octave and shorten the time limit.
    pub fn with_program(program: Option<Vec<String>>, support: PathBuf, timeout: Duration) -> Self {
        if program.is_some() {
            let written = std::fs::create_dir_all(&support)
                .and_then(|()| std::fs::write(support.join("__ccosel_run__.m"), RUNNER));
            if let Err(e) = written {
                eprintln!("octave: writing {}: {e}", support.display());
            }
        }
        Self {
            program,
            support,
            timeout,
            sessions: Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
        }
    }

    pub fn available(&self) -> bool {
        self.program.is_some()
    }

    /// Start job `(client, seq)` for `user`, unless it already exists. `Ok(false)` means there
    /// is no Octave to run it with.
    pub fn run(
        &self,
        jail: &Jail,
        user: Option<&str>,
        req: &OctaveRunReq<'_>,
    ) -> Result<bool, u32> {
        let Some(program) = &self.program else {
            return Ok(false);
        };
        let who = user.unwrap_or_default().to_owned();
        // Resolved now, not in the job, so a path the caller may not read fails the call
        // itself rather than turning up later as job output.
        let input = match req.input {
            OctaveInput::Code(code) => Input::Code(code.to_owned()),
            OctaveInput::File(path) => Input::File(jail.authorize(path, user, Need::Read)?),
            OctaveInput::Restart => Input::Restart,
        };

        let key = (who.clone(), req.client, req.seq);
        let mut jobs = self.jobs.lock().unwrap();
        jobs.retain(|_, job| match *job.finished_at.lock().unwrap() {
            Some(at) => at.elapsed() < JOB_RETENTION,
            None => true,
        });
        if jobs.contains_key(&key) {
            return Ok(true);
        }

        let session = self.session(&who, jail, user);
        let job = Arc::new(Job::new(self.timeout));
        jobs.insert(key, job.clone());
        drop(jobs);

        let program = program.clone();
        let jail_root = jail.root().to_path_buf();
        std::thread::spawn(move || {
            let result = session.execute(&program, &jail_root, input, &job);
            *job.result.lock().unwrap() = Some(result);
            *job.finished_at.lock().unwrap() = Some(Instant::now());
        });
        Ok(true)
    }

    /// How job `(client, seq)` is going, with its output from byte `from` on.
    pub fn poll(&self, user: Option<&str>, req: &OctavePollReq) -> Result<OctaveStatus, u32> {
        let key = (user.unwrap_or_default().to_owned(), req.client, req.seq);
        let job = self
            .jobs
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .ok_or(server_error::NOT_FOUND)?;
        Ok(job.snapshot(req.from as usize))
    }

    /// Move job `(client, seq)`'s time limit, or stop it. Replies with its limit afterwards.
    pub fn control(&self, user: Option<&str>, req: &OctaveControlReq) -> Result<u64, u32> {
        let key = (user.unwrap_or_default().to_owned(), req.client, req.seq);
        let job = self
            .jobs
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .ok_or(server_error::NOT_FOUND)?;
        let mut limit = job.limit.lock().unwrap();
        if job.result.lock().unwrap().is_none() {
            match req.action {
                OctaveAction::Extend { minutes } => {
                    let more = Duration::from_secs(u64::from(minutes) * 60);
                    *limit = (*limit + more).min(Duration::from_millis(MAX_LIMIT_MS));
                }
                OctaveAction::Stop => job.stop.store(true, Ordering::Relaxed),
            }
        }
        Ok(limit.as_millis() as u64)
    }

    /// The caller's session, made if need be, after stopping any that have sat idle too long.
    fn session(&self, who: &str, jail: &Jail, user: Option<&str>) -> Arc<Session> {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|name, s| name == who || !s.idle());
        sessions
            .entry(who.to_owned())
            .or_insert_with(|| {
                // Your own folder when you have one, which is also where `print` and `save`
                // put their files by default. Anonymous callers start at the top.
                let cwd = user
                    .and_then(|u| access::ensure_home(jail.root(), u).ok())
                    .unwrap_or_else(|| jail.root().to_path_buf());
                let n = sessions_made();
                Arc::new(Session::new(cwd, self.support.join(format!("session-{n}"))))
            })
            .clone()
    }
}

/// Numbers session folders, so two sessions never share one.
fn sessions_made() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// `name` on the `PATH`, if it is there.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

enum Input {
    Code(String),
    File(PathBuf),
    Restart,
}

struct Job {
    started: Instant,
    /// When, from `started`, the job is stopped for running too long.
    limit: Mutex<Duration>,
    /// Its user asked for it to be interrupted.
    stop: AtomicBool,
    output: Mutex<Output>,
    result: Mutex<Option<OctaveResult>>,
    finished_at: Mutex<Option<Instant>>,
}

#[derive(Default)]
struct Output {
    text: String,
    truncated: bool,
}

impl Job {
    fn new(limit: Duration) -> Self {
        Self {
            started: Instant::now(),
            limit: Mutex::new(limit),
            stop: AtomicBool::new(false),
            output: Mutex::new(Output::default()),
            result: Mutex::new(None),
            finished_at: Mutex::new(None),
        }
    }

    fn push_line(&self, line: &str) {
        let mut out = self.output.lock().unwrap();
        if out.truncated {
            return;
        }
        if out.text.len() + line.len() + 1 > MAX_OUTPUT_BYTES {
            out.truncated = true;
            return;
        }
        out.text.push_str(line);
        out.text.push('\n');
    }

    fn snapshot(&self, from: usize) -> OctaveStatus {
        // Result first: once it is set, the output is complete, so a snapshot that says
        // `finished` never misses the last lines.
        let result = self.result.lock().unwrap().clone();
        let out = self.output.lock().unwrap();
        let mut from = from.min(out.text.len());
        while !out.text.is_char_boundary(from) {
            from -= 1;
        }
        OctaveStatus {
            finished: result.is_some(),
            elapsed_ms: self.started.elapsed().as_millis() as u64,
            limit_ms: self.limit.lock().unwrap().as_millis() as u64,
            output: out.text[from..].to_owned(),
            next: out.text.len() as u32,
            output_truncated: out.truncated,
            result,
        }
    }
}

/// One user's Octave.
struct Session {
    /// Where a new process starts.
    cwd: PathBuf,
    /// This session's scripts: what was typed is written here to be run.
    dir: PathBuf,
    /// Held while a job runs, so jobs run one at a time, in order.
    process: Mutex<Option<Process>>,
    last_used: Mutex<Instant>,
}

struct Process {
    child: Child,
    stdin: ChildStdin,
    /// stdout and stderr, a line at a time. Disconnects when the process exits.
    lines: Receiver<String>,
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Session {
    fn new(cwd: PathBuf, dir: PathBuf) -> Self {
        Self {
            cwd,
            dir,
            process: Mutex::new(None),
            last_used: Mutex::new(Instant::now()),
        }
    }

    /// Unused for long enough to stop, and not running anything now.
    fn idle(&self) -> bool {
        self.last_used.lock().unwrap().elapsed() > SESSION_IDLE && self.process.try_lock().is_ok()
    }

    fn execute(
        &self,
        program: &[String],
        jail_root: &Path,
        input: Input,
        job: &Job,
    ) -> OctaveResult {
        let mut process = self.process.lock().unwrap();
        *self.last_used.lock().unwrap() = Instant::now();

        let (kind, script) = match input {
            Input::Restart => {
                // Dropping the process kills it; an empty script then reports the fresh one.
                *process = None;
                ("code", self.write_script(""))
            }
            Input::Code(code) => ("code", self.write_script(&code)),
            Input::File(path) => ("file", Ok(path)),
        };
        let script = match script {
            Ok(s) => s,
            Err(e) => return failed(job, &format!("could not write the script: {e}")),
        };

        if process.is_none() {
            match self.start(program) {
                Ok(p) => *process = Some(p),
                Err(e) => return failed(job, &format!("could not start Octave: {e}")),
            }
        }
        let p = process.as_mut().expect("started above");

        let tag = job_tag();
        let command = format!(
            "__ccosel_run__(\"{kind}\", \"{}\", \"{tag}\")\n",
            octave_string(&script.display().to_string())
        );
        if p.stdin.write_all(command.as_bytes()).is_err() || p.stdin.flush().is_err() {
            *process = None;
            return ended(
                job,
                "Octave has stopped. The next command starts a new session.",
            );
        }

        let mut tag = tag;
        let mut report = Vec::new();
        // Once interrupted: when, and the tag the interrupted run would have reported with.
        let mut stopped: Option<(Instant, String)> = None;
        loop {
            let now = Instant::now();
            if let Some((at, _)) = &stopped
                && now >= *at + STOP_GRACE
            {
                *process = None;
                return ended(
                    job,
                    "Octave didn't stop when asked, so the session was restarted and its \
                     variables are gone.",
                );
            }
            // Read each time round: the user can move it while the job runs.
            let deadline = job.started + *job.limit.lock().unwrap();
            if now >= deadline {
                *process = None;
                return ended(
                    job,
                    "Stopped: this ran past its time limit. The session was restarted, so its \
                     variables are gone. Extend the limit while it runs if it needs longer.",
                );
            }
            if stopped.is_none() && job.stop.load(Ordering::Relaxed) {
                // Interrupt it, then ask for a report. The interrupt abandons the job's whole
                // command line, report and all, and Octave goes on to read the next one. That
                // report is under a new tag, so whatever the interrupted run already printed
                // under the old one can't be taken for it.
                interrupt(&p.child);
                let fresh = job_tag();
                let command = self.write_script("").map(|script| {
                    format!(
                        "__ccosel_run__(\"code\", \"{}\", \"{fresh}\")\n",
                        octave_string(&script.display().to_string())
                    )
                });
                let sent = command.is_ok_and(|c| {
                    p.stdin.write_all(c.as_bytes()).is_ok() && p.stdin.flush().is_ok()
                });
                if !sent {
                    *process = None;
                    return ended(
                        job,
                        "Octave has stopped. The next command starts a new session.",
                    );
                }
                report.clear();
                stopped = Some((now, core::mem::replace(&mut tag, fresh)));
            }
            let wait = deadline.saturating_duration_since(now).min(TICK);
            match p.lines.recv_timeout(wait) {
                Ok(line) => match line.strip_prefix(&tag) {
                    Some(" DONE") => break,
                    Some(rest) => report.push(rest.trim_start().to_owned()),
                    None => match &stopped {
                        Some((_, old)) if line.starts_with(old.as_str()) => {}
                        _ => job.push_line(&line),
                    },
                },
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    *process = None;
                    return ended(
                        job,
                        "Octave has stopped. The next command starts a new session.",
                    );
                }
            }
        }
        *self.last_used.lock().unwrap() = Instant::now();
        let mut result = parse_report(&report, jail_root);
        if stopped.is_some() {
            job.push_line("error: Stopped. Variables it set before stopping are kept.");
            result.error = true;
        }
        result
    }

    fn write_script(&self, code: &str) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join("cell.m");
        std::fs::write(&path, code)?;
        Ok(path)
    }

    fn start(&self, program: &[String]) -> std::io::Result<Process> {
        let (cmd, args) = program.split_first().expect("a program has a command");
        let mut child = Command::new(cmd)
            .args(args)
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let (tx, lines) = channel();
        forward_lines(child.stdout.take().expect("piped"), tx.clone());
        forward_lines(child.stderr.take().expect("piped"), tx);
        let mut stdin = child.stdin.take().expect("piped");

        let support = self.dir.parent().unwrap_or(&self.dir).display().to_string();
        // No pager (it would wait for a key nobody can press) and figures drawn off screen.
        // Then one figure opened and closed: the gnuplot toolkit prints a long warning the
        // first time it draws, one no `warning("off", id)` reaches and nobody using this app
        // can act on, so it is set off here and thrown away with the rest of the start-up.
        let tag = job_tag();
        let init = format!(
            "more off\n\
             addpath(\"{}\")\n\
             set(0, \"defaultfigurevisible\", \"off\")\n\
             try, close(figure()), end\n\
             printf(\"%s\\n\", \"{tag}\")\n",
            octave_string(&support)
        );
        stdin.write_all(init.as_bytes())?;
        stdin.flush()?;
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match lines.recv_timeout(left) {
                Ok(line) if line == tag => break,
                Ok(_) => {}
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(std::io::Error::other("Octave did not start"));
                }
            }
        }
        Ok(Process {
            child,
            stdin,
            lines,
        })
    }
}

/// Interrupt what Octave is running, as Ctrl-C at its prompt would. It stays running, with its
/// variables, and goes on to read its next command.
#[cfg(unix)]
fn interrupt(child: &Child) {
    // SAFETY: `kill` only sends a signal; the pid is our own child, which `Process` reaps
    // only when it is dropped, so the pid can't have been reused by now.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGINT);
    }
}

/// Elsewhere there is no interrupt to send: the job runs on, and the grace period ends it.
#[cfg(not(unix))]
fn interrupt(_child: &Child) {}

/// Reads `from` a line at a time into `tx`, on its own thread, until it closes.
fn forward_lines(from: impl Read + Send + 'static, tx: Sender<String>) {
    std::thread::spawn(move || {
        for line in BufReader::new(from).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
}

/// A job's tag: unpredictable, so no output can end a job early or forge its report.
fn job_tag() -> String {
    format!("@ccosel-{:016x}", rand::random::<u64>())
}

/// `s` inside a double-quoted Octave string.
fn octave_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn failed(job: &Job, message: &str) -> OctaveResult {
    job.push_line(&format!("error: {message}"));
    OctaveResult {
        error: true,
        ..OctaveResult::default()
    }
}

fn ended(job: &Job, message: &str) -> OctaveResult {
    job.push_line(&format!("error: {message}"));
    OctaveResult {
        error: true,
        ended: true,
        ..OctaveResult::default()
    }
}

/// A line as Octave reported it: legend entry, x, y.
type RawLine = (String, Vec<f64>, Vec<f64>);

/// Turn `__ccosel_run__`'s report (its lines, tag removed) into the reply's result.
pub fn parse_report(lines: &[String], jail_root: &Path) -> OctaveResult {
    let mut result = OctaveResult::default();
    // Lines are scaled against their whole axes, so they are gathered first and scaled once
    // the report is read: `(figure, title, lines)` per axes.
    let mut raw: Vec<(usize, String, Vec<RawLine>)> = Vec::new();
    for line in lines {
        let mut fields = line.split('\t');
        let mut next = || fields.next().unwrap_or_default().to_owned();
        match next().as_str() {
            "STATUS" => {
                result.error = next() == "1";
                result.cwd = jail_path(Path::new(&next()), jail_root);
            }
            "VAR" => {
                if result.variables.len() == MAX_VARIABLES {
                    result.variables_truncated = true;
                    continue;
                }
                result.variables.push(Variable {
                    name: next(),
                    class: next(),
                    size: next(),
                    value: next(),
                    attributes: next(),
                });
            }
            "FIG" if result.figures.len() < MAX_FIGURES => {
                result.figures.push(Figure {
                    number: next().parse().unwrap_or(0),
                    name: next(),
                    axes: Vec::new(),
                });
            }
            "AXES" if !result.figures.is_empty() => {
                let fig = result.figures.len() - 1;
                if raw.iter().filter(|(f, ..)| *f == fig).count() < MAX_AXES {
                    raw.push((fig, next(), Vec::new()));
                }
            }
            "LINE" => {
                let (label, xs, ys) = (next(), numbers(&next()), numbers(&next()));
                if let Some((_, _, lines)) = raw.last_mut()
                    && lines.len() < MAX_SERIES
                {
                    lines.push((label, xs, ys));
                }
            }
            _ => {}
        }
    }
    for (fig, title, lines) in raw {
        result.figures[fig].axes.push(scale_axes(title, &lines));
    }
    result
}

/// Space-separated numbers as `%.7g` prints them. Anything unreadable is NaN, a gap, so the
/// points either side of it stay where they are.
fn numbers(s: &str) -> Vec<f64> {
    s.split_ascii_whitespace()
        .map(|n| n.parse().unwrap_or(f64::NAN))
        .collect()
}

/// Resample each line to [`SAMPLES`] evenly spaced points and scale them all to one `0..=255`
/// range, so lines in the same axes can be compared by eye.
fn scale_axes(title: String, lines: &[RawLine]) -> Axes {
    let finite = |v: &[f64]| {
        v.iter()
            .copied()
            .filter(|n| n.is_finite())
            .collect::<Vec<_>>()
    };
    let all_x: Vec<f64> = lines.iter().flat_map(|(_, x, _)| finite(x)).collect();
    let all_y: Vec<f64> = lines.iter().flat_map(|(_, _, y)| finite(y)).collect();
    let (x_min, x_max) = range(&all_x);
    let (y_min, y_max) = range(&all_y);
    let series = lines
        .iter()
        .map(|(label, x, y)| Series {
            label: label.clone(),
            samples: resample(x, y)
                .into_iter()
                .map(|v| to_byte(v, y_min, y_max))
                .collect(),
        })
        .collect();
    Axes {
        title,
        x_min: fmt_num(x_min),
        x_max: fmt_num(x_max),
        y_min: fmt_num(y_min),
        y_max: fmt_num(y_max),
        series,
    }
}

fn range(v: &[f64]) -> (f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    let min = v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (min, max)
}

/// `y` at [`SAMPLES`] evenly spaced `x`, by linear interpolation, when `x` only ever goes up
/// (the usual `plot(x, y)`); otherwise `y` in the order it was given, which is still the right
/// shape for `plot(y)` and the best a one-value-per-column graph can do for anything else.
fn resample(x: &[f64], y: &[f64]) -> Vec<f64> {
    let n = x.len().min(y.len());
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![y[0]; SAMPLES];
    }
    let (x, y) = (&x[..n], &y[..n]);
    let rising = x.iter().all(|v| v.is_finite()) && x.windows(2).all(|w| w[0] <= w[1]);
    let (lo, hi) = (x[0], x[n - 1]);
    (0..SAMPLES)
        .map(|k| {
            let t = k as f64 / (SAMPLES - 1) as f64;
            if !rising || hi <= lo {
                return y[((t * (n - 1) as f64).round() as usize).min(n - 1)];
            }
            let at = lo + t * (hi - lo);
            // The first point at or past `at`; `at` lies between it and the one before.
            let i = x.partition_point(|v| *v < at).clamp(1, n - 1);
            let (x0, x1, y0, y1) = (x[i - 1], x[i], y[i - 1], y[i]);
            if x1 == x0 {
                y1
            } else {
                y0 + (y1 - y0) * (at - x0) / (x1 - x0)
            }
        })
        .collect()
}

/// `v` on a `0..=255` scale from `min` to `max`. A flat line sits in the middle; a gap (NaN)
/// can't be drawn by `Ui::plot`, so it sits at the bottom.
fn to_byte(v: f64, min: f64, max: f64) -> u8 {
    if !v.is_finite() {
        return 0;
    }
    if max <= min {
        return 128;
    }
    (((v - min) / (max - min)) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// A number for an axis label: short, like Octave's own `%g`.
fn fmt_num(v: f64) -> String {
    if v == 0.0 {
        return "0".to_owned();
    }
    let a = v.abs();
    if !(1e-3..1e6).contains(&a) {
        return format!("{v:.3e}");
    }
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// `path` as a jail path (`/home/alice`), or empty when it is outside the jail.
fn jail_path(path: &Path, jail_root: &Path) -> String {
    let Ok(rel) = path.strip_prefix(jail_root) else {
        return String::new();
    };
    let mut out = String::new();
    for part in rel.components() {
        out.push('/');
        out.push_str(&part.as_os_str().to_string_lossy());
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

#[cfg(test)]
mod tests;
