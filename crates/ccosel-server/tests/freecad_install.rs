//! Installing FreeCAD on first use, against a local HTTP server standing in for GitHub and a
//! small shell script standing in for the AppImage. It "unpacks" to a `freecadcmd` that runs
//! the fake worker, so the whole path — download, checksum, unpack, first rebuild — runs
//! without the network or FreeCAD.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ccosel_proto::cad::{CadOp, RegenResult};
use ccosel_server::cad_api::Cad;
use ccosel_server::freecad_install::{Installer, State};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn temp(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-install-{}-{name}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An "AppImage" whose `--appimage-extract` makes `squashfs-root/usr/bin/freecadcmd`, which in
/// turn runs the fake worker whatever arguments it is given.
fn fake_appimage() -> Vec<u8> {
    let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fake_cad_worker.py")
        .display()
        .to_string();
    format!(
        "#!/bin/sh\n\
         [ \"$1\" = --appimage-extract ] || exit 1\n\
         mkdir -p squashfs-root/usr/bin\n\
         printf '#!/bin/sh\\nexec python3 {worker}\\n' > squashfs-root/usr/bin/freecadcmd\n\
         chmod +x squashfs-root/usr/bin/freecadcmd\n"
    )
    .into_bytes()
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Serves `body` for every request, or a 404 when it is `None`.
async fn serve(body: Option<Vec<u8>>) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let reply = match body {
                    Some(b) => {
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            b.len()
                        )
                        .into_bytes();
                        r.extend(b);
                        r
                    }
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = stream.write_all(&reply).await;
            });
        }
    });
    addr
}

async fn settle(installer: &Installer) -> State {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let s = installer.state();
        if matches!(s, State::Ready(_) | State::Failed(_)) {
            return s;
        }
        assert!(
            Instant::now() < deadline,
            "the install never finished: {s:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn square() -> CadOp {
    CadOp::Polygon {
        points: vec![[0.0; 3], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]],
        normal: [0.0, 0.0, 1.0],
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_rebuild_installs_freecad_then_builds() {
    let image = fake_appimage();
    let addr = serve(Some(image.clone())).await;
    let dir = temp("first");
    let installer = Installer::new(
        format!("http://{addr}/FreeCAD.AppImage"),
        sha(&image),
        dir.join("freecad"),
    );
    let cad = Arc::new(Cad::with_installer(Some(installer.clone()), &dir.join("cad")).unwrap());
    assert!(cad.available(), "installable counts as available");

    let first = cad.regenerate(&[square()]).unwrap();
    assert!(!first.finished);
    assert!(
        first.note.starts_with("Installing FreeCAD"),
        "{}",
        first.note
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    let result = loop {
        let status = cad.regenerate(&[square()]).unwrap();
        if let Some(r) = status.result {
            assert!(status.note.is_empty());
            break r;
        }
        assert!(Instant::now() < deadline, "never built");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(matches!(result, RegenResult::Ok(_)), "{result:?}");
    let State::Ready(cmd) = installer.state() else {
        panic!("installed")
    };
    assert!(cmd.ends_with("squashfs-root/usr/bin/freecadcmd"));
    assert!(
        !dir.join("freecad/FreeCAD.AppImage").exists(),
        "the image is deleted once unpacked"
    );

    // A server started later finds the finished install without downloading again.
    let again = Installer::new(
        "http://127.0.0.1:9/unused".to_owned(),
        sha(&image),
        dir.join("freecad"),
    );
    assert_eq!(again.state(), State::Ready(cmd));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_that_does_not_match_the_checksum_is_never_run() {
    let addr = serve(Some(fake_appimage())).await;
    let dir = temp("checksum");
    let installer = Installer::new(
        format!("http://{addr}/FreeCAD.AppImage"),
        "00".repeat(32),
        dir.join("freecad"),
    );
    installer.start();
    let State::Failed(e) = settle(&installer).await else {
        panic!("should fail")
    };
    assert!(e.contains("checksum"), "{e}");
    assert!(!dir.join("freecad/squashfs-root").exists());
    assert!(!dir.join("freecad/FreeCAD.AppImage.part").exists());
    assert!(!dir.join("freecad/FreeCAD.AppImage").exists());

    // The rebuild that was waiting is told why.
    let cad = Arc::new(Cad::with_installer(Some(installer.clone()), &dir.join("cad")).unwrap());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = cad.regenerate(&[square()]).unwrap();
        if let Some(RegenResult::Failed { message, .. }) = status.result {
            assert!(message.contains("Couldn't install FreeCAD"), "{message}");
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_download_fails_cleanly_and_can_be_retried() {
    let addr = serve(None).await;
    let dir = temp("missing");
    let installer = Installer::new(
        format!("http://{addr}/FreeCAD.AppImage"),
        "00".repeat(32),
        dir.join("freecad"),
    );
    installer.start();
    let State::Failed(e) = settle(&installer).await else {
        panic!("should fail")
    };
    assert!(e.contains("404"), "{e}");
    assert!(
        State::Failed(e)
            .describe()
            .starts_with("Couldn't install FreeCAD")
    );
    // Starting again after a failure tries again.
    installer.start();
    assert!(matches!(
        installer.state(),
        State::Downloading { .. } | State::Failed(_)
    ));
    settle(&installer).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn export_before_the_install_finishes_asks_to_try_again() {
    let image = fake_appimage();
    let addr = serve(Some(image.clone())).await;
    let dir = temp("export");
    let installer = Installer::new(
        format!("http://{addr}/FreeCAD.AppImage"),
        sha(&image),
        dir.join("freecad"),
    );
    let cad = Cad::with_installer(Some(installer.clone()), &dir.join("cad")).unwrap();
    let jail_dir = dir.join("jail");
    std::fs::create_dir_all(&jail_dir).unwrap();
    let jail = ccosel_server::fs_api::Jail::new(&jail_dir).unwrap();
    let (code, detail) = cad
        .export(
            &jail,
            None,
            &ccosel_proto::cad::ExportReq {
                ops: vec![square()],
                format: ccosel_proto::cad::ExportFormat::Step,
                path: "/m.step".to_owned(),
            },
        )
        .unwrap_err();
    assert_eq!(code, ccosel_proto::server_error::UNAVAILABLE);
    assert!(detail.contains("Try again"), "{detail}");
    // It did start the install.
    assert!(matches!(settle(&installer).await, State::Ready(_)));
}

#[test]
fn progress_reads_like_a_person_wrote_it() {
    assert_eq!(
        State::Downloading {
            done: 410_000_000,
            total: 820_000_000
        }
        .describe(),
        "Installing FreeCAD on the server: downloading, 50% of 820 MB"
    );
    assert!(State::Unpacking.describe().ends_with("unpacking…"));
    assert!(
        State::Downloading { done: 5, total: 0 }
            .describe()
            .ends_with("downloading…")
    );
}

#[test]
fn the_pinned_release_matches_this_machine() {
    let dir = temp("pinned");
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => {
            let i = Installer::pinned(&dir).unwrap();
            assert!(i.url.ends_with("Linux-x86_64-py311.AppImage"));
            assert_eq!(i.sha256.len(), 64);
            assert!(i.dir.ends_with(ccosel_server::freecad_install::VERSION));
        }
        ("linux", "aarch64") => assert!(Installer::pinned(&dir).unwrap().url.contains("aarch64")),
        _ => assert!(Installer::pinned(&dir).is_none()),
    }
}
