use std::process::Command;

use super::*;

fn pr(number: u64, state: PrState) -> Pr {
    Pr {
        number,
        state,
        head: String::new(),
    }
}

#[test]
fn the_worktree_list_is_read_with_branches_locks_and_missing_directories() {
    let porcelain = "\
worktree /r/CCOSEL
HEAD aaaa
branch refs/heads/main

worktree /r/CCOSEL-issue-7
HEAD bbbb
branch refs/heads/agent/7-thing
locked

worktree /r/CCOSEL-review
HEAD cccc
detached

worktree /r/gone
HEAD dddd
branch refs/heads/old
prunable gitdir file points to non-existent location
";
    let trees = parse_worktree_list(porcelain);
    assert_eq!(
        trees,
        vec![
            Worktree {
                path: "/r/CCOSEL".into(),
                branch: Some("main".into()),
                locked: false,
                prunable: false,
            },
            Worktree {
                path: "/r/CCOSEL-issue-7".into(),
                branch: Some("agent/7-thing".into()),
                locked: true,
                prunable: false,
            },
            Worktree {
                path: "/r/CCOSEL-review".into(),
                branch: None,
                locked: false,
                prunable: false,
            },
            Worktree {
                path: "/r/gone".into(),
                branch: Some("old".into()),
                locked: false,
                prunable: true,
            },
        ]
    );
}

#[test]
fn gh_pr_list_output_is_read() {
    let json = r#"[{"headRefOid":"3e83","number":96,"state":"MERGED"},
                   {"headRefOid":"","number":90,"state":"CLOSED"},
                   {"headRefOid":"ab","number":97,"state":"OPEN"}]"#;
    assert_eq!(
        parse_pr_list(json).unwrap(),
        vec![
            Pr {
                number: 96,
                state: PrState::Merged,
                head: "3e83".into()
            },
            pr(90, PrState::Closed),
            Pr {
                number: 97,
                state: PrState::Open,
                head: "ab".into()
            },
        ]
    );
    assert!(parse_pr_list(r#"[{"number":1,"state":"DRAFT"}]"#).is_err());
    assert!(parse_pr_list("not json").is_err());
}

#[test]
fn a_finished_clean_pushed_worktree_is_removed_under_its_latest_pr() {
    assert_eq!(
        verdict(
            &[pr(90, PrState::Closed), pr(96, PrState::Merged)],
            false,
            0
        ),
        Verdict::Remove(96, PrState::Merged)
    );
    assert_eq!(
        verdict(&[pr(5, PrState::Closed)], false, 0),
        Verdict::Remove(5, PrState::Closed)
    );
}

#[test]
fn anything_unfinished_or_unsaved_is_kept_and_says_why() {
    let keep = |why: &str| Verdict::Keep(why.into());
    assert_eq!(verdict(&[], false, 0), keep("no pull request"));
    assert_eq!(
        verdict(&[pr(1, PrState::Merged), pr(2, PrState::Open)], false, 0),
        keep("PR #2 is open")
    );
    assert_eq!(
        verdict(&[pr(1, PrState::Merged)], true, 0),
        keep("uncommitted changes")
    );
    assert_eq!(
        verdict(&[pr(1, PrState::Merged)], false, 1),
        keep("1 commit not pushed")
    );
    assert_eq!(
        verdict(&[pr(1, PrState::Merged)], false, 3),
        keep("3 commits not pushed")
    );
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[test]
fn commits_count_as_pushed_when_on_a_remote_or_at_a_pr_head() {
    let scratch = tempfile::tempdir().unwrap();
    let upstream = scratch.path().join("upstream");
    let repo = scratch.path().join("repo");
    std::fs::create_dir_all(&upstream).unwrap();
    git(&upstream, &["init", "--bare", "-q"]);
    git(
        scratch.path(),
        &["clone", "-q", upstream.to_str().unwrap(), "repo"],
    );
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "one"]);
    git(&repo, &["push", "-q", "origin", "main"]);
    assert_eq!(unpushed_commits(&repo, &[]).unwrap(), 0);

    // Two local commits: not on the remote, so not pushed...
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "two"]);
    let pr_head = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "three"]);
    assert_eq!(unpushed_commits(&repo, &[]).unwrap(), 2);
    // ...until a PR ended at them, as a squash-merged branch whose remote copy was deleted did.
    assert_eq!(unpushed_commits(&repo, &[&pr_head]).unwrap(), 1);
    // A PR head git has never seen vouches for nothing, and doesn't break the count.
    let unknown = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(unpushed_commits(&repo, &[unknown, ""]).unwrap(), 2);
}
