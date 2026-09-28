//! The matcher, checked against `git` itself: build a tree of files and `.gitignore`s, ask git
//! which files it would add (`git ls-files --others --exclude-standard`), and require the
//! filter to keep exactly those. Git is the spec; this is how the matcher is held to it.
//!
//! Needs `git` on `PATH`, which CI and every development machine for this repo have.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_gitignore::Filter;

fn temp_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-gitignore-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes `files` (path, contents) under a fresh repo, then compares git's verdict with the
/// filter's for every file. Returns a description of each disagreement.
fn disagreements(files: &[(String, String)]) -> Vec<String> {
    let root = temp_dir();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git must be installed to run this test");
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    git(&["init", "-q"]);
    for (path, contents) in files {
        let full = root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, contents).unwrap();
    }
    let kept_by_git: BTreeSet<String> = git(&["ls-files", "--others", "--exclude-standard", "-z"])
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();

    // What the shell does: collect every .gitignore, shallowest first, then ask per file.
    let mut ignores: Vec<&(String, String)> = files
        .iter()
        .filter(|(p, _)| p.ends_with(".gitignore"))
        .collect();
    ignores.sort_by_key(|(p, _)| p.matches('/').count());
    let mut filter = Filter::new();
    for (path, contents) in ignores {
        let base = Path::new(path).parent().unwrap().to_str().unwrap();
        filter.add(base, contents);
    }

    let mut out = Vec::new();
    for (path, _) in files {
        let git_keeps = kept_by_git.contains(path);
        let we_keep = !filter.is_ignored(path);
        if git_keeps != we_keep {
            out.push(format!(
                "{path}: git keeps={git_keeps}, filter keeps={we_keep}"
            ));
        }
    }
    let _ = fs::remove_dir_all(&root);
    out
}

fn tree(entries: &[(&str, &str)]) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|(p, c)| ((*p).to_owned(), (*c).to_owned()))
        .collect()
}

#[test]
fn agrees_with_git_on_a_realistic_project() {
    let files = tree(&[
        (
            ".gitignore",
            "/target\n**/*.rs.bk\n*.log\n!important.log\nnode_modules/\n.env\ndocs/_build/\n",
        ),
        ("Cargo.toml", "[package]"),
        ("src/main.rs", "fn main(){}"),
        ("src/main.rs.bk", ""),
        ("target/release/app", ""),
        ("target/debug/deps/x.d", ""),
        ("src/target/keep.rs", ""),
        ("a.log", ""),
        ("important.log", ""),
        ("deep/x/important.log", ""),
        ("deep/x/other.log", ""),
        ("web/node_modules/pkg/index.js", ""),
        ("web/app.js", ""),
        (".env", ""),
        ("config/.env", ""),
        ("docs/_build/html/index.html", ""),
        ("docs/index.md", ""),
        ("crates/inner/.gitignore", "!*.log\n/generated/\nscratch*\n"),
        ("crates/inner/debug.log", ""),
        ("crates/inner/generated/out.rs", ""),
        ("crates/inner/src/generated/kept.rs", ""),
        ("crates/inner/scratch.txt", ""),
        ("crates/inner/src/scratchpad/a.rs", ""),
        ("crates/inner/src/lib.rs", ""),
    ]);
    assert_eq!(disagreements(&files), Vec::<String>::new());
}

#[test]
fn agrees_with_git_on_the_tricky_corners() {
    let files = tree(&[
        (
            ".gitignore",
            "logs/\n!logs/keep.log\ncache/*\n!cache/keep\na/**/z\n**/deep\nx/**\n/*.tmp\n\
             [Bb]uild\n?.one\n\\#hash\n\\!bang\ntrail  \nspace\\ \nfile[0-9]\n[!a]only\n",
        ),
        ("logs/keep.log", ""),
        ("logs/other.log", ""),
        ("cache/keep", ""),
        ("cache/drop", ""),
        ("a/z", ""),
        ("a/b/c/z", ""),
        ("a/bz", ""),
        ("q/deep/f", ""),
        ("deep", ""),
        ("x/y/z", ""),
        ("xx", ""),
        ("top.tmp", ""),
        ("sub/top.tmp", ""),
        ("Build/o", ""),
        ("build", ""),
        ("q/build", ""),
        ("1.one", ""),
        ("12.one", ""),
        ("#hash", ""),
        ("!bang", ""),
        ("trail", ""),
        ("space ", ""),
        ("space", ""),
        ("file3", ""),
        ("filex", ""),
        ("bonly", ""),
        ("aonly", ""),
    ]);
    assert_eq!(disagreements(&files), Vec::<String>::new());
}

/// A tiny deterministic PRNG, so the random trees are the same on every run and machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[(self.next() % xs.len() as u64) as usize]
    }
}

#[test]
fn agrees_with_git_on_random_trees() {
    const NAMES: &[&str] = &["a", "b", "ab", "x.rs", "y.log", "target", "keep", "deep"];
    const PATTERNS: &[&str] = &[
        "a",
        "b",
        "*.log",
        "!*.log",
        "target",
        "/target",
        "target/",
        "!keep",
        "a/",
        "/a",
        "a/b",
        "a/*",
        "!a/keep",
        "**/b",
        "a/**",
        "a/**/x.rs",
        "*",
        "!*",
        "?",
        "[ab]",
        "[!a]*",
        "deep/",
        "!deep",
        "*.rs",
        "/*/keep",
        "x.*",
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut failures = Vec::new();
    for case in 0..150 {
        let mut files = Vec::new();
        let mut paths = BTreeSet::new();
        for _ in 0..12 {
            let depth = 1 + rng.next() % 4;
            let parts: Vec<&str> = (0..depth).map(|_| rng.pick(NAMES)).collect();
            paths.insert(parts.join("/"));
        }
        // A path can't be both a file and a directory.
        let dirs: BTreeSet<String> = paths
            .iter()
            .flat_map(|p| {
                let parts: Vec<&str> = p.split('/').collect();
                (1..parts.len())
                    .map(|n| parts[..n].join("/"))
                    .collect::<Vec<_>>()
            })
            .collect();
        for p in paths.iter().filter(|p| !dirs.contains(*p)) {
            files.push((p.clone(), String::new()));
        }
        for dir in std::iter::once(String::new()).chain(dirs.iter().cloned()) {
            if !rng.next().is_multiple_of(3) {
                continue;
            }
            let lines: Vec<&str> = (0..1 + rng.next() % 4)
                .map(|_| rng.pick(PATTERNS))
                .collect();
            let path = if dir.is_empty() {
                ".gitignore".to_owned()
            } else {
                format!("{dir}/.gitignore")
            };
            files.push((path, lines.join("\n") + "\n"));
        }
        for d in disagreements(&files) {
            failures.push(format!("case {case}: {d}\n  tree: {files:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
