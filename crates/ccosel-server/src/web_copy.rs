//! Pictures, video and sound in a form every browser shows, decided and made on the server.
//!
//! The Viewer hands media straight to the browser, so whatever the browser can't decode comes
//! out blank: HEIC and HEVC outside Safari, and everywhere SVG, TIFF, camera raw, OpenEXR,
//! Photoshop, AVI, WMV, MPEG-2, 10-bit H.264, WMA, AC-3 and more. For the Viewer, the server
//! looks at what a file really is (its first bytes, or ffprobe; never only its name) and either
//! sends it as it is, when every browser shows it, or makes a copy that every browser does: a
//! picture as WebP (an SVG as PNG), a video as H.264 MP4, a recording as AAC. `/files/<path>?
//! inline=1&as=web` sends whichever it is.
//!
//! What was decided, and the copy, are kept in a cache folder **outside** the jail, named by a
//! hash of the original's real path, size and modification time, so changing the file means
//! deciding again and viewing it again costs nothing. The cache has a size limit and drops the
//! least recently used first. Nothing here checks permissions: the callers check them, exactly
//! as for `/files`, before they ask, and a copy is only ever found from the real path a check
//! has just passed.
//!
//! Deciding and converting is a job, polled with the `WebCopy` call the way a build is polled,
//! so the Viewer can show a long video conversion's progress instead of a blank player.

pub mod plan;
pub mod sniff;
pub mod svg;
mod tools;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub use ccosel_proto::fs::Shown;
use ccosel_proto::fs::WebCopyStatus;
use sha2::{Digest, Sha256};

use crate::image_info;

pub use plan::Probe;
pub use tools::Tools;

/// What a copy is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Webp,
    Png,
    Mp4,
    /// AAC in an MP4 (`.m4a`) container.
    Aac,
}

impl Target {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Webp => "webp",
            Self::Png => "png",
            Self::Mp4 => "mp4",
            Self::Aac => "m4a",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Webp => "image/webp",
            Self::Png => "image/png",
            Self::Mp4 => "video/mp4",
            Self::Aac => "audio/mp4",
        }
    }

    fn from_extension(ext: &str) -> Option<Self> {
        [Self::Webp, Self::Png, Self::Mp4, Self::Aac]
            .into_iter()
            .find(|t| t.extension() == ext)
    }

    /// How long one conversion may run before it is stopped.
    pub fn time_limit(self) -> Duration {
        match self {
            Self::Mp4 => Duration::from_secs(30 * 60),
            Self::Webp | Self::Png | Self::Aac => Duration::from_secs(2 * 60),
        }
    }
}

/// `content_type` if it is one a file is ever sent to the Viewer as unconverted: pictures,
/// video and sound, never anything a browser would run. A cached decision is read back through
/// this, so even a tampered cache file can't make `/files` send HTML.
pub fn media_type(content_type: &str) -> Option<&'static str> {
    [
        "image/jpeg",
        "image/png",
        "image/gif",
        "image/webp",
        "image/avif",
        "image/bmp",
        "image/x-icon",
        "video/mp4",
        "video/webm",
        "audio/mpeg",
        "audio/aac",
        "audio/mp4",
        "audio/flac",
        "audio/wav",
        "audio/ogg",
        "audio/webm",
    ]
    .into_iter()
    .find(|t| *t == content_type)
}

/// How a copy is made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recipe {
    /// A picture's first frame, by ffmpeg or ImageMagick. See [`sniff::Picture::Convert`].
    Picture {
        magick: Option<&'static str>,
        magick_first: bool,
        /// How EXIF says to turn it to be the right way up, 1 (as it is) to 8. Applied by the
        /// converter itself: ImageMagick 6 ignores the tag in a PNG or WebP, and the copy keeps
        /// no tag for a browser to apply a second time.
        orientation: u32,
    },
    /// An SVG, drawn by resvg here in the server.
    Svg,
    Video {
        /// The video is already H.264 every browser plays, so it only needs a new container.
        copy_video: bool,
        /// `None` without sound; `Some(true)` if the sound is AAC already, so it is copied.
        audio: Option<bool>,
        /// HDR (PQ or HLG), mapped to ordinary brightness.
        tone_map: bool,
        deinterlace: bool,
        /// How long it is, for progress.
        duration_us: Option<u64>,
    },
    Audio {
        /// Above 48 kHz, so brought down to it.
        resample: bool,
    },
}

impl Recipe {
    pub fn target(&self) -> Target {
        match self {
            Self::Picture { .. } => Target::Webp,
            Self::Svg => Target::Png,
            Self::Video { .. } => Target::Mp4,
            Self::Audio { .. } => Target::Aac,
        }
    }
}

