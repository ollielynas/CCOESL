//! `GET /files/...?inline=1&as=web` and the `WebCopy` call, end to end, with a fake converter so
//! no ffmpeg is needed. What matters most here: a copy is only ever sent to someone who may read
//! the original, under the same `.access` rules as `/files`.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ccosel_proto::fs::{PathReq, WebCopy, WebCopyStatus};
use ccosel_proto::{Rpc, WireReply, WireRequest, WireResult, server_error};
use ccosel_server::auth::AuthState;
use ccosel_server::fs_api::Jail;
use ccosel_server::web_copy::{Converted, Converter, Target, WebCopies};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Writes `<TARGET>:` and the input, after `delay` for video. Counts its calls.
struct Fake {
    calls: AtomicUsize,
    video_delay: Duration,
}

impl Converter for Fake {
    fn convert(
        &self,
        input: &Path,
        output: &Path,
        target: Target,
        _: Duration,
        progress: &(dyn Fn(u16) + Sync),
    ) -> Result<Converted, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if target == Target::Mp4 {
            progress(250);
            std::thread::sleep(self.video_delay);
        }
        let mut bytes = format!("{}:", target.extension()).into_bytes();
        bytes.extend(fs::read(input).unwrap());
        fs::write(output, bytes).unwrap();
        Ok(Converted::Made)
    }
}

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
    fs::write(jail.join("Shared/IMG_1.HEIC"), b"heic").unwrap();
    fs::write(jail.join("Shared/clip.mov"), b"hevc").unwrap();
    fs::write(jail.join("Shared/song.caf"), b"alac").unwrap();
    fs::write(jail.join("Shared/photo.jpg"), b"jpeg").unwrap();
    fs::write(jail.join("Private/secret.heic"), b"secret").unwrap();

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

#[tokio::test]
async fn an_apple_photo_is_sent_as_a_jpeg_named_for_it() {
    let s = site(Duration::ZERO).await;
    let r = get(s.alice, "/files/Shared/IMG_1.HEIC?inline=1&as=web").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("image/jpeg"));
    assert_eq!(
        r.header("content-disposition"),
        Some("inline; filename=\"IMG_1.jpg\"")
    );
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.body, b"jpg:heic");

    // Again, from the cache: no second conversion, and the same for another reader.
    let again = get(s.bob, "/files/Shared/IMG_1.HEIC?inline=1&as=web").await;
    assert_eq!(again.body, b"jpg:heic");
    assert_eq!(s.fake.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn apple_audio_is_sent_as_aac() {
    let s = site(Duration::ZERO).await;
    let r = get(s.alice, "/files/Shared/song.caf?inline=1&as=web").await;
    assert_eq!(r.header("content-type"), Some("audio/mp4"));
    assert_eq!(r.body, b"m4a:alac");
}

#[tokio::test]
async fn without_as_web_or_for_other_formats_the_original_is_sent() {
    let s = site(Duration::ZERO).await;
    let r = get(s.alice, "/files/Shared/IMG_1.HEIC?inline=1").await;
    assert_eq!(r.body, b"heic");
    assert_eq!(r.header("content-type"), Some("image/heic"));
    let r = get(s.alice, "/files/Shared/photo.jpg?inline=1&as=web").await;
    assert_eq!(r.body, b"jpeg");
    // `as=web` on a download (no `inline`) is still the download.
    let r = get(s.alice, "/files/Shared/IMG_1.HEIC?as=web").await;
    assert_eq!(r.body, b"heic");
    assert_eq!(s.fake.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_copy_never_reaches_someone_who_may_not_read_the_original() {
    let s = site(Duration::ZERO).await;
    // Alice may read it, and her view puts a copy in the cache.
    let r = get(s.alice, "/files/Private/secret.heic?inline=1&as=web").await;
    assert_eq!(r.body, b"jpg:secret");
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
        seen_progress |= status.permille == Some(250);
        if status.finished {
            break status;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(seen_progress, "progress was reported while it ran");
    assert_eq!(done.error, None);
    let r = get(s.alice, "/files/Shared/clip.mov?inline=1&as=web").await;
    assert_eq!(r.header("content-type"), Some("video/mp4"));
    assert_eq!(r.body, b"mp4:hevc");
    assert_eq!(
        s.fake.calls.load(Ordering::SeqCst),
        1,
        "one job, however much it was polled"
    );
}

#[tokio::test]
async fn web_copy_refuses_formats_that_need_no_copy() {
    let s = site(Duration::ZERO).await;
    assert_eq!(
        web_copy(s.alice, "/Shared/photo.jpg").await,
        Err(server_error::MALFORMED)
    );
    assert!(s.jail.join("Shared/photo.jpg").exists());
}
