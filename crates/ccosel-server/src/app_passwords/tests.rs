use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

fn temp_file(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-app-passwords-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join(name)
}

#[test]
fn a_new_password_signs_in_as_its_owner_only() {
    let store = AppPasswords::in_memory();
    let made = store.create("alice", "Laptop").unwrap();
    assert_eq!(made.password.len(), PASSWORD_LEN);
    assert_eq!(made.info.name, "Laptop");
    assert!(store.verify("alice", &made.password));
    assert!(!store.verify("bob", &made.password));
    assert!(!store.verify("alice", "not-it"));
    assert!(!store.verify("alice", ""));
}

#[test]
fn each_user_sees_only_their_own() {
    let store = AppPasswords::in_memory();
    let a = store.create("alice", "Laptop").unwrap();
    store.create("bob", "Phone").unwrap();
    let b = store.create("alice", "Desktop").unwrap();
    let names: Vec<_> = store
        .list("alice")
        .into_iter()
        .map(|i| (i.id, i.name))
        .collect();
    assert_eq!(
        names,
        [
            (a.info.id, "Laptop".to_owned()),
            (b.info.id, "Desktop".to_owned())
        ]
    );
    assert_eq!(store.list("carol"), []);
}

#[test]
fn a_revoked_password_stops_working() {
    let store = AppPasswords::in_memory();
    let made = store.create("alice", "Laptop").unwrap();
    assert_eq!(store.revoke("alice", &made.info.id), Ok(()));
    assert!(!store.verify("alice", &made.password));
    assert_eq!(store.list("alice"), []);
    assert_eq!(
        store.revoke("alice", &made.info.id),
        Err(server_error::NOT_FOUND)
    );
}

#[test]
fn nobody_can_revoke_someone_elses() {
    let store = AppPasswords::in_memory();
    let made = store.create("alice", "Laptop").unwrap();
    assert_eq!(
        store.revoke("bob", &made.info.id),
        Err(server_error::NOT_FOUND)
    );
    assert!(store.verify("alice", &made.password));
}

#[test]
fn names_must_be_short_printable_and_not_blank() {
    let store = AppPasswords::in_memory();
    for bad in [
        "",
        "   ",
        "tab\there",
        &"x".repeat(MAX_APP_PASSWORD_NAME + 1),
    ] {
        assert_eq!(
            store.create("alice", bad).map(|_| ()),
            Err(server_error::MALFORMED),
            "{bad:?}"
        );
    }
    assert_eq!(
        store.create("alice", "  Laptop ").unwrap().info.name,
        "Laptop"
    );
}

#[test]
fn there_is_a_limit_per_user() {
    let store = AppPasswords::in_memory();
    for i in 0..MAX_APP_PASSWORDS {
        store.create("alice", &format!("device {i}")).unwrap();
    }
    assert_eq!(
        store.create("alice", "one more").map(|_| ()),
        Err(server_error::LIMIT)
    );
    // Someone else's count is their own.
    assert!(store.create("bob", "Phone").is_ok());
}

#[test]
fn use_is_recorded() {
    let store = AppPasswords::in_memory();
    let made = store.create("alice", "Laptop").unwrap();
    assert_eq!(store.list("alice")[0].last_used_s, None);
    assert!(store.verify("alice", &made.password));
    let used = store.list("alice")[0].last_used_s.unwrap();
    assert!(used >= made.info.created_s);
}

#[test]
fn the_store_survives_a_restart_and_keeps_only_hashes() {
    let file = temp_file("app-passwords.json");
    let made = {
        let store = AppPasswords::open(file.clone()).unwrap();
        assert_eq!(store.file(), Some(file.as_path()));
        store.create("alice", "Laptop").unwrap()
    };

    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        !text.contains(&made.password),
        "the password itself is never saved"
    );
    assert!(text.contains(&hash(&made.password)));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "readable by the server's account only");
    }

    let store = AppPasswords::open(file.clone()).unwrap();
    assert!(store.verify("alice", &made.password));
    store.revoke("alice", &made.info.id).unwrap();
    let store = AppPasswords::open(file).unwrap();
    assert!(!store.verify("alice", &made.password));
}

#[test]
fn a_damaged_store_is_an_error_not_an_empty_one() {
    let file = temp_file("app-passwords.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "{ not json").unwrap();
    assert!(AppPasswords::open(file.clone()).is_err());

    // A folder where the file should be cannot be read either.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert!(AppPasswords::open(file).is_err());
}

#[test]
fn a_failed_save_changes_nothing() {
    let file = temp_file("app-passwords.json");
    let store = AppPasswords::open(file.clone()).unwrap();
    let made = store.create("alice", "Laptop").unwrap();

    // Make saving impossible: the temporary file's name is taken by a folder.
    std::fs::create_dir(file.with_extension("json.tmp")).unwrap();
    assert_eq!(
        store.create("alice", "Phone").map(|_| ()),
        Err(server_error::IO)
    );
    assert_eq!(store.list("alice").len(), 1);
    assert_eq!(store.revoke("alice", &made.info.id), Err(server_error::IO));
    assert!(store.verify("alice", &made.password), "still not revoked");
}

#[test]
fn same_compares_whole_strings() {
    assert!(same("abc", "abc"));
    assert!(!same("abc", "abd"));
    assert!(!same("abc", "ab"));
}

#[test]
fn the_throttle_blocks_a_key_after_too_many_failures_until_the_window_ends() {
    let throttle = Throttle::default();
    let start = Instant::now();
    let alice = ["login:alice".to_owned()];
    for _ in 0..THROTTLE_FAILURES {
        assert_eq!(throttle.blocked(&alice, start), None);
        throttle.fail(&alice, start);
    }
    assert_eq!(throttle.blocked(&alice, start), Some(THROTTLE_WINDOW));
    // Others are not affected.
    assert_eq!(throttle.blocked(&["login:bob".to_owned()], start), None);
    // A list holding a blocked key is blocked.
    let both = ["login:bob".to_owned(), "login:alice".to_owned()];
    assert!(throttle.blocked(&both, start).is_some());

    let later = start + THROTTLE_WINDOW;
    assert_eq!(throttle.blocked(&alice, later), None);
    // And a new window starts from scratch.
    throttle.fail(&alice, later);
    assert_eq!(throttle.blocked(&alice, later), None);
}

#[test]
fn an_old_window_restarts_on_the_next_failure() {
    let throttle = Throttle::default();
    let start = Instant::now();
    let key = ["ip:127.0.0.1".to_owned()];
    for _ in 0..THROTTLE_FAILURES - 1 {
        throttle.fail(&key, start);
    }
    // No `blocked` call in between has swept it, so the old count is still stored.
    throttle.fail(&key, start + THROTTLE_WINDOW);
    assert_eq!(throttle.blocked(&key, start + THROTTLE_WINDOW), None);
}
