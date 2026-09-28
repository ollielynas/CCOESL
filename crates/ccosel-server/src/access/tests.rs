use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

fn tree() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-access-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Docs/Apps")).unwrap();
    fs::create_dir_all(dir.join("Team/Drafts")).unwrap();
    fs::create_dir_all(dir.join("Open")).unwrap();
    fs::write(dir.join("Docs").join(ACCESS_FILE), "read: *\n").unwrap();
    fs::write(
        dir.join("Team").join(ACCESS_FILE),
        "# the team\nread: @users\nwrite: alice bob  # not carol\n",
    )
    .unwrap();
    fs::write(dir.join("Team/Drafts").join(ACCESS_FILE), "write: alice\n").unwrap();
    fs::write(dir.join("Team/plan.md"), "plan").unwrap();
    dir
}

fn p(path: &str) -> Vec<String> {
    components(path).unwrap()
}

const RW: Perms = Perms {
    read: true,
    write: true,
};
const RO: Perms = Perms {
    read: true,
    write: false,
};

#[test]
fn with_no_access_file_everything_is_open() {
    let root = tree();
    assert_eq!(perms(&root, &p("/Open/anything.md"), None), RW);
    assert_eq!(perms(&root, &p("/"), None), RW);
}

#[test]
fn a_folder_passes_its_rules_down_to_files_and_subfolders() {
    let root = tree();
    // Read-only for everyone, signed in or not, all the way down.
    for user in [None, Some("alice")] {
        assert_eq!(perms(&root, &p("/Docs"), user), RO);
        assert_eq!(perms(&root, &p("/Docs/Apps/files.md"), user), RO);
        assert_eq!(perms(&root, &p("/Docs/Apps/new-folder/x.md"), user), RO);
    }
}

#[test]
fn users_and_groups_are_matched() {
    let root = tree();
    let plan = p("/Team/plan.md");
    assert_eq!(perms(&root, &plan, None), Perms::NONE);
    assert_eq!(perms(&root, &plan, Some("carol")), RO);
    assert_eq!(perms(&root, &plan, Some("bob")), RW);
}

#[test]
fn the_nearest_access_file_wins() {
    let root = tree();
    let draft = p("/Team/Drafts/idea.md");
    assert_eq!(perms(&root, &draft, Some("alice")), RW);
    // Drafts has no read line, and write implies read, so only alice gets in: bob's rights
    // on /Team do not reach into a folder with its own rules.
    assert_eq!(perms(&root, &draft, Some("bob")), Perms::NONE);
}

#[test]
fn home_folders_are_private_to_their_owner() {
    let root = tree();
    // A `.access` inside a home folder cannot open it up.
    fs::create_dir_all(root.join("home/bob")).unwrap();
    fs::write(
        root.join("home/bob").join(ACCESS_FILE),
        "read: *\nwrite: *\n",
    )
    .unwrap();

    assert_eq!(perms(&root, &p("/home/alice/notes.md"), Some("alice")), RW);
    assert_eq!(
        perms(&root, &p("/home/bob/notes.md"), Some("alice")),
        Perms::NONE
    );
    assert_eq!(perms(&root, &p("/home/bob"), None), Perms::NONE);
    assert_eq!(perms(&root, &p("/home"), Some("alice")), RO);
    assert_eq!(perms(&root, &p("/home"), None), Perms::NONE);
}

#[test]
fn access_files_cannot_be_named_and_paths_cannot_climb() {
    assert_eq!(components("/Docs/.access"), Err(server_error::DENIED));
    assert_eq!(components("/Docs/../x"), Err(server_error::DENIED));
    assert_eq!(components("/a/./b").unwrap(), vec!["a", "b"]);
    assert!(components("").unwrap().is_empty());
}

#[test]
fn an_unreadable_access_file_denies_rather_than_falling_through() {
    let root = tree();
    // A *directory* called `.access` cannot be read as rules. Falling back to the parent's
    // (open) rules would be the wrong way to fail.
    fs::create_dir_all(root.join("Open/Locked").join(ACCESS_FILE)).unwrap();
    assert_eq!(
        perms(&root, &p("/Open/Locked/x"), Some("alice")),
        Perms::NONE
    );
}

#[test]
fn real_paths_are_checked_the_same_way() {
    let root = tree().canonicalize().unwrap();
    assert_eq!(
        perms_of_real(&root, &root.join("Docs/Apps"), None),
        Some(RO)
    );
    assert_eq!(
        perms_of_real(&root, &root.join("Docs").join(ACCESS_FILE), None),
        Some(Perms::NONE)
    );
    assert_eq!(perms_of_real(&root, Path::new("/etc"), None), None);
}

#[test]
fn user_names_that_could_escape_home_are_refused() {
    for ok in ["alice", "bob.smith", "b-o_b", "a@b"] {
        assert!(User::valid_name(ok), "{ok}");
    }
    for bad in ["", ".", "..", ".hidden", "a/b", "a\\b", &"x".repeat(65)] {
        assert!(!User::valid_name(bad), "{bad:?}");
    }
}

#[test]
fn ensure_home_is_idempotent() {
    let root = tree();
    let a = ensure_home(&root, "alice").unwrap();
    assert!(a.is_dir());
    assert_eq!(ensure_home(&root, "alice").unwrap(), a);
}
