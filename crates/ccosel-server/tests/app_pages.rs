//! `/app/{id}`: every app's own page is the boot page, whatever the id. The shell is what reads
//! the id and decides whether it names an app, so a new app needs no server change.

use std::fs;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_server::auth::AuthState;
use ccosel_server::fs_api::Jail;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BOOT_PAGE: &str = "<!doctype html><title>CCOSEL</title>";

async fn spawn() -> SocketAddr {
    // One directory per call: tests in this file run on parallel threads of one process.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-app-pages-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    let (jail_dir, web_dir) = (dir.join("jail"), dir.join("web"));
    fs::create_dir_all(&jail_dir).unwrap();
    fs::create_dir_all(web_dir.join("dist")).unwrap();
    fs::write(web_dir.join("index.html"), BOOT_PAGE).unwrap();
    fs::write(web_dir.join("dist/clock.wasm"), b"\0asm").unwrap();

    let app = ccosel_server::app(Jail::new(&jail_dir).unwrap(), web_dir, AuthState::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

/// Status and body of a plain `GET`.
async fn get(addr: SocketAddr, target: &str) -> (u16, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!("GET {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let status = std::str::from_utf8(&raw[9..12]).unwrap().parse().unwrap();
    (status, raw[split + 4..].to_vec())
}

#[tokio::test]
async fn an_app_page_is_the_boot_page() {
    let addr = spawn().await;
    for target in ["/app/clock", "/app/clock/", "/app/file-browser"] {
        let (status, body) = get(addr, target).await;
        assert_eq!(status, 200, "{target}");
        assert_eq!(body, BOOT_PAGE.as_bytes(), "{target}");
    }
}

/// The server doesn't know the catalog, so an unknown app still gets the page; the shell says
/// there is no such app. What matters is that an app added later is reachable with no change here.
#[tokio::test]
async fn any_id_gets_the_page() {
    let addr = spawn().await;
    let (status, body) = get(addr, "/app/an-app-added-tomorrow").await;
    assert_eq!(status, 200);
    assert_eq!(body, BOOT_PAGE.as_bytes());
}

/// The page loads its modules by absolute path, which must still come from `web/`.
#[tokio::test]
async fn modules_are_still_served_beside_app_pages() {
    let addr = spawn().await;
    let (status, body) = get(addr, "/dist/clock.wasm").await;
    assert_eq!(status, 200);
    assert_eq!(body, b"\0asm");
    assert_eq!(get(addr, "/app/clock/extra").await.0, 404);
}
