//! `GET /files/...?inline=1`: what the Viewer shows a file with. Pictures, audio, video and PDF
//! come with their own type and can be seeked in; anything that could run script is still only a
//! download.

use std::fs;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_server::auth::AuthState;
use ccosel_server::fs_api::Jail;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn spawn() -> SocketAddr {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-inline-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    let (jail, web) = (dir.join("jail"), dir.join("web"));
    fs::create_dir_all(jail.join("Media")).unwrap();
    fs::create_dir_all(&web).unwrap();
    fs::write(jail.join("Media/clip.mp4"), b"0123456789").unwrap();
    fs::write(jail.join("Media/photo.JPG"), b"jpeg").unwrap();
    fs::write(jail.join("Media/page.html"), b"<script>alert(1)</script>").unwrap();
    fs::write(jail.join("Media/drawing.svg"), b"<svg onload=alert(1)/>").unwrap();

    let app = ccosel_server::app(Jail::new(&jail).unwrap(), web, AuthState::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
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

async fn get(addr: SocketAddr, target: &str, extra: &str) -> Reply {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!("GET {target} HTTP/1.1\r\nHost: {addr}\r\n{extra}Connection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
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

#[tokio::test]
async fn a_video_is_shown_with_its_own_type() {
    let addr = spawn().await;
    let r = get(addr, "/files/Media/clip.mp4?inline=1", "").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("video/mp4"));
    assert_eq!(
        r.header("content-disposition"),
        Some("inline; filename=\"clip.mp4\"")
    );
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.body, b"0123456789");
}

/// Browsers ask for ranges to seek in audio and video, and Safari won't play without them.
#[tokio::test]
async fn a_range_is_answered_with_just_that_range() {
    let addr = spawn().await;
    let r = get(
        addr,
        "/files/Media/clip.mp4?inline=1",
        "Range: bytes=2-5\r\n",
    )
    .await;
    assert_eq!(r.status, 206);
    assert_eq!(r.body, b"2345");
    assert_eq!(r.header("content-range"), Some("bytes 2-5/10"));
    assert_eq!(r.header("content-type"), Some("video/mp4"));
}

#[tokio::test]
async fn the_extension_is_matched_whatever_its_case() {
    let addr = spawn().await;
    let r = get(addr, "/files/Media/photo.JPG?inline=1", "").await;
    assert_eq!(r.header("content-type"), Some("image/jpeg"));
}

/// HTML and SVG shown in the page would run their scripts as this site.
#[tokio::test]
async fn what_could_run_script_is_still_only_a_download() {
    let addr = spawn().await;
    for file in ["page.html", "drawing.svg"] {
        let r = get(addr, &format!("/files/Media/{file}?inline=1"), "").await;
        assert_eq!(r.status, 200);
        assert_eq!(
            r.header("content-type"),
            Some("application/octet-stream"),
            "{file}"
        );
        assert_eq!(
            r.header("content-disposition"),
            Some(format!("attachment; filename=\"{file}\"").as_str())
        );
        assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    }
}

#[tokio::test]
async fn without_inline_everything_is_a_download() {
    let addr = spawn().await;
    let r = get(addr, "/files/Media/clip.mp4", "").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("application/octet-stream"));
    assert_eq!(
        r.header("content-disposition"),
        Some("attachment; filename=\"clip.mp4\"")
    );
    assert_eq!(r.body, b"0123456789");
}

#[tokio::test]
async fn a_folder_or_a_missing_file_is_not_served() {
    let addr = spawn().await;
    assert_eq!(get(addr, "/files/Media?inline=1", "").await.status, 404);
    // Refused before it gets here, as it was before `inline` existed: the jail can't vouch for a
    // path that doesn't exist.
    assert_eq!(get(addr, "/files/Media/nope.mp4", "").await.status, 403);
}
