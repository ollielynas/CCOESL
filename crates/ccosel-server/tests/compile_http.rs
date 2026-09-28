//! The whole Rust Compiler loop over real sockets: ask `/rpc` to build a directory, then
//! fetch the binary it reports back from `/files`. Those two halves are the feature — a
//! `CompileResult` whose path the download route refuses would be useless.

use std::fs;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_proto::build::{CompileReq, CompileResult, CompileStatus};
use ccosel_proto::fs::{DirListing, ListDirReq};
use ccosel_proto::{Method, WireReply, WireRequest, WireResult, scratch};
use ccosel_server::auth::AuthState;
use ccosel_server::fs_api::Jail;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Serves a jail holding one zero-dependency crate, so the build never touches the network.
async fn spawn() -> SocketAddr {
    // One directory per call, not per process: this file's tests run in parallel threads of one
    // process, and a shared pid-named directory is the race #9 fixed in `rpc_http.rs` (one
    // test's `remove_dir_all` deleting the directory another was still building in).
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-compile-http-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // Still cleared: a previous run whose pid was reused may have left this name behind.
    let _ = fs::remove_dir_all(&dir);
    let crate_dir = dir.join("hello");
    fs::create_dir_all(crate_dir.join("src")).unwrap();
    fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"hello\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        crate_dir.join("src/main.rs"),
        "fn main() { println!(\"hi\"); }",
    )
    .unwrap();

    let jail = Jail::new(&dir).unwrap();
    let app = ccosel_server::app(jail, dir.clone(), AuthState::default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

/// Splits a raw HTTP response into (status line, body).
fn split_response(raw: Vec<u8>) -> (String, Vec<u8>) {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("no header terminator");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    (head, raw[split + 4..].to_vec())
}

async fn post_rpc(addr: SocketAddr, body: Vec<u8>) -> Vec<u8> {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "POST /rpc HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/octet-stream\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(&body).await.unwrap();

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    split_response(raw).1
}

async fn get(addr: SocketAddr, path: &str) -> (String, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    split_response(raw)
}

async fn post(addr: SocketAddr, path: &str, body: &[u8]) -> (String, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    split_response(raw)
}

async fn list_dir(addr: SocketAddr, path: &str) -> DirListing {
    let args = postcard::to_allocvec(&ListDirReq { path }).unwrap();
    let batch = vec![WireRequest {
        seq: 1,
        method: Method::ListDir as u16,
        args: &args,
    }];
    let reply = post_rpc(addr, postcard::to_allocvec(&batch).unwrap()).await;
    let replies: Vec<WireReply> = postcard::from_bytes(&reply).unwrap();
    let WireResult::Ok(bytes) = replies[0].result else {
        panic!("expected Ok, got {:?}", replies[0].result);
    };
    postcard::from_bytes(bytes).unwrap()
}

fn compile_batch(path: &str) -> Vec<u8> {
    let args = postcard::to_allocvec(&CompileReq {
        path,
        generation: 1,
    })
    .unwrap();
    let batch = vec![WireRequest {
        seq: 1,
        method: Method::Compile as u16,
        args: &args,
    }];
    postcard::to_allocvec(&batch).unwrap()
}

/// Polls `/rpc` the way the app's own loop does, until the job reports itself finished.
async fn build_over_rpc(addr: SocketAddr, path: &str) -> CompileStatus {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let reply = post_rpc(addr, compile_batch(path)).await;
        let replies: Vec<WireReply> = postcard::from_bytes(&reply).unwrap();
        let WireResult::Ok(bytes) = replies[0].result else {
            panic!("expected Ok, got {:?}", replies[0].result);
        };
        let status: CompileStatus = postcard::from_bytes(bytes).unwrap();
        if status.finished {
            return status;
        }
        assert!(std::time::Instant::now() < deadline, "build never finished");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn compiles_over_rpc_and_hands_the_binary_back_over_http() {
    let addr = spawn().await;

    let status = build_over_rpc(addr, "/hello").await;
    let result: CompileResult = status.result.expect("a finished job carries its result");

    assert!(result.success, "build failed:\n{}", result.output);
    assert_eq!(result.binaries.len(), 1);
    assert_eq!(result.binaries[0].name, "hello");

    // The path the app is told about has to be the path the download route accepts.
    let (head, body) = get(addr, &format!("/files{}", result.binaries[0].path)).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(
        head.contains("attachment; filename=\"hello\""),
        "a binary should download rather than render: {head}"
    );
    assert_eq!(body.len() as u64, result.binaries[0].size);
    assert_eq!(&body[..4], b"\x7fELF", "that is not the compiled binary");
}

#[tokio::test]
async fn the_download_route_is_jailed_too() {
    let addr = spawn().await;
    let (head, _) = get(addr, "/files/../../etc/passwd").await;
    assert!(
        !head.starts_with("HTTP/1.1 200"),
        "the jail must hold for downloads, not just listings: {head}"
    );
}

/// The Compiler's upload path end to end: a scratch folder, a project uploaded into it the way
/// the shell does (one POST per file), built, and its binary downloaded. And the folder stays
/// out of the listing Files shows.
#[tokio::test]
async fn a_project_uploaded_to_a_scratch_folder_builds_and_stays_out_of_sight() {
    let addr = spawn().await;

    let (head, body) = post(addr, "/scratch", b"").await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    let id: u32 = String::from_utf8(body).unwrap().parse().unwrap();
    let project = format!("{}/uploaded", scratch::path(id));

    let files: [(&str, &str, &[u8]); 2] = [
        (
            "",
            "Cargo.toml",
            b"[package]\nname = \"uploaded\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
        ("/src", "main.rs", b"fn main() {}"),
    ];
    for (dir, name, bytes) in files {
        let url = format!("/upload?path={project}{dir}&filename={name}");
        let (head, _) = post(addr, &url, bytes).await;
        assert!(head.starts_with("HTTP/1.1 200"), "{url}: {head}");
    }

    let status = build_over_rpc(addr, &project).await;
    let result = status.result.unwrap();
    assert!(result.success, "build failed:\n{}", result.output);
    let (head, _) = get(addr, &format!("/files{}", result.binaries[0].path)).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");

    let root = list_dir(addr, "/").await;
    assert!(
        root.entries.iter().all(|e| e.name != scratch::DIR),
        "{:?}",
        root.entries.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    // But the folder itself lists fine, which is how the Compiler finds the project in it.
    let inside = list_dir(addr, &scratch::path(id)).await;
    assert_eq!(inside.entries.len(), 1);
    assert_eq!(inside.entries[0].name, "uploaded");
}
