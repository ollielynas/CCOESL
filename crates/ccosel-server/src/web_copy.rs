//! Copies of Apple-format files that every browser can show, made on the server.
//!
//! The Viewer hands pictures, audio and video straight to the browser, so the formats only
//! Safari decodes (HEIC photos, HEVC and ProRes video, Apple Lossless and other Apple audio, and
//! TIFF) come out blank anywhere else. For those, `/files/<path>?inline=1&as=web` sends a copy
//! made with ffmpeg instead: JPEG, H.264 MP4, or AAC.
//!
//! A copy is kept in a cache folder **outside** the jail, named by a hash of the original's real
//! path, size and modification time, so changing the file makes a new one and viewing it again
//! costs nothing. The cache has a size limit and drops the least recently used copies first.
//! Nothing here checks permissions: the callers check them, exactly as for `/files`, before they
//! ask, and a copy is only ever found from the real path a check has just passed.
//!
//! Making a copy is a job, polled with the `WebCopy` call the way a build is polled, so the
//! Viewer can show a long video conversion's progress instead of a blank player.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use ccosel_proto::fs::WebCopyStatus;
use sha2::{Digest, Sha256};

/// What a file becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Jpeg,
    Mp4,
    /// AAC in an MP4 (`.m4a`) container.
    Aac,
}

impl Target {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Mp4 => "mp4",
            Self::Aac => "m4a",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Mp4 => "video/mp4",
            Self::Aac => "audio/mp4",
        }
    }

    /// How long one conversion may run before it is stopped.
    pub fn time_limit(self) -> Duration {
        match self {
            Self::Mp4 => Duration::from_secs(30 * 60),
            Self::Jpeg | Self::Aac => Duration::from_secs(2 * 60),
        }
    }
}

/// What `path` is converted to, by its extension, or `None` for a file browsers show as it is.
pub fn target_for(path: &Path) -> Option<Target> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "heic" | "heif" | "tif" | "tiff" => Target::Jpeg,
        "mov" => Target::Mp4,
        "m4a" | "caf" | "aif" | "aiff" => Target::Aac,
        _ => return None,
    })
}

/// How a conversion went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Converted {
    /// The output file was written.
    Made,
    /// The input already plays everywhere (an `.m4a` that is already AAC): send it as it is.
    AlreadyFine,
}

/// Something that turns a file into a web-friendly one. ffmpeg in the server; a fake in tests,
/// so CI needs no ffmpeg.
pub trait Converter: Send + Sync {
    /// Convert `input` into `output` as `target`, within `limit`, reporting progress in
    /// thousandths when it can tell. On failure, the reason, for a person to read.
    fn convert(
        &self,
        input: &Path,
        output: &Path,
        target: Target,
        limit: Duration,
        progress: &(dyn Fn(u16) + Sync),
    ) -> Result<Converted, String>;
}

/// What a finished job left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ready {
    /// Send this copy.
    Copy(PathBuf, Target),
    /// Send the original.
    Original,
}

const UNKNOWN: u32 = u32::MAX;

/// A failed job is remembered this long, so polling shows its error rather than retrying.
const FAILURE_RETENTION: Duration = Duration::from_secs(10 * 60);

struct Job {
    permille: AtomicU32,
    done: Mutex<Option<(Result<Ready, String>, Instant)>>,
}

/// The cache of copies, and the jobs making them.
pub struct WebCopies {
    dir: PathBuf,
    max_bytes: u64,
    converter: Arc<dyn Converter>,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}