/// What a file is, and how it is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Every browser shows it as it is, sent with this type.
    AsIs {
        shown: Shown,
        content_type: &'static str,
    },
    /// A copy is made.
    Convert { shown: Shown, recipe: Recipe },
    /// It isn't a picture, video or recording, or not one anything here can read.
    NotMedia,
}

impl Plan {
    fn shown(&self) -> Shown {
        match self {
            Self::AsIs { shown, .. } | Self::Convert { shown, .. } => *shown,
            Self::NotMedia => Shown::NotMedia,
        }
    }
}

/// Something that looks inside and converts files. ffmpeg and ImageMagick in the server; a fake
/// in tests, so CI needs neither.
pub trait Converter: Send + Sync {
    /// What ffprobe makes of `input`, within `limit`, or `None` if it can't read it.
    fn probe(&self, input: &Path, limit: Duration) -> Option<Probe>;

    /// Make the copy of `input` at `output` by `recipe`, within `limit`, reporting progress in
    /// thousandths when it can tell. On failure, the reason, for a person to read. Never asked
    /// for [`Recipe::Svg`], which is drawn here.
    fn convert(
        &self,
        input: &Path,
        output: &Path,
        recipe: &Recipe,
        limit: Duration,
        progress: &(dyn Fn(u16) + Sync),
    ) -> Result<(), String>;
}

/// How long ffprobe may look at a file.
const PROBE_LIMIT: Duration = Duration::from_secs(30);

/// How much of a file is read to recognise a picture: its header, and for APNG the chunks before
/// the first image data.
const HEAD_BYTES: u64 = 1 << 20;

/// The most of a GIF read to count its frames. A GIF's first frame is rarely more than a few
/// megabytes; one whose is bigger than this is shown still.
const GIF_BYTES: u64 = 64 << 20;

/// What a finished job left: how the file is shown and what to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    pub shown: Shown,
    /// The copy to send, or `None` for the original.
    pub copy: Option<(PathBuf, Target)>,
    /// What the original is sent as, when it is sent as it is. Empty for [`Shown::NotMedia`].
    pub content_type: String,
    /// The size of what is shown, the right way up, when it is a picture or video.
    pub size: Option<(u32, u32)>,
}

impl Ready {
    /// One line, for the cache: `shown copy-extension type width height`, `-` for none.
    fn to_line(&self) -> String {
        let shown = match self.shown {
            Shown::Picture => "picture",
            Shown::Animated => "animated",
            Shown::Video => "video",
            Shown::Audio => "audio",
            Shown::NotMedia => "none",
        };
        let copy = self.copy.as_ref().map_or("-", |(_, t)| t.extension());
        let content_type = if self.content_type.is_empty() {
            "-"
        } else {
            &self.content_type
        };
        let (w, h) = self
            .size
            .map_or(("-".to_owned(), "-".to_owned()), |(w, h)| {
                (w.to_string(), h.to_string())
            });
        format!("v1 {shown} {copy} {content_type} {w} {h}")
    }

    /// What [`Self::to_line`] wrote, with the copy at `copy_path` for its extension.
    fn from_line(line: &str, copy_path: impl Fn(Target) -> PathBuf) -> Option<Self> {
        let mut parts = line.split_whitespace();
        if parts.next()? != "v1" {
            return None;
        }
        let shown = match parts.next()? {
            "picture" => Shown::Picture,
            "animated" => Shown::Animated,
            "video" => Shown::Video,
            "audio" => Shown::Audio,
            "none" => Shown::NotMedia,
            _ => return None,
        };
        let copy = match parts.next()? {
            "-" => None,
            ext => {
                let target = Target::from_extension(ext)?;
                Some((copy_path(target), target))
            }
        };
        let content_type = match parts.next()? {
            "-" => String::new(),
            t => t.to_owned(),
        };
        let size = match (parts.next()?.parse().ok(), parts.next()?.parse().ok()) {
            (Some(w), Some(h)) => Some((w, h)),
            _ => None,
        };
        Some(Self {
            shown,
            copy,
            content_type,
            size,
        })
    }
}

const UNKNOWN: u32 = u32::MAX;

/// A failed job is remembered this long, so polling shows its error rather than retrying.
const FAILURE_RETENTION: Duration = Duration::from_secs(10 * 60);

struct Job {
    permille: AtomicU32,
    /// What the file is, once that's decided.
    shown: Mutex<Option<Shown>>,
    done: Mutex<Option<(Result<Ready, String>, Instant)>>,
}

