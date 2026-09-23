//! Drives `POST /upload` (and the `GET /files/...` round trip) over a real socket, exactly as
//! the shell will.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_server::fs_api::Jail;
use ccosel_server::upload_api::MAX_UPLOAD_BYTES;

/// One jail directory per call, not per process, for the same reason `rpc_http.rs` does this:
/// the tests in this file run on parallel threads of one process, so a pid-only name would be
/// shared and one test's directory could be torn down mid-use by another (see the race fixed
/// in #9).
async fn spawn() -> (SocketAddr, PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-upload-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let jail = Jail::new(&dir).unwrap();
    let app = ccosel_server::app(jail, dir.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, dir)
}

/// A minimal HTTP/1.1 request, matching the approach `rpc_http.rs` uses: no client crate for a
/// handful of protocol lines, and it keeps the test honest about what actually goes over the
/// wire.
///
/// Writing the body ignores errors rather than unwrapping: the oversized-body test sends more
/// than the server accepts, and a real server can reply with 413 and close its read side before
/// this end finishes writing, which is a normal `EPIPE`/`ECONNRESET`, not a test bug.
async fn request(addr: SocketAddr, method: &str, target: &str, body: &[u8]) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "{method} {target} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("no header terminator");
    let status_line = std::str::from_utf8(&raw[..split])
        .unwrap()
        .lines()
        .next()
        .unwrap();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, raw[split + 4..].to_vec())
}

async fn upload(addr: SocketAddr, path: &str, filename: &str, body: &[u8]) -> u16 {
    request(
        addr,
        "POST",
        &format!("/upload?path={path}&filename={filename}"),
        body,
    )
    .await
    .0
}

async fn get(addr: SocketAddr, target: &str) -> (u16, Vec<u8>) {
    request(addr, "GET", target, &[]).await
}

#[tokio::test]
async fn writes_a_file_and_creates_missing_directories() {
    let (addr, dir) = spawn().await;

    let status = upload(addr, "new/nested/dir", "hello.txt", b"hi there").await;

    assert_eq!(status, 200);
    let on_disk = fs::read(dir.join("new/nested/dir/hello.txt")).unwrap();
    assert_eq!(on_disk, b"hi there");
}

#[tokio::test]
async fn round_trips_through_files() {
    let (addr, _dir) = spawn().await;

    assert_eq!(upload(addr, "/", "roundtrip.txt", b"payload").await, 200);

    let (status, body) = get(addr, "/files/roundtrip.txt").await;
    assert_eq!(status, 200);
    assert_eq!(body, b"payload");
}

#[tokio::test]
async fn overwrites_an_existing_file() {
    // Defined behaviour: an upload to a path/filename that already exists replaces it, rather
    // than erroring or renaming. There is only one verb.
    let (addr, _dir) = spawn().await;

    assert_eq!(upload(addr, "/", "same.txt", b"first").await, 200);
    assert_eq!(upload(addr, "/", "same.txt", b"second, longer").await, 200);

    let (status, body) = get(addr, "/files/same.txt").await;
    assert_eq!(status, 200);
    assert_eq!(body, b"second, longer");
}

#[tokio::test]
async fn two_concurrent_uploads_to_different_files_both_succeed() {
    let (addr, dir) = spawn().await;

    let (a, b) = tokio::join!(
        upload(addr, "/", "one.txt", b"one"),
        upload(addr, "/", "two.txt", b"two"),
    );

    assert_eq!(a, 200);
    assert_eq!(b, 200);
    assert_eq!(fs::read(dir.join("one.txt")).unwrap(), b"one");
    assert_eq!(fs::read(dir.join("two.txt")).unwrap(), b"two");
}

#[tokio::test]
async fn refuses_a_filename_containing_a_forward_slash() {
    let (addr, dir) = spawn().await;

    // Percent-encoded so it lands as one query value ("a/b") rather than being read as two.
    let status = upload(addr, "/", "a%2Fb", b"x").await;

    assert!((400..500).contains(&status), "got {status}");
    assert!(
        fs::read_dir(&dir).unwrap().next().is_none(),
        "wrote nothing"
    );
}

#[tokio::test]
async fn refuses_a_filename_containing_a_backslash() {
    let (addr, dir) = spawn().await;

    let status = upload(addr, "/", "a%5Cb", b"x").await; // "a\b", percent-encoded for the wire

    assert!((400..500).contains(&status), "got {status}");
    assert!(
        fs::read_dir(&dir).unwrap().next().is_none(),
        "wrote nothing"
    );
}

#[tokio::test]
async fn refuses_a_filename_containing_dotdot() {
    let (addr, dir) = spawn().await;

    let status = upload(addr, "/", "a..b", b"x").await;

    assert!((400..500).contains(&status), "got {status}");
    assert!(
        fs::read_dir(&dir).unwrap().next().is_none(),
        "wrote nothing"
    );
}

#[tokio::test]
async fn refuses_an_empty_filename() {
    let (addr, dir) = spawn().await;

    let status = upload(addr, "/", "", b"x").await;

    assert!((400..500).contains(&status), "got {status}");
    assert!(
        fs::read_dir(&dir).unwrap().next().is_none(),
        "wrote nothing"
    );
}

#[tokio::test]
async fn refuses_a_path_that_climbs_out_of_the_jail() {
    let (addr, dir) = spawn().await;

    let status = upload(addr, "..", "escape.txt", b"x").await;

    assert!((400..500).contains(&status), "got {status}");
    assert!(
        !dir.parent().unwrap().join("escape.txt").exists(),
        "wrote nothing outside the jail"
    );
}

#[tokio::test]
async fn refuses_a_path_that_climbs_out_via_a_nested_dotdot() {
    let (addr, dir) = spawn().await;

    let status = upload(addr, "a/../../escape", "escape.txt", b"x").await;

    assert!((400..500).contains(&status), "got {status}");
    assert!(
        !dir.parent().unwrap().join("escape/escape.txt").exists(),
        "wrote nothing outside the jail"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn refuses_a_path_through_a_symlink_that_points_out_of_the_jail() {
    let (addr, dir) = spawn().await;

    // A directory that sits outside the jail entirely, plus a symlink inside the jail that
    // points at it.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let outside = std::env::temp_dir().join(format!(
        "ccosel-upload-outside-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&outside);
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("escape-link")).unwrap();

    let status = upload(addr, "escape-link", "pwned.txt", b"x").await;

    assert!((400..500).contains(&status), "got {status}");
    assert!(
        !outside.join("pwned.txt").exists(),
        "wrote nothing through the symlink"
    );

    let _ = fs::remove_dir_all(&outside);
}

#[tokio::test]
async fn refuses_a_body_over_the_size_limit() {
    let (addr, dir) = spawn().await;

    let oversized = vec![0u8; MAX_UPLOAD_BYTES + 1];
    let status = upload(addr, "/", "too-big.bin", &oversized).await;

    assert!((400..500).contains(&status), "got {status}");
    assert!(
        !dir.join("too-big.bin").exists(),
        "an oversized body is rejected before the handler ever writes"
    );
}
