//! The Music Score loop over real sockets: ask `/rpc` to engrave a score, then fetch the pages
//! and the PDF it reports from `/engraved`. Those two halves are the feature: a result whose
//! files the route refuses would be useless.
//!
//! LilyPond itself is stood in for by a script (see `fake_lilypond`), so this runs anywhere; the
//! test against the real program is `tests/lilypond.rs`. This file has one test on purpose: it
//! sets `CCOSEL_LILYPOND`, which no other test in the same process may see half-set.

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;

use ccosel_proto::score::{EngraveReq, EngraveResult, EngraveStatus, page_url, pdf_url};
use ccosel_proto::{Method, WireReply, WireRequest, WireResult};
use ccosel_server::auth::AuthState;
use ccosel_server::fs_api::Jail;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn request(addr: SocketAddr, head: String, body: &[u8]) -> (String, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (
        String::from_utf8_lossy(&raw[..split]).into_owned(),
        raw[split + 4..].to_vec(),
    )
}

async fn get(addr: SocketAddr, path: &str) -> (String, Vec<u8>) {
    let head = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    request(addr, head, b"").await
}

async fn engrave(addr: SocketAddr, source: &str) -> EngraveResult {
    let args = postcard::to_allocvec(&EngraveReq { source }).unwrap();
    let batch = postcard::to_allocvec(&vec![WireRequest {
        seq: 1,
        method: Method::Engrave as u16,
        args: &args,
    }])
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let head = format!(
            "POST /rpc HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            batch.len()
        );
        let (_, reply) = request(addr, head, &batch).await;
        let replies: Vec<WireReply> = postcard::from_bytes(&reply).unwrap();
        let WireResult::Ok(bytes) = replies[0].result else {
            panic!("expected Ok, got {:?}", replies[0].result);
        };
        let status: EngraveStatus = postcard::from_bytes(bytes).unwrap();
        if let Some(result) = status.result {
            return result;
        }
        assert!(std::time::Instant::now() < deadline, "never finished");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn engraves_over_rpc_and_serves_the_pages_and_pdf_over_http() {
    let dir = std::env::temp_dir().join(format!("ccosel-score-http-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("jail")).unwrap();
    let script = dir.join("lilypond");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf 'PAGE1' > score-page1.png\nprintf 'PAGE2' > score-page2.png\n\
         printf 'PDF' > score.pdf\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: the only test in this binary, so no other thread reads the environment meanwhile.
    unsafe { std::env::set_var("CCOSEL_LILYPOND", &script) };

    let jail = Jail::new(dir.join("jail")).unwrap();
    let app = ccosel_server::app(jail, dir.clone(), AuthState::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let result = engrave(addr, "{ c'4 d' e' f' }").await;
    assert!(result.success, "{}", result.output);
    assert_eq!(result.pages, 2);

    // The URLs the app is told to use are the ones the route answers.
    let (head, body) = get(addr, &page_url(&result.key, 2)).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(head.contains("content-type: image/png"), "{head}");
    assert!(head.contains("x-content-type-options: nosniff"), "{head}");
    assert_eq!(body, b"PAGE2");

    let (head, body) = get(addr, &pdf_url(&result.key)).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(
        head.contains("attachment; filename=\"score.pdf\""),
        "the PDF downloads: {head}"
    );
    assert_eq!(body, b"PDF");

    // Nothing else in the result's directory, and no other result, is reachable.
    for path in [
        format!("/engraved/{}/score.ly", result.key),
        format!("/engraved/{}/page-3.png", result.key),
        format!("/engraved/{}/score.midi", result.key),
        format!("/engraved/{}/..%2Fscore.ly", result.key),
        "/engraved/00000000000000000000000000000000/page-1.png".to_owned(),
    ] {
        let (head, _) = get(addr, &path).await;
        assert!(head.starts_with("HTTP/1.1 404"), "{path}: {head}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
