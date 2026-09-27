//! Run under node by `cargo xtask test-wasm`: this crate only compiles for wasm32.

use wasm_bindgen_test::wasm_bindgen_test;

use super::*;

fn target(dest: &str, rel: &str) -> Option<(String, String)> {
    upload_target(dest, rel)
}

fn pair(dir: &str, name: &str) -> Option<(String, String)> {
    Some((dir.to_owned(), name.to_owned()))
}

#[wasm_bindgen_test]
fn the_picked_folder_keeps_its_name_under_the_destination() {
    assert_eq!(
        target("/Documents", "photos/a.jpg"),
        pair("/Documents/photos", "a.jpg")
    );
    assert_eq!(
        target("/Documents", "photos/2024/june/a.jpg"),
        pair("/Documents/photos/2024/june", "a.jpg")
    );
}

#[wasm_bindgen_test]
fn uploading_to_the_root_does_not_double_the_slash() {
    assert_eq!(target("/", "photos/a.jpg"), pair("/photos", "a.jpg"));
    assert_eq!(
        target("/Documents/", "p/a.jpg"),
        pair("/Documents/p", "a.jpg")
    );
}

#[wasm_bindgen_test]
fn a_bare_file_name_goes_straight_into_the_destination() {
    assert_eq!(target("/Documents", "a.jpg"), pair("/Documents", "a.jpg"));
    assert_eq!(target("/", "a.jpg"), pair("/", "a.jpg"));
}

#[wasm_bindgen_test]
fn empty_dot_and_dotdot_parts_are_refused() {
    for rel in [
        "",
        "photos/",
        "/a.jpg",
        "photos//a.jpg",
        "photos/../a.jpg",
        "./a.jpg",
        "..",
    ] {
        assert_eq!(target("/Documents", rel), None, "{rel:?}");
    }
}

#[wasm_bindgen_test]
fn upload_url_encodes_each_value() {
    assert_eq!(
        upload_url("/uploads/42", "main.rs"),
        "/upload?path=%2Fuploads%2F42&filename=main.rs"
    );
    assert_eq!(
        upload_url("/my project", "read me.txt"),
        "/upload?path=%2Fmy%20project&filename=read%20me.txt"
    );
    // Characters that would otherwise end or split the query string.
    assert_eq!(
        upload_url("/a#b", "c?d&e=f%g+h.txt"),
        "/upload?path=%2Fa%23b&filename=c%3Fd%26e%3Df%25g%2Bh.txt"
    );
    assert_eq!(
        upload_url("/café", "résumé 😀.pdf"),
        "/upload?path=%2Fcaf%C3%A9&filename=r%C3%A9sum%C3%A9%20%F0%9F%98%80.pdf"
    );
    assert_eq!(
        upload_url("/", "a-b_c.d~e"),
        "/upload?path=%2F&filename=a-b_c.d~e"
    );
}

#[wasm_bindgen_test]
fn same_site_paths_and_http_urls_may_be_opened() {
    for url in [
        "/files/notes.md",
        "/files/a%20b.txt",
        "http://example.com/x",
        "https://example.com/x?y=z",
        "HTTPS://example.com",
    ] {
        assert!(may_open(url), "{url}");
    }
}

#[wasm_bindgen_test]
fn every_other_scheme_is_refused() {
    for url in [
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        " javascript:alert(1)",
        "\tjavascript:alert(1)",
        "java\nscript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "file:///etc/passwd",
        "blob:https://example.com/uuid",
        "//evil.example/x",
        "/\\evil.example/x",
        "notes.md",
        "",
        "   ",
    ] {
        assert!(!may_open(url), "{url:?}");
    }
}

#[wasm_bindgen_test]
fn open_url_refuses_before_touching_the_browser() {
    let err = open_url("javascript:alert(1)").unwrap_err();
    assert!(err.contains("refused"), "{err}");
}

#[wasm_bindgen_test]
fn status_reads_progress_then_the_outcome() {
    let mut u = Upload {
        folder: "photos".to_owned(),
        total: 12,
        done: 3,
        ..Default::default()
    };
    assert_eq!(u.status(), "Uploading photos: 3/12");
    u.done = 12;
    u.finished = true;
    assert_eq!(u.status(), "Uploaded photos: 12 files");
    u.failures = vec!["a".into(), "b".into()];
    assert_eq!(u.status(), "Uploaded photos: 10 of 12 files, 2 failed");
    let unnamed = Upload {
        total: 2,
        ..Default::default()
    };
    assert_eq!(unnamed.status(), "Uploading folder: 0/2");
}

#[wasm_bindgen_test]
fn finished_uploads_are_handed_over_once_and_running_ones_stay() {
    let uploads = Uploads::default();
    assert_eq!(uploads.status(), None);
    uploads.all.borrow_mut().extend([
        Upload {
            seq: 1,
            folder: "a".into(),
            total: 4,
            done: 1,
            ..Default::default()
        },
        Upload {
            seq: 2,
            finished: true,
            ..Default::default()
        },
    ]);
    assert_eq!(uploads.status().as_deref(), Some("2 uploads running"));
    let finished = uploads.take_finished();
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].seq, 2);
    assert!(uploads.take_finished().is_empty());
    assert_eq!(uploads.status().as_deref(), Some("Uploading a: 1/4"));
}
