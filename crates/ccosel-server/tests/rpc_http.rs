//! Drives `POST /rpc` over a real socket, exactly as the shell will.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_proto::fs::{DirListing, ListDirReq};
use ccosel_proto::info::{ServerInfoReply, ServerInfoReq};
use ccosel_proto::{Method, PROTO_VERSION, WireReply, WireRequest, WireResult, server_error};
use ccosel_server::auth::AuthState;
use ccosel_server::fs_api::Jail;

/// The jail root a spawned server serves, returned alongside its address so a test can assert
/// on `ServerInfo`'s `root` without guessing the temp path back.
async fn spawn() -> (SocketAddr, PathBuf) {
    // One directory per call, not per process: the tests in this file run on parallel threads
    // of one process, so a pid-only name is shared, and one test's `remove_dir_all` deleted the
    // directory another was in the middle of `create_dir_all`-ing (NotFound, about 2% of runs).
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-rpc-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // Still cleared: a previous run whose pid was reused may have left this name behind.
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Projects")).unwrap();
    fs::write(dir.join("hello.txt"), b"hi").unwrap();

    let jail = Jail::new(&dir).unwrap();
    let root = jail.root().to_path_buf();
    let app = ccosel_server::app(jail, dir.clone(), AuthState::default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, root)
}

/// Minimal HTTP/1.1 POST. Avoids pulling a client crate into dev-dependencies for four lines
/// of protocol, and keeps the test honest about what actually goes over the wire.
async fn post_rpc(addr: SocketAddr, body: Vec<u8>) -> Vec<u8> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("no header terminator");
    raw[split + 4..].to_vec()
}

fn encode(reqs: &[(u32, &str)]) -> Vec<u8> {
    let args: Vec<Vec<u8>> = reqs
        .iter()
        .map(|(_, path)| postcard::to_allocvec(&ListDirReq { path }).unwrap())
        .collect();
    let batch: Vec<WireRequest> = reqs
        .iter()
        .zip(&args)
        .map(|((seq, _), a)| WireRequest {
            seq: *seq,
            method: Method::ListDir as u16,
            args: a,
        })
        .collect();
    postcard::to_allocvec(&batch).unwrap()
}

#[tokio::test]
async fn lists_a_real_directory_over_http() {
    let (addr, _root) = spawn().await;
    let reply = post_rpc(addr, encode(&[(1, "/")])).await;
    let replies: Vec<WireReply> = postcard::from_bytes(&reply).unwrap();

    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].seq, 1);
    let WireResult::Ok(bytes) = replies[0].result else {
        panic!("expected Ok, got {:?}", replies[0].result);
    };
    let listing: DirListing = postcard::from_bytes(bytes).unwrap();
    let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Projects", "hello.txt"]);
}

#[tokio::test]
async fn a_batch_returns_one_reply_per_call_and_one_failure_does_not_sink_the_rest() {
    // Coalesced calls share a request, so a single bad path must fail only its own entry.
    let (addr, _root) = spawn().await;
    let reply = post_rpc(
        addr,
        encode(&[(7, "/"), (8, "/../escape"), (9, "/Projects")]),
    )
    .await;
    let replies: Vec<WireReply> = postcard::from_bytes(&reply).unwrap();

    assert_eq!(replies.len(), 3);
    assert_eq!(
        replies.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![7, 8, 9],
        "replies are matched by seq, not position, but order is preserved here"
    );
    assert!(matches!(replies[0].result, WireResult::Ok(_)));
    assert!(matches!(
        replies[1].result,
        WireResult::Err { code, .. } if code == server_error::DENIED
    ));
    assert!(matches!(replies[2].result, WireResult::Ok(_)));
}

#[tokio::test]
async fn a_malformed_batch_is_rejected_not_crashed() {
    let (addr, _root) = spawn().await;
    let body = post_rpc(addr, vec![0xff; 32]).await;
    // 400 with a text body, not a postcard batch.
    assert!(postcard::from_bytes::<Vec<WireReply>>(&body).is_err());
}

#[tokio::test]
async fn reports_server_info_over_http() {
    let (addr, root) = spawn().await;
    let args = postcard::to_allocvec(&ServerInfoReq).unwrap();
    let batch: Vec<WireRequest> = vec![WireRequest {
        seq: 1,
        method: Method::ServerInfo as u16,
        args: &args,
    }];
    let reply = post_rpc(addr, postcard::to_allocvec(&batch).unwrap()).await;
    let replies: Vec<WireReply> = postcard::from_bytes(&reply).unwrap();

    assert_eq!(replies.len(), 1);
    let WireResult::Ok(bytes) = replies[0].result else {
        panic!("expected Ok, got {:?}", replies[0].result);
    };
    let info: ServerInfoReply = postcard::from_bytes(bytes).unwrap();
    assert_eq!(info.proto_version, PROTO_VERSION);
    assert_eq!(info.root, root.display().to_string());
    assert_eq!(
        info.rpc_calls, 1,
        "this call is the first the server has answered"
    );
    assert!(info.cpus >= 1);
    if cfg!(target_os = "linux") {
        let (used, total) = (info.mem_used_kib.unwrap(), info.mem_total_kib.unwrap());
        assert!(used <= total && total > 0);
        assert!(info.load_milli.is_some());
    }
}

/// One call of `method` with `args`, over HTTP, returning its reply payload or error code.
async fn call<T: serde::Serialize>(
    addr: SocketAddr,
    method: Method,
    args: &T,
) -> Result<Vec<u8>, u32> {
    let args = postcard::to_allocvec(args).unwrap();
    let batch = vec![WireRequest {
        seq: 1,
        method: method as u16,
        args: &args,
    }];
    let reply = post_rpc(addr, postcard::to_allocvec(&batch).unwrap()).await;
    let replies: Vec<WireReply> = postcard::from_bytes(&reply).unwrap();
    match replies[0].result {
        WireResult::Ok(bytes) => Ok(bytes.to_vec()),
        WireResult::Err { code, .. } => Err(code),
    }
}

#[tokio::test]
async fn runs_octave_over_http_or_says_it_cannot() {
    use ccosel_proto::octave::{OctaveInput, OctavePollReq, OctaveRunReq, OctaveStatus};

    let (addr, _root) = spawn().await;
    let run = OctaveRunReq {
        client: 9,
        seq: 1,
        input: OctaveInput::Code("z = 5;"),
    };
    let started: bool =
        postcard::from_bytes(&call(addr, Method::OctaveRun, &run).await.unwrap()).unwrap();
    let poll = OctavePollReq {
        client: 9,
        seq: 1,
        from: 0,
    };
    if !started {
        // No Octave here: nothing was started, so there is nothing to follow.
        assert_eq!(
            call(addr, Method::OctavePoll, &poll).await,
            Err(server_error::NOT_FOUND)
        );
        return;
    }
    for _ in 0..600 {
        let status: OctaveStatus =
            postcard::from_bytes(&call(addr, Method::OctavePoll, &poll).await.unwrap()).unwrap();
        if let Some(result) = status.result {
            assert!(!result.error, "{}", status.output);
            assert!(
                result
                    .variables
                    .iter()
                    .any(|v| v.name == "z" && v.value == "5")
            );
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the Octave job never finished");
}
