//! `cargo xtask prune-worktrees`: remove the worktrees whose pull request is finished.
//!
//! Every ticket gets its own worktree (`../<repo>-issue-<n>`), each with a `target/` of several
//! gigabytes, and nothing removes them when the PR is merged, so they pile up beside the repo. This
//! removes a worktree, and its local branch, only when all of these hold:
//!
//! - its branch has a pull request on GitHub and none of them is still open;
//! - it has no uncommitted changes or untracked files;
//! - every commit on it is on the remote, or is the commit the PR ended at. A squash-merge leaves
//!   the branch's own commits off `main`, so "reachable from a remote" alone would keep every one.
//!
//! The main checkout, the one this runs from, the review checkout (`cargo xtask review` manages
//! it), and detached or locked worktrees are never touched. Anything kept is listed with why.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{capture, primary_checkout, review_dir_for, root, run};

const USAGE: &str = "usage: cargo xtask prune-worktrees [--dry-run]";

/// One entry of `git worktree list --porcelain`.
#[derive(Debug, PartialEq, Eq)]
struct Worktree {
    path: PathBuf,
    /// The checked-out branch, without `refs/heads/`. `None` when detached.
    branch: Option<String>,
    locked: bool,
    /// Its directory is gone; `git worktree prune` forgets it.
    prunable: bool,
}

