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
    assert_eq!(u.status(), "Uploading photos: 3/12 files, 0 B of 0 B");
    u.done = 12;
    u.finished = true;
    assert_eq!(u.status(), "Uploaded photos: 12 files, 0 B");
    u.failures = vec!["a".into(), "b".into()];
    assert_eq!(u.status(), "Uploaded photos: 10 of 12 files, 2 failed");
    let unnamed = Upload {
        total: 2,
        ..Default::default()
    };
    assert_eq!(unnamed.status(), "Uploading folder: 0/2 files, 0 B of 0 B");
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
    assert_eq!(
        uploads.status().as_deref(),
        Some("Uploading a: 1/4 files, 0 B of 0 B")
    );
}

fn strings(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| (*s).to_owned()).collect()
}

#[wasm_bindgen_test]
fn a_project_upload_leaves_out_gitignored_files_and_git_itself() {
    let paths = strings(&[
        "hello/Cargo.toml",
        "hello/.gitignore",
        "hello/src/main.rs",
        "hello/target/release/hello",
        "hello/target/debug/deps/x.d",
        "hello/.git/HEAD",
        "hello/.git/objects/ab/cdef",
        "hello/notes.log",
        "hello/web/.gitignore",
        "hello/web/node_modules/pkg/index.js",
        "hello/web/dist/app.js",
        "hello/web/keep.log",
    ]);
    let gitignores = vec![
        (
            "hello/web/.gitignore".to_owned(),
            "dist/\n!*.log\n".to_owned(),
        ),
        (
            "hello/.gitignore".to_owned(),
            "/target\n*.log\nnode_modules/\n".to_owned(),
        ),
    ];
    let keep = project_files(&paths, &gitignores);
    let kept: Vec<&str> = paths
        .iter()
        .zip(&keep)
        .filter_map(|(p, k)| k.then_some(p.as_str()))
        .collect();
    assert_eq!(
        kept,
        vec![
            "hello/Cargo.toml",
            "hello/.gitignore",
            "hello/src/main.rs",
            "hello/web/.gitignore",
            "hello/web/keep.log",
        ]
    );
}

#[wasm_bindgen_test]
fn without_a_gitignore_only_git_is_left_out() {
    let paths = strings(&[
        "p/src/main.rs",
        "p/target/x",
        "p/.git/config",
        "p/.github/ci.yml",
    ]);
    assert_eq!(project_files(&paths, &[]), vec![true, true, false, true]);
}

#[wasm_bindgen_test]
fn status_mentions_ignored_files() {
    let mut u = Upload {
        folder: "hello".into(),
        project: true,
        total: 3,
        done: 1,
        skipped: 120,
        ..Default::default()
    };
    assert_eq!(
        u.status(),
        "Uploading hello: 1/3 files, 0 B of 0 B, 120 ignored"
    );
    u.done = 3;
    u.finished = true;
    assert_eq!(u.status(), "Uploaded hello: 3 files, 0 B, 120 ignored");
    u.failures = vec!["x".into()];
    assert_eq!(
        u.status(),
        "Uploaded hello: 2 of 3 files, 1 failed, 120 ignored"
    );
}

#[wasm_bindgen_test]
fn a_dropped_tree_flattens_to_the_paths_a_picked_folder_would_have() {
    let dropped = vec![
        Entry::Dir(
            "photos".to_owned(),
            vec![
                Entry::File("a.jpg".to_owned(), 1),
                Entry::Dir(
                    "2024".to_owned(),
                    vec![
                        Entry::File("b.jpg".to_owned(), 2),
                        Entry::Dir("empty".to_owned(), vec![]),
                    ],
                ),
            ],
        ),
        Entry::File("notes.txt".to_owned(), 3),
    ];
    assert_eq!(
        flatten(dropped),
        vec![
            ("photos/a.jpg".to_owned(), 1),
            ("photos/2024/b.jpg".to_owned(), 2),
            ("notes.txt".to_owned(), 3),
        ]
    );
    assert_eq!(flatten::<()>(vec![]), vec![]);
    // Every flattened path is one `upload_target` accepts.
    assert_eq!(
        target("/Documents", "photos/2024/b.jpg"),
        pair("/Documents/photos/2024", "b.jpg")
    );
}