impl WebCopies {
    /// A cache in `dir`, created if it is missing, holding at most `max_bytes` of copies.
    pub fn new(dir: PathBuf, max_bytes: u64, converter: Arc<dyn Converter>) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            max_bytes,
            converter,
            jobs: Mutex::new(HashMap::new()),
        }
    }

    /// The name a copy of the real file `real` has: a hash of where it is, how big, when it
    /// last changed, and what it becomes. Any change to the file is a different name.
    pub fn key(real: &Path, target: Target) -> Option<String> {
        let meta = std::fs::metadata(real).ok()?;
        let mtime = meta
            .modified()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?;
        let mut hash = Sha256::new();
        hash.update(real.as_os_str().as_encoded_bytes());
        hash.update([0]);
        hash.update(meta.len().to_le_bytes());
        hash.update(mtime.as_nanos().to_le_bytes());
        hash.update(target.extension().as_bytes());
        Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    fn copy_path(&self, key: &str, target: Target) -> PathBuf {
        self.dir.join(format!("{key}.{}", target.extension()))
    }

    /// Marks a file that needs no copy, so asking again doesn't convert it again.
    fn original_marker(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.original"))
    }

    /// The copy of `real` if it is already made, or that the original will do. Touches it, so
    /// it counts as recently used.
    pub fn cached(&self, real: &Path, target: Target) -> Option<Ready> {
        let key = Self::key(real, target)?;
        let copy = self.copy_path(&key, target);
        if copy.is_file() {
            touch(&copy);
            return Some(Ready::Copy(copy, target));
        }
        let marker = self.original_marker(&key);
        if marker.is_file() {
            touch(&marker);
            return Some(Ready::Original);
        }
        None
    }

    /// Start making the copy of `real` if it isn't made or being made, and say where it has got
    /// to. Never waits for it.
    pub fn status(&self, real: &Path, target: Target) -> WebCopyStatus {
        if self.cached(real, target).is_some() {
            return finished(None);
        }
        let Some(key) = Self::key(real, target) else {
            return finished(Some("the file couldn't be read".to_owned()));
        };
        let job = self.job(&key, real, target);
        match &*job.done.lock().unwrap() {
            Some((Ok(_), _)) => finished(None),
            Some((Err(why), _)) => finished(Some(why.clone())),
            None => WebCopyStatus {
                finished: false,
                permille: match job.permille.load(Ordering::Relaxed) {
                    UNKNOWN => None,
                    n => Some(n.min(999) as u16),
                },
                error: None,
            },
        }
    }

    /// The finished result for `real`, waiting up to `wait` for a job to finish. `None` if it
    /// is still running after that.
    pub fn wait(
        &self,
        real: &Path,
        target: Target,
        wait: Duration,
    ) -> Option<Result<Ready, String>> {
        if let Some(ready) = self.cached(real, target) {
            return Some(Ok(ready));
        }
        let key = Self::key(real, target).ok_or("the file couldn't be read");
        let key = match key {
            Ok(k) => k,
            Err(e) => return Some(Err(e.to_owned())),
        };
        let job = self.job(&key, real, target);
        let until = Instant::now() + wait;
        loop {
            if let Some((result, _)) = &*job.done.lock().unwrap() {
                return Some(result.clone());
            }
            if Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The job for `key`, started if there isn't one.
    fn job(&self, key: &str, real: &Path, target: Target) -> Arc<Job> {
        let mut jobs = self.jobs.lock().unwrap();
        // Successes are in the cache from now on, so only their job entries are dropped; a
        // failure stays a while, so polling shows why rather than starting again.
        jobs.retain(|_, job| match &*job.done.lock().unwrap() {
            None => true,
            Some((Ok(_), _)) => false,
            Some((Err(_), at)) => at.elapsed() < FAILURE_RETENTION,
        });
        if let Some(job) = jobs.get(key) {
            return job.clone();
        }
        let job = Arc::new(Job {
            permille: AtomicU32::new(UNKNOWN),
            done: Mutex::new(None),
        });
        jobs.insert(key.to_owned(), job.clone());
        drop(jobs);

        let worker = job.clone();
        let converter = self.converter.clone();
        let (input, key) = (real.to_path_buf(), key.to_owned());
        let copy = self.copy_path(&key, target);
        let part = self.dir.join(format!("{key}.part.{}", target.extension()));
        let marker = self.original_marker(&key);
        let (dir, max_bytes) = (self.dir.clone(), self.max_bytes);
        std::thread::spawn(move || {
            let progress = |p: u16| worker.permille.store(u32::from(p), Ordering::Relaxed);
            let result = converter
                .convert(&input, &part, target, target.time_limit(), &progress)
                .and_then(|done| match done {
                    Converted::Made => std::fs::rename(&part, &copy)
                        .map(|()| Ready::Copy(copy.clone(), target))
                        .map_err(|e| format!("couldn't keep the copy: {e}")),
                    Converted::AlreadyFine => {
                        let _ = std::fs::write(&marker, b"");
                        Ok(Ready::Original)
                    }
                });
            if result.is_err() {
                let _ = std::fs::remove_file(&part);
            }
            evict(&dir, max_bytes);
            *worker.done.lock().unwrap() = Some((result, Instant::now()));
        });
        job
    }
}

fn finished(error: Option<String>) -> WebCopyStatus {
    WebCopyStatus {
        finished: true,
        permille: Some(1000),
        error,
    }
}

/// Mark `path` as just used.
fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Delete the least recently used copies in `dir` until they fit in `max_bytes`. Files still
/// being written (`.part.`) are left alone.
pub fn evict(dir: &Path, max_bytes: u64) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut copies: Vec<(SystemTime, u64, PathBuf)> = read
        .filter_map(Result::ok)
        .filter(|e| !e.file_name().to_string_lossy().contains(".part."))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .collect();
    let mut total: u64 = copies.iter().map(|(_, len, _)| len).sum();
    copies.sort();
    for (_, len, path) in copies {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

/// The converter the server runs: ffmpeg, and `heif-convert` (libheif) for a HEIC photo
/// ffmpeg can't read.
pub struct Ffmpeg;

impl Converter for Ffmpeg {
    fn convert(
        &self,
        input: &Path,
        output: &Path,
        target: Target,
        limit: Duration,
        progress: &(dyn Fn(u16) + Sync),
    ) -> Result<Converted, String> {
        let deadline = Instant::now() + limit;
        match target {
            Target::Jpeg => {
                let ffmpeg = run(
                    ffmpeg_args()
                        .arg("-i")
                        .arg(input)
                        .args(["-frames:v", "1", "-q:v", "3"])
                        .arg(output),
                    deadline,
                    None,
                );
                let heif = matches!(
                    input
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref(),
                    Some("heic" | "heif")
                );
                match ffmpeg {
                    Err(e) if heif => run(
                        Command::new("heif-convert")
                            .args(["-q", "90"])
                            .arg(input)
                            .arg(output),
                        deadline,
                        None,
                    )
                    .map_err(|e2| format!("{e}; heif-convert: {e2}")),
                    other => other,
                }?;
            }
            Target::Aac => {
                if probe(input, "a").as_deref() == Some("aac")
                    && input
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("m4a"))
                {
                    return Ok(Converted::AlreadyFine);
                }
                run(
                    ffmpeg_args()
                        .arg("-i")
                        .arg(input)
                        .args([
                            "-vn",
                            "-c:a",
                            "aac",
                            "-b:a",
                            "192k",
                            "-movflags",
                            "+faststart",
                        ])
                        .args(["-f", "ipod"])
                        .arg(output),
                    deadline,
                    None,
                )?;
            }
            Target::Mp4 => {
                let duration = duration_us(input);
                // H.264 needs no re-encoding, only a new container; anything else does.
                let video: &[&str] = if probe(input, "v").as_deref() == Some("h264") {
                    &["-c:v", "copy"]
                } else {
                    &[
                        "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt",
                        "yuv420p",
                    ]
                };
                let audio: &[&str] = if probe(input, "a").as_deref() == Some("aac") {
                    &["-c:a", "copy"]
                } else {
                    &["-c:a", "aac", "-b:a", "160k"]
                };
                let report = |line: &str| {
                    if let (Some(total), Some(at)) = (
                        duration,
                        line.strip_prefix("out_time_us=")
                            .and_then(|v| v.trim().parse::<u64>().ok()),
                    ) && total > 0
                    {
                        progress((at.min(total) * 1000 / total) as u16);
                    }
                };
                run(
                    ffmpeg_args()
                        .arg("-i")
                        .arg(input)
                        .args(["-map", "0:v:0", "-map", "0:a:0?"])
                        .args(video)
                        .args(audio)
                        .args([
                            "-movflags",
                            "+faststart",
                            "-f",
                            "mp4",
                            "-progress",
                            "pipe:1",
                        ])
                        .arg(output),
                    deadline,
                    Some(&report),
                )?;
            }
        }
        Ok(Converted::Made)
    }
}

fn ffmpeg_args() -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"]);
    cmd
}

/// The codec of the first `kind` (`v` or `a`) stream, as ffprobe names it.
fn probe(input: &Path, kind: &str) -> Option<String> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams"])
        .arg(format!("{kind}:0"))
        .args(["-show_entries", "stream=codec_name", "-of", "csv=p=0"])
        .arg(input)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!name.is_empty()).then_some(name)
}

fn duration_us(input: &Path) -> Option<u64> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(input)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let secs: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some((secs * 1_000_000.0) as u64)
}

/// Run `cmd` until it exits or `deadline`, handing each stdout line to `on_line`. On failure,
/// the end of what it said on stderr.
fn run(
    cmd: &mut Command,
    deadline: Instant,
    on_line: Option<&(dyn Fn(&str) + Sync)>,
) -> Result<(), String> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run {name}: {e}"))?;
    let stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let (status, said) = std::thread::scope(|scope| {
        scope.spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(f) = on_line {
                    f(&line);
                }
            }
        });
        let said = scope.spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break None,
            }
        };
        (status, said.join().unwrap_or_default())
    });
    match status {
        Some(s) if s.success() => Ok(()),
        Some(_) => {
            let said = said.trim();
            let mut start = said.len().saturating_sub(300);
            while !said.is_char_boundary(start) {
                start += 1;
            }
            Err(format!(
                "{name} couldn't convert it: {}",
                said[start..].trim()
            ))
        }
        None => Err(format!("{name} took too long and was stopped")),
    }
}

#[cfg(test)]
mod tests;
