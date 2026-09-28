use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_proto::settings::{BackgroundChoice, ThemeChoice};

use super::*;

fn jail_root() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-settings-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn changed() -> Settings {
    Settings {
        theme: ThemeChoice::Dark,
        background: BackgroundChoice::Custom,
        custom_image: "/home/alice/beach.jpg".to_owned(),
        reduce_motion: true,
        save_data: false,
        pinned: vec!["file-browser".to_owned(), "clock".to_owned()],
    }
}

#[test]
fn nothing_saved_reads_as_the_defaults() {
    let root = jail_root();
    assert_eq!(get(&root, Some("alice")), Ok(Settings::default()));
}

#[test]
fn what_is_saved_is_what_is_read_back() {
    let root = jail_root();
    set(&root, Some("alice"), &changed()).unwrap();
    assert_eq!(get(&root, Some("alice")), Ok(changed()));
    assert!(root.join("home/alice/.settings.json").is_file());
}

#[test]
fn each_user_has_their_own() {
    let root = jail_root();
    set(&root, Some("alice"), &changed()).unwrap();
    assert_eq!(get(&root, Some("bob")), Ok(Settings::default()));
}

#[test]
fn nobody_signed_in_has_nowhere_to_keep_them() {
    let root = jail_root();
    assert_eq!(get(&root, None), Err(server_error::NO_USER));
    assert_eq!(set(&root, None, &changed()), Err(server_error::NO_USER));
}

/// A file from before a field existed still loads, with the missing field at its default.
#[test]
fn an_older_file_fills_in_what_it_lacks() {
    let root = jail_root();
    std::fs::create_dir_all(root.join("home/alice")).unwrap();
    std::fs::write(
        root.join("home/alice/.settings.json"),
        r#"{"theme":"Light"}"#,
    )
    .unwrap();
    let got = get(&root, Some("alice")).unwrap();
    assert_eq!(got.theme, ThemeChoice::Light);
    assert_eq!(got.background, BackgroundChoice::Auto);
    assert!(got.pinned.is_empty());
}

#[test]
fn a_corrupt_file_reads_as_the_defaults() {
    let root = jail_root();
    std::fs::create_dir_all(root.join("home/alice")).unwrap();
    std::fs::write(root.join("home/alice/.settings.json"), "{not json").unwrap();
    assert_eq!(get(&root, Some("alice")), Ok(Settings::default()));
}
