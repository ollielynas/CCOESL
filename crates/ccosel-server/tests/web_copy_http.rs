//! `GET /files/...?inline=1&as=web` and the `WebCopy` call, end to end, with a fake converter so
//! no ffmpeg is needed. What matters most here: a copy is only ever sent to someone who may read
//! the original, under the same `.access` rules as `/files`.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ccosel_proto::fs::{PathReq, Shown, WebCopy, WebCopyStatus};
use ccosel_proto::{Rpc, WireReply, WireRequest, WireResult, server_error};
use ccosel_server::auth::AuthState;
use ccosel_server::fs_api::Jail;
use ccosel_server::web_copy::plan::{AudioStream, VideoStream};
use ccosel_server::web_copy::{Converter, Probe, Recipe, WebCopies};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Probes by the file's extension (an HEVC `.mov`, ALAC `.caf`, MP3 `.mp3`), and converts by
/// writing `<extension>:` and the input, after `delay` for video. Counts its calls.
struct Fake {
    calls: AtomicUsize,
    video_delay: Duration,
}

impl Converter for Fake {
    fn probe(&self, input: &Path, _: Duration) -> Option<Probe> {
        let ext = input.extension()?.to_str()?;
        let stream = |codec: &str| AudioStream {
            codec: codec.to_owned(),
            sample_rate: 48_000,
        };
        Some(match ext {
            "mov" => Probe {
                format: "mov,mp4,m4a,3gp,3g2,mj2".to_owned(),
                duration_us: Some(1_000_000),
                video: Some(VideoStream {
                    codec: "hevc".to_owned(),
                    pix_fmt: "yuv420p".to_owned(),
                    width: 1920,
                    height: 1080,
                    ..VideoStream::default()
                }),
                audio: None,
            },
            "caf" => Probe {
                format: "caf".to_owned(),
                audio: Some(stream("alac")),
                ..Probe::default()
            },
            "mp3" => Probe {
                format: "mp3".to_owned(),
                audio: Some(stream("mp3")),
                ..Probe::default()
            },
            _ => Probe {
                format: "tty".to_owned(),
                ..Probe::default()
            },
        })
    }

    fn convert(
        &self,
        input: &Path,
        output: &Path,
        recipe: &Recipe,
        _: Duration,
        progress: &(dyn Fn(u16) + Sync),
    ) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if matches!(recipe, Recipe::Video { .. }) {
            progress(250);
            std::thread::sleep(self.video_delay);
        }
        let mut bytes = format!("{}:", recipe.target().extension()).into_bytes();
        bytes.extend(fs::read(input).unwrap());
        fs::write(output, bytes).unwrap();
        Ok(())
    }
}

/// The start of an HEIC photo.
const HEIC: &[u8] = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic";
/// The start of a JPEG.
const JPEG: &[u8] = b"\xFF\xD8\xFF\xE0\0\x10JFIF\0";
/// The start of a 3×2 PNG.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x03\0\0\0\x02\x08\x02\0\0\0\0\0\0\0";

struct Site {
    alice: SocketAddr,
    bob: SocketAddr,
    fake: Arc<Fake>,
    jail: PathBuf,
}

async fn site(video_delay: Duration) -> Site {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-web-copy-http-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    let (jail, web) = (dir.join("jail"), dir.join("web"));
    fs::create_dir_all(jail.join("Shared")).unwrap();
    fs::create_dir_all(jail.join("Private")).unwrap();
    fs::create_dir_all(&web).unwrap();
    fs::write(jail.join("Private/.access"), "read: alice\nwrite: alice\n").unwrap();
    fs::write(jail.join("Shared/IMG_1.HEIC"), HEIC).unwrap();
    fs::write(jail.join("Shared/clip.mov"), b"hevc").unwrap();
    fs::write(jail.join("Shared/song.caf"), b"alac").unwrap();
    fs::write(jail.join("Shared/tune.mp3"), b"ID3mp3").unwrap();
    fs::write(jail.join("Shared/photo.jpg"), JPEG).unwrap();
    fs::write(jail.join("Shared/really_a.jpg"), PNG).unwrap();
    fs::write(jail.join("Shared/notes.jpg"), b"just words").unwrap();
    fs::write(
        jail.join("Shared/page.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><script>alert(1)</script><rect width="4" height="4"/></svg>"#,
    )
    .unwrap();
    let mut secret = HEIC.to_vec();
    secret.extend(b"secret");
    fs::write(jail.join("Private/secret.heic"), secret).unwrap();

    let fake = Arc::new(Fake {
        calls: AtomicUsize::new(0),
        video_delay,
    });
    let cache = dir.join("cache");
    let serve = |user: &str| {
        let app = ccosel_server::app_with_copies(
            Jail::new(&jail).unwrap(),
            web.clone(),
            AuthState::default(),
            WebCopies::new(cache.clone(), 1 << 20, fake.clone()),
        );
        let app = ccosel_server::as_user(app, user.to_owned());
        async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            addr
        }
    };
    Site {
        alice: serve("alice").await,
        bob: serve("bob").await,
        fake,
        jail,
    }
}

