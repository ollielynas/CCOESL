# CCOSEL

A browser-hosted desktop environment: a wasm shell (egui) loads small wasm "apps" from a server
on the local network. `ARCHITECTURE.md` has the design; `README.md` the goals.

## Layout

- `crates/` and `xtask/` — the root cargo workspace (ABI, protocol, SDK, hosts, server, shell).
- `apps/` — a **separate** cargo workspace of guest apps. It has its own `Cargo.lock` and its own
  size-optimised release profile, because `[profile]` is workspace-global. Run cargo for an app
  from inside `apps/`.
- `web/` — the static page that boots the shell.

## Definition of done

`cargo xtask ci` passes. It runs exactly what GitHub Actions runs, one `cargo xtask <name>` per
CI job: `fmt`, `clippy` (`-D warnings`, native and wasm32), `test`, `test-wasm`, `coverage`, and
`build-web` (which also enforces the per-app wire-size budget). Run a single one while iterating.

Needs locally: `cargo-llvm-cov`, `wasm-bindgen-cli` at the version pinned in the root
`Cargo.toml`, and `node`. The Rust toolchain is pinned in `rust-toolchain.toml`.

## Working on a ticket

- Never commit to `main`, never push to it, and **never merge a pull request** — the owner
  merges. Branch off `origin/main` as `agent/<issue>-<slug>`, and open a PR that says
  `Closes #<issue>`.
- Don't weaken a gate to get green: no lowering the coverage bar, no `#[ignore]`, no
  `#[allow(...)]` without a stated reason, no edits to `.github/workflows/`. If a gate looks
  wrong, say so in the PR.
- Review threads are resolved by the owner, not by you: reply to feedback and push fixes, but do
  not resolve the thread. Merging requires every thread resolved.
- `/work-issue <n>` and `/address-feedback <pr>` (in `.claude/commands/`) are the whole loop.

## Testing apps

Every app must reach the line-coverage bar (`APP_COVERAGE_MIN_LINES` in `xtask/src/main.rs`,
currently 70%), measured over that app's own `src/` only. `cargo xtask coverage` finds apps by
scanning `apps/`, so a new app is gated with no CI change.

- Tests go in `src/tests.rs`, declared `#[cfg(test)] mod tests;`. Inline `#[cfg(test)]` blocks
  in an app's source are rejected, because test code is always executed and would inflate the
  number.
- Use `ccosel_sdk::testing::Harness` (enabled by a dev-dependency on `ccosel-sdk` with
  `features = ["testing"]`). It plays the shell and the server: `frame()`, `click("label")`,
  `reply::<ListDir>(&..)`, `fail::<..>(code)`, then assert on `labels()` / `buttons()`. A click
  reaches the app one frame after it happens — that is the real contract, not a harness quirk.
  Working examples: `apps/clock/src/tests.rs`, `apps/file-browser/src/tests.rs`.

## Things that are easy to trip over

- `ccosel-host-web`'s tests are `cfg(target_arch = "wasm32")`. Native `cargo test` reports `0
  passed` for them; they run under node via `cargo xtask test-wasm`.
- The `wasm-bindgen` crate and CLI must match exactly (`=0.2.127` in the root `Cargo.toml`).
- Apps are a recurring download to every client, so avoid float `Display` and other size
  regressions; `build-web` fails an app over 100 KiB gzipped.
- An app is a crate under `apps/` listed in `apps/Cargo.toml`, registered in
  `crates/ccosel-shell/src/registry.rs` and in the `guests` list in `build_web()`.
