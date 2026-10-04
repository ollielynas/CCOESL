//! Permissions end to end: the file RPCs, `/files` and `/upload`, over a real socket, as
//! different users sharing one jail.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_proto::fs::{
    Access, AccessReply, CreateDir, DirListing, EntryKind, FileText, ListDir, ListDirReq, PathReq,
    ReadFile, Remove, Search, SearchReply, SearchReq, WriteFile, WriteFileReq,
};
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

    // bob sees only his own folder in /home, and cannot reach alice's by name. What he may not
    // read answers exactly as if it were not there, so he cannot even learn that it exists.
    assert_eq!(names(s.bob, "/home").await.unwrap(), vec!["bob"]);
    assert_eq!(
        names(s.alice, "/home").await.unwrap(),
        vec!["alice"],
        "and alice only hers"
    );
    assert_eq!(
        read(s.bob, "/home/alice/diary.md").await.unwrap_err(),
        server_error::NOT_FOUND
    );
    assert_eq!(
        write(s.bob, "/home/alice/diary.md", "hi").await,
        Err(server_error::NOT_FOUND)
    );
    assert_eq!(
        names(s.bob, "/home/alice").await,
        Err(server_error::NOT_FOUND)
    );
    // Anonymous callers have no home, and cannot look in /home at all.
    assert_eq!(names(s.anon, "/home").await, Err(server_error::NOT_FOUND));
    assert_eq!(
        read(s.anon, "/home/alice/diary.md").await.unwrap_err(),
        server_error::NOT_FOUND
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
    assert_eq!(request(s.bob, "POST", upload, b"x").await.0, 404);
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
        server_error::NOT_FOUND
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
        server_error::NOT_FOUND
    );
    assert_eq!(
        write(s.bob, "/Shared/peek/diary.md", "gotcha").await,
        Err(server_error::NOT_FOUND)
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
        404
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

async fn listing(addr: SocketAddr, path: &str) -> DirListing {
    call::<ListDir>(addr, &ListDirReq { path }).await.unwrap()
}

/// Each way of asking about a folder bob may not read gets the same answer as for one that is
/// not there at all, so no call tells him `secret` exists.
#[tokio::test]
async fn a_hidden_folder_is_indistinguishable_from_a_missing_one() {
    let s = site().await;
    fs::create_dir_all(s.root.join("Shared/secret")).unwrap();
    fs::write(s.root.join("Shared/secret/.access"), "read: alice\n").unwrap();
    fs::write(s.root.join("Shared/secret/plans.md"), "plans").unwrap();

    assert_eq!(names(s.bob, "/Shared").await.unwrap(), vec!["notes.md"]);
    assert_eq!(
        names(s.alice, "/Shared").await.unwrap(),
        vec!["secret", "notes.md"]
    );

    let (hidden, missing) = ("/Shared/secret", "/Shared/nothing");
    assert_eq!(names(s.bob, hidden).await, names(s.bob, missing).await);
    assert_eq!(names(s.bob, hidden).await, Err(server_error::NOT_FOUND));
    let file = |dir: &str| format!("{dir}/plans.md");
    assert_eq!(
        read(s.bob, &file(hidden)).await.unwrap_err(),
        read(s.bob, &file(missing)).await.unwrap_err()
    );
    assert_eq!(
        write(s.bob, &file(hidden), "x").await,
        write(s.bob, &file(missing), "x").await
    );
    let new = |dir: &str| format!("{dir}/new");
    assert_eq!(
        call::<CreateDir>(s.bob, &PathReq { path: &new(hidden) }).await,
        call::<CreateDir>(
            s.bob,
            &PathReq {
                path: &new(missing)
            }
        )
        .await
    );
    assert_eq!(
        call::<Access>(s.bob, &PathReq { path: hidden }).await,
        call::<Access>(s.bob, &PathReq { path: missing }).await
    );
    let get = |dir: &str| format!("/files{dir}/plans.md");
    assert_eq!(
        request(s.bob, "GET", &get(hidden), &[]).await.0,
        request(s.bob, "GET", &get(missing), &[]).await.0
    );
    let search = |dir: &'static str| SearchReq {
        path: dir,
        query: "plans",
        suffix: "",
    };
    assert_eq!(
        call::<Search>(s.bob, &search(hidden)).await.unwrap_err(),
        call::<Search>(s.bob, &search(missing)).await.unwrap_err()
    );
    assert_eq!(
        call::<Access>(
            s.bob,
            &PathReq {
                path: "/Shared/secret"
            }
        )
        .await
        .unwrap(),
        AccessReply {
            read: false,
            write: false,
            user: Some("bob".into())
        }
    );
    assert_eq!(
        request(
            s.bob,
            "POST",
            "/upload?path=/Shared/secret&filename=x.md",
            b"x"
        )
        .await
        .0,
        404
    );
    assert!(!s.root.join("Shared/secret/x.md").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_listing_leaves_out_links_to_what_the_caller_cannot_read() {
    let s = site().await;
    write(s.alice, "/home/alice/diary.md", "secret")
        .await
        .unwrap();
    let link = |target: PathBuf, name: &str| {
        std::os::unix::fs::symlink(target, s.root.join("Shared").join(name)).unwrap();
    };
    link(s.root.join("home/alice"), "to-alices-home");
    link(s.root.join("home/alice/diary.md"), "to-alices-diary");
    link(s.root.join("nowhere"), "dangling");
    link(std::env::temp_dir(), "out-of-the-jail");
    link(s.root.join("Docs/Apps/files.md"), "to-the-docs");

    // bob sees only the link to somewhere he may read, and may not change what it leads to.
    let bob = listing(s.bob, "/Shared").await;
    let seen: Vec<_> = bob
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.kind, e.writable))
        .collect();
    assert_eq!(
        seen,
        vec![
            ("notes.md", EntryKind::File, true),
            ("to-the-docs", EntryKind::Symlink, false),
        ]
    );

    // alice may read her own home, so the links into it are hers to see.
    let alice: Vec<_> = listing(s.alice, "/Shared")
        .await
        .entries
        .into_iter()
        .map(|e| (e.name, e.writable))
        .collect();
    assert!(alice.contains(&("to-alices-home".into(), true)));
    assert!(alice.contains(&("to-alices-diary".into(), true)));
    assert!(
        !alice
            .iter()
            .any(|(n, _)| n == "dangling" || n == "out-of-the-jail")
    );
}

