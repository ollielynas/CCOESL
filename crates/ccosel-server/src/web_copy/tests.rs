use std::sync::atomic::AtomicUsize;

use resvg::tiny_skia;

use super::sniff::Picture;
use super::*;

/// Probes every file as `probe`, and converts by writing `COPY:` and the input's bytes after
/// `delay`, reporting half way; or fails. Counts its calls.
#[derive(Default)]
pub struct Fake {
    probe: Option<Probe>,
    calls: AtomicUsize,
    probes: AtomicUsize,
    delay: Duration,
    fail: bool,
}

impl Converter for Fake {
    fn probe(&self, _: &Path, _: Duration) -> Option<Probe> {
        self.probes.fetch_add(1, Ordering::SeqCst);
        self.probe.clone()
    }

    fn convert(
        &self,
        input: &Path,
        output: &Path,
        _: &Recipe,
        _: Duration,
        progress: &(dyn Fn(u16) + Sync),
    ) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        progress(500);
        std::thread::sleep(self.delay);
        if self.fail {
            return Err("it looks damaged".to_owned());
        }
        let mut bytes = b"COPY:".to_vec();
        bytes.extend(std::fs::read(input).map_err(|e| e.to_string())?);
        std::fs::write(output, bytes).map_err(|e| e.to_string())
    }
}

fn temp(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-web-copy-{}-{name}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    dir
}

fn copies(dir: &Path, fake: Fake, max_bytes: u64) -> (WebCopies, Arc<Fake>) {
    let fake = Arc::new(fake);
    (
        WebCopies::new(dir.join("cache"), max_bytes, fake.clone()),
        fake,
    )
}

