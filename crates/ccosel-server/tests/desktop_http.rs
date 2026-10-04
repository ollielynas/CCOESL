//! Remembering each person's desktop, over a real socket, as different users sharing one jail.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_proto::desktop::{DesktopLayout, LoadDesktop, MAX_WINDOWS, SaveDesktop, SavedWindow};
use ccosel_proto::fs::{DirListing, ListDir, ListDirReq, PathReq, ReadFile};
use ccosel_proto::{Rpc, WireReply, WireRequest, WireResult, server_error};
use ccosel_server::fs_api::Jail;

struct Site {
    root: PathBuf,
    anon: SocketAddr,
    alice: SocketAddr,
    bob: SocketAddr,
}

async fn site() -> Site {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-desktop-http-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let root = Jail::new(&dir).unwrap().root().to_path_buf();
    let spawn = |user: Option<&str>| {
        let mut app = ccosel_server::app(
            Jail::new(&dir).unwrap(),
            dir.clone(),
            ccosel_server::auth::AuthState::default(),
        );
        if let Some(name) = user {
            app = ccosel_server::as_user(app, name.to_owned());
        }
        async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            addr
        }
    };
    Site {
        root,
        anon: spawn(None).await,
        alice: spawn(Some("alice")).await,
        bob: spawn(Some("bob")).await,
    }
}

async fn request(addr: SocketAddr, method: &str, target: &str, body: &[u8]) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
    let status = std::str::from_utf8(&raw[..split]).unwrap()[9..12]
        .parse()
        .unwrap();
    (status, raw[split + 4..].to_vec())
}

async fn call<M: Rpc>(addr: SocketAddr, req: &M::Req<'_>) -> Result<M::Reply, u32> {
    let args = postcard::to_allocvec(req).unwrap();
    let batch = postcard::to_allocvec(&vec![WireRequest {
        seq: 1,
        method: M::METHOD as u16,
        args: &args,
    }])
    .unwrap();
    let (status, body) = request(addr, "POST", "/rpc", &batch).await;
    assert_eq!(status, 200);
    let replies: Vec<WireReply> = postcard::from_bytes(&body).unwrap();
    match replies[0].result {
        WireResult::Ok(bytes) => Ok(postcard::from_bytes(bytes).unwrap()),
        WireResult::Err { code, .. } => Err(code),
    }
}

fn window(app: &str, arg: Option<&str>, x: f32) -> SavedWindow {
    SavedWindow {
        app: app.to_owned(),
        arg: arg.map(str::to_owned),
        rect: [x, 40.0, x + 400.0, 340.0],
        minimized: false,
        maximized: false,
    }
}

fn layout(windows: Vec<SavedWindow>) -> DesktopLayout {
    DesktopLayout { windows }
}

#[tokio::test]
async fn a_new_person_starts_with_nothing_open() {
    let s = site().await;
    assert_eq!(call::<LoadDesktop>(s.alice, &()).await, Ok(layout(vec![])));
}

#[tokio::test]
async fn what_was_saved_comes_back_and_only_for_its_owner() {
    let s = site().await;
    let mine = layout(vec![
        window("file-browser", None, 10.0),
        SavedWindow {
            maximized: true,
            ..window("viewer", Some("/home/alice/beach.jpg"), 90.0)
        },
    ]);
    call::<SaveDesktop>(s.alice, &mine).await.unwrap();
    assert_eq!(call::<LoadDesktop>(s.alice, &()).await, Ok(mine.clone()));
    assert_eq!(
        call::<LoadDesktop>(s.bob, &()).await,
        Ok(layout(vec![])),
        "someone else's desktop is their own"
    );

    // A later save replaces it.
    let later = layout(vec![window("clock", None, 0.0)]);
    call::<SaveDesktop>(s.alice, &later).await.unwrap();
    assert_eq!(call::<LoadDesktop>(s.alice, &()).await, Ok(later));
}

#[tokio::test]
async fn nothing_is_kept_for_someone_not_signed_in() {
    let s = site().await;
    let some = layout(vec![window("clock", None, 0.0)]);
    assert_eq!(call::<SaveDesktop>(s.anon, &some).await, Ok(()));
    assert_eq!(call::<LoadDesktop>(s.anon, &()).await, Ok(layout(vec![])));
    assert!(!s.root.join(".desktops").exists());
}

#[tokio::test]
async fn a_desktop_with_too_many_windows_is_refused() {
    let s = site().await;
    let many = layout(
        (0..=MAX_WINDOWS)
            .map(|i| window("clock", None, i as f32))
            .collect(),
    );
    assert_eq!(
        call::<SaveDesktop>(s.alice, &many).await,
        Err(server_error::TOO_LARGE)
    );
    let huge = layout(vec![window("viewer", Some(&"x".repeat(70_000)), 0.0)]);
    assert_eq!(
        call::<SaveDesktop>(s.alice, &huge).await,
        Err(server_error::TOO_LARGE)
    );
}

#[tokio::test]
async fn a_damaged_saved_desktop_starts_empty() {
    let s = site().await;
    fs::create_dir_all(s.root.join(".desktops")).unwrap();
    fs::write(s.root.join(".desktops/alice.layout"), b"\xff\xff\xff").unwrap();
    assert_eq!(call::<LoadDesktop>(s.alice, &()).await, Ok(layout(vec![])));
}

/// The layouts list which files someone had open, so the folder holding them is nobody's to
/// see: not in a listing, not readable, not downloadable.
#[tokio::test]
async fn saved_desktops_are_out_of_everyones_reach() {
    let s = site().await;
    call::<SaveDesktop>(s.alice, &layout(vec![window("clock", None, 0.0)]))
        .await
        .unwrap();
    assert!(s.root.join(".desktops/alice.layout").exists());

    for who in [s.anon, s.alice, s.bob] {
        let listing: DirListing = call::<ListDir>(who, &ListDirReq { path: "/" })
            .await
            .unwrap();
        assert!(listing.entries.iter().all(|e| e.name != ".desktops"));
        assert!(
            call::<ReadFile>(
                who,
                &PathReq {
                    path: "/.desktops/alice.layout"
                }
            )
            .await
            .is_err()
        );
        let (status, _) = request(who, "GET", "/files/.desktops/alice.layout", &[]).await;
        assert_ne!(status, 200);
    }
}
