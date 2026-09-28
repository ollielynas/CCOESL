//! Permissions end to end: the file RPCs, `/files` and `/upload`, over a real socket, as
//! different users sharing one jail.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_proto::fs::{
    Access, AccessReply, CreateDir, DirListing, FileText, ListDir, ListDirReq, PathReq, ReadFile,
    Search, SearchReply, SearchReq, WriteFile, WriteFileReq,
};
use ccosel_proto::settings::{GetSettings, ListApps, SetSettings, Settings, ThemeChoice};
use ccosel_proto::{Rpc, WireReply, WireRequest, WireResult, server_error};
use ccosel_server::fs_api::Jail;

/// Three servers over one jail: anonymous, alice and bob. They share the disk, so what one
/// user writes the others can (or cannot) see.
struct Site {
    root: PathBuf,
    anon: SocketAddr,
    alice: SocketAddr,
    bob: SocketAddr,
}

async fn site() -> Site {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-access-http-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Docs/Apps")).unwrap();
    fs::create_dir_all(dir.join("Shared")).unwrap();
    fs::write(dir.join("Docs/.access"), "read: *\n").unwrap();
    fs::write(
        dir.join("Docs/Apps/files.md"),
        "# Files\n\nBrowse the server.\n",
    )
    .unwrap();
    fs::write(dir.join("Shared/notes.md"), "shopping list\nmilk\n").unwrap();

    let jail_root = Jail::new(&dir).unwrap().root().to_path_buf();
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
        root: jail_root,
        anon: spawn(None).await,
        alice: spawn(Some("alice")).await,
        bob: spawn(Some("bob")).await,
    }
}

/// Minimal HTTP/1.1, as in the other server tests: the status and the body.
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

/// One RPC. `Err` carries the server's error code.
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

async fn read(addr: SocketAddr, path: &str) -> Result<FileText, u32> {
    call::<ReadFile>(addr, &PathReq { path }).await
}

async fn write(addr: SocketAddr, path: &str, text: &str) -> Result<(), u32> {
    call::<WriteFile>(
        addr,
        &WriteFileReq {
            path,
            text,
            create_only: false,
        },
    )
    .await
}

async fn names(addr: SocketAddr, path: &str) -> Result<Vec<String>, u32> {
    let listing: DirListing = call::<ListDir>(addr, &ListDirReq { path }).await?;
    Ok(listing.entries.into_iter().map(|e| e.name).collect())
}

#[tokio::test]
async fn open_folders_can_be_read_and_written_by_anyone() {
    let s = site().await;
    let text = read(s.anon, "/Shared/notes.md").await.unwrap();
    assert_eq!(text.text, "shopping list\nmilk\n");
    assert!(text.writable);

    write(s.alice, "/Shared/notes.md", "eggs\n").await.unwrap();
    assert_eq!(
        read(s.bob, "/Shared/notes.md").await.unwrap().text,
        "eggs\n"
    );
}

#[tokio::test]
async fn a_read_only_folder_is_read_only_for_everyone_and_everything_in_it() {
    let s = site().await;
    for who in [s.anon, s.alice] {
        let doc = read(who, "/Docs/Apps/files.md").await.unwrap();
        assert!(doc.text.starts_with("# Files"));
        assert!(!doc.writable, "the UI is told not to offer Edit");

        assert_eq!(
            write(who, "/Docs/Apps/files.md", "vandalised").await,
            Err(server_error::DENIED)
        );
        assert_eq!(
            write(who, "/Docs/Apps/new.md", "x").await,
            Err(server_error::DENIED)
        );
        assert_eq!(
            call::<CreateDir>(
                who,
                &PathReq {
                    path: "/Docs/Apps/sub"
                }
            )
            .await,
            Err(server_error::DENIED)
        );
    }
    // Nothing reached the disk.
    assert!(
        fs::read_to_string(s.root.join("Docs/Apps/files.md"))
            .unwrap()
            .starts_with("# Files")
    );
    assert!(!s.root.join("Docs/Apps/new.md").exists());
}

