//! Engraves LilyPond source into sheet music, as a **job**, for the Music Score app.
//!
//! The same shape as `build_api`: `Engrave` starts LilyPond and returns at once, and the app
//! polls. The job is keyed by a hash of the source, so engraving text that hasn't changed finds
//! the finished job and runs nothing. Results (one PNG per page, a PDF, and a MIDI file when the
//! score asks for one) are written to a directory per key, outside the jail, and served at
//! `/engraved/<key>/<file>` by [`Engraver::file`].
//!
//! **Engraving runs untrusted code.** A `.ly` file can embed Scheme, and LilyPond's `-dsafe`
//! mode, which used to limit it, was removed in 2.23.12 as unfixable. So engraving a score runs
//! whatever it says as the server's user, like building a project does. See "Known limitation"
//! in `ARCHITECTURE.md`.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ccosel_proto::fs::MAX_TEXT_BYTES;
use ccosel_proto::score::{EngraveResult, EngraveStatus, ScoreMessage};
use ccosel_proto::server_error;
use sha2::{Digest, Sha256};

/// A score that takes this long is not a score someone is waiting on. Real ones take seconds.
const ENGRAVE_TIMEOUT: Duration = Duration::from_secs(60);

/// The log is tail-capped like the Compiler's: an error is usually the last thing it says.
const MAX_OUTPUT_BYTES: usize = 32 * 1024;

/// Results unused for this long are deleted. A poll or a fetch of one of its files counts as use,
/// so a score someone has open keeps its pages.
const RETENTION: Duration = Duration::from_secs(30 * 60);

/// LilyPond runs at once at most. Each is a process using a core and a few hundred megabytes; past
/// this, a new score is refused until one finishes rather than slowing every one down.
const MAX_RUNNING: usize = 4;

/// The name the source is engraved under, and so what LilyPond's messages call it.
const SOURCE: &str = "score.ly";

/// Pixels per inch of the page pictures: an A4 page comes out about 900 pixels wide, enough to
/// read on screen without each page being a large download.
const RESOLUTION: &str = "-dresolution=110";

struct Job {
    started: Instant,
    last_used: Mutex<Instant>,
    /// `Some` once LilyPond has finished, one way or the other.
    result: Mutex<Option<EngraveResult>>,
}

/// Every score this server has engraved recently, and where their files are.
pub struct Engraver {
    /// One directory per key inside, created as needed. Removed when the engraver is dropped.
    dir: PathBuf,
    program: OsString,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}

impl Engraver {
    /// Engraves with `lilypond` from `PATH`, or the program `CCOSEL_LILYPOND` names.
    pub fn new() -> std::io::Result<Self> {
        let program = std::env::var_os("CCOSEL_LILYPOND").unwrap_or_else(|| "lilypond".into());
        Self::with_program(program)
    }

    /// Engraves with `program` instead of LilyPond. For tests.
    pub fn with_program(program: impl Into<OsString>) -> std::io::Result<Self> {
        // A directory of its own, so two servers on one machine (or two tests in one process)
        // never share results.
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ccosel-engraved-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            program: program.into(),
            jobs: Mutex::new(HashMap::new()),
        })
    }

    /// Start engraving `source` unless it already has been, then report where it has got to.
    /// Never waits for LilyPond.
    pub fn engrave(&self, source: &str) -> Result<EngraveStatus, u32> {
        if source.len() > MAX_TEXT_BYTES {
            return Err(server_error::TOO_LARGE);
        }
        let key = key_of(source);
        let job = {
            let mut jobs = self.jobs.lock().unwrap();
            self.sweep(&mut jobs, Instant::now());
            match jobs.get(&key) {
                Some(job) => job.clone(),
                None => {
                    let running = jobs
                        .values()
                        .filter(|job| job.result.lock().unwrap().is_none())
                        .count();
                    if running >= MAX_RUNNING {
                        return Err(server_error::LIMIT);
                    }
                    let job = Arc::new(Job {
                        started: Instant::now(),
                        last_used: Mutex::new(Instant::now()),
                        result: Mutex::new(None),
                    });
                    jobs.insert(key.clone(), job.clone());
                    let worker = job.clone();
                    let dir = self.dir.join(&key);
                    let program = self.program.clone();
                    let source = source.to_owned();
                    std::thread::spawn(move || {
                        let result = run(&program, &dir, &key, &source);
                        *worker.result.lock().unwrap() = Some(result);
                    });
                    job
                }
            }
        };
        *job.last_used.lock().unwrap() = Instant::now();
        let result = job.result.lock().unwrap().clone();
        Ok(EngraveStatus {
            finished: result.is_some(),
            elapsed_ms: job.started.elapsed().as_millis() as u64,
            result,
        })
    }

    /// The file `/engraved/<key>/<name>` stands for, and the type to send it as. `None` for any
    /// name an engraving doesn't make, and for a result that is unfinished or gone.
    pub fn file(&self, key: &str, name: &str) -> Option<(PathBuf, &'static str)> {
        let kind = served_type(name)?;
        let job = self.jobs.lock().unwrap().get(key)?.clone();
        job.result.lock().unwrap().as_ref()?;
        *job.last_used.lock().unwrap() = Instant::now();
        let path = self.dir.join(key).join(name);
        path.is_file().then_some((path, kind))
    }

    /// Forget finished results unused for [`RETENTION`], and delete their files.
    fn sweep(&self, jobs: &mut HashMap<String, Arc<Job>>, now: Instant) {
        jobs.retain(|key, job| {
            let finished = job.result.lock().unwrap().is_some();
            let idle = now.saturating_duration_since(*job.last_used.lock().unwrap());
            let keep = !finished || idle < RETENTION;
            if !keep {
                let _ = std::fs::remove_dir_all(self.dir.join(key));
            }
            keep
        });
    }
}

