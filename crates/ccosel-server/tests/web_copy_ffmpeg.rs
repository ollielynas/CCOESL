//! The real converter against real ffmpeg. CI has no ffmpeg yet, so this runs only with
//! `CCOSEL_TEST_FFMPEG=1`; without it, it says so and checks nothing:
//!
//! ```sh
//! CCOSEL_TEST_FFMPEG=1 cargo test -p ccosel-server --test web_copy_ffmpeg -- --nocapture
//! ```
//!
//! The inputs are made with ffmpeg itself, so nothing binary is checked in. It can't make a
//! HEIC, so HEIC is left to trying a real iPhone photo by hand.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use ccosel_server::web_copy::{Converted, Converter, Ffmpeg, Target};

fn enabled() -> bool {
    let on = std::env::var("CCOSEL_TEST_FFMPEG").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set CCOSEL_TEST_FFMPEG=1 to run against the real ffmpeg");
    }
    on
}

fn dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ccosel-ffmpeg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

fn codec(path: &Path, kind: &str) -> String {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams"])
        .arg(format!("{kind}:0"))
        .args(["-show_entries", "stream=codec_name", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn convert(input: &Path, target: Target) -> (Converted, PathBuf) {
    let output = input.with_extension(format!("out.{}", target.extension()));
    let done = Ffmpeg
        .convert(input, &output, target, Duration::from_secs(120), &|_| {})
        .unwrap_or_else(|e| panic!("{}: {e}", input.display()));
    (done, output)
}

const VIDEO: [&str; 4] = [
    "-f",
    "lavfi",
    "-i",
    "testsrc=duration=2:size=320x240:rate=25",
];
const AUDIO: [&str; 4] = ["-f", "lavfi", "-i", "sine=frequency=440:duration=2"];

#[test]
fn prores_and_hevc_mov_become_h264_mp4() {
    if !enabled() {
        return;
    }
    let d = dir();
    for (name, codec_args) in [
        ("prores.mov", ["-c:v", "prores_ks"]),
        ("hevc.mov", ["-c:v", "libx265"]),
    ] {
        let input = d.join(name);
        let mut args = VIDEO.to_vec();
        args.extend(AUDIO);
        args.extend(codec_args);
        args.extend(["-tag:v", "hvc1", "-c:a", "alac", "-shortest"]);
        if name.starts_with("prores") {
            args.retain(|a| *a != "-tag:v" && *a != "hvc1");
        }
        make(&input, &args);
        let (done, out) = convert(&input, Target::Mp4);
        assert_eq!(done, Converted::Made);
        assert_eq!(codec(&out, "v"), "h264", "{name}");
        assert_eq!(codec(&out, "a"), "aac", "{name}");
    }
}

#[test]
fn apple_audio_becomes_aac_and_aac_is_left_alone() {
    if !enabled() {
        return;
    }
    let d = dir();
    for (name, codec_args) in [
        ("alac.m4a", vec!["-c:a", "alac"]),
        ("alac.caf", vec!["-c:a", "alac"]),
        ("pcm.aiff", vec![]),
    ] {
        let input = d.join(name);
        let mut args = AUDIO.to_vec();
        args.extend(codec_args);
        make(&input, &args);
        let (done, out) = convert(&input, Target::Aac);
        assert_eq!(done, Converted::Made, "{name}");
        assert_eq!(codec(&out, "a"), "aac", "{name}");
    }
    let aac = d.join("aac.m4a");
    let mut args = AUDIO.to_vec();
    args.extend(["-c:a", "aac"]);
    make(&aac, &args);
    assert_eq!(convert(&aac, Target::Aac).0, Converted::AlreadyFine);
}

#[test]
fn tiff_becomes_jpeg() {
    if !enabled() {
        return;
    }
    let input = dir().join("scan.tiff");
    make(
        &input,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240",
            "-frames:v",
            "1",
        ],
    );
    let (done, out) = convert(&input, Target::Jpeg);
    assert_eq!(done, Converted::Made);
    assert_eq!(codec(&out, "v"), "mjpeg");
}
