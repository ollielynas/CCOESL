//! The path jail is the server's security boundary. These are the tests that matter most in
//! this crate: everything else is a wrong answer, this is a data breach.

use std::fs;

use ccosel_proto::fs::ListDirReq;
use ccosel_proto::server_error;
use ccosel_server::fs_api::Jail;

/// Each test gets its own tree: cargo runs them in parallel threads within one process, so a
/// shared directory means they delete each other's fixtures.
fn temp_root(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ccosel-jail-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("inside/nested")).unwrap();
    fs::write(dir.join("inside/a.txt"), b"a").unwrap();
    fs::write(dir.join("inside/nested/b.txt"), b"b").unwrap();
    // A sibling the jail must never reach.
    fs::create_dir_all(dir.join("outside")).unwrap();
    fs::write(dir.join("outside/secret.txt"), b"secret").unwrap();
    dir
}

fn jail(root: &std::path::Path) -> Jail {
    Jail::new(root.join("inside")).unwrap()
}

#[test]
fn lists_a_real_directory() {
    let root = temp_root("lists_a_real_directory");
    let j = jail(&root);
    let listing = j.list_dir(&ListDirReq { path: "/" }).unwrap();

    let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["nested", "a.txt"], "dirs first, then name");
    assert!(listing.entries[0].is_dir());
    assert_eq!(listing.entries[1].size, 1);
    assert!(!listing.truncated);
}

#[test]
fn navigates_into_subdirectories() {
    let root = temp_root("navigates_into_subdirectories");
    let j = jail(&root);
    let listing = j.list_dir(&ListDirReq { path: "/nested" }).unwrap();
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].name, "b.txt");
}

#[test]
fn refuses_to_climb_out() {
    let root = temp_root("refuses_to_climb_out");
    let j = jail(&root);
    for attempt in [
        "..",
        "../outside",
        "/../outside",
        "nested/../../outside",
        "/nested/../../outside/secret.txt",
        "....//outside",
    ] {
        let err = j
            .resolve(attempt)
            .expect_err(&format!("{attempt:?} escaped the jail"));
        assert!(
            err == server_error::DENIED || err == server_error::NOT_FOUND,
            "{attempt:?} gave {err}"
        );
    }
}

#[test]
fn refuses_absolute_paths_outside_the_root() {
    let root = temp_root("refuses_absolute_paths_outside_the_root");
    let j = jail(&root);
    let outside = root.join("outside").to_string_lossy().into_owned();
    assert!(j.resolve(&outside).is_err());
    assert!(j.resolve("/etc/passwd").is_err());
}

#[cfg(unix)]
#[test]
fn refuses_a_symlink_that_points_out_of_the_jail() {
    // The case pure string handling cannot catch: a link *inside* the jail aiming outside it.
    // This is why `resolve` canonicalises rather than only checking components.
    let root = temp_root("refuses_a_symlink_that_points_out_of_the_jail");
    std::os::unix::fs::symlink(root.join("outside"), root.join("inside/escape")).unwrap();

    let j = jail(&root);
    assert_eq!(j.resolve("/escape"), Err(server_error::DENIED));
    assert_eq!(
        j.resolve("/escape/secret.txt"),
        Err(server_error::DENIED),
        "reading through the link must fail too"
    );
}

#[test]
fn missing_paths_and_files_report_distinctly() {
    let root = temp_root("missing_paths_and_files_report_distinctly");
    let j = jail(&root);
    assert_eq!(
        j.list_dir(&ListDirReq { path: "/nope" }).unwrap_err(),
        server_error::NOT_FOUND
    );
    assert_eq!(
        j.list_dir(&ListDirReq { path: "/a.txt" }).unwrap_err(),
        server_error::NOT_A_DIRECTORY
    );
}