#[tokio::test]
async fn a_listing_says_what_the_caller_may_change() {
    let s = site().await;
    let root: Vec<_> = listing(s.anon, "/")
        .await
        .entries
        .into_iter()
        .map(|e| (e.name, e.writable))
        .collect();
    assert_eq!(
        root,
        vec![("Docs".into(), false), ("Shared".into(), true)],
        "and no /home for someone not signed in"
    );
    let docs = listing(s.anon, "/Docs/Apps").await;
    assert!(!docs.entries[0].writable, "a file has its folder's rules");

    write(s.alice, "/home/alice/diary.md", "secret")
        .await
        .unwrap();
    let home = listing(s.alice, "/home").await;
    assert_eq!(home.entries.len(), 1);
    assert!(home.entries[0].writable);
    assert!(listing(s.alice, "/home/alice").await.entries[0].writable);
}

async fn remove(addr: SocketAddr, path: &str) -> Result<(), u32> {
    call::<Remove>(addr, &PathReq { path }).await
}

#[tokio::test]
async fn remove_deletes_a_file_or_a_whole_folder_where_the_caller_may_write() {
    let s = site().await;
    fs::create_dir_all(s.root.join("Shared/old/deeper")).unwrap();
    fs::write(s.root.join("Shared/old/deeper/a.txt"), "a").unwrap();

    remove(s.anon, "/Shared/notes.md").await.unwrap();
    assert!(!s.root.join("Shared/notes.md").exists());
    remove(s.anon, "/Shared/old").await.unwrap();
    assert!(!s.root.join("Shared/old").exists());

    assert_eq!(
        remove(s.anon, "/Shared/old").await,
        Err(server_error::NOT_FOUND),
        "removing something already gone says so"
    );
}

#[tokio::test]
async fn remove_is_refused_in_a_read_only_folder() {
    let s = site().await;
    assert_eq!(
        remove(s.alice, "/Docs/Apps/files.md").await,
        Err(server_error::DENIED)
    );
    assert_eq!(remove(s.alice, "/Docs").await, Err(server_error::DENIED));
    assert!(s.root.join("Docs/Apps/files.md").exists());
}

#[tokio::test]
async fn remove_is_refused_for_a_folder_holding_one_the_caller_may_not_change() {
    let s = site().await;
    fs::create_dir_all(s.root.join("Shared/project/locked")).unwrap();
    fs::write(s.root.join("Shared/project/locked/.access"), "read: *\n").unwrap();
    fs::write(s.root.join("Shared/project/locked/a.txt"), "a").unwrap();

    assert_eq!(
        remove(s.alice, "/Shared/project").await,
        Err(server_error::DENIED)
    );
    assert!(s.root.join("Shared/project/locked/a.txt").exists());
}

#[tokio::test]
async fn nobody_removes_the_root_their_home_or_someone_elses_files() {
    let s = site().await;
    write(s.bob, "/home/bob/secret.txt", "mine").await.unwrap();

    assert_eq!(remove(s.anon, "/").await, Err(server_error::DENIED));
    assert_eq!(
        remove(s.alice, "/home/alice").await,
        Err(server_error::DENIED)
    );
    // Bob's home is as absent to alice as a missing folder, so it is not found, not denied.
    assert_eq!(
        remove(s.alice, "/home/bob/secret.txt").await,
        Err(server_error::NOT_FOUND)
    );
    assert!(s.root.join("home/bob/secret.txt").exists());

    // Inside her own home, alice can.
    write(s.alice, "/home/alice/diary.md", "x").await.unwrap();
    remove(s.alice, "/home/alice/diary.md").await.unwrap();
    assert!(!s.root.join("home/alice/diary.md").exists());
}

#[tokio::test]
async fn archive_runs_as_a_job_over_rpc_and_respects_permissions() {
    use ccosel_proto::archive::{Archive, ArchiveAction, ArchiveReq, ArchiveResult};

    let s = site().await;
    let req = |path| ArchiveReq {
        path,
        action: ArchiveAction::Gzip,
        generation: 1,
    };
    // Nobody may write in /Docs, so nothing can be made there.
    assert_eq!(
        call::<Archive>(s.alice, &req("/Docs/Apps/files.md")).await,
        Err(server_error::DENIED)
    );
    let made = loop {
        let status = call::<Archive>(s.alice, &req("/Shared/notes.md"))
            .await
            .unwrap();
        if let Some(result) = status.result {
            break result;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert_eq!(made, ArchiveResult::Made("/Shared/notes.md.gz".to_owned()));
    assert!(s.root.join("Shared/notes.md.gz").is_file());
}
