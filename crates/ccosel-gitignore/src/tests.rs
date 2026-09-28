use super::*;

fn filter(files: &[(&str, &str)]) -> Filter {
    let mut f = Filter::new();
    for (base, contents) in files {
        f.add(base, contents);
    }
    f
}

fn ignored(f: &Filter, paths: &[&str]) -> Vec<String> {
    paths
        .iter()
        .filter(|p| f.is_ignored(p))
        .map(|p| (*p).to_owned())
        .collect()
}

#[test]
fn a_cargo_projects_target_directory_is_ignored() {
    let f = filter(&[("", "/target\n")]);
    assert!(f.is_ignored("target/release/app"));
    assert!(f.is_ignored("target/debug/.fingerprint/x/y"));
    assert!(!f.is_ignored("src/main.rs"));
    assert!(!f.is_ignored("src/target/x.rs"), "anchored to the root");
}

#[test]
fn an_unanchored_name_matches_at_any_depth() {
    let f = filter(&[("", "node_modules\n*.log\n")]);
    assert_eq!(
        ignored(
            &f,
            &[
                "node_modules/a.js",
                "web/node_modules/b.js",
                "a.log",
                "x/y/z.log",
                "log.txt"
            ]
        ),
        vec![
            "node_modules/a.js",
            "web/node_modules/b.js",
            "a.log",
            "x/y/z.log"
        ]
    );
}

#[test]
fn a_trailing_slash_matches_directories_only() {
    let f = filter(&[("", "build/\n")]);
    assert!(f.is_ignored("build/out.o"));
    assert!(f.is_ignored("sub/build/out.o"));
    assert!(!f.is_ignored("build"), "a file called build is kept");
    assert!(!f.is_ignored("sub/build"));
}

#[test]
fn a_middle_slash_anchors_the_pattern() {
    let f = filter(&[("", "doc/frotz\n")]);
    assert!(f.is_ignored("doc/frotz"));
    assert!(f.is_ignored("doc/frotz/x"));
    assert!(!f.is_ignored("a/doc/frotz"));
}

#[test]
fn negation_re_includes_a_file() {
    let f = filter(&[("", "*.txt\n!keep.txt\n")]);
    assert!(f.is_ignored("a.txt"));
    assert!(!f.is_ignored("keep.txt"));
    assert!(!f.is_ignored("sub/keep.txt"));
}

#[test]
fn the_last_matching_line_wins() {
    let f = filter(&[("", "!a.txt\n*.txt\n")]);
    assert!(f.is_ignored("a.txt"));
}

#[test]
fn nothing_inside_an_ignored_directory_can_be_re_included() {
    let f = filter(&[("", "logs/\n!logs/keep.log\n")]);
    assert!(f.is_ignored("logs/keep.log"));
    // But ignoring the directory's *contents* rather than the directory leaves room to.
    let f = filter(&[("", "logs/*\n!logs/keep.log\n")]);
    assert!(f.is_ignored("logs/other.log"));
    assert!(!f.is_ignored("logs/keep.log"));
}

#[test]
fn a_nested_gitignore_applies_below_its_own_directory_and_overrides_its_parent() {
    let f = filter(&[("", "*.tmp\n"), ("sub", "!special.tmp\n/local\n")]);
    assert!(f.is_ignored("a.tmp"));
    assert!(f.is_ignored("other/special.tmp"));
    assert!(!f.is_ignored("sub/special.tmp"));
    assert!(f.is_ignored("sub/local/x"));
    assert!(!f.is_ignored("local/x"), "anchored to sub/, not the root");
}

#[test]
fn double_star_forms() {
    let lead = filter(&[("", "**/foo\n")]);
    assert!(lead.is_ignored("foo"));
    assert!(lead.is_ignored("a/b/foo"));

    let lead_nested = filter(&[("", "**/foo/bar\n")]);
    assert!(lead_nested.is_ignored("foo/bar"));
    assert!(lead_nested.is_ignored("x/foo/bar"));
    assert!(!lead_nested.is_ignored("x/bar"));

    let trail = filter(&[("", "abc/**\n")]);
    assert!(trail.is_ignored("abc/x"));
    assert!(trail.is_ignored("abc/x/y"));
    assert!(!trail.is_ignored("abc"));

    let middle = filter(&[("", "a/**/b\n")]);
    for p in ["a/b", "a/x/b", "a/x/y/b"] {
        assert!(middle.is_ignored(p), "{p}");
    }
    assert!(!middle.is_ignored("a/xb"));

    // Not at a segment boundary: an ordinary `*`, which doesn't cross `/`.
    let plain = filter(&[("", "/a**z\n")]);
    assert!(plain.is_ignored("abcz"));
    assert!(!plain.is_ignored("ab/cz"));
}

#[test]
fn single_star_and_question_mark_do_not_cross_directories() {
    let f = filter(&[("", "/a/*.rs\n/?.md\n")]);
    assert!(f.is_ignored("a/x.rs"));
    assert!(!f.is_ignored("a/b/x.rs"));
    assert!(f.is_ignored("x.md"));
    assert!(!f.is_ignored("xy.md"));
}

#[test]
fn character_classes() {
    let f = filter(&[("", "*.[oa]\nfile[0-9]\n[!x]y\n[]]z\n")]);
    assert_eq!(
        ignored(
            &f,
            &["x.o", "x.a", "x.c", "file7", "filex", "ay", "xy", "]z"]
        ),
        vec!["x.o", "x.a", "file7", "ay", "]z"]
    );
    // An unclosed `[` is literal.
    let f = filter(&[("", "a[b\n")]);
    assert!(f.is_ignored("a[b"));
}

#[test]
fn comments_blank_lines_escapes_and_whitespace() {
    let f = filter(&[(
        "",
        "# a comment\n\n\\#hash\n\\!bang\ntrailing   \nkept\\ \r\nwin\r\n",
    )]);
    assert!(f.is_ignored("#hash"));
    assert!(f.is_ignored("!bang"));
    assert!(f.is_ignored("trailing"));
    assert!(f.is_ignored("kept "));
    assert!(!f.is_ignored("kept"));
    assert!(f.is_ignored("win"), "CRLF line endings");
    assert!(!f.is_ignored("# a comment"));
}

#[test]
fn nothing_is_ignored_without_rules() {
    let f = Filter::new();
    assert!(!f.is_ignored("target/x"));
    let f = filter(&[("", "!\n/\n#x\n")]);
    assert!(!f.is_ignored("a"));
}

#[test]
fn paths_with_stray_slashes_are_normalised() {
    let f = filter(&[("/proj/", "/target\n")]);
    assert!(f.is_ignored("/proj/target/x"));
    assert!(!f.is_ignored("proj/src/x"));
}