/// Polls until the job for `real` finishes.
fn settle(c: &WebCopies, real: &Path) -> WebCopyStatus {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let s = c.status(real);
        if s.finished {
            return s;
        }
        assert!(Instant::now() < until, "never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x03\0\0\0\x02\x08\x02\0\0\0\0\0\0\0";

fn as_is(content_type: &'static str, animated: bool) -> Option<Picture> {
    Some(Picture::AsIs {
        content_type,
        animated,
    })
}

fn convert(magick: &'static str, magick_first: bool) -> Option<Picture> {
    Some(Picture::Convert {
        magick: Some(magick),
        magick_first,
    })
}

/// A PNG chunk: length, type, data, and a CRC nobody here checks.
fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = (data.len() as u32).to_be_bytes().to_vec();
    c.extend(kind);
    c.extend(data);
    c.extend([0; 4]);
    c
}

/// A GIF with `frames` 1×1 frames, no colour tables.
fn gif(frames: usize) -> Vec<u8> {
    let mut g = b"GIF89a\x01\0\x01\0\0\0\0".to_vec();
    // A graphic control extension, which animated GIFs have before each frame.
    for _ in 0..frames {
        g.extend(b"\x21\xF9\x04\0\x0A\0\0\0");
        g.extend(b"\x2C\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0");
    }
    g.push(0x3B);
    g
}

#[test]
fn pictures_every_browser_shows_are_sent_as_they_are() {
    let p = sniff::picture;
    assert_eq!(
        p(b"\xFF\xD8\xFF\xE0\0\x10JFIF", "jpg"),
        as_is("image/jpeg", false)
    );
    assert_eq!(p(PNG, "png"), as_is("image/png", false));
    assert_eq!(
        p(b"RIFF\0\0\0\0WEBPVP8 ", "webp"),
        as_is("image/webp", false)
    );
    assert_eq!(
        p(b"BM\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0", "bmp"),
        as_is("image/bmp", false)
    );
    assert_eq!(p(b"\0\0\x01\0\x01\0", "ico"), as_is("image/x-icon", false));
    assert_eq!(
        p(b"\0\0\0\x1cftypavif\0\0\0\0avifmif1miaf", "avif"),
        as_is("image/avif", false)
    );
    // Whatever the name says.
    assert_eq!(p(PNG, "jpg"), as_is("image/png", false));
    assert_eq!(p(PNG, ""), as_is("image/png", false));
}

#[test]
fn moving_pictures_are_told_from_still_ones() {
    let p = sniff::picture;
    assert_eq!(p(&gif(1), "gif"), as_is("image/gif", false));
    assert_eq!(p(&gif(2), "gif"), as_is("image/gif", true));
    assert_eq!(p(&gif(61), "gif"), as_is("image/gif", true));
    assert_eq!(
        sniff::gif_frames(&gif(3)),
        2,
        "counts no further than it needs"
    );
    assert_eq!(sniff::gif_frames(b"GIF89a"), 0, "a header and nothing else");

    let apng = [PNG, &chunk(b"acTL", &[0; 8]), &chunk(b"IDAT", &[0; 4])].concat();
    assert_eq!(p(&apng, "png"), as_is("image/png", true));
    let late = [PNG, &chunk(b"IDAT", &[0; 4]), &chunk(b"acTL", &[0; 8])].concat();
    assert_eq!(
        p(&late, "png"),
        as_is("image/png", false),
        "acTL counts only before IDAT"
    );

    assert_eq!(
        p(b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\x02\0\0\0", "webp"),
        as_is("image/webp", true)
    );
    assert_eq!(
        p(b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\x10\0\0\0", "webp"),
        as_is("image/webp", false),
        "an alpha flag alone isn't animation"
    );
    assert_eq!(
        p(b"\0\0\0\x1cftypavis\0\0\0\0avifavismsf1", "avif"),
        as_is("image/avif", true)
    );
}

#[test]
fn other_pictures_are_converted_with_the_right_reader() {
    let p = sniff::picture;
    let jpeg_arith = b"\xFF\xD8\xFF\xE0\0\x04ab\xFF\xC9\0\x0b";
    assert_eq!(p(jpeg_arith, "jpg"), convert("jpeg", true));
    assert_eq!(
        p(b"\xFF\xD8\xFF\xE0\0\x04ab\xFF\xC2\0\x0b", "jpg"),
        as_is("image/jpeg", false),
        "progressive is fine"
    );
    assert_eq!(
        p(b"\0\0\0\x18ftypheic\0\0\0\0mif1heic", "heic"),
        convert("heic", false)
    );
    assert_eq!(
        p(b"\0\0\0\x18ftypisom\0\0\0\0isomavc1", "mp4"),
        None,
        "a video"
    );
    assert_eq!(p(b"II*\0\x08\0\0\0", "tif"), convert("tiff", true));
    assert_eq!(
        p(b"MM\0*\0\0\0\x08", "DNG".to_lowercase().as_str()),
        convert("dng", true)
    );
    assert_eq!(p(b"8BPS\0\x01", "psd"), convert("psd", true));
    assert_eq!(p(b"\xFF\x0A\0", "jxl"), convert("jxl", false));
    assert_eq!(p(b"\0\0\0\x0CJXL \r\n\x87\n", "jxl"), convert("jxl", false));
    assert_eq!(p(b"\0\0\0\x0CjP  \r\n\x87\n", "jp2"), convert("jp2", false));
    assert_eq!(p(b"\xFF\x4F\xFF\x51", "j2k"), convert("j2k", false));
    assert_eq!(p(b"\x76\x2F\x31\x01", "exr"), convert("exr", false));
    assert_eq!(p(b"#?RADIANCE\n", "hdr"), convert("hdr", false));
    assert_eq!(p(b"qoif\0\0", "qoi"), convert("qoi", false));
    assert_eq!(p(b"DDS |\0", "dds"), convert("dds", false));
    assert_eq!(p(b"\x01\xDA\x01\x01", "sgi"), convert("sgi", false));
    assert_eq!(p(b"/* XPM */\n", "xpm"), convert("xpm", false));
    assert_eq!(p(b"P6\n640 480\n", "ppm"), convert("pnm", false));
    assert_eq!(p(b"Pf\n640 480\n", "pfm"), convert("pfm", false));
    assert_eq!(p(b"P7\nWIDTH 1\n", "pam"), convert("pam", false));
    assert_eq!(p(b"\0\0\x02\0\x01\0", "cur"), convert("cur", false));
    // No magic bytes: the name decides.
    assert_eq!(p(b"\0\0\x02\0\0\0", "tga"), convert("tga", false));
    assert_eq!(p(b"\x0A\x05\x01\x08", "pcx"), convert("pcx", false));
    assert_eq!(p(b"\0\0\x80\x01", "wbmp"), convert("wbmp", false));
    assert_eq!(p(b"\0\0\x80\x01", "bin"), None);
}

#[test]
fn svg_is_recognised_by_its_contents() {
    let p = sniff::picture;
    assert_eq!(
        p(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>", "txt"),
        Some(Picture::Svg)
    );
    assert_eq!(
        p(
            b"\xEF\xBB\xBF<?xml version=\"1.0\"?>\n<!DOCTYPE x>\n<svg/>",
            "svg"
        ),
        Some(Picture::Svg)
    );
    assert_eq!(p(b"<html><body>no drawing</body></html>", "svg"), None);
    assert_eq!(p(b"{\"svg\": true}", "jpg"), None, "JSON isn't a picture");
    assert_eq!(p(b"", "png"), None);
}

fn video(codec: &str, pix_fmt: &str) -> VideoStream {
    VideoStream {
        codec: codec.to_owned(),
        profile: "High".to_owned(),
        pix_fmt: pix_fmt.to_owned(),
        field_order: "progressive".to_owned(),
        width: 320,
        height: 240,
        ..VideoStream::default()
    }
}

fn audio(codec: &str) -> AudioStream {
    AudioStream {
        codec: codec.to_owned(),
        sample_rate: 48_000,
    }
}

use plan::{AudioStream, VideoStream};

fn probe(format: &str, v: Option<VideoStream>, a: Option<AudioStream>) -> Probe {
    Probe {
        format: format.to_owned(),
        duration_us: Some(3_000_000),
        video: v,
        audio: a,
    }
}

const MP4: &str = "mov,mp4,m4a,3gp,3g2,mj2";
const MKV: &str = "matroska,webm";

fn plays(content_type: &'static str, shown: Shown) -> Plan {
    Plan::AsIs {
        shown,
        content_type,
    }
}

#[test]
fn video_every_browser_plays_is_sent_as_it_is() {
    let h264 = || Some(video("h264", "yuv420p"));
    assert_eq!(
        plan::media(&probe(MP4, h264(), Some(audio("aac"))), "mp4"),
        plays("video/mp4", Shown::Video)
    );
    assert_eq!(
        plan::media(&probe(MP4, h264(), None), "m4v"),
        plays("video/mp4", Shown::Video)
    );
    assert_eq!(
        plan::media(
            &probe(MKV, Some(video("vp9", "yuv420p")), Some(audio("opus"))),
            "webm"
        ),
        plays("video/webm", Shown::Video)
    );
    assert_eq!(
        plan::media(
            &probe(MKV, Some(video("vp8", "yuv420p")), Some(audio("vorbis"))),
            "webm"
        ),
        plays("video/webm", Shown::Video)
    );
}

fn recipe(p: Plan) -> Recipe {
    match p {
        Plan::Convert { recipe, .. } => recipe,
        other => panic!("not converted: {other:?}"),
    }
}

#[test]
fn other_video_becomes_h264_copying_what_it_can() {
    // H.264 that only needs a new container.
    let r = recipe(plan::media(
        &probe(MP4, Some(video("h264", "yuv420p")), Some(audio("aac"))),
        "mov",
    ));
    assert_eq!(
        r,
        Recipe::Video {
            copy_video: true,
            audio: Some(true),
            tone_map: false,
            deinterlace: false,
            duration_us: Some(3_000_000),
        }
    );
    // A WebM named .mp4 isn't sent as an MP4.
    let r = recipe(plan::media(
        &probe(MKV, Some(video("vp9", "yuv420p")), None),
        "mp4",
    ));
    assert!(matches!(
        r,
        Recipe::Video {
            copy_video: false,
            audio: None,
            ..
        }
    ));
    // H.264 browsers don't decode is re-encoded, not copied.
    for (pix_fmt, profile) in [
        ("yuv420p10le", "High 10"),
        ("yuv444p", "High 4:4:4 Predictive"),
        ("yuv422p", "High 4:2:2"),
        ("yuvj420p", "High"),
        ("yuv420p", "High 4:4:4 Predictive"),
    ] {
        let mut v = video("h264", pix_fmt);
        v.profile = profile.to_owned();
        let r = recipe(plan::media(&probe(MP4, Some(v), Some(audio("mp3"))), "mp4"));
        assert!(
            matches!(
                r,
                Recipe::Video {
                    copy_video: false,
                    audio: Some(false),
                    ..
                }
            ),
            "{profile} {pix_fmt}: {r:?}"
        );
    }
    // HDR is tone-mapped, interlaced is deinterlaced.
    let mut hdr = video("hevc", "yuv420p10le");
    hdr.transfer = "smpte2084".to_owned();
    assert!(matches!(
        recipe(plan::media(&probe(MP4, Some(hdr), None), "mp4")),
        Recipe::Video { tone_map: true, .. }
    ));
    let mut hlg = video("vp9", "yuv420p10le");
    hlg.transfer = "arib-std-b67".to_owned();
    assert!(matches!(
        recipe(plan::media(&probe(MKV, Some(hlg), None), "webm")),
        Recipe::Video { tone_map: true, .. }
    ));
    let mut tff = video("h264", "yuv420p");
    tff.field_order = "tt".to_owned();
    assert!(matches!(
        recipe(plan::media(&probe("mpegts", Some(tff), None), "ts")),
        Recipe::Video {
            deinterlace: true,
            copy_video: false,
            ..
        }
    ));
    for (format, codec) in [
        ("avi", "mpeg4"),
        ("asf", "wmv2"),
        ("flv", "flv1"),
        ("mpeg", "mpeg2video"),
        ("rm", "rv20"),
        ("h264", "h264"),
    ] {
        let p = plan::media(&probe(format, Some(video(codec, "yuv420p")), None), "x");
        assert!(
            matches!(
                p,
                Plan::Convert {
                    shown: Shown::Video,
                    ..
                }
            ),
            "{format}"
        );
    }
}

#[test]
fn sound_every_browser_plays_is_sent_as_it_is_and_the_rest_becomes_aac() {
    let a = |format: &str, codec: &str, ext: &str| {
        plan::media(&probe(format, None, Some(audio(codec))), ext)
    };
    assert_eq!(a("mp3", "mp3", "mp3"), plays("audio/mpeg", Shown::Audio));
    assert_eq!(a("aac", "aac", "aac"), plays("audio/aac", Shown::Audio));
    assert_eq!(a(MP4, "aac", "m4a"), plays("audio/mp4", Shown::Audio));
    assert_eq!(a("flac", "flac", "flac"), plays("audio/flac", Shown::Audio));
    assert_eq!(
        a("wav", "pcm_s24le", "wav"),
        plays("audio/wav", Shown::Audio)
    );
    assert_eq!(a("ogg", "opus", "opus"), plays("audio/ogg", Shown::Audio));
    assert_eq!(a("ogg", "vorbis", "ogg"), plays("audio/ogg", Shown::Audio));
    for (format, codec, ext) in [
        (MP4, "alac", "m4a"),
        ("caf", "pcm_s16le", "caf"),
        ("aiff", "pcm_s16be", "aiff"),
        ("w64", "pcm_s16le", "w64"),
        ("asf", "wmav2", "wma"),
        ("ac3", "ac3", "ac3"),
        ("eac3", "eac3", "eac3"),
        ("mp3", "mp2", "mp2"),
        // MP3 called .wav: sent as AAC rather than as a WAV that isn't one.
        ("mp3", "mp3", "wav"),
    ] {
        assert_eq!(
            a(format, codec, ext),
            Plan::Convert {
                shown: Shown::Audio,
                recipe: Recipe::Audio { resample: false }
            },
            "{format} {codec} .{ext}"
        );
    }
    let mut hi = audio("pcm_s24le");
    hi.sample_rate = 192_000;
    assert_eq!(
        plan::media(&probe("w64", None, Some(hi)), "w64"),
        Plan::Convert {
            shown: Shown::Audio,
            recipe: Recipe::Audio { resample: true }
        }
    );
}

#[test]
fn what_isnt_media_is_said_so() {
    assert_eq!(
        plan::media(&probe("tty", Some(video("ansi", "")), None), "txt"),
        Plan::NotMedia
    );
    assert_eq!(
        plan::media(&probe("mp3", None, None), "mp3"),
        Plan::NotMedia
    );
    // A picture format only ffmpeg knows, and a "picture" it only knows by its name.
    let mut sized = video("png", "rgb24");
    sized.width = 10;
    sized.height = 10;
    assert!(matches!(
        plan::media(&probe("png_pipe", Some(sized), None), "png"),
        Plan::Convert {
            shown: Shown::Picture,
            ..
        }
    ));
    let mut no_size = video("mjpeg", "");
    no_size.width = 0;
    no_size.height = 0;
    assert_eq!(
        plan::media(&probe("image2", Some(no_size), None), "jpg"),
        Plan::NotMedia
    );
}

#[test]
fn ffprobes_json_is_read() {
    let json = r#"{
        "streams": [
            {"index": 0, "codec_name": "mjpeg", "codec_type": "video", "width": 600, "height": 600,
             "disposition": {"attached_pic": 1}},
            {"index": 1, "codec_name": "h264", "profile": "High", "codec_type": "video",
             "width": 1920, "height": 1080, "pix_fmt": "yuv420p", "field_order": "progressive",
             "color_transfer": "bt709", "disposition": {"attached_pic": 0},
             "side_data_list": [{"side_data_type": "Display Matrix", "rotation": -90}]},
            {"index": 2, "codec_name": "aac", "codec_type": "audio", "sample_rate": "44100"}
        ],
        "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "12.500000"}
    }"#;
    let p = Probe::from_json(json).unwrap();
    assert_eq!(p.format, MP4);
    assert_eq!(p.duration_us, Some(12_500_000));
    let v = p.video.as_ref().unwrap();
    assert_eq!(v.codec, "h264", "the cover art isn't the video");
    assert_eq!(v.rotation, -90);
    assert_eq!(
        v.shown_size(),
        Some((1080, 1920)),
        "turned for its rotation"
    );
    assert_eq!(p.audio.as_ref().unwrap().sample_rate, 44_100);
    assert_eq!(Probe::from_json("not json"), None);
    let empty = Probe::from_json("{}").unwrap();
    assert_eq!(
        (empty.video, empty.audio, empty.duration_us),
        (None, None, None)
    );
}

#[test]
fn re_encoded_video_is_filtered_for_every_browser() {
    let plain = tools::video_filters(false, false);
    assert!(
        plain.contains("out_range=tv"),
        "full range brought to TV range"
    );
    assert!(plain.ends_with("format=yuv420p"));
    assert!(!plain.contains("tonemap") && !plain.contains("bwdif"));
    let all = tools::video_filters(true, true);
    assert!(all.starts_with("bwdif"));
    assert!(all.contains("tonemap=hable"));
}

#[test]
fn only_media_types_are_ever_sent_unconverted() {
    assert_eq!(media_type("image/png"), Some("image/png"));
    assert_eq!(media_type("audio/ogg"), Some("audio/ogg"));
    for bad in ["text/html", "image/svg+xml", "application/javascript", ""] {
        assert_eq!(media_type(bad), None, "{bad}");
    }
}

#[test]
fn a_decision_is_written_and_read_back() {
    let copy = |t: Target| PathBuf::from(format!("/c/k.{}", t.extension()));
    for ready in [
        Ready {
            shown: Shown::Animated,
            copy: None,
            content_type: "image/gif".to_owned(),
            size: Some((200, 150)),
        },
        Ready {
            shown: Shown::Video,
            copy: Some((copy(Target::Mp4), Target::Mp4)),
            content_type: "video/mp4".to_owned(),
            size: None,
        },
        Ready {
            shown: Shown::NotMedia,
            copy: None,
            content_type: String::new(),
            size: None,
        },
    ] {
        assert_eq!(Ready::from_line(&ready.to_line(), copy), Some(ready));
    }
    assert_eq!(Ready::from_line("v0 picture - image/png 1 1", copy), None);
    assert_eq!(Ready::from_line("v1 sculpture - - - -", copy), None);
}

#[test]
fn the_key_follows_the_files_path_size_and_time() {
    let dir = temp("key");
    let a = dir.join("files/a.heic");
    let b = dir.join("files/b.heic");
    std::fs::write(&a, "one").unwrap();
    std::fs::write(&b, "one").unwrap();
    let ka = WebCopies::key(&a).unwrap();
    assert_eq!(ka, WebCopies::key(&a).unwrap(), "stable");
    assert_ne!(ka, WebCopies::key(&b).unwrap(), "by path");

    std::fs::write(&a, "longer").unwrap();
    assert_ne!(ka, WebCopies::key(&a).unwrap(), "by size");
    let k2 = WebCopies::key(&a).unwrap();
    let f = std::fs::File::options().append(true).open(&a).unwrap();
    f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000))
        .unwrap();
    assert_ne!(k2, WebCopies::key(&a).unwrap(), "by time");
    assert_eq!(WebCopies::key(&dir.join("missing")), None);
}

#[test]
fn a_picture_browsers_show_is_sent_as_it_is_with_its_size() {
    let dir = temp("as-is");
    let src = dir.join("files/shot.jpg");
    // Really a PNG, 3×2.
    std::fs::write(&src, PNG).unwrap();
    let (c, fake) = copies(&dir, Fake::default(), 1 << 20);
    let s = settle(&c, &src);
    assert_eq!(s.error, None);
    assert_eq!(s.shown, Some(Shown::Picture));
    assert_eq!((s.width, s.height), (Some(3), Some(2)));
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0, "nothing converted");
    assert_eq!(
        fake.probes.load(Ordering::SeqCst),
        0,
        "a picture needs no ffprobe"
    );
    let ready = c.cached(&src).unwrap();
    assert_eq!(ready.copy, None);
    assert_eq!(ready.content_type, "image/png", "sent as what it is");
}

