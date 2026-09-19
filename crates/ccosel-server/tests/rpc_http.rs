//! Drives `POST /rpc` over a real socket, exactly as the shell will.

use std::fs;
use std::net::SocketAddr;

use ccosel_proto::fs::{DirListing, ListDirReq};
use ccosel_proto::{Method, WireReply, WireRequest, WireResult, server_error};
use ccosel_server::fs_api::Jail;

async fn spawn() -> SocketAddr {
    let dir = std::env::temp_dir().join(format!("ccosel-rpc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Projects")).unwrap();
    fs::write(dir.join("hello.txt"), b"hi").unwrap();

    let jail = Jail::new(&dir).unwrap();
    let app = ccosel_server::app(jail, dir.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
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
    let addr = spawn().await;
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
    let addr = spawn().await;
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
    let addr = spawn().await;
    let body = post_rpc(addr, vec![0xff; 32]).await;
    // 400 with a text body, not a postcard batch.
    assert!(postcard::from_bytes::<Vec<WireReply>>(&body).is_err());
}