/// The cache of decisions and copies, and the jobs making them.
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

    /// The name everything about the real file `real` is kept under: a hash of where it is, how
    /// big, and when it last changed. Any change to the file is a different name.
    pub fn key(real: &Path) -> Option<String> {
        let meta = std::fs::metadata(real).ok()?;
        let mtime = meta
            .modified()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?;
        let mut hash = Sha256::new();
        hash.update(b"web-copy v2\0");
        hash.update(real.as_os_str().as_encoded_bytes());
        hash.update([0]);
        hash.update(meta.len().to_le_bytes());
        hash.update(mtime.as_nanos().to_le_bytes());
        Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    fn copy_path(&self, key: &str, target: Target) -> PathBuf {
        self.dir.join(format!("{key}.{}", target.extension()))
    }

    fn plan_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.plan"))
    }

    /// What was decided for `real`, and its copy, if both are still kept. Touches them, so they
    /// count as recently used.
    pub fn cached(&self, real: &Path) -> Option<Ready> {
        let key = Self::key(real)?;
        let plan = self.plan_path(&key);
        let line = std::fs::read_to_string(&plan).ok()?;
        let ready = Ready::from_line(&line, |t| self.copy_path(&key, t))?;
        if let Some((copy, _)) = &ready.copy {
            if !copy.is_file() {
                return None;
            }
            touch(copy);
        }
        touch(&plan);
        Some(ready)
    }

    /// Start deciding about `real`, and making its copy, if that isn't done or being done, and
    /// say where it has got to. Never waits.
    pub fn status(&self, real: &Path) -> WebCopyStatus {
        if let Some(ready) = self.cached(real) {
            return finished(Ok(ready));
        }
        let Some(key) = Self::key(real) else {
            return finished(Err("the file couldn't be read".to_owned()));
        };
        let job = self.job(&key, real);
        if let Some((result, _)) = &*job.done.lock().unwrap() {
            return finished(result.clone());
        }
        let shown = *job.shown.lock().unwrap();
        WebCopyStatus {
            finished: false,
            permille: match job.permille.load(Ordering::Relaxed) {
                UNKNOWN => None,
                n => Some(n.min(999) as u16),
            },
            error: None,
            shown,
            width: None,
            height: None,
        }
    }

    /// The finished result for `real`, waiting for it up to `short` once it's known to be a
    /// video, which can take minutes, or `long` otherwise. `None` if it is still running then.
    pub fn wait(
        &self,
        real: &Path,
        short: Duration,
        long: Duration,
    ) -> Option<Result<Ready, String>> {
        if let Some(ready) = self.cached(real) {
            return Some(Ok(ready));
        }
        let Some(key) = Self::key(real) else {
            return Some(Err("the file couldn't be read".to_owned()));
        };
        let job = self.job(&key, real);
        let start = Instant::now();
        loop {
            if let Some((result, _)) = &*job.done.lock().unwrap() {
                return Some(result.clone());
            }
            let limit = match *job.shown.lock().unwrap() {
                Some(Shown::Video) => short,
                _ => long,
            };
            if start.elapsed() >= limit {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The job for `key`, started if there isn't one.
    fn job(&self, key: &str, real: &Path) -> Arc<Job> {
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
            shown: Mutex::new(None),
            done: Mutex::new(None),
        });
        jobs.insert(key.to_owned(), job.clone());
        drop(jobs);

        let worker = job.clone();
        let converter = self.converter.clone();
        let (input, key) = (real.to_path_buf(), key.to_owned());
        let (dir, max_bytes) = (self.dir.clone(), self.max_bytes);
        std::thread::spawn(move || {
            let result = make(&*converter, &input, &dir, &key, &worker);
            if let Ok(ready) = &result {
                let _ = std::fs::write(dir.join(format!("{key}.plan")), ready.to_line());
            }
            evict(&dir, max_bytes);
            *worker.done.lock().unwrap() = Some((result, Instant::now()));
        });
        job
    }
}

