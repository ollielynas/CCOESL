use crate::paths::*;

#[test]
fn parent_name_and_join() {
    assert_eq!(parent("/Docs/Apps/files.md"), "/Docs/Apps");
    assert_eq!(parent("/Docs"), "/");
    assert_eq!(parent("/"), "/");
    assert_eq!(name("/Docs/Apps/files.md"), "files.md");
    assert_eq!(name("/Docs/"), "Docs");
    assert_eq!(join("/", "a"), "/a");
    assert_eq!(join("/Docs", "a"), "/Docs/a");
}

#[test]
fn links_resolve_against_the_documents_folder() {
    let from = "/Docs/Apps/files.md";
    assert_eq!(
        resolve(from, "clock.md").as_deref(),
        Some("/Docs/Apps/clock.md")
    );
    assert_eq!(
        resolve(from, "./clock.md").as_deref(),
        Some("/Docs/Apps/clock.md")
    );
    assert_eq!(
        resolve(from, "../README.md").as_deref(),
        Some("/Docs/README.md")
    );
    assert_eq!(
        resolve(from, "/Shared/x.md#part").as_deref(),
        Some("/Shared/x.md")
    );
    assert_eq!(resolve(from, "../../../../etc").as_deref(), Some("/etc"));
    assert_eq!(resolve(from, "#top"), None);
}

#[test]
fn documents_and_titles() {
    assert!(is_doc("/a/B.MD"));
    assert!(is_doc("x.markdown"));
    assert!(!is_doc("x.txt"));
    assert_eq!(title("/a/getting-started.md"), "getting-started");
    assert_eq!(title("notes.markdown"), "notes");
    assert_eq!(title("Folder"), "Folder");
}

#[test]
fn new_names_are_checked_and_documents_get_an_extension() {
    assert_eq!(new_name(" Plan ", true).as_deref(), Some("Plan.md"));
    assert_eq!(new_name("Plan.md", true).as_deref(), Some("Plan.md"));
    assert_eq!(new_name("Team", false).as_deref(), Some("Team"));
    for bad in ["", "  ", ".hidden", "a/b", "a\\b", &"x".repeat(121)] {
        assert_eq!(new_name(bad, true), None, "{bad:?}");
    }
}

#[test]
fn download_urls_are_encoded() {
    assert_eq!(download_url("/Docs/a b#1.png"), "/files/Docs/a%20b%231.png");
}
