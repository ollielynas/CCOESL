//! Runs `cargo build` inside the jail, as a **job** rather than a call.
//!
//! The heaviest thing this server does, and the only thing here that outlives its request: a
//! build takes minutes, so `compile` starts one and returns a snapshot immediately. The client
//! polls. That is what `ARCHITECTURE.md` means by "anything over ~2s is a Job, not an RPC" —
//! minus the push channel, which polling stands in for until a WebSocket exists.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ccosel_proto::build::{BuiltBinary, CompileReq, CompileResult, CompileStatus};
use ccosel_proto::server_error;

use crate::fs_api::Jail;

/// A build that runs this long is not coming back on its own. Killing it is what keeps one
/// runaway `build.rs` from holding a thread and a job slot forever.
const BUILD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Tail-capped like a directory listing is entry-capped: a guest must never be handed an
/// unbounded reply, and a build failure's own error is usually the last thing cargo printed.
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Finished jobs older than this are dropped. Long enough that an app which stopped polling
/// (occluded window, reconnect) can still come back for its result.
const JOB_RETENTION: Duration = Duration::from_secs(30 * 60);

/// What a build reports about itself while it runs.
#[derive(Default)]
struct Progress {
    units_done: u32,
    units_total: u32,
    current: String,
}

struct Job {
    started: Instant,
    progress: Mutex<Progress>,
    /// `Some` once the build has finished, one way or the other.
    result: Mutex<Option<CompileResult>>,
    finished_at: Mutex<Option<Instant>>,
}

/// Every build this server has run, keyed by what the client asked for.
///
/// Keyed by `(resolved dir, generation)` rather than a server-minted id: the client's own
/// idempotency key *is* the handle, so a dropped reply costs a retry rather than a lost job.
#[derive(Default)]
pub struct Jobs {
    map: Mutex<HashMap<(PathBuf, u32), Arc<Job>>>,
}

impl Jobs {
    pub fn new() -> Self {
        Self::default()
    }

    fn sweep(map: &mut HashMap<(PathBuf, u32), Arc<Job>>) {
        map.retain(|_, job| {
            let finished = *job.finished_at.lock().unwrap();
            match finished {
                Some(at) => at.elapsed() < JOB_RETENTION,
                None => true,
            }
        });
    }
}

/// Start the job if this is the first time we have seen `(path, generation)`, then report where
/// it has got to. Never waits for the build.
pub fn compile(jail: &Jail, jobs: &Jobs, req: &CompileReq<'_>) -> Result<CompileStatus, u32> {
    let dir = jail.resolve(req.path)?;

    let meta = std::fs::metadata(&dir).map_err(|_| server_error::IO)?;
    if !meta.is_dir() {
        return Err(server_error::NOT_A_DIRECTORY);
    }
    let manifest = dir.join("Cargo.toml");
    if !manifest.is_file() {
        return Err(server_error::NOT_A_CARGO_PROJECT);
    }

    let key = (dir.clone(), req.generation);
    let job = {
        let mut map = jobs.map.lock().unwrap();
        Jobs::sweep(&mut map);

        match map.get(&key) {
            Some(existing) => existing.clone(),
            None => {
                let job = Arc::new(Job {
                    started: Instant::now(),
                    progress: Mutex::new(Progress::default()),
                    result: Mutex::new(None),
                    finished_at: Mutex::new(None),
                });
                map.insert(key, job.clone());

                // Its own thread, not the request's: the whole point is that this outlives the
                // call that started it.
                let root = jail.root().to_path_buf();
                let worker = job.clone();
                std::thread::spawn(move || {
                    let outcome = run_build(&manifest, &dir, &root, &worker);
                    *worker.result.lock().unwrap() = Some(outcome);
                    *worker.finished_at.lock().unwrap() = Some(Instant::now());
                });
                job
            }
        }
    };

    Ok(snapshot(&job))
}

fn snapshot(job: &Job) -> CompileStatus {
    let result = job.result.lock().unwrap().clone();
    let progress = job.progress.lock().unwrap();
    CompileStatus {
        finished: result.is_some(),
        units_done: progress.units_done,
        units_total: progress.units_total,
        current: progress.current.clone(),
        elapsed_ms: job.started.elapsed().as_millis() as u64,
        result,
    }
}

/// Runs the build to completion, updating `job` as cargo reports progress.
fn run_build(manifest: &Path, dir: &Path, jail_root: &Path, job: &Job) -> CompileResult {
    // A denominator before the numerator starts moving. Cargo does not publish its unit graph
    // on stable, so the resolved package count is the honest estimate available — and it costs
    // a fraction of a second against a build measured in minutes.
    if let Some(total) = package_count(manifest) {
        job.progress.lock().unwrap().units_total = total;
    }

    match spawn_build(manifest, job) {
        Ok(Some(run)) => finish(run, dir, jail_root),
        // Killed at the deadline.
        Ok(None) => CompileResult {
            success: false,
            output: "build exceeded the server's time limit and was stopped".to_owned(),
            output_truncated: false,
            binaries: Vec::new(),
        },
        Err(e) => CompileResult {
            success: false,
            output: format!("could not run cargo: {e}"),
            output_truncated: false,
            binaries: Vec::new(),
        },
    }
}