/// Decide what `input` is and make its copy if it needs one, as `key` in `dir`.
fn make(
    converter: &dyn Converter,
    input: &Path,
    dir: &Path,
    key: &str,
    job: &Job,
) -> Result<Ready, String> {
    let (plan, probe) = decide(converter, input);
    *job.shown.lock().unwrap() = Some(plan.shown());
    let video_size = || probe.as_ref()?.video.as_ref()?.shown_size();
    match plan {
        Plan::NotMedia => Ok(Ready {
            shown: Shown::NotMedia,
            copy: None,
            content_type: String::new(),
            size: None,
        }),
        Plan::AsIs {
            shown,
            content_type,
        } => Ok(Ready {
            shown,
            copy: None,
            content_type: content_type.to_owned(),
            size: match shown {
                Shown::Picture | Shown::Animated => picture_size(input),
                Shown::Video => video_size(),
                Shown::Audio | Shown::NotMedia => None,
            },
        }),
        Plan::Convert { shown, recipe } => {
            let target = recipe.target();
            let copy = dir.join(format!("{key}.{}", target.extension()));
            let part = dir.join(format!("{key}.part.{}", target.extension()));
            let progress = |p: u16| job.permille.store(u32::from(p), Ordering::Relaxed);
            let made = if recipe == Recipe::Svg {
                svg::render(input, &part).map(Some)
            } else {
                converter
                    .convert(input, &part, &recipe, target.time_limit(), &progress)
                    .map(|()| None)
            };
            let svg_size = match made {
                Ok(size) => size,
                Err(why) => {
                    let _ = std::fs::remove_file(&part);
                    return Err(why);
                }
            };
            std::fs::rename(&part, &copy).map_err(|e| format!("couldn't keep the copy: {e}"))?;
            let size = match shown {
                Shown::Video => video_size(),
                Shown::Picture | Shown::Animated => svg_size.or_else(|| picture_size(&copy)),
                Shown::Audio | Shown::NotMedia => None,
            };
            Ok(Ready {
                shown,
                copy: Some((copy, target)),
                content_type: target.content_type().to_owned(),
                size,
            })
        }
    }
}

/// What `input` is, from its first bytes if it's a picture, or else from ffprobe, whose probe
/// comes back too.
pub fn decide(converter: &dyn Converter, input: &Path) -> (Plan, Option<Probe>) {
    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let mut head = read_head(input, HEAD_BYTES);
    let mut picture = sniff::picture(&head, &ext);
    // A GIF whose first frame fills the head may have more after it.
    if let Some(sniff::Picture::AsIs {
        content_type: "image/gif",
        animated: false,
    }) = picture
        && head.len() as u64 == HEAD_BYTES
    {
        head = read_head(input, GIF_BYTES);
        picture = Some(sniff::Picture::AsIs {
            content_type: "image/gif",
            animated: sniff::gif_frames(&head) > 1,
        });
    }
    let plan = match picture {
        // Turned by its EXIF: browsers don't all apply that (outside JPEG, few do; and the
        // shell's own decoding may not), so it's sent turned the right way up instead.
        Some(sniff::Picture::AsIs {
            content_type: content_type @ ("image/jpeg" | "image/png" | "image/webp"),
            animated: false,
        }) if image_info::orientation(input) != 1 => Plan::Convert {
            shown: Shown::Picture,
            recipe: Recipe::Picture {
                magick: Some(&content_type["image/".len()..]),
                magick_first: true,
                orientation: image_info::orientation(input),
            },
        },
        Some(sniff::Picture::AsIs {
            content_type,
            animated,
        }) => Plan::AsIs {
            shown: if animated {
                Shown::Animated
            } else {
                Shown::Picture
            },
            content_type,
        },
        Some(sniff::Picture::Convert {
            magick,
            magick_first,
        }) => Plan::Convert {
            shown: Shown::Picture,
            recipe: Recipe::Picture {
                magick,
                magick_first,
                // HEIC's and camera raw's decoders turn them already, by their own tags.
                orientation: if matches!(magick, Some("tiff" | "psd" | "jpeg")) {
                    image_info::orientation(input)
                } else {
                    1
                },
            },
        },
        Some(sniff::Picture::Svg) => Plan::Convert {
            shown: Shown::Picture,
            recipe: Recipe::Svg,
        },
        None if head.is_empty() => Plan::NotMedia,
        None => {
            let probe = converter.probe(input, PROBE_LIMIT);
            let plan = probe
                .as_ref()
                .map_or(Plan::NotMedia, |p| plan::media(p, &ext));
            return (plan, probe);
        }
    };
    (plan, None)
}

fn read_head(path: &Path, max: u64) -> Vec<u8> {
    let mut head = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(max).read_to_end(&mut head);
    }
    head
}

/// A picture's size. What's sent is always the right way up (one with an orientation tag is
/// converted), so it's the size as it's shown.
fn picture_size(path: &Path) -> Option<(u32, u32)> {
    let size = imagesize::size(path).ok()?;
    Some((
        u32::try_from(size.width).ok()?,
        u32::try_from(size.height).ok()?,
    ))
}

fn finished(result: Result<Ready, String>) -> WebCopyStatus {
    match result {
        Ok(ready) => WebCopyStatus {
            finished: true,
            permille: Some(1000),
            error: None,
            shown: Some(ready.shown),
            width: ready.size.map(|s| s.0),
            height: ready.size.map(|s| s.1),
        },
        Err(why) => WebCopyStatus {
            finished: true,
            permille: Some(1000),
            error: Some(why),
            shown: None,
            width: None,
            height: None,
        },
    }
}

/// Mark `path` as just used.
fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Delete the least recently used files in `dir` until they fit in `max_bytes`. Files still
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

#[cfg(test)]
mod tests;