/// A JPEG of `w`×`h` whose EXIF says it's turned to `orientation`.
fn jpeg_with_orientation(w: u16, h: u16, orientation: u16) -> Vec<u8> {
    let mut tiff = b"II*\0\x08\0\0\0\x01\0".to_vec();
    tiff.extend(0x0112u16.to_le_bytes());
    tiff.extend(3u16.to_le_bytes());
    tiff.extend(1u32.to_le_bytes());
    tiff.extend(orientation.to_le_bytes());
    tiff.extend([0, 0, 0, 0, 0, 0]);
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend(tiff);
    let mut j = b"\xFF\xD8\xFF\xE1".to_vec();
    j.extend(((app1.len() + 2) as u16).to_be_bytes());
    j.extend(app1);
    j.extend(b"\xFF\xC0\0\x11\x08");
    j.extend(h.to_be_bytes());
    j.extend(w.to_be_bytes());
    j.extend(b"\x03\x01\x22\0\x02\x11\x01\x03\x11\x01");
    j.extend(b"\xFF\xD9");
    j
}

#[test]
fn a_photo_tagged_as_turned_is_sent_turned_the_right_way_up() {
    let dir = temp("orientation");
    let (c, _) = copies(&dir, Fake::default(), 1 << 20);
    let fake = Fake::default();
    for orientation in 0..=9u16 {
        let src = dir.join(format!("files/o{orientation}.jpg"));
        std::fs::write(&src, jpeg_with_orientation(640, 480, orientation)).unwrap();
        let (plan, _) = decide(&fake, &src);
        if (2..=8).contains(&orientation) {
            assert_eq!(
                plan,
                Plan::Convert {
                    shown: Shown::Picture,
                    recipe: Recipe::Picture {
                        magick: Some("jpeg"),
                        magick_first: true,
                        orientation: u32::from(orientation),
                    }
                },
                "orientation {orientation}"
            );
        } else {
            // 1, and 0 and 9, which aren't orientations: as it is.
            assert_eq!(
                plan,
                Plan::AsIs {
                    shown: Shown::Picture,
                    content_type: "image/jpeg"
                },
                "orientation {orientation}"
            );
            assert_eq!(
                (settle(&c, &src).width, c.cached(&src).unwrap().size),
                (Some(640), Some((640, 480)))
            );
        }
    }
}