struct Reply {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim())
        })
    }
}

async fn request(addr: SocketAddr, method: &str, target: &str, body: &[u8]) -> Reply {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "{method} {target} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    Reply {
        status: head[9..12].parse().unwrap(),
        head,
        body: raw[split + 4..].to_vec(),
    }
}

async fn get(addr: SocketAddr, target: &str) -> Reply {
    request(addr, "GET", target, b"").await
}

async fn web_copy(addr: SocketAddr, path: &str) -> Result<WebCopyStatus, u32> {
    let args = postcard::to_allocvec(&PathReq { path }).unwrap();
    let batch = postcard::to_allocvec(&vec![WireRequest {
        seq: 1,
        method: WebCopy::METHOD as u16,
        args: &args,
    }])
    .unwrap();
    let reply = request(addr, "POST", "/rpc", &batch).await;
    let replies: Vec<WireReply> = postcard::from_bytes(&reply.body).unwrap();
    match replies[0].result {
        WireResult::Ok(bytes) => Ok(postcard::from_bytes(bytes).unwrap()),
        WireResult::Err { code, .. } => Err(code),
    }
}

fn with(prefix: &str, original: &[u8]) -> Vec<u8> {
    let mut b = prefix.as_bytes().to_vec();
    b.extend(original);
    b
}

#[tokio::test]
async fn an_apple_photo_is_sent_as_webp_named_for_it() {
    let s = site(Duration::ZERO).await;
    let r = get(s.alice, "/files/Shared/IMG_1.HEIC?inline=1&as=web").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("image/webp"));
    assert_eq!(
        r.header("content-disposition"),
        Some("inline; filename=\"IMG_1.webp\"")
    );
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.body, with("webp:", HEIC));

    // Again, from the cache: no second conversion, and the same for another reader.
    let again = get(s.bob, "/files/Shared/IMG_1.HEIC?inline=1&as=web").await;
    assert_eq!(again.body, with("webp:", HEIC));
    assert_eq!(s.fake.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn apple_audio_is_sent_as_aac_and_mp3_as_it_is() {
    let s = site(Duration::ZERO).await;
    let r = get(s.alice, "/files/Shared/song.caf?inline=1&as=web").await;
    assert_eq!(r.header("content-type"), Some("audio/mp4"));
    assert_eq!(r.body, b"m4a:alac");
    let r = get(s.alice, "/files/Shared/tune.mp3?inline=1&as=web").await;
    assert_eq!(r.header("content-type"), Some("audio/mpeg"));
    assert_eq!(r.body, b"ID3mp3");
    assert_eq!(s.fake.calls.load(Ordering::SeqCst), 1);
}

/// An SVG can run script in the page, so it's only ever sent drawn, as a PNG.
#[tokio::test]
async fn an_svg_is_sent_drawn_never_as_itself() {
    let s = site(Duration::ZERO).await;
    let r = get(s.alice, "/files/Shared/page.svg?inline=1&as=web").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("image/png"));
    assert!(r.body.starts_with(b"\x89PNG"));
    assert!(!String::from_utf8_lossy(&r.body).contains("alert"));
    assert_eq!(
        web_copy(s.alice, "/Shared/page.svg").await.unwrap().shown,
        Some(Shown::Picture)
    );
    // Not inline at all without `as=web`: a download.
    let r = get(s.alice, "/files/Shared/page.svg?inline=1").await;
    assert_eq!(r.header("content-type"), Some("application/octet-stream"));
}

