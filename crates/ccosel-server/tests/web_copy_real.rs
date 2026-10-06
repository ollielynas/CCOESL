//! Web copies made by the real tools: ffprobe, ffmpeg and ImageMagick. CI has none of them yet,
//! so these are opt-in, and without their variable each says so and checks nothing:
//!
//! ```sh
//! # Inputs made with ffmpeg itself, so nothing binary is checked in:
//! CCOSEL_TEST_FFMPEG=1 cargo test -p ccosel-server --test web_copy_real -- --nocapture
//! # Every file in the media test pack (its `images`, `video` and `audio` folders):
//! CCOSEL_TEST_MEDIA_PACK=/path/to/media-test-pack \
//!     cargo test --release -p ccosel-server --test web_copy_real -- --nocapture
//! ```
//!
//! With `CCOSEL_TEST_MEDIA_OUT=<folder>` as well, every copy is also saved there under its
//! original's path, for looking at by eye.
//!
//! What both check is the promise the Viewer relies on: whatever the server sends is something
//! every browser shows. So a copy, looked at again, must be shown as it is.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ccosel_server::web_copy::{Plan, Ready, Shown, Tools, WebCopies, decide};

fn enabled(var: &str) -> Option<String> {
    let value = std::env::var(var).ok().filter(|v| !v.is_empty());
    if value.is_none() {
        eprintln!("skipped: set {var} to run against the real converters");
    }
    value
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ccosel-web-real-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn copies(dir: &Path) -> WebCopies {
    WebCopies::new(dir.join("cache"), 4 << 30, Arc::new(Tools))
}

/// Everything about `input`, made and waited for, or why there is nothing.
fn view(c: &WebCopies, input: &Path) -> Result<Ready, String> {
    c.wait(input, Duration::from_secs(1800), Duration::from_secs(1800))
        .expect("finished within its time limit")
}

/// Whatever is sent for `ready` (the copy, or the original) is something every browser shows:
/// looked at again, it's shown as it is, as the same kind of thing.
fn check_sendable(input: &Path, ready: &Ready) -> Result<(), String> {
    let Some((copy, _)) = &ready.copy else {
        return Ok(());
    };
    match decide(&Tools, copy).0 {
        Plan::AsIs { shown, .. } if shown == ready.shown || ready.shown == Shown::Animated => {
            Ok(())
        }
        other => Err(format!(
            "{}: its copy {} isn't sendable as it is: {other:?}",
            input.display(),
            copy.display()
        )),
    }
}

/// Make `out` with ffmpeg from a generated test source.
fn make(out: &Path, args: &[&str]) {
    let ok = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(out)
        .status()
        .unwrap()
        .success();
    assert!(ok, "couldn't make {}", out.display());
}

const VIDEO: [&str; 4] = [
    "-f",
    "lavfi",
    "-i",
    "testsrc=duration=2:size=320x240:rate=25",
];
const AUDIO: [&str; 4] = ["-f", "lavfi", "-i", "sine=frequency=440:duration=2"];

#[test]
fn generated_files_become_what_every_browser_shows() {
    if enabled("CCOSEL_TEST_FFMPEG").is_none() {
        return;
    }
    let d = temp("generated");
    let c = copies(&d);
    let cases: Vec<(&str, Vec<&str>, Shown, bool)> = vec![
        (
            "prores.mov",
            [
                &VIDEO[..],
                &AUDIO[..],
                &["-c:v", "prores_ks", "-c:a", "alac", "-shortest"],
            ]
            .concat(),
            Shown::Video,
            true,
        ),
        (
            "hevc.mp4",
            [&VIDEO[..], &["-c:v", "libx265", "-tag:v", "hvc1"]].concat(),
            Shown::Video,
            true,
        ),
        (
            "h264.mp4",
            [
                &VIDEO[..],
                &AUDIO[..],
                &[
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-shortest",
                ],
            ]
            .concat(),
            Shown::Video,
            false,
        ),
        (
            "high10.mp4",
            [&VIDEO[..], &["-c:v", "libx264", "-pix_fmt", "yuv420p10le"]].concat(),
            Shown::Video,
            true,
        ),
        (
            "mpeg2.mpg",
            [&VIDEO[..], &["-c:v", "mpeg2video"]].concat(),
            Shown::Video,
            true,
        ),
        (
            "alac.m4a",
            [&AUDIO[..], &["-c:a", "alac"]].concat(),
            Shown::Audio,
            true,
        ),
        ("pcm.aiff", AUDIO.to_vec(), Shown::Audio, true),
        (
            "aac.m4a",
            [&AUDIO[..], &["-c:a", "aac"]].concat(),
            Shown::Audio,
            false,
        ),
        (
            "ac3.ac3",
            [&AUDIO[..], &["-c:a", "ac3"]].concat(),
            Shown::Audio,
            true,
        ),
        (
            "scan.tiff",
            vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240",
                "-frames:v",
                "1",
            ],
            Shown::Picture,
            true,
        ),
        (
            "still.png",
            vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240",
                "-frames:v",
                "1",
            ],
            Shown::Picture,
            false,
        ),
    ];
    for (name, args, shown, converted) in cases {
        let input = d.join(name);
        make(&input, &args);
        let ready = view(&c, &input).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(ready.shown, shown, "{name}");
        assert_eq!(ready.copy.is_some(), converted, "{name}: converted?");
        check_sendable(&input, &ready).unwrap();
    }
}