#[test]
fn turning_is_the_same_in_both_tools() {
    assert!(tools::magick_turn(1).is_empty());
    assert_eq!(tools::ffmpeg_turn(1), None);
    for o in 2..=8 {
        assert!(!tools::magick_turn(o).is_empty(), "{o}");
        assert!(tools::ffmpeg_turn(o).is_some(), "{o}");
    }
    assert_eq!(tools::magick_turn(6), ["-rotate", "90"]);
    assert_eq!(tools::ffmpeg_turn(6), Some("transpose=1"));
    assert_eq!(tools::magick_turn(8), ["-rotate", "270"]);
    assert_eq!(tools::ffmpeg_turn(8), Some("transpose=2"));
}

#[test]
fn moving_pictures_are_marked_so() {
    let dir = temp("gif");
    let src = dir.join("files/cat.gif");
    std::fs::write(&src, gif(5)).unwrap();
    let (c, _) = copies(&dir, Fake::default(), 1 << 20);
    assert_eq!(settle(&c, &src).shown, Some(Shown::Animated));
}

#[test]
fn a_copy_is_made_once_and_then_found() {
    let dir = temp("once");
    let src = dir.join("files/p.tif");
    std::fs::write(&src, b"II*\0pixels").unwrap();
    let (c, fake) = copies(&dir, Fake::default(), 1 << 20);

    let s = settle(&c, &src);
    assert_eq!((s.error, s.shown), (None, Some(Shown::Picture)));
    let ready = c.cached(&src).unwrap();
    let Some((copy, Target::Webp)) = &ready.copy else {
        panic!("no copy: {ready:?}");
    };
    assert_eq!(std::fs::read(copy).unwrap(), b"COPY:II*\0pixels");
    assert_eq!(ready.content_type, "image/webp");

    for _ in 0..3 {
        assert!(c.status(&src).finished);
    }
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1, "converted once");
    // A new cache over the same folder (the server restarted) still has it.
    let (again, fake2) = copies(&dir, Fake::default(), 1 << 20);
    assert!(again.status(&src).finished);
    assert_eq!(fake2.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_changed_file_is_looked_at_again() {
    let dir = temp("changed");
    let src = dir.join("files/p.tif");
    std::fs::write(&src, b"II*\0one").unwrap();
    let (c, fake) = copies(&dir, Fake::default(), 1 << 20);
    settle(&c, &src);
    std::fs::write(&src, b"II*\0two!").unwrap();
    assert!(c.cached(&src).is_none());
    settle(&c, &src);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    let (copy, _) = c.cached(&src).unwrap().copy.unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), b"COPY:II*\0two!");
}