#[tokio::test]
async fn a_picture_is_sent_as_what_it_really_is() {
    let s = site(Duration::ZERO).await;
    let r = get(s.alice, "/files/Shared/photo.jpg?inline=1&as=web").await;
    assert_eq!(
        (r.header("content-type"), &r.body[..]),
        (Some("image/jpeg"), JPEG)
    );
    let r = get(s.alice, "/files/Shared/really_a.jpg?inline=1&as=web").await;
    assert_eq!(r.header("content-type"), Some("image/png"));
    assert_eq!(
        r.header("content-disposition"),
        Some("inline; filename=\"really_a.jpg\""),
        "its own name: it wasn't converted"
    );
    let status = web_copy(s.alice, "/Shared/really_a.jpg").await.unwrap();
    assert_eq!((status.width, status.height), (Some(3), Some(2)));
    assert_eq!(s.fake.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn something_that_isnt_media_is_never_sent_as_media() {
    let s = site(Duration::ZERO).await;
    let r = get(s.alice, "/files/Shared/notes.jpg?inline=1&as=web").await;
    assert_eq!(r.status, 415);
    let status = web_copy(s.alice, "/Shared/notes.jpg").await.unwrap();
    assert_eq!(
        (status.finished, status.shown),
        (true, Some(Shown::NotMedia))
    );
}

#[tokio::test]
async fn without_as_web_the_original_is_sent() {
    let s = site(Duration::ZERO).await;
    let r = get(s.alice, "/files/Shared/IMG_1.HEIC?inline=1").await;
    assert_eq!(r.body, HEIC);
    assert_eq!(r.header("content-type"), Some("image/heic"));
    // `as=web` on a download (no `inline`) is still the download.
    let r = get(s.alice, "/files/Shared/IMG_1.HEIC?as=web").await;
    assert_eq!(r.body, HEIC);
    assert_eq!(s.fake.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_copy_never_reaches_someone_who_may_not_read_the_original() {
    let s = site(Duration::ZERO).await;
    // Alice may read it, and her view puts a copy in the cache.
    let r = get(s.alice, "/files/Private/secret.heic?inline=1&as=web").await;
    assert!(r.body.ends_with(b"secret"));
    // Bob may not: refused exactly as the original is, cached copy or not.
    let r = get(s.bob, "/files/Private/secret.heic?inline=1&as=web").await;
    assert_eq!(r.status, 403);
    assert!(!String::from_utf8_lossy(&r.body).contains("secret"));
    assert_eq!(
        get(s.bob, "/files/Private/secret.heic?inline=1")
            .await
            .status,
        403
    );
    assert_eq!(
        web_copy(s.bob, "/Private/secret.heic").await,
        Err(server_error::NOT_FOUND)
    );
    assert_eq!(s.fake.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_slow_video_is_a_job_whose_progress_can_be_polled() {
    // Past the two seconds a fetch waits for a video.
    let s = site(Duration::from_millis(2_500)).await;
    let first = web_copy(s.alice, "/Shared/clip.mov").await.unwrap();
    assert!(!first.finished);
    // Fetching it before it's ready says so rather than waiting minutes.
    let early = get(s.alice, "/files/Shared/clip.mov?inline=1&as=web").await;
    assert_eq!(early.status, 202);

    let mut seen_progress = false;
    let done = loop {
        let status = web_copy(s.alice, "/Shared/clip.mov").await.unwrap();
        seen_progress |= status.permille == Some(250) && status.shown == Some(Shown::Video);
        if status.finished {
            break status;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(seen_progress, "progress was reported while it ran");
    assert_eq!(done.error, None);
    assert_eq!((done.width, done.height), (Some(1920), Some(1080)));
    let r = get(s.alice, "/files/Shared/clip.mov?inline=1&as=web").await;
    assert_eq!(r.header("content-type"), Some("video/mp4"));
    assert_eq!(r.body, b"mp4:hevc");
    assert_eq!(
        s.fake.calls.load(Ordering::SeqCst),
        1,
        "one job, however much it was polled"
    );
    assert!(
        s.jail.join("Shared/clip.mov").exists(),
        "the original is left alone"
    );
}