#[wasm_bindgen_test]
fn an_upload_is_named_after_the_one_thing_chosen_or_counts_them() {
    assert_eq!(
        upload_label(&strings(&["photos/a.jpg", "photos/2024/b.jpg"])),
        "photos"
    );
    assert_eq!(upload_label(&strings(&["notes.txt"])), "notes.txt");
    assert_eq!(
        upload_label(&strings(&["photos/a.jpg", "notes.txt", "b.txt"])),
        "3 items"
    );
    assert_eq!(upload_label(&[]), "folder");
}

#[wasm_bindgen_test]
fn byte_counts_read_in_the_nearest_unit() {
    assert_eq!(human_bytes(0), "0 B");
    assert_eq!(human_bytes(1023), "1023 B");
    assert_eq!(human_bytes(1024), "1.0 KB");
    assert_eq!(human_bytes(1536), "1.5 KB");
    assert_eq!(human_bytes(64 * 1024 * 1024), "64.0 MB");
    assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    assert_eq!(human_bytes(5000 * 1024 * 1024 * 1024), "5000.0 GB");
}

#[wasm_bindgen_test]
fn files_over_the_server_limit_are_refused_before_reading() {
    let max = ccosel_proto::upload::MAX_FILE_BYTES;
    assert_eq!(refuse_size(0), None);
    assert_eq!(refuse_size(max), None);
    assert_eq!(
        refuse_size(max + 1).as_deref(),
        Some("64.0 MB is over the 64.0 MB upload limit")
    );
    assert_eq!(
        refuse_size(100 * 1024 * 1024).as_deref(),
        Some("100.0 MB is over the 64.0 MB upload limit")
    );
}

#[wasm_bindgen_test]
fn a_drop_goes_where_the_windows_upload_button_would_send_it() {
    let folders = [(7, "/Documents".to_owned()), (8, "/Other".to_owned())];
    assert_eq!(
        drop_target(&folders, &[9]),
        Some(DropTarget::Folder(7, "/Documents".to_owned()))
    );
    assert_eq!(drop_target(&[], &[9, 10]), Some(DropTarget::Project(9)));
    assert_eq!(drop_target(&[], &[]), None);
}

#[wasm_bindgen_test]
fn progress_counts_bytes_and_falls_back_to_files() {
    let mut u = Upload {
        total: 4,
        done: 1,
        bytes_total: 1000,
        bytes_done: 750,
        ..Default::default()
    };
    assert_eq!(u.fraction(), 0.75);
    assert_eq!(u.status(), "Uploading folder: 1/4 files, 750 B of 1000 B");
    u.bytes_total = 0;
    u.bytes_done = 0;
    assert_eq!(u.fraction(), 0.25, "no sizes known: by files");
    assert_eq!(Upload::default().fraction(), 0.0);
    u.finished = true;
    assert_eq!(u.fraction(), 1.0);
}

#[wasm_bindgen_test]
fn overall_progress_weighs_uploads_by_their_bytes() {
    let uploads = Uploads::default();
    assert_eq!(uploads.progress(), None);
    uploads.all.borrow_mut().extend([
        Upload {
            seq: 1,
            bytes_total: 300,
            bytes_done: 300,
            ..Default::default()
        },
        Upload {
            seq: 2,
            bytes_total: 100,
            ..Default::default()
        },
    ]);
    assert_eq!(uploads.progress(), Some(0.75));

    let sizeless = Uploads::default();
    sizeless.all.borrow_mut().extend([
        Upload {
            total: 2,
            done: 1,
            ..Default::default()
        },
        Upload::default(),
    ]);
    assert_eq!(sizeless.progress(), Some(0.25));
}