fn parse_worktree_list(porcelain: &str) -> Vec<Worktree> {
    porcelain
        .split("\n\n")
        .filter_map(|entry| {
            let mut lines = entry.lines();
            let path = lines.next()?.strip_prefix("worktree ")?;
            let mut tree = Worktree {
                path: PathBuf::from(path),
                branch: None,
                locked: false,
                prunable: false,
            };
            for line in lines {
                if let Some(branch) = line.strip_prefix("branch ") {
                    let branch = branch.strip_prefix("refs/heads/").unwrap_or(branch);
                    tree.branch = Some(branch.to_owned());
                } else if line == "locked" || line.starts_with("locked ") {
                    tree.locked = true;
                } else if line == "prunable" || line.starts_with("prunable ") {
                    tree.prunable = true;
                }
            }
            Some(tree)
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrState {
    Open,
    Merged,
    Closed,
}

/// A pull request whose head is a worktree's branch.
#[derive(Debug, PartialEq, Eq)]
struct Pr {
    number: u64,
    state: PrState,
    /// The commit the PR's branch was at, last GitHub saw it.
    head: String,
}

/// Reads `gh pr list --json number,state,headRefOid`.
fn parse_pr_list(json: &str) -> Result<Vec<Pr>> {
    let list: Vec<serde_json::Value> =
        serde_json::from_str(json).context("gh pr list printed something other than a list")?;
    list.iter()
        .map(|pr| {
            let state = match pr["state"].as_str() {
                Some("OPEN") => PrState::Open,
                Some("MERGED") => PrState::Merged,
                Some("CLOSED") => PrState::Closed,
                other => bail!("unexpected PR state {other:?}"),
            };
            Ok(Pr {
                number: pr["number"].as_u64().context("PR without a number")?,
                state,
                head: pr["headRefOid"].as_str().unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Finished: the latest PR on the branch, and how it ended.
    Remove(u64, PrState),
    Keep(String),
}

/// Whether a worktree is finished with, from its branch's PRs, whether it has local changes, and
/// how many of its commits are on neither the remote nor a PR.
fn verdict(prs: &[Pr], dirty: bool, unpushed: usize) -> Verdict {
    let Some(latest) = prs.iter().max_by_key(|pr| pr.number) else {
        return Verdict::Keep("no pull request".into());
    };
    if let Some(open) = prs.iter().find(|pr| pr.state == PrState::Open) {
        return Verdict::Keep(format!("PR #{} is open", open.number));
    }
    if dirty {
        return Verdict::Keep("uncommitted changes".into());
    }
    if unpushed > 0 {
        let s = if unpushed == 1 { "" } else { "s" };
        return Verdict::Keep(format!("{unpushed} commit{s} not pushed"));
    }
    Verdict::Remove(latest.number, latest.state)
}

/// Commits on `dir`'s HEAD that are on no remote branch and in none of `heads`.
fn unpushed_commits(dir: &Path, heads: &[&str]) -> Result<usize> {
    // A merged PR's branch is usually deleted on GitHub, and its head commit may never have been
    // fetched; a head git doesn't have can't vouch for anything, and would fail rev-list.
    let known: Vec<&str> = heads
        .iter()
        .copied()
        .filter(|head| {
            !head.is_empty()
                && capture(
                    dir,
                    "git",
                    &["cat-file", "-e", &format!("{head}^{{commit}}")],
                )
                .is_ok()
        })
        .collect();
    let mut args = vec!["rev-list", "--count", "HEAD", "--not", "--remotes"];
    args.extend(known);
    capture(dir, "git", &args)?
        .parse()
        .context("git rev-list --count printed something other than a number")
}

fn judge(dir: &Path, branch: &str) -> Result<Verdict> {
    let prs = parse_pr_list(&capture(
        dir,
        "gh",
        &[
            "pr",
            "list",
            "--head",
            branch,
            "--state",
            "all",
            "--json",
            "number,state,headRefOid",
        ],
    )?)?;
    let dirty = !capture(dir, "git", &["status", "--porcelain"])?.is_empty();
    let heads: Vec<&str> = prs.iter().map(|pr| pr.head.as_str()).collect();
    Ok(verdict(&prs, dirty, unpushed_commits(dir, &heads)?))
}

pub fn prune_worktrees(args: &[String]) -> Result<()> {
    let mut dry_run = false;
    for arg in args {
        match arg.as_str() {
            "--dry-run" | "-n" => dry_run = true,
            other => bail!("unknown argument {other:?}\n{USAGE}"),
        }
    }
    if std::process::Command::new("gh")
        .arg("--version")
        .output()
        .is_err()
    {
        bail!("the GitHub CLI (`gh`) is required: https://cli.github.com");
    }
    let here = root();
    let primary = primary_checkout(&here)?;
    let review = review_dir_for(&primary);

    // So "on the remote" means what is on GitHub now.
    run(&primary, "git", &["fetch", "--prune", "--quiet", "origin"])?;

    let mut removed = 0;
    for tree in parse_worktree_list(&capture(
        &primary,
        "git",
        &["worktree", "list", "--porcelain"],
    )?) {
        let path = &tree.path;
        let verdict = if *path == primary {
            continue;
        } else if tree.prunable {
            Verdict::Keep("directory is gone; forgotten by `git worktree prune`".into())
        } else if *path == review {
            Verdict::Keep("the review checkout".into())
        } else if here.starts_with(path) {
            Verdict::Keep("this command is running from it".into())
        } else if tree.locked {
            Verdict::Keep("locked".into())
        } else if let Some(branch) = &tree.branch {
            judge(path, branch).unwrap_or_else(|e| Verdict::Keep(format!("couldn't check: {e:#}")))
        } else {
            Verdict::Keep("detached HEAD".into())
        };
        let branch = tree.branch.as_deref().unwrap_or("-");
        match verdict {
            Verdict::Keep(why) => println!("keep    {}  [{branch}]  {why}", path.display()),
            Verdict::Remove(pr, state) => {
                let how = if state == PrState::Merged {
                    "merged"
                } else {
                    "closed"
                };
                let verb = if dry_run { "would remove" } else { "remove" };
                println!("{verb}  {}  [{branch}]  PR #{pr} {how}", path.display());
                if !dry_run {
                    run(
                        &primary,
                        "git",
                        &["worktree", "remove", path.to_str().unwrap()],
                    )?;
                    run(&primary, "git", &["branch", "-D", "--quiet", branch])?;
                }
                removed += 1;
            }
        }
    }
    if !dry_run {
        run(&primary, "git", &["worktree", "prune"])?;
    }
    let s = if removed == 1 { "" } else { "s" };
    if dry_run {
        println!("\n{removed} worktree{s} would be removed. Run without --dry-run to remove them.");
    } else {
        println!("\nremoved {removed} worktree{s}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
