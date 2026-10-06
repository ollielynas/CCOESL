//! The converter the server runs: ffprobe to see what a file holds, ffmpeg for video, sound and
//! most pictures, and ImageMagick for the pictures it reads better (TIFF in all its colour
//! spaces, camera raw through LibRaw, Photoshop, arithmetic-coded JPEG) or that the installed
//! ffmpeg can't (HEIC before ffmpeg 7.1).
//!
//! ImageMagick is only ever told which format to read (`tiff:/path`), from a fixed list, so it
//! never guesses one from the file: that guess is how its worst security holes (MVG, MSL,
//! `url:`) were reached. It runs with memory and disk limits, and everything runs with a time
//! limit, past which it is killed.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::plan::Probe;
use super::{Converter, Recipe};

/// The longest side a converted picture is made at: plenty for a window, and a bound on what
/// the browser has to hold.
pub const PICTURE_SIDE: u32 = 4096;

/// The longest side a converted video is made at. Re-encoding 4K takes many times as long as
/// 1080p, for a picture that is shown in a window.
pub const VIDEO_SIDE: u32 = 1920;

/// ffmpeg, ffprobe and ImageMagick.
pub struct Tools;

impl Converter for Tools {
    fn probe(&self, input: &Path, limit: Duration) -> Option<Probe> {
        let out = run(
            Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-of",
                    "json",
                    "-show_format",
                    "-show_streams",
                ])
                .arg(input),
            Instant::now() + limit,
            None,
        )
        .ok()?;
        Probe::from_json(&out)
    }

    fn convert(
        &self,
        input: &Path,
        output: &Path,
        recipe: &Recipe,
        limit: Duration,
        progress: &(dyn Fn(u16) + Sync),
    ) -> Result<(), String> {
        let deadline = Instant::now() + limit;
        let made = self.make(input, output, recipe, deadline, progress);
        // What the tools said is for whoever runs the server; a person viewing the file gets a
        // sentence. Running out of time is said as it is.
        made.map_err(|detail| {
            eprintln!("web copy of {}: {detail}", input.display());
            if detail.ends_with(TOO_LONG) {
                return "it took too long to convert, and was stopped".to_owned();
            }
            match recipe {
                Recipe::Picture { .. } | Recipe::Svg => {
                    "it looks damaged, or isn't a kind of picture this can read"
                }
                Recipe::Video { .. } => "it looks damaged, or isn't a kind of video this can read",
                Recipe::Audio { .. } => {
                    "it looks damaged, or isn't a kind of recording this can read"
                }
            }
            .to_owned()
        })
    }
}

/// How [`run`] ends what it says when it stopped a tool for taking too long.
const TOO_LONG: &str = "took too long and was stopped";

impl Tools {
    fn make(
        &self,
        input: &Path,
        output: &Path,
        recipe: &Recipe,
        deadline: Instant,
        progress: &(dyn Fn(u16) + Sync),
    ) -> Result<(), String> {
        match *recipe {
            Recipe::Picture {
                magick,
                magick_first,
            } => {
                let by_ffmpeg = || ffmpeg_picture(input, output, deadline);
                let by_magick = |coder| magick_picture(coder, input, output, deadline);
                match (magick, magick_first) {
                    (None, _) => by_ffmpeg(),
                    (Some(coder), true) => by_magick(coder)
                        .or_else(|e| by_ffmpeg().map_err(|e2| format!("{e}; then {e2}"))),
                    (Some(coder), false) => by_ffmpeg()
                        .or_else(|e| by_magick(coder).map_err(|e2| format!("{e}; then {e2}"))),
                }
            }
            Recipe::Svg => Err("an SVG is drawn by the server itself".to_owned()),
            Recipe::Video {
                copy_video,
                audio,
                tone_map,
                deinterlace,
                duration_us,
            } => {
                let mut cmd = ffmpeg();
                cmd.arg("-i").arg(input).args(["-map", "0:v:0"]);
                if copy_video {
                    cmd.args(["-c:v", "copy"]);
                } else {
                    cmd.args(["-vf", &video_filters(tone_map, deinterlace)])
                        .args([
                            "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt",
                            "yuv420p",
                        ]);
                }
                match audio {
                    None => {}
                    Some(true) => {
                        cmd.args(["-map", "0:a:0", "-c:a", "copy"]);
                    }
                    Some(false) => {
                        cmd.args(["-map", "0:a:0", "-c:a", "aac", "-b:a", "160k"]);
                    }
                }
                cmd.args([
                    "-movflags",
                    "+faststart",
                    "-f",
                    "mp4",
                    "-progress",
                    "pipe:1",
                ])
                .arg(output);
                let report = |line: &str| {
                    if let (Some(total), Some(at)) = (
                        duration_us,
                        line.strip_prefix("out_time_us=")
                            .and_then(|v| v.trim().parse::<u64>().ok()),
                    ) && total > 0
                    {
                        progress((at.min(total) * 1000 / total) as u16);
                    }
                };
                run(&mut cmd, deadline, Some(&report)).map(drop)
            }
            Recipe::Audio { resample } => {
                let mut cmd = ffmpeg();
                cmd.arg("-i")
                    .arg(input)
                    .args(["-vn", "-map", "0:a:0", "-c:a", "aac", "-b:a", "192k"]);
                if resample {
                    cmd.args(["-ar", "48000"]);
                }
                cmd.args(["-movflags", "+faststart", "-f", "ipod"])
                    .arg(output);
                run(&mut cmd, deadline, None).map(drop)
            }
        }
    }
}