#[tokio::test]
async fn each_user_has_a_private_home_folder() {
    let s = site().await;
    // Any call creates the caller's home, so it is there to be listed.
    let me: AccessReply = call::<Access>(
        s.alice,
        &PathReq {
            path: "/home/alice",
        },
    )
    .await
    .unwrap();
    assert_eq!(
        me,
        AccessReply {
            read: true,
            write: true,
            user: Some("alice".into())
        }
    );
    write(s.alice, "/home/alice/diary.md", "secret")
        .await
        .unwrap();
    call::<CreateDir>(
        s.alice,
        &PathReq {
            path: "/home/alice/Work",
        },
    )
    .await
    .unwrap();

    // bob sees only his own folder in /home, and cannot reach alice's by name.
    assert_eq!(names(s.bob, "/home").await.unwrap(), vec!["bob"]);
    assert_eq!(
        names(s.alice, "/home").await.unwrap(),
        vec!["alice"],
        "and alice only hers"
    );
    assert_eq!(
        read(s.bob, "/home/alice/diary.md").await.unwrap_err(),
        server_error::DENIED
    );
    assert_eq!(
        write(s.bob, "/home/alice/diary.md", "hi").await,
        Err(server_error::DENIED)
    );
    assert_eq!(names(s.bob, "/home/alice").await, Err(server_error::DENIED));
    // Anonymous callers have no home, and cannot look in /home at all.
    assert_eq!(names(s.anon, "/home").await, Err(server_error::DENIED));
    assert_eq!(
        read(s.anon, "/home/alice/diary.md").await.unwrap_err(),
        server_error::DENIED
    );

    // The same rules hold for /files and /upload, not just the RPCs.
    assert_eq!(
        request(s.alice, "GET", "/files/home/alice/diary.md", &[])
            .await
            .0,
        200
    );
    assert_eq!(
        request(s.bob, "GET", "/files/home/alice/diary.md", &[])
            .await
            .0,
        403
    );
    assert_eq!(
        request(s.anon, "GET", "/files/home/alice/diary.md", &[])
            .await
            .0,
        403
    );
    let upload = "/upload?path=/home/alice/Work&filename=x.txt";
    assert_eq!(request(s.bob, "POST", upload, b"x").await.0, 403);
    assert_eq!(request(s.alice, "POST", upload, b"x").await.0, 200);
    assert_eq!(
        request(s.anon, "POST", "/upload?path=/Docs&filename=x.md", b"x")
            .await
            .0,
        403
    );
}

#[tokio::test]
async fn access_files_are_invisible_and_untouchable() {
    let s = site().await;
    assert_eq!(names(s.anon, "/Docs").await.unwrap(), vec!["Apps"]);
    assert_eq!(
        read(s.alice, "/Docs/.access").await.unwrap_err(),
        server_error::DENIED
    );
    // Not even where the folder is writable: whoever could write one could grant themselves
    // anything.
    assert_eq!(
        write(s.alice, "/Shared/.access", "write: *").await,
        Err(server_error::DENIED)
    );
    assert_eq!(
        request(s.anon, "GET", "/files/Docs/.access", &[]).await.0,
        403
    );
    assert_eq!(
        request(
            s.alice,
            "POST",
            "/upload?path=/Shared&filename=.access",
            b"write: *"
        )
        .await
        .0,
        403
    );
}

#[tokio::test]
async fn create_only_refuses_to_overwrite() {
    let s = site().await;
    let new = |path| WriteFileReq {
        path,
        text: "# New\n",
        create_only: true,
    };
    call::<WriteFile>(s.alice, &new("/Shared/new.md"))
        .await
        .unwrap();
    assert_eq!(
        call::<WriteFile>(s.alice, &new("/Shared/new.md")).await,
        Err(server_error::EXISTS)
    );
    assert_eq!(
        call::<WriteFile>(s.alice, &new("/Shared")).await,
        Err(server_error::EXISTS)
    );
    assert_eq!(
        call::<CreateDir>(s.alice, &PathReq { path: "/Shared" }).await,
        Err(server_error::EXISTS)
    );
    // The folder a new file goes in has to exist.
    assert_eq!(
        write(s.alice, "/Nowhere/new.md", "x").await,
        Err(server_error::NOT_FOUND)
    );
}

#[tokio::test]
async fn read_file_refuses_what_is_not_small_text() {
    let s = site().await;
    fs::write(s.root.join("Shared/blob.bin"), [0xff, 0xfe, 0x00]).unwrap();
    assert_eq!(
        read(s.anon, "/Shared/blob.bin").await.unwrap_err(),
        server_error::NOT_TEXT
    );
    assert_eq!(
        read(s.anon, "/Shared").await.unwrap_err(),
        server_error::NOT_A_DIRECTORY
    );
    let big = "x".repeat(ccosel_proto::fs::MAX_TEXT_BYTES + 1);
    fs::write(s.root.join("Shared/big.md"), &big).unwrap();
    assert_eq!(
        read(s.anon, "/Shared/big.md").await.unwrap_err(),
        server_error::TOO_LARGE
    );
    assert_eq!(
        write(s.anon, "/Shared/big2.md", &big).await,
        Err(server_error::TOO_LARGE)
    );
}