/// Files whose names say they're deliberately broken. The pack's README promises every broken
/// file's name signals it.
fn broken(path: &str) -> bool {
    [
        "zero_bytes",
        "magic_only",
        "random_bytes",
        "truncated",
        "corrupt",
        "bad_crc",
        "lying",
        "garbage",
        "no_eoi",
        "bad_ifd",
        "absurd",
        "header_only",
        "broken_ftyp",
        "text_file_named",
        "text_as_",
        "html_named",
        "json_as_",
    ]
    .iter()
    .any(|sign| path.contains(sign))
}

/// Every file under `dir`, relative to `root`.
fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            out.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
}

#[test]
fn every_file_in_the_media_pack() {
    let Some(pack) = enabled("CCOSEL_TEST_MEDIA_PACK") else {
        return;
    };
    let pack = PathBuf::from(pack);
    let mut paths = Vec::new();
    for top in ["images", "video", "audio"] {
        walk(&pack, &pack.join(top), &mut paths);
    }
    paths.sort();
    assert!(paths.len() > 300, "read {} paths", paths.len());

    let d = temp("pack");
    let c = copies(&d);
    let out = std::env::var_os("CCOSEL_TEST_MEDIA_OUT").map(PathBuf::from);
    let mut failures = Vec::new();
    let mut table = Vec::new();
    for rel in &paths {
        let input = pack.join(rel);
        if !input.is_file() {
            failures.push(format!("{rel}: missing from the pack"));
            continue;
        }
        let start = Instant::now();
        let result = view(&c, &input);
        let took = start.elapsed();
        let outcome = match &result {
            Ok(r) => format!(
                "{:?}{}{}",
                r.shown,
                r.copy
                    .as_ref()
                    .map_or(" as is".to_owned(), |(_, t)| format!(
                        " → {}",
                        t.extension()
                    )),
                r.size.map_or(String::new(), |(w, h)| format!(" {w}×{h}")),
            ),
            Err(e) => format!("ERROR {}", e.replace('\n', " ")),
        };
        table.push(format!("{:>6} ms  {rel}  →  {outcome}", took.as_millis()));
        if let (
            Some(out),
            Ok(Ready {
                copy: Some((copy, target)),
                ..
            }),
        ) = (&out, &result)
        {
            let dest = out.join(format!("{rel}.{}", target.extension()));
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::copy(copy, dest).unwrap();
        }
        match &result {
            _ if broken(rel) => {}
            Ok(r) if r.shown == Shown::NotMedia => {
                failures.push(format!("{rel}: not recognised as media"));
            }
            Ok(r) => {
                if let Err(e) = check_sendable(&input, r) {
                    failures.push(e);
                }
                if matches!(r.shown, Shown::Picture | Shown::Animated) && r.size.is_none() {
                    failures.push(format!("{rel}: a picture with no size"));
                }
            }
            Err(e) => failures.push(format!("{rel}: {e}")),
        }
    }
    println!("{}", table.join("\n"));
    assert!(
        failures.is_empty(),
        "{} of {} files failed:\n{}",
        failures.len(),
        paths.len(),
        failures.join("\n")
    );
}