/// The filters a re-encoded video goes through: HDR mapped to ordinary brightness, fields
/// woven into frames, the size brought within [`VIDEO_SIDE`] and made even (which H.264 in
/// 4:2:0 needs), and the colours into the range every browser expects.
pub fn video_filters(tone_map: bool, deinterlace: bool) -> String {
    let mut filters = Vec::new();
    if deinterlace {
        filters.push("bwdif=mode=send_frame".to_owned());
    }
    if tone_map {
        filters.push(
            "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,\
             zscale=t=bt709:m=bt709:r=tv"
                .to_owned(),
        );
    }
    let side = VIDEO_SIDE;
    // `out_range=tv`: a full-range source (MJPEG, a phone's camera) otherwise stays flagged
    // full range, which not every browser plays.
    filters.push(format!(
        "scale='max(2,trunc(min(iw,{side}*iw/max(iw,ih))/2)*2)':\
         'max(2,trunc(min(ih,{side}*ih/max(iw,ih))/2)*2)':out_range=tv"
    ));
    filters.push("format=yuv420p".to_owned());
    filters.join(",")
}

fn ffmpeg() -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"]);
    cmd
}

/// The first frame of a picture ffmpeg reads, as WebP within [`PICTURE_SIDE`].
fn ffmpeg_picture(input: &Path, output: &Path, deadline: Instant) -> Result<(), String> {
    let side = PICTURE_SIDE;
    run(
        ffmpeg()
            .arg("-i")
            .arg(input)
            .args(["-frames:v", "1", "-vf"])
            .arg(format!(
                "scale='max(1,min(iw,{side}*iw/max(iw,ih)))':'max(1,min(ih,{side}*ih/max(iw,ih)))'"
            ))
            .args(["-c:v", "libwebp", "-quality", "90", "-f", "webp"])
            .arg(output),
        deadline,
        None,
    )
    .map(drop)
}

/// The command ImageMagick runs as: `magick` from version 7, `convert` before.
fn magick_program() -> &'static str {
    static PROGRAM: OnceLock<&'static str> = OnceLock::new();
    PROGRAM.get_or_init(|| {
        let seven = Command::new("magick")
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if seven { "magick" } else { "convert" }
    })
}

/// The first frame of a picture ImageMagick reads as `coder`, turned the right way up, as WebP
/// within [`PICTURE_SIDE`].
fn magick_picture(
    coder: &str,
    input: &Path,
    output: &Path,
    deadline: Instant,
) -> Result<(), String> {
    let side = PICTURE_SIDE;
    let mut source = std::ffi::OsString::from(format!("{coder}:"));
    source.push(input);
    source.push("[0]");
    let mut dest = std::ffi::OsString::from("webp:");
    dest.push(output);
    run(
        Command::new(magick_program())
            .args([
                "-limit", "memory", "256MiB", "-limit", "map", "512MiB", "-limit", "disk", "1GiB",
            ])
            .arg(source)
            .args(["-auto-orient", "-resize"])
            .arg(format!("{side}x{side}>"))
            .args(["-quality", "90"])
            .arg(dest),
        deadline,
        None,
    )
    .map(drop)
}

/// Run `cmd` until it exits or `deadline`, handing each line it prints to `on_line`, and
/// answering with all it printed. On failure, the end of what it said on stderr.
fn run(
    cmd: &mut Command,
    deadline: Instant,
    on_line: Option<&(dyn Fn(&str) + Sync)>,
) -> Result<String, String> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run {name}: {e}"))?;
    let stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let (status, printed, said) = std::thread::scope(|scope| {
        let printed = scope.spawn(move || {
            let mut all = String::new();
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(f) = on_line {
                    f(&line);
                }
                all.push_str(&line);
                all.push('\n');
            }
            all
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
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => break None,
            }
        };
        (
            status,
            printed.join().unwrap_or_default(),
            said.join().unwrap_or_default(),
        )
    });
    match status {
        Some(s) if s.success() => Ok(printed),
        Some(_) => {
            let said = said.trim();
            let mut start = said.len().saturating_sub(300);
            while !said.is_char_boundary(start) {
                start += 1;
            }
            Err(format!("{name} couldn't read it: {}", said[start..].trim()))
        }
        None => Err(format!("{name} {TOO_LONG}")),
    }
}