#[tokio::test]
async fn search_finds_names_and_lines_but_only_where_the_caller_can_read() {
    let s = site().await;
    write(s.alice, "/home/alice/milk.md", "milk is private")
        .await
        .unwrap();

    let search = |path, query| SearchReq {
        path,
        query,
        suffix: ".md",
    };
    let found: SearchReply = call::<Search>(s.alice, &search("/", "MILK")).await.unwrap();
    let paths: Vec<&str> = found.hits.iter().map(|h| h.path.as_str()).collect();
    assert_eq!(paths, vec!["/Shared/notes.md", "/home/alice/milk.md"]);
    assert_eq!(found.hits[0].line, "milk");
    assert_eq!(
        found.hits[1].line, "milk is private",
        "a name hit shows its line too"
    );

    let bobs: SearchReply = call::<Search>(s.bob, &search("/", "milk")).await.unwrap();
    assert_eq!(bobs.hits.len(), 1, "alice's home is not searched for bob");

    // A name-only hit, and the suffix filter.
    let by_name = call::<Search>(s.anon, &search("/Docs", "files"))
        .await
        .unwrap();
    assert_eq!(by_name.hits[0].path, "/Docs/Apps/files.md");
    let none = call::<Search>(
        s.anon,
        &SearchReq {
            path: "/",
            query: "files",
            suffix: ".txt",
        },
    )
    .await
    .unwrap();
    assert!(none.hits.is_empty());
    assert!(
        call::<Search>(s.anon, &search("/", "  "))
            .await
            .unwrap()
            .hits
            .is_empty()
    );
    assert_eq!(
        call::<Search>(s.anon, &search("/home", "milk"))
            .await
            .unwrap_err(),
        server_error::DENIED
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_cannot_lend_a_private_folder_public_permissions() {
    let s = site().await;
    write(s.alice, "/home/alice/diary.md", "secret")
        .await
        .unwrap();
    std::os::unix::fs::symlink(s.root.join("home/alice"), s.root.join("Shared/peek")).unwrap();

    assert_eq!(
        read(s.bob, "/Shared/peek/diary.md").await.unwrap_err(),
        server_error::DENIED
    );
    assert_eq!(
        write(s.bob, "/Shared/peek/diary.md", "gotcha").await,
        Err(server_error::DENIED)
    );
    assert_eq!(
        request(s.bob, "GET", "/files/Shared/peek/diary.md", &[])
            .await
            .0,
        403
    );
    assert_eq!(
        request(
            s.bob,
            "POST",
            "/upload?path=/Shared/peek&filename=x.md",
            b"x"
        )
        .await
        .0,
        403
    );
    assert_eq!(
        fs::read_to_string(s.root.join("home/alice/diary.md")).unwrap(),
        "secret"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn search_does_not_follow_a_symlink_back_to_its_own_folder() {
    let s = site().await;
    std::os::unix::fs::symlink(s.root.join("Shared"), s.root.join("Shared/again")).unwrap();
    let found = call::<Search>(
        s.anon,
        &SearchReq {
            path: "/Shared",
            query: "milk",
            suffix: ".md",
        },
    )
    .await
    .unwrap();
    assert_eq!(found.hits.len(), 1);
    assert!(!found.truncated);
}

#[tokio::test]
async fn settings_are_kept_per_user() {
    let s = site().await;
    let dark = Settings {
        theme: ThemeChoice::Dark,
        pinned: vec!["clock".to_owned()],
        ..Settings::default()
    };
    call::<SetSettings>(s.alice, &dark).await.unwrap();
    assert_eq!(call::<GetSettings>(s.alice, &()).await, Ok(dark));
    assert_eq!(
        call::<GetSettings>(s.bob, &()).await,
        Ok(Settings::default())
    );
}

#[tokio::test]
async fn settings_need_someone_signed_in() {
    let s = site().await;
    assert_eq!(
        call::<GetSettings>(s.anon, &()).await,
        Err(server_error::NO_USER)
    );
    assert_eq!(
        call::<SetSettings>(s.anon, &Settings::default()).await,
        Err(server_error::NO_USER)
    );
}

/// The settings file lives in the user's own folder, but only the settings methods reach it:
/// Files can't show it, read it, overwrite it or download it.
#[tokio::test]
async fn the_settings_file_is_invisible_to_the_file_api() {
    let s = site().await;
    call::<SetSettings>(s.alice, &Settings::default())
        .await
        .unwrap();
    assert!(s.root.join("home/alice/.settings.json").is_file());

    assert!(
        !names(s.alice, "/home/alice")
            .await
            .unwrap()
            .contains(&".settings.json".to_owned())
    );
    assert_eq!(
        read(s.alice, "/home/alice/.settings.json")
            .await
            .map(|t| t.text),
        Err(server_error::DENIED)
    );
    assert_eq!(
        write(s.alice, "/home/alice/.settings.json", "{}").await,
        Err(server_error::DENIED)
    );
    let (status, _) = request(s.alice, "GET", "/files/home/alice/.settings.json", b"").await;
    assert_ne!(status, 200);
}

#[tokio::test]
async fn the_server_leaves_listing_apps_to_the_shell() {
    let s = site().await;
    assert_eq!(
        call::<ListApps>(s.alice, &()).await,
        Err(server_error::UNKNOWN_METHOD)
    );
}
