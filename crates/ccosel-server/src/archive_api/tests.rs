use std::sync::atomic::AtomicUsize;

use super::*;

/// A fresh jail with an empty open folder `/Shared`, and one `/Docs` anyone may only read.
fn jail() -> (Jail, PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-archive-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Shared")).unwrap();
    fs::create_dir_all(dir.join("Docs")).unwrap();
    fs::write(dir.join("Docs/.access"), "read: *\n").unwrap();
    fs::write(dir.join("Docs/guide.md"), "# Guide\n").unwrap();
    let jail = Jail::new(&dir).unwrap();
    let root = jail.root().to_path_buf();
    (jail, root)
}

/// Polls like the app does until the job finishes.
fn finish(
    jail: &Jail,
    jobs: &Jobs,
    path: &str,
    action: ArchiveAction,
    user: Option<&str>,
) -> Result<ArchiveResult, u32> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let req = ArchiveReq {
            path,
            action,
            generation: 1,
        };
        let status = archive(jail, jobs, &req, user)?;
        if let Some(result) = status.result {
            assert!(status.finished);
            return Ok(result);
        }
        assert!(Instant::now() < deadline, "the job never finished");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn run_ok(jail: &Jail, path: &str, action: ArchiveAction) -> String {
    match finish(jail, &Jobs::new(), path, action, None).unwrap() {
        ArchiveResult::Made(made) => made,
        ArchiveResult::Failed(why) => panic!("{path}: {why}"),
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn a_folder_compresses_and_extracts_back_to_the_same_files() {
    let (jail, root) = jail();
    let photos = root.join("Shared/photos");
    fs::create_dir_all(photos.join("2024/summer")).unwrap();
    fs::write(photos.join("a.txt"), "first").unwrap();
    fs::write(photos.join("2024/summer/b.txt"), "second").unwrap();
    fs::write(photos.join("2024/.access"), "read: *\nwrite: *\n").unwrap();

    assert_eq!(
        run_ok(&jail, "/Shared/photos", ArchiveAction::Compress),
        "/Shared/photos.tar.gz"
    );
    // Extracting next to the original: a suffix, never a merge into it.
    assert_eq!(
        run_ok(&jail, "/Shared/photos.tar.gz", ArchiveAction::Extract),
        "/Shared/photos (2)"
    );
    let copy = root.join("Shared/photos (2)/photos");
    assert_eq!(fs::read_to_string(copy.join("a.txt")).unwrap(), "first");
    assert_eq!(
        fs::read_to_string(copy.join("2024/summer/b.txt")).unwrap(),
        "second"
    );
    assert!(
        !copy.join("2024/.access").exists(),
        "a folder's rules aren't carried in an archive"
    );
    assert_eq!(
        names_in(&root.join("Shared")),
        ["photos", "photos (2)", "photos.tar.gz"],
        "no temporary files left over"
    );
}

#[test]
fn a_file_gzips_and_gunzips_without_overwriting_the_original() {
    let (jail, root) = jail();
    fs::write(root.join("Shared/notes.txt"), "milk\neggs\n").unwrap();

    assert_eq!(
        run_ok(&jail, "/Shared/notes.txt", ArchiveAction::Gzip),
        "/Shared/notes.txt.gz"
    );
    assert_eq!(
        run_ok(&jail, "/Shared/notes.txt.gz", ArchiveAction::Extract),
        "/Shared/notes (2).txt"
    );
    assert_eq!(
        fs::read_to_string(root.join("Shared/notes (2).txt")).unwrap(),
        "milk\neggs\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("Shared/notes.txt")).unwrap(),
        "milk\neggs\n"
    );
}

#[test]
fn compressing_again_gets_a_new_name() {
    let (jail, root) = jail();
    fs::write(root.join("Shared/a.txt"), "a").unwrap();
    run_ok(&jail, "/Shared/a.txt", ArchiveAction::Compress);
    assert_eq!(
        run_ok(&jail, "/Shared/a.txt", ArchiveAction::Compress),
        "/Shared/a (2).txt.tar.gz"
    );
}

#[test]
fn a_plain_tar_extracts_with_folders_links_inside_and_the_executable_bit() {
    let (jail, root) = jail();
    let mut tar = RawTar::default();
    tar.dir("app/");
    tar.file_mode("app/run.sh", b"#!/bin/sh\n", 0o755);
    tar.file("app/lib/data.txt", b"data");
    tar.symlink("app/latest", "lib/data.txt");
    tar.hard_link("app/copy.txt", "app/lib/data.txt");
    fs::write(root.join("Shared/app.tar"), tar.finish()).unwrap();

    run_ok(&jail, "/Shared/app.tar", ArchiveAction::Extract);
    let out = root.join("Shared/app/app");
    assert_eq!(fs::read_to_string(out.join("latest")).unwrap(), "data");
    assert_eq!(fs::read_to_string(out.join("copy.txt")).unwrap(), "data");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(out.join("run.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
        let mode = fs::metadata(out.join("lib/data.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644);
    }
}

#[test]
fn polling_the_same_press_never_runs_it_twice() {
    let (jail, root) = jail();
    fs::write(root.join("Shared/a.txt"), "a").unwrap();
    let jobs = Jobs::new();
    finish(&jail, &jobs, "/Shared/a.txt", ArchiveAction::Gzip, None).unwrap();
    finish(&jail, &jobs, "/Shared/a.txt", ArchiveAction::Gzip, None).unwrap();
    assert_eq!(names_in(&root.join("Shared")), ["a.txt", "a.txt.gz"]);
}

// Permissions.

#[test]
fn nothing_is_made_where_the_caller_may_not_write() {
    let (jail, _) = jail();
    let jobs = Jobs::new();
    for action in [ArchiveAction::Compress, ArchiveAction::Gzip] {
        assert_eq!(
            finish(&jail, &jobs, "/Docs/guide.md", action, Some("bob")),
            Err(server_error::DENIED)
        );
    }
}

#[test]
fn nothing_is_read_that_the_caller_may_not_read() {
    let (jail, root) = jail();
    fs::create_dir_all(root.join("Shared/mixed/private")).unwrap();
    fs::write(
        root.join("Shared/mixed/private/.access"),
        "read: alice\nwrite: alice\n",
    )
    .unwrap();
    fs::write(root.join("Shared/mixed/private/secret.txt"), "s").unwrap();
    let jobs = Jobs::new();
    // A folder holding one bob can't read is not his to archive, even partly.
    assert_eq!(
        finish(
            &jail,
            &jobs,
            "/Shared/mixed",
            ArchiveAction::Compress,
            Some("bob")
        ),
        Err(server_error::DENIED)
    );
    assert_eq!(
        finish(
            &jail,
            &jobs,
            "/Shared/mixed/private/secret.txt",
            ArchiveAction::Gzip,
            Some("bob")
        ),
        Err(server_error::NOT_FOUND)
    );
}

#[test]
fn the_top_of_the_server_and_folders_are_refused_where_a_file_is_needed() {
    let (jail, root) = jail();
    fs::create_dir_all(root.join("Shared/dir.tar")).unwrap();
    let jobs = Jobs::new();
    assert_eq!(
        finish(&jail, &jobs, "/", ArchiveAction::Compress, None),
        Err(server_error::DENIED)
    );
    assert_eq!(
        finish(&jail, &jobs, "/Shared", ArchiveAction::Gzip, None),
        Err(server_error::DENIED)
    );
    assert_eq!(
        finish(
            &jail,
            &jobs,
            "/Shared/dir.tar",
            ArchiveAction::Extract,
            None
        ),
        Err(server_error::DENIED)
    );
    fs::write(root.join("Shared/a.zip"), "PK").unwrap();
    assert_eq!(
        finish(&jail, &jobs, "/Shared/a.zip", ArchiveAction::Extract, None),
        Err(server_error::MALFORMED)
    );
}

// Hostile archives. Each must be refused whole, leave nothing behind in `/Shared`, and touch
// nothing outside it.

/// Extracts `tar` from `/Shared/evil.tar` and checks it was refused cleanly, returning why.
fn refused(tar: Vec<u8>, limits: Limits) -> String {
    let (jail, root) = jail();
    fs::write(root.join("outside.txt"), "untouched").unwrap();
    fs::write(root.join("Shared/evil.tar"), tar).unwrap();
    let jobs = Jobs::with_limits(limits);
    let result = finish(
        &jail,
        &jobs,
        "/Shared/evil.tar",
        ArchiveAction::Extract,
        None,
    )
    .unwrap();
    let ArchiveResult::Failed(why) = result else {
        panic!("extracted: {result:?}");
    };
    assert_eq!(
        names_in(&root.join("Shared")),
        ["evil.tar"],
        "a refused archive leaves nothing"
    );
    assert_eq!(names_in(&root), ["Docs", "Shared", "outside.txt"]);
    assert_eq!(
        fs::read_to_string(root.join("outside.txt")).unwrap(),
        "untouched"
    );
    why
}

fn refused_tar(build: impl FnOnce(&mut RawTar)) -> String {
    let mut tar = RawTar::default();
    tar.file("ok.txt", b"fine");
    build(&mut tar);
    refused(tar.finish(), LIMITS)
}

#[test]
fn an_absolute_path_is_refused() {
    let why = refused_tar(|t| t.file("/tmp/ccosel-escaped.txt", b"x"));
    assert!(why.contains("absolute"), "{why}");
}

#[test]
fn climbing_out_with_dot_dot_is_refused() {
    let why = refused_tar(|t| t.file("../outside.txt", b"overwritten"));
    assert!(why.contains(".."), "{why}");
    let why = refused_tar(|t| t.file("a/../../outside.txt", b"overwritten"));
    assert!(why.contains(".."), "{why}");
}

#[test]
fn a_symlink_leading_outside_is_refused() {
    let why = refused_tar(|t| t.symlink("up", "../../outside.txt"));
    assert!(why.contains("outside the folder"), "{why}");
    let why = refused_tar(|t| t.symlink("abs", "/etc/passwd"));
    assert!(why.contains("outside the folder"), "{why}");
    let why = refused_tar(|t| t.symlink("deep/link", "../../x"));
    assert!(why.contains("outside the folder"), "{why}");
}

#[test]
fn writing_through_a_symlink_is_refused_even_one_leading_inside() {
    // `l` leads to `sub`, inside; but `l/x` is written through a link, which is how a link
    // made by one entry gets another entry somewhere it shouldn't go.
    let why = refused_tar(|t| {
        t.dir("sub/");
        t.symlink("l", "sub");
        t.file("l/x.txt", b"through");
    });
    assert!(why.contains("isn't a folder"), "{why}");
    // Nor can a file entry replace a link, and so write to wherever it leads.
    let why = refused_tar(|t| {
        t.symlink("same", "ok.txt");
        t.file("same", b"through");
    });
    assert!(why.contains("isn't a file"), "{why}");
}

#[test]
fn a_hard_link_leading_outside_is_refused() {
    let why = refused_tar(|t| t.hard_link("h", "../outside.txt"));
    assert!(why.contains("outside the folder"), "{why}");
    let why = refused_tar(|t| t.hard_link("h", "/etc/passwd"));
    assert!(why.contains("outside the folder"), "{why}");
    let why = refused_tar(|t| t.hard_link("h", "not-in-the-archive.txt"));
    assert!(why.contains("doesn't contain"), "{why}");
}

#[test]
fn a_permissions_file_is_refused() {
    let why = refused_tar(|t| t.file(".access", b"write: *\n"));
    assert!(why.contains("permissions file"), "{why}");
    let why = refused_tar(|t| t.file("sub/.access", b"write: *\n"));
    assert!(why.contains("permissions file"), "{why}");
    let why = refused_tar(|t| t.symlink("rules", "sub/.access"));
    assert!(why.contains("outside the folder"), "{why}");
}

#[test]
fn an_archive_that_unpacks_too_big_is_stopped() {
    let mut tar = RawTar::default();
    tar.file("small.txt", &[0; 600]);
    tar.file("big.txt", &[0; 600]);
    let why = refused(
        tar.finish(),
        Limits {
            max_bytes: 1000,
            max_entries: 100,
        },
    );
    assert!(why.contains("more than"), "{why}");
}

#[test]
fn an_archive_with_too_many_entries_is_stopped() {
    let mut tar = RawTar::default();
    for i in 0..5 {
        tar.file(&format!("f{i}.txt"), b"x");
    }
    let why = refused(
        tar.finish(),
        Limits {
            max_bytes: 1 << 20,
            max_entries: 3,
        },
    );
    assert!(why.contains("more than 3 entries"), "{why}");
}

#[test]
fn a_gz_bomb_is_stopped_and_leaves_nothing() {
    let (jail, root) = jail();
    let mut gz = GzEncoder::new(Vec::new(), Compression::best());
    gz.write_all(&vec![0u8; 1 << 20]).unwrap();
    let small = gz.finish().unwrap();
    assert!(small.len() < 10_000, "a megabyte of zeros packs small");
    fs::write(root.join("Shared/bomb.bin.gz"), small).unwrap();
    let jobs = Jobs::with_limits(Limits {
        max_bytes: 64 << 10,
        max_entries: 10,
    });
    let result = finish(
        &jail,
        &jobs,
        "/Shared/bomb.bin.gz",
        ArchiveAction::Extract,
        None,
    );
    assert!(matches!(result, Ok(ArchiveResult::Failed(ref why)) if why.contains("more than")));
    assert_eq!(names_in(&root.join("Shared")), ["bomb.bin.gz"]);
}

#[test]
fn a_corrupt_archive_fails_and_leaves_nothing() {
    let (jail, root) = jail();
    fs::write(root.join("Shared/junk.tar.gz"), b"not gzip at all").unwrap();
    let result = finish(
        &jail,
        &Jobs::new(),
        "/Shared/junk.tar.gz",
        ArchiveAction::Extract,
        None,
    );
    assert!(matches!(result, Ok(ArchiveResult::Failed(ref why)) if why.contains("valid")));
    assert_eq!(names_in(&root.join("Shared")), ["junk.tar.gz"]);
}

#[test]
fn free_names_count_up_before_the_extension() {
    let (_, root) = jail();
    let dir = root.join("Shared");
    assert_eq!(free_name(&dir, "a.tar.gz"), "a.tar.gz");
    fs::write(dir.join("a.tar.gz"), "").unwrap();
    fs::write(dir.join("a (2).tar.gz"), "").unwrap();
    assert_eq!(free_name(&dir, "a.tar.gz"), "a (3).tar.gz");
    fs::write(dir.join(".hidden"), "").unwrap();
    assert_eq!(free_name(&dir, ".hidden"), ".hidden (2)");
    fs::create_dir(dir.join("plain")).unwrap();
    assert_eq!(free_name(&dir, "plain"), "plain (2)");
}

/// A tar written byte by byte, so it can hold what the `tar` crate refuses to write: absolute
/// paths, `..`, and links anywhere.
#[derive(Default)]
struct RawTar(Vec<u8>);

impl RawTar {
    fn entry(&mut self, name: &str, kind: u8, link: &str, body: &[u8], mode: u32) {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
        header[108..115].copy_from_slice(b"0000000");
        header[116..123].copy_from_slice(b"0000000");
        header[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
        header[136..147].copy_from_slice(b"00000000000");
        header[156] = kind;
        header[157..157 + link.len()].copy_from_slice(link.as_bytes());
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        self.0.extend_from_slice(&header);
        self.0.extend_from_slice(body);
        self.0.resize(self.0.len().div_ceil(512) * 512, 0);
    }

    fn file(&mut self, name: &str, body: &[u8]) {
        self.entry(name, b'0', "", body, 0o644);
    }

    fn file_mode(&mut self, name: &str, body: &[u8], mode: u32) {
        self.entry(name, b'0', "", body, mode);
    }

    fn dir(&mut self, name: &str) {
        self.entry(name, b'5', "", b"", 0o755);
    }

    fn symlink(&mut self, name: &str, to: &str) {
        self.entry(name, b'2', to, b"", 0o777);
    }

    fn hard_link(&mut self, name: &str, to: &str) {
        self.entry(name, b'1', to, b"", 0o644);
    }

    fn finish(mut self) -> Vec<u8> {
        self.0.extend_from_slice(&[0; 1024]);
        self.0
    }
}
