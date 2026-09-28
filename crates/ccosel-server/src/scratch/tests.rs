use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

fn jail_root() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-scratch-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn starting_up_wipes_what_a_previous_run_left() {
    let root = jail_root();
    std::fs::create_dir_all(root.join(".scratch/77/proj")).unwrap();
    std::fs::write(root.join(".scratch/77/proj/main.rs"), "x").unwrap();
    std::fs::write(root.join("kept.txt"), "x").unwrap();
    let _s = Scratch::new(&root).unwrap();
    assert!(root.join(".scratch").is_dir());
    assert!(!root.join(".scratch/77").exists());
    assert!(root.join("kept.txt").exists(), "only .scratch is touched");
}

#[test]
fn create_makes_a_distinct_empty_folder_each_time() {
    let root = jail_root();
    let s = Scratch::new(&root).unwrap();
    let a = s.create().unwrap();
    let b = s.create().unwrap();
    assert_ne!(a, b);
    assert_ne!(a, 0);
    for id in [a, b] {
        let dir = root.join(".scratch").join(id.to_string());
        assert!(dir.is_dir());
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
        assert!(s.exists(id));
    }
}

#[test]
fn a_folder_used_within_the_ttl_survives_a_sweep() {
    let root = jail_root();
    let s = Scratch::new(&root).unwrap();
    let id = s.create().unwrap();
    assert_eq!(s.sweep(Instant::now() + s.ttl / 2), 0);
    assert!(s.exists(id));
}

#[test]
fn an_unused_folder_is_deleted_after_the_ttl() {
    let root = jail_root();
    let s = Scratch::new(&root).unwrap();
    let id = s.create().unwrap();
    std::fs::write(root.join(format!(".scratch/{id}/f")), "x").unwrap();
    assert_eq!(s.sweep(Instant::now() + s.ttl + Duration::from_secs(1)), 1);
    assert!(!s.exists(id));
    assert!(!root.join(format!(".scratch/{id}")).exists());
}

#[test]
fn touching_any_path_inside_a_folder_keeps_it_alive() {
    let root = jail_root();
    let s = Scratch::new(&root).unwrap();
    let id = s.create().unwrap();
    let created = *s.used.lock().unwrap().get(&id).unwrap();
    std::thread::sleep(Duration::from_millis(5));
    s.touch(&format!("/.scratch/{id}/proj/target/release/app"));
    let touched = *s.used.lock().unwrap().get(&id).unwrap();
    assert!(touched > created);
    // Swept at a moment past the TTL from creation but not from the touch.
    assert_eq!(s.sweep(created + s.ttl + Duration::from_millis(1)), 0);
    assert!(s.exists(id));
}

#[test]
fn touching_other_paths_is_harmless() {
    let root = jail_root();
    let s = Scratch::new(&root).unwrap();
    s.touch("/");
    s.touch("/Documents/.scratch/1");
    s.touch("/.scratch/notanumber/x");
    s.touch("/.scratch/4242/x");
    assert!(s.used.lock().unwrap().is_empty());
}

#[test]
fn a_sweep_removes_folders_the_server_did_not_make() {
    let root = jail_root();
    let s = Scratch::new(&root).unwrap();
    let id = s.create().unwrap();
    // e.g. an upload straight into a made-up id, which the upload route would allow.
    std::fs::create_dir_all(root.join(".scratch/999/x")).unwrap();
    std::fs::write(root.join(".scratch/stray.txt"), "x").unwrap();
    assert_eq!(s.sweep(Instant::now()), 2);
    assert!(s.exists(id));
    assert!(!root.join(".scratch/999").exists());
}