impl Drop for Engraver {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Names a source's result: the first 128 bits of its SHA-256, in hex. A collision would show one
/// person's score for another's text, so this is a real hash rather than a fast one.
fn key_of(source: &str) -> String {
    Sha256::digest(source.as_bytes())[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The type a result's file is served as, for the names an engraving makes; `None` for any other
/// name, which is how a request for `../something` is refused.
fn served_type(name: &str) -> Option<&'static str> {
    match name {
        "score.pdf" => Some("application/pdf"),
        "score.midi" => Some("audio/midi"),
        _ => {
            let n = name.strip_prefix("page-")?.strip_suffix(".png")?;
            let ok = !n.is_empty() && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit());
            ok.then_some("image/png")
        }
    }
}

/// Engraves `source` into `dir` and says how it went.
fn run(program: &std::ffi::OsStr, dir: &Path, key: &str, source: &str) -> EngraveResult {
    let failed = |output: String| EngraveResult {
        success: false,
        key: key.to_owned(),
        pages: 0,
        pdf: false,
        midi: false,
        messages: Vec::new(),
        output,
        output_truncated: false,
    };
    if let Err(e) =
        std::fs::create_dir_all(dir).and_then(|()| std::fs::write(dir.join(SOURCE), source))
    {
        return failed(format!("could not prepare the score: {e}"));
    }
    let (success, log) = match spawn(program, dir) {
        Ok(Some(done)) => done,
        Ok(None) => {
            return failed("engraving took longer than the server allows and was stopped".into());
        }
        Err(e) => return failed(format!("could not run LilyPond: {e}")),
    };
    let outputs = collect(dir);
    let messages = parse_messages(&log);
    let (output, output_truncated) = cap_tail(log, MAX_OUTPUT_BYTES);
    EngraveResult {
        success: success && outputs.pages > 0 && !messages.iter().any(|m| m.error),
        key: key.to_owned(),
        pages: outputs.pages,
        pdf: outputs.pdf,
        midi: outputs.midi,
        messages,
        output,
        output_truncated,
    }
}

/// Runs LilyPond in `dir`. `Ok(None)` means it ran out of time and was killed.
fn spawn(program: &std::ffi::OsStr, dir: &Path) -> std::io::Result<Option<(bool, String)>> {
    let mut child = Command::new(program)
        .current_dir(dir)
        .args([
            "--pdf",
            "--png",
            RESOLUTION,
            // Point-and-click links are for a desktop PDF viewer and an editor on the same
            // machine; here they only make the PDF bigger and give away the server's paths.
            "-dno-point-and-click",
            "--output=score",
            SOURCE,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    // Drained while it runs, so a long log can't fill a pipe and stall LilyPond until it times out.
    let (status, out, err) = std::thread::scope(|scope| {
        let out = scope.spawn(move || {
            let mut s = String::new();
            let _ = stdout.read_to_string(&mut s);
            s
        });
        let err = scope.spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        let deadline = Instant::now() + ENGRAVE_TIMEOUT;
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
            std::thread::sleep(Duration::from_millis(50));
        };
        (
            status,
            out.join().unwrap_or_default(),
            err.join().unwrap_or_default(),
        )
    });
    let Some(status) = status? else {
        return Ok(None);
    };
    // LilyPond writes its progress and its messages to stderr; stdout is normally empty.
    let mut log = err;
    log.push_str(&out);
    Ok(Some((status.success(), log)))
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// What an engraving left in its directory.
#[derive(Debug, Default, PartialEq, Eq)]
struct Outputs {
    pages: u32,
    pdf: bool,
    midi: bool,
}

/// Renames what LilyPond wrote in `dir` to the names it is served under, and counts it.
///
/// LilyPond names its output after `--output`, which is `score` here: `score.png` for a
/// one-page score but `score-page1.png`, `score-page2.png`, ... for more; `score.pdf`; and
/// `score.midi` (or `score.mid`, which older versions write). A file with several `\book`s or
/// `\score`s with `\midi` gets numbered files (`score-1.pdf`, `score-1.midi`); the first one is
/// served.
fn collect(dir: &Path) -> Outputs {
    let names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    let mut pages = page_files(&names);
    if pages.is_empty() && names.iter().any(|n| n == "score.png") {
        pages.push("score.png".to_owned());
    }
    for (i, name) in pages.iter().enumerate() {
        let _ = std::fs::rename(dir.join(name), dir.join(format!("page-{}.png", i + 1)));
    }
    let first = |exts: &[&str]| {
        let mut found: Vec<&String> = names
            .iter()
            .filter(|n| n.starts_with("score") && exts.iter().any(|ext| n.ends_with(ext)))
            .collect();
        // `score.pdf` before `score-1.pdf`, and `score-2` before `score-10`.
        found.sort_by_key(|n| (n.len(), n.to_string()));
        found.first().map(|n| n.to_string())
    };
    let place = |exts: &[&str], served: &str| match first(exts) {
        Some(name) if name == served => true,
        Some(name) => std::fs::rename(dir.join(name), dir.join(served)).is_ok(),
        None => false,
    };
    let pdf = place(&[".pdf"], "score.pdf");
    let midi = place(&[".midi", ".mid"], "score.midi");
    Outputs {
        pages: pages.len() as u32,
        pdf,
        midi,
    }
}

/// The multi-page pictures among `names` (`score-page<n>.png`), in page order.
fn page_files(names: &[String]) -> Vec<String> {
    let mut pages: Vec<(u32, &String)> = names
        .iter()
        .filter_map(|name| {
            let n = name.strip_prefix("score-page")?.strip_suffix(".png")?;
            Some((n.parse().ok()?, name))
        })
        .collect();
    pages.sort();
    pages.into_iter().map(|(_, name)| name.clone()).collect()
}

/// The errors and warnings in LilyPond's log that are about the source.
///
/// They look like `score.ly:12:4: error: syntax error, unexpected '}'`, sometimes without the
/// column, followed by lines quoting the source, which are left out. Messages about LilyPond's
/// own files (`/usr/share/lilypond/...`) are about the source too, as a rule, but can't point at
/// a line of it, so they are kept with line 0. So is `fatal error: ...`.
fn parse_messages(log: &str) -> Vec<ScoreMessage> {
    let mut out = Vec::new();
    for line in log.lines() {
        if let Some(text) = line.strip_prefix("fatal error: ") {
            out.push(ScoreMessage {
                line: 0,
                column: 0,
                error: true,
                text: text.trim().to_owned(),
            });
            continue;
        }
        let Some((place, error, text)) = split_kind(line) else {
            continue;
        };
        // `file:line:column`, or `file:line`. Read from the right, since a path may hold a `:`.
        let Some((head, last)) = place.rsplit_once(':') else {
            continue;
        };
        let Ok(last) = last.parse::<u32>() else {
            continue;
        };
        let (file, line_no, column) = match head.rsplit_once(':') {
            Some((file, l)) if l.parse::<u32>().is_ok() => (file, l.parse().unwrap_or(0), last),
            _ => (head, last, 0),
        };
        let ours = file == SOURCE || file.ends_with(&format!("/{SOURCE}"));
        out.push(ScoreMessage {
            line: if ours { line_no } else { 0 },
            column: if ours { column } else { 0 },
            error,
            text: text.trim().to_owned(),
        });
    }
    out
}

/// `place: error: text` or `place: warning: text`, as `(place, is_error, text)`.
fn split_kind(line: &str) -> Option<(&str, bool, &str)> {
    for (marker, error) in [(": error: ", true), (": warning: ", false)] {
        if let Some(at) = line.find(marker) {
            return Some((&line[..at], error, &line[at + marker.len()..]));
        }
    }
    None
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

#[cfg(test)]
mod tests;
