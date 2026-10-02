# Contributing

Everything reaches `main` through a pull request whose checks pass. That holds for everyone:
the owner, outside contributors, and the Claude agents that work tickets.

## The short version

1. Start from a ticket, or open one (see [Tickets](#tickets)).
2. Branch from `main` and make the change **with tests**.
3. Run `cargo xtask ci`. It runs exactly what GitHub runs.
4. Open a pull request that says `Closes #<issue>`.
5. CI must be green and every review conversation resolved. The owner then squash-merges.

## One-time setup

You need `rustup`, `node`, `curl` and Docker (for `cargo xtask test-image`); the rest is two
`cargo install`s.

```sh
cargo install cargo-llvm-cov --locked                       # the per-app coverage gate
cargo install wasm-bindgen-cli --version 0.2.127 --locked   # must match the pin in Cargo.toml
cargo xtask ci                                              # first run also fetches the pinned toolchain
```

The Rust version, `clippy`, `rustfmt`, `llvm-tools` and the `wasm32` target come from
`rust-toolchain.toml`, so `rustup` sets them up on first use. If `wasm-bindgen-cli` and the
`wasm-bindgen` crate disagree on version the build fails outright, so keep them in step.

## Layout

| Path | What it is |
|---|---|
| `crates/`, `xtask/` | The root cargo workspace: ABI, protocol, SDK, hosts, server, shell. |
| `apps/` | A **separate** cargo workspace of guest apps. It has its own `Cargo.lock` and a size-optimised release profile (`[profile]` is workspace-global, so it can't share one). Run cargo for an app from inside `apps/`. |
| `web/` | The static page that boots the shell. |
| `data/shared/` | What `cargo xtask serve` serves as the server's files. `Docs/` in it is the user documentation the Docs app shows. |

`ARCHITECTURE.md` explains the design and `README.md` the goals.

## What CI checks

Each CI job is one `cargo xtask` command. To reproduce a red job, run the same command locally.

| Job | Command | Fails when | Usual fix |
|---|---|---|---|
| `fmt` | `cargo xtask fmt` | Code isn't rustfmt-formatted, in either workspace | `cargo fmt --all`, and again inside `apps/` |
| `clippy` | `cargo xtask clippy` | Any warning (`-D warnings`), native or wasm32 | Fix the lint. Use `#[allow]` only with a reason in a comment |
| `test` | `cargo xtask test` | A native test fails, in either workspace | — |
| `test-wasm` | `cargo xtask test-wasm` | A browser-backend test fails under node | — |
| `apps-coverage` | `cargo xtask coverage` | An app has no documentation page, or is under the coverage bar | See [Adding an app](#adding-an-app) and [Testing apps](#testing-apps) |
| `build-web` | `cargo xtask build-web` | The build breaks, or an app exceeds 100 KiB gzipped | Trim the app; avoid float `Display` |
| `test-image` | `cargo xtask test-image` | The Docker image doesn't build, or a person couldn't use it: sign in through `/idp`, use the apps, build in the Compiler, keep their account and files when the container is replaced | Read the step it names; the container's log is printed below it |
| `ci-ok` | *(GitHub only)* | Any job above did not succeed | It's the one required check |

`cargo xtask ci` runs all of them in order. `cargo xtask` alone lists every command.

`test-image` is the slowest, because it builds the whole image and waits for Keycloak to start
twice. It publishes the container on free loopback ports, so a dev server on 8777 doesn't get in
its way, and it removes the container and volume it made. `cargo xtask test-image --no-build`
reruns the checks against the last image it built.

## Testing apps

Every app must reach the line-coverage bar (`APP_COVERAGE_MIN_LINES` in `xtask/src/main.rs`,
currently **70%**). It is measured **per app**, over that app's own `src/` only, so a
well-tested app cannot hide an untested one and the SDK it links doesn't count.
`cargo xtask coverage` finds apps by scanning `apps/`, so a new app is gated automatically.

- Put tests in `src/tests.rs` and declare them with `#[cfg(test)] mod tests;`. Inline
  `#[cfg(test)]` blocks in an app's source are rejected, because test code always runs and would
  inflate the percentage.
- Test through `ccosel_sdk::testing::Harness`. It plays the shell and the server, so no browser
  is needed: `frame()`, `click("label")`, `reply::<M>(&..)` / `fail::<M>(code)` to answer an RPC,
  then assert on `labels()`, `buttons()` and the app's own state (`h.app`).
- A click reaches the app **one frame after it happens**. That is the real contract of the
  system, not a harness quirk: `click(..)` then `frame()` for the app to see it.

Real examples: `apps/clock/src/tests.rs` (time as an input) and `apps/file-browser/src/tests.rs`
(answering `ListDir`, navigating, filtering).

## Adding an app

1. Run `cargo xtask new-app <name>` (or `make new-app NAME=<name>`). This creates the crate
   under `apps/` and adds it to the workspace members. It prints what still needs wiring by hand.
2. Wire the remaining pieces:
   - Register it in `crates/ccosel-shell/src/registry.rs` (the catalog) and in the `guests`
     list in `build_web()` in `xtask/src/main.rs`. Neither is checked automatically yet.
   - Being in the catalog gives it its own page at `/app/<id>`, with nothing to add on the
     server. List that link in `data/shared/Docs/app-links.md`; `cargo xtask test-wasm` fails
     until you do.
3. **Write its user documentation** in `data/shared/Docs/Apps/<name>.md`, and link it from the
   list in `data/shared/Docs/README.md`. `new-app` creates a stub; replace it. This is what
   people read in the Docs app, so write it for them: what the app is for and how to use each
   part of it, not how it is built. `cargo xtask coverage` fails for an app without a page.
   - These pages are **read-only in the app** (`data/shared/Docs/.access` says `read: *` and
     grants no `write`). They change only by editing the files in the repository, through a
     pull request like any other change. Update the page in the same PR as any change to what
     the app does.
4. Give the app real state to test: clippy rejects `MyApp::default()` on a unit struct.

## Wire IDs

Some numbers are part of the wire ABI: `Method` in `ccosel-proto`, `OpCode` in
`ccosel-abi`. Values already on `main` are frozen — never renumber them. When you introduce a
**new** one, give it a randomly generated value, not the next sequential number. Sequential
ids collide the moment two branches both append; random ones do not.

Generate the value from a real RNG. Do not invent a number that looks random — that is
how parallel agents end up picking the same id:

```sh
python3 -c 'import random; print(random.randint(0, 65535))'   # u16, for Method
python3 -c 'import random; print(random.randint(0, 255))'     # u8, for OpCode
```

or with Node:

```sh
node -e 'console.log(Math.floor(Math.random() * 65536))'
node -e 'console.log(Math.floor(Math.random() * 256))'
```

For `OpCode`, the value must fall outside the reserved ranges listed in
`crates/ccosel-abi/src/opcode.rs`. Re-run the command if it lands inside one.

## Tickets

Work is tracked as GitHub issues. Use one of the task forms:

- **Agent task** for work a Claude agent should do with `/work-issue`.
- **Human task** for work that is too complex for an agent run and must be done manually.

Both forms ask for a goal and acceptance criteria that a test or reviewer can check. Write them so
someone with no context could finish the work.

- **Outside contributors:** pick an open issue, or open one first for anything larger than a
  small fix so the direction is agreed before you build it. Fork, branch, and open a PR.
- **The owner can hand an Agent task ticket to a Claude agent** with `/work-issue <number>`.
  Agents must not start **Human task** tickets. The agent works in its own git worktree on a branch
  named `agent/<number>-<slug>`, runs `cargo xtask ci` until it is green, opens a PR that says
  `Closes #<number>`, and stops. It never merges.
- Feedback goes on the PR as ordinary review comments. For an agent's PR the owner then runs
  `/address-feedback <pr-number>`, and the agent replies to each comment and pushes fixes.

## Pull requests and review

- Keep a PR to one change. Fill in the PR template; it's a checklist of what the checks can't see.
- Branch names are free-form for people. `agent/<number>-<slug>` is reserved for agents.
- **`main` is protected**, for everyone including the owner: changes arrive only by pull
  request; `ci-ok` must pass; the branch must be up to date with `main`; every review
  conversation must be resolved; no force-pushes or deletion. PRs are squash-merged.
- If your branch is behind `main`, either `git merge origin/main` or rebase. Merging works
  everywhere, including for agents, who are not allowed to force-push.
- The owner reviews with comments and resolves the conversations, then merges. GitHub doesn't
  offer Approve on a PR you authored, and agents open their PRs under the owner's account, so for
  agent PRs the review is the comments plus the Merge button.
- From a fork, a maintainer may need to approve your first CI run before it starts.

## Trying a pull request locally

CI shows a change passes; only running it shows what it looks like. With the
[GitHub CLI](https://cli.github.com) (`gh`) installed and logged in:

```sh
cargo xtask review 10                    # check PR #10 out and serve it on http://localhost:8777
cargo xtask review 10 --checkout-only    # just check it out, to read it in your editor
```

- The PR goes in a separate checkout, `../<repo>-review`, so **the checkout you ran it from is
  never touched**.
- That checkout is **thrown away and recreated on every run** (`git worktree remove --force`),
  so nothing done in it persists to the next review: edits, new files, and whatever the served
  PR wrote under `data/` are all gone. Only its build output (`target/` and `apps/target/`) is
  kept, parked in `../<repo>-review-cache` meanwhile, so the first build is slow and later ones
  are not. Don't keep work there.
- Run it from your own checkout, not from inside the review checkout it is about to replace.
- A wrong PR number or a missing `gh` login fails before anything is created.
- Comments, resolving conversations and merging still happen on GitHub; editors such as Zed
  can't review a PR yet. This command is for *running* one.
- Only one server can use port 8777, so stop the previous one (Ctrl-C) before starting another.
- **Only run a PR you've read and trust.** Building it executes its code (build scripts, tests),
  and the dev server listens on all network interfaces, not just localhost. That matters most for
  PRs from forks.

## Troubleshooting

| You see | Cause and fix |
|---|---|
| `cargo-llvm-cov is required for the coverage gate` | `cargo install cargo-llvm-cov --locked` |
| `wasm-bindgen` schema-version mismatch | Reinstall the CLI at the version pinned in the root `Cargo.toml` |
| `ccosel-host-web` or `ccosel-shell` shows `0 passed` under `cargo test` | Expected. Both crates are wasm32-only; run `cargo xtask test-wasm` |
| `has inline #[cfg(test)] code` | Move it to `src/tests.rs` and declare it with `#[cfg(test)] mod tests;` |
| `no user documentation for: <app>` | Write `data/shared/Docs/Apps/<app>.md` (see [Adding an app](#adding-an-app)) |
| An app is under the bar | The table printed by `cargo xtask coverage` shows hits/lines per app; add tests for the branches you haven't covered |
| `use of default to create a unit struct` | Give the app a field, or construct it as `MyApp` rather than `MyApp::default()` |
| `run this from your own checkout` (from `cargo xtask review`) | You ran it inside `../<repo>-review`, which it replaces; run it from your usual checkout |
| `docker is required for test-image` | Install Docker, and make sure your user can run `docker` without `sudo` |
| `login is off` (from `cargo xtask test-image`) | Keycloak didn't start or couldn't be set up in the container. Its reason is in the container log printed after the error |