fn a_video() -> Fake {
    Fake {
        probe: Some(probe(
            "avi",
            Some(video("mpeg4", "yuv420p")),
            Some(audio("mp3")),
        )),
        ..Fake::default()
    }
}

#[test]
fn a_long_conversion_reports_progress_and_a_short_wait_gives_up() {
    let dir = temp("progress");
    let src = dir.join("files/v.avi");
    std::fs::write(&src, "RIFF....AVI ").unwrap();
    let (c, fake) = copies(
        &dir,
        Fake {
            delay: Duration::from_millis(400),
            ..a_video()
        },
        1 << 20,
    );
    let first = c.status(&src);
    assert!(!first.finished);
    // Once it's known to be a video, only the short wait applies.
    let until = Instant::now() + Duration::from_secs(5);
    while c.status(&src).shown != Some(Shown::Video) {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(c.status(&src).permille, Some(500));
    assert_eq!(
        c.wait(&src, Duration::from_millis(30), Duration::from_secs(60)),
        None,
        "still going"
    );
    let done = settle(&c, &src);
    assert_eq!(
        (done.permille, done.shown),
        (Some(1000), Some(Shown::Video))
    );
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        1,
        "one job however often it's polled"
    );
    let ready = c
        .wait(&src, Duration::ZERO, Duration::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(ready.copy.unwrap().1, Target::Mp4);
    assert_eq!(ready.size, Some((320, 240)), "from the probe");
}

#[test]
fn a_failure_is_reported_and_not_retried_straight_away() {
    let dir = temp("fail");
    let src = dir.join("files/v.avi");
    std::fs::write(&src, "RIFF....AVI ").unwrap();
    let (c, fake) = copies(
        &dir,
        Fake {
            fail: true,
            ..a_video()
        },
        1 << 20,
    );
    let s = settle(&c, &src);
    assert_eq!(s.error.as_deref(), Some("it looks damaged"));
    assert_eq!(c.status(&src).error.as_deref(), Some("it looks damaged"));
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert!(
        std::fs::read_dir(dir.join("cache"))
            .unwrap()
            .filter_map(Result::ok)
            .all(|e| !e.file_name().to_string_lossy().contains(".part.")),
        "no half-made copy left"
    );
    assert_eq!(
        c.wait(&src, Duration::ZERO, Duration::ZERO),
        Some(Err("it looks damaged".to_owned()))
    );
}

#[test]
fn something_that_isnt_media_says_so() {
    let dir = temp("not-media");
    let (c, fake) = copies(&dir, Fake::default(), 1 << 20);
    let text = dir.join("files/notes.jpg");
    std::fs::write(&text, "just some words").unwrap();
    let s = settle(&c, &text);
    assert_eq!((s.shown, s.error), (Some(Shown::NotMedia), None));
    assert_eq!(fake.probes.load(Ordering::SeqCst), 1, "ffprobe had its say");

    let empty = dir.join("files/zero.mp4");
    std::fs::write(&empty, "").unwrap();
    assert_eq!(settle(&c, &empty).shown, Some(Shown::NotMedia));
    assert_eq!(
        fake.probes.load(Ordering::SeqCst),
        1,
        "nothing to ask about"
    );
    assert_eq!(
        c.status(&dir.join("files/missing.png")).error.as_deref(),
        Some("the file couldn't be read")
    );
}

#[test]
fn the_least_recently_used_go_first() {
    let dir = temp("evict");
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let old = SystemTime::now() - Duration::from_secs(100);
    for (name, age) in [("a.mp4", 10), ("b.mp4", 20), ("c.mp4", 30)] {
        let p = cache.join(name);
        std::fs::write(&p, [0u8; 100]).unwrap();
        let f = std::fs::File::options().append(true).open(&p).unwrap();
        f.set_modified(old + Duration::from_secs(age)).unwrap();
    }
    std::fs::write(cache.join("d.part.mp4"), [0u8; 100]).unwrap();
    evict(&cache, 150);
    let left: Vec<_> = {
        let mut v: Vec<_> = std::fs::read_dir(&cache)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        left,
        ["c.mp4", "d.part.mp4"],
        "oldest out; half-made ones kept"
    );
}

/// A 4×4 PNG, all red.
fn red_png() -> Vec<u8> {
    let mut p = tiny_skia::Pixmap::new(4, 4).unwrap();
    p.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
    p.encode_png().unwrap()
}

/// The alpha of the drawing's middle pixel.
fn middle_alpha(png: &Path) -> u8 {
    let p = tiny_skia::Pixmap::load_png(png).unwrap();
    p.pixel(p.width() / 2, p.height() / 2).unwrap().alpha()
}

fn render(dir: &Path, name: &str, svg: &str) -> Result<(u32, u32), String> {
    let input = dir.join(format!("{name}.svg"));
    std::fs::write(&input, svg).unwrap();
    svg::render(&input, &dir.join(format!("{name}.png")))
}

#[test]
fn an_svg_is_drawn_at_a_sharp_size_and_reports_its_own() {
    let dir = temp("svg");
    let size = render(
        &dir,
        "plain",
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="240"><rect width="320" height="240" fill="navy"/></svg>"#,
    )
    .unwrap();
    assert_eq!(size, (320, 240));
    let p = tiny_skia::Pixmap::load_png(dir.join("plain.png")).unwrap();
    assert_eq!((p.width(), p.height()), (svg::SIDE, svg::SIDE * 3 / 4));
    assert_eq!(middle_alpha(&dir.join("plain.png")), 255);

    // No size of its own, and one absurdly big: both drawn, at the usual size.
    assert!(
        render(
            &dir,
            "nosize",
            r#"<svg xmlns="http://www.w3.org/2000/svg"><circle cx="50" cy="50" r="40"/></svg>"#
        )
        .is_ok()
    );
    assert!(render(&dir, "huge", r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 1000000 1000000"><rect width="1000000" height="1000000"/></svg>"#).is_ok());
    assert!(render(&dir, "junk", "<svg").is_err());
}

/// The server can read files the person viewing can't. An SVG must never be a way to see one.
#[test]
fn an_svg_never_draws_anything_from_outside_itself() {
    let dir = temp("svg-refs");
    let secret = dir.join("files/secret.png");
    std::fs::write(&secret, red_png()).unwrap();
    let image = |href: &str| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image xlink:href="{href}" width="10" height="10"/></svg>"#
        )
    };
    // The same picture inside the file is drawn: so the test can tell.
    let data = format!(
        "data:image/png;base64,{}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, red_png())
    );
    render(&dir, "data", &image(&data)).unwrap();
    assert_eq!(middle_alpha(&dir.join("data.png")), 255);

    for (name, href) in [
        ("absolute", secret.to_string_lossy().into_owned()),
        ("file-url", format!("file://{}", secret.display())),
        ("relative", "files/secret.png".to_owned()),
        ("remote", "http://example.invalid/tracker.png".to_owned()),
    ] {
        render(&dir, name, &image(&href)).unwrap();
        assert_eq!(
            middle_alpha(&dir.join(format!("{name}.png"))),
            0,
            "{name}: {href}"
        );
    }
}

#[test]
fn an_svgs_scripts_and_entities_are_harmless() {
    let dir = temp("svg-tricks");
    // Drawn, and nothing in it runs: the script and link are just ignored.
    render(&dir, "script", r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><script>alert(1)</script><rect width="100" height="100" fill="green"/><a href="javascript:alert(2)"><text x="10" y="50">click</text></a></svg>"#).unwrap();
    assert_eq!(middle_alpha(&dir.join("script.png")), 255);
    // Entity expansion is bounded by the parser; this one is small enough to just draw.
    render(&dir, "laughs", r#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">]><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>&lol2;</text></svg>"#).unwrap();
}

#[test]
fn an_svg_too_big_to_draw_is_refused() {
    let dir = temp("svg-big");
    let input = dir.join("big.svg");
    let f = std::fs::File::create(&input).unwrap();
    f.set_len(svg::MAX_BYTES + 1).unwrap();
    let why = svg::render(&input, &dir.join("big.png")).unwrap_err();
    assert!(why.contains("too big"), "{why}");
}

#[test]
fn an_svg_goes_through_the_cache_as_a_png() {
    let dir = temp("svg-cache");
    let src = dir.join("files/logo.svg");
    std::fs::write(
        &src,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32"><rect width="64" height="32"/></svg>"#,
    )
    .unwrap();
    let (c, fake) = copies(&dir, Fake::default(), 1 << 20);
    let s = settle(&c, &src);
    assert_eq!(
        (s.shown, s.width, s.height),
        (Some(Shown::Picture), Some(64), Some(32))
    );
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        0,
        "drawn here, not by a tool"
    );
    assert_eq!(c.cached(&src).unwrap().copy.unwrap().1, Target::Png);
}