fn finish(run: BuildRun, dir: &Path, jail_root: &Path) -> CompileResult {
    let binaries = run
        .artifacts
        .into_iter()
        // Only the project's own binaries — a dependency can produce an "executable" artifact
        // too (a build-script helper, say), and those never live inside this project.
        .filter(|p| p.starts_with(dir))
        .filter_map(|p| built_binary(jail_root, &p))
        .collect();

    let (output, output_truncated) = cap_tail(run.log, MAX_OUTPUT_BYTES);
    CompileResult {
        success: run.success,
        output,
        output_truncated,
        binaries,
    }
}

/// Cargo's resolved package count. `None` if cargo could not answer — offline with no
/// lockfile, say — in which case the app shows an indeterminate bar rather than a wrong one.
fn package_count(manifest: &Path) -> Option<u32> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(manifest)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let n = meta.get("packages")?.as_array()?.len();
    u32::try_from(n).ok()
}

struct BuildRun {
    success: bool,
    log: String,
    artifacts: Vec<PathBuf>,
}

/// `Ok(None)` means the deadline hit and the child was killed rather than left running.
///
/// Both pipes are drained on their own threads *while* the build runs. That is not just
/// deadlock avoidance (cargo's JSON from a crate with real warnings outgrows a pipe buffer) —
/// it is where progress comes from, since a snapshot has to be available mid-build.
fn spawn_build(manifest: &Path, job: &Job) -> std::io::Result<Option<BuildRun>> {
    let mut child = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--color=never",
            "--message-format=json-render-diagnostics",
            "--manifest-path",
        ])
        .arg(manifest)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The server must not inherit a caller's CARGO_TARGET_DIR: each project builds into
        // its own target/ so artifact paths are predictable and jailed.
        .env_remove("CARGO_TARGET_DIR")
        .spawn()?;

    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");

    // Scoped, so the readers can borrow `job` directly and are guaranteed joined before this
    // returns — no `Arc` needed to hand progress back.
    let (status, mut log, artifacts, stderr_text) = std::thread::scope(|scope| {
        // stdout carries the JSON: one `compiler-artifact` per unit, fresh or not, which is
        // what the bar counts.
        let reader = scope.spawn(|| {
            let mut log = String::new();
            let mut artifacts = Vec::new();
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                match msg.get("reason").and_then(|r| r.as_str()) {
                    Some("compiler-message") => {
                        if let Some(rendered) =
                            msg.pointer("/message/rendered").and_then(|v| v.as_str())
                        {
                            log.push_str(rendered);
                        }
                    }
                    Some("compiler-artifact") => {
                        if let Some(exe) = msg.get("executable").and_then(|v| v.as_str()) {
                            artifacts.push(PathBuf::from(exe));
                        }
                        job.progress.lock().unwrap().units_done += 1;
                    }
                    _ => {}
                }
            }
            (log, artifacts)
        });

        // stderr carries the human status lines — `   Compiling serde v1.0.229` — the only
        // place cargo names what it is working on right now.
        let status_reader = scope.spawn(|| {
            let mut raw = String::new();
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(unit) = line.trim_start().strip_prefix("Compiling ") {
                    job.progress.lock().unwrap().current = unit.trim().to_owned();
                }
                raw.push_str(&line);
                raw.push('\n');
            }
            raw
        });

        let deadline = Instant::now() + BUILD_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(Some(status)),
                Ok(None) => {}
                Err(e) => break Err(e),
            }
            if Instant::now() >= deadline {
                kill(&mut child);
                break Ok(None);
            }
            std::thread::sleep(Duration::from_millis(100));
        };

        let (log, artifacts) = reader.join().unwrap_or_default();
        let stderr_text = status_reader.join().unwrap_or_default();
        (status, log, artifacts, stderr_text)
    });

    let Some(status) = status? else {
        return Ok(None);
    };

    // Cargo failing to even start a build (a malformed manifest, say) never reaches the JSON
    // path at all — that error is on stderr, as plain text.
    if log.is_empty() && !status.success() {
        log = stderr_text;
    }

    job.progress.lock().unwrap().current = String::new();

    Ok(Some(BuildRun {
        success: status.success(),
        log,
        artifacts,
    }))
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn built_binary(jail_root: &Path, exe: &Path) -> Option<BuiltBinary> {
    let size = std::fs::metadata(exe).ok()?.len();
    let name = exe.file_name()?.to_str()?.to_owned();
    let rel = exe.strip_prefix(jail_root).ok()?;
    Some(BuiltBinary {
        name,
        path: jail_path(rel),
        size,
    })
}

/// A jail-relative path as `ListDirReq` expects one: `/`-rooted, forward slashes.
fn jail_path(rel: &Path) -> String {
    let mut out = String::from("/");
    for (i, part) in rel.components().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(&part.as_os_str().to_string_lossy());
    }
    out
}

/// Keep the last `max` bytes of `s`, on a char boundary.
fn cap_tail(mut s: String, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s, false);
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s.drain(..start);
    (s, true)
}
