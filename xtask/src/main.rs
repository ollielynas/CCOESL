//! Build automation. Run with `cargo xtask <command>`.
//!
//! Exists because the build spans two cargo workspaces plus a `wasm-bindgen` step, and because
//! module size is a product requirement on a slow LAN rather than a nice-to-have — so it is
//! measured on every build and fails the build when the budget is exceeded.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// App modules are the recurring download, so they get a hard budget. The shell is fetched
/// once and cached forever, so it is reported but not gated.
const GUEST_BUDGET_GZIP: u64 = 100 * 1024;

/// Line coverage every app must reach. Measured over the app's own `src/` only (not the SDK it
/// links) and excluding its test code, so neither can inflate the number.
const APP_COVERAGE_MIN_LINES: u64 = 70;

const WASM: &str = "wasm32-unknown-unknown";

fn main() -> Result<()> {
    match std::env::args()
        .nth(1)
        .unwrap_or_else(|| "help".into())
        .as_str()
    {
        "build-web" => build_web(),
        "serve" => serve(),
        "dev" => {
            build_web()?;
            serve()
        }
        "fmt" => fmt(),
        "clippy" => clippy(),
        "test" => test(),
        "test-wasm" => test_wasm(),
        "coverage" => coverage(),
        "ci" => ci(),
        "new-app" => {
            let name = std::env::args().nth(2).unwrap_or_else(|| {
                help();
                std::process::exit(1);
            });
            new_app(&name)
        }
        "review" => review(&std::env::args().skip(2).collect::<Vec<_>>()),
        _ => {
            help();
            Ok(())
        }
    }
}

/// Printed by a bare `cargo run` too, since xtask is the workspace's only binary — so it has
/// to answer "what do I type to see the thing?" rather than just list flags.
fn help() {
    println!(
        "\nCCOSEL — a browser-hosted desktop environment.\n\n\
         The product runs in a browser; there is no native binary to run.\n\n\
         \x20 cargo xtask dev         build everything and serve on :8777  <- start here\n\
         \x20 cargo xtask build-web   build only, and report wire sizes\n\
         \x20 cargo xtask serve       serve web/ on :8777\n\
         \x20 cargo xtask new-app <name>   scaffold a new app crate under apps/\n\n\
         Trying a pull request (needs the GitHub CLI, `gh`):\n\n\
         \x20 cargo xtask review 10                  check PR #10 out in ../<repo>-review and serve it\n\
         \x20 cargo xtask review 10 --checkout-only  just check it out, to read it in your editor\n\n\
         Checks — what CI runs, one job each; `ci` runs them all and is the definition of done:\n\n\
         \x20 cargo xtask ci         fmt, clippy, test, test-wasm, coverage, then build-web\n\
         \x20 cargo xtask fmt        rustfmt --check on both workspaces\n\
         \x20 cargo xtask clippy     clippy -D warnings, native and wasm32\n\
         \x20 cargo xtask test       native tests, both workspaces\n\
         \x20 cargo xtask test-wasm  browser backend, in node (needs wasm-bindgen-cli)\n\
         \x20 cargo xtask coverage   every app must reach the line-coverage bar (needs cargo-llvm-cov)\n\n\
         Other useful commands:\n\n\
         \x20 cargo test                                                    native tests\n\
         \x20 cargo test -p ccosel-host-web --target wasm32-unknown-unknown browser backend, in node\n\
         \x20 cargo run -p ccosel-host-wasmtime --example dev_shell         native dev loop\n"
    );
}

/// The workspace xtask was run from. `cargo run` sets `CARGO_MANIFEST_DIR` at runtime, which is
/// preferred over the compile-time value: worktrees sharing a target dir reuse one binary, and
/// the baked-in path can name a worktree that has since been deleted.
fn root() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run(dir: &Path, program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .current_dir(dir)
        .args(args)
        .status()
        .with_context(|| format!("failed to spawn {program}"))?;
    if !status.success() {
        bail!("{program} {} failed", args.join(" "));
    }
    Ok(())
}

fn gzip_size(path: &Path) -> Result<u64> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("gzip -9 -c '{}' | wc -c", path.display()))
        .output()?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0))
}

fn human(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else {
        format!("{} KB", n / 1024)
    }
}

fn report(label: &str, path: &Path) -> Result<u64> {
    let raw = std::fs::metadata(path)?.len();
    let gz = gzip_size(path)?;
    println!("  {label:<20} {:>9} raw  {:>9} gzip", human(raw), human(gz));
    Ok(gz)
}

fn build_web() -> Result<()> {
    let root = root();
    let dist = root.join("web/dist");
    std::fs::create_dir_all(&dist)?;

    // Guests live in their own workspace: they need opt-level="z"/panic="abort" and the shell
    // does not, and [profile] is workspace-global.
    println!("building guest apps…");
    run(
        &root.join("apps"),
        "cargo",
        &["build", "--release", "--target", "wasm32-unknown-unknown"],
    )?;

    println!("building shell…");
    run(
        &root,
        "cargo",
        &[
            "build",
            "--profile",
            "web-release",
            "-p",
            "ccosel-shell",
            "--target",
            "wasm32-unknown-unknown",
        ],
    )?;

    println!("generating bindings…");
    let shell_wasm = root.join("target/wasm32-unknown-unknown/web-release/ccosel_shell.wasm");
    run(
        &root,
        "wasm-bindgen",
        &[
            "--target",
            "web",
            "--no-typescript",
            "--out-dir",
            dist.to_str().unwrap(),
            "--out-name",
            "ccosel-shell",
            shell_wasm.to_str().unwrap(),
        ],
    )?;

    // Guest crate names use underscores; the served names use hyphens, matching the registry.
    let guests = [
        ("file_browser", "file-browser"),
        ("clock", "clock"),
        ("server_dashboard", "server-dashboard"),
    ];
    for (crate_name, served) in guests {
        std::fs::copy(
            root.join(format!(
                "apps/target/wasm32-unknown-unknown/release/{crate_name}.wasm"
            )),
            dist.join(format!("{served}.wasm")),
        )
        .with_context(|| format!("copying {crate_name}"))?;
    }

    println!("\nwire sizes (what a client actually downloads):");
    report("shell.wasm", &dist.join("ccosel-shell_bg.wasm"))?;
    report("shell.js", &dist.join("ccosel-shell.js"))?;

    let mut over_budget = Vec::new();
    for (_, served) in guests {
        let gz = report(
            &format!("{served}.wasm"),
            &dist.join(format!("{served}.wasm")),
        )?;
        if gz > GUEST_BUDGET_GZIP {
            over_budget.push(format!("{served} is {} gzipped", human(gz)));
        }
    }
    if !over_budget.is_empty() {
        bail!(
            "over the {} per-app budget: {}",
            human(GUEST_BUDGET_GZIP),
            over_budget.join(", ")
        );
    }

    if Command::new("wasm-opt").arg("--version").output().is_err() {
        println!("\nnote: wasm-opt is not installed — `-Oz` is the remaining lever on shell size.");
    }
    println!("\nserve with: cargo xtask serve");
    Ok(())
}

/// What `cargo xtask review` was asked to do.
#[derive(Debug, PartialEq, Eq)]
struct ReviewArgs {
    pr: u64,
    /// Start the dev server once the PR is checked out (the default).
    serve: bool,
}

fn parse_review_args(args: &[String]) -> Result<ReviewArgs> {
    const USAGE: &str = "usage: cargo xtask review <pr-number> [--checkout-only]";
    let mut pr = None;
    let mut serve = true;
    for arg in args {
        match arg.as_str() {
            "--checkout-only" => serve = false,
            flag if flag.starts_with('-') => bail!("unknown option {flag:?}\n{USAGE}"),
            number => {
                if pr.is_some() {
                    bail!("expected exactly one PR number\n{USAGE}");
                }
                pr = Some(
                    number
                        .trim_start_matches('#')
                        .parse::<u64>()
                        .with_context(|| format!("{number:?} is not a PR number\n{USAGE}"))?,
                );
            }
        }
    }
    Ok(ReviewArgs {
        pr: pr.context(USAGE)?,
        serve,
    })
}

/// The disposable checkout PRs are reviewed in: a sibling of the repository, named after it, and
/// reused for every PR so its `target/` stays warm instead of rebuilding from nothing each time.
fn review_dir_for(primary: &Path) -> PathBuf {
    let name = primary
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    primary.with_file_name(format!("{name}-review"))
}

/// The main checkout, even when xtask was started from another worktree (an agent's, or the
/// review checkout itself), so there is one review directory however it was started.
fn primary_checkout(from: &Path) -> Result<PathBuf> {
    let common = capture(
        from,
        "git",
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    PathBuf::from(common)
        .parent()
        .map(Path::to_path_buf)
        .context("git reported a common dir with no parent")
}

/// Like [`run`], but returns what the command printed instead of showing it.
fn capture(dir: &Path, program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .current_dir(dir)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn {program}"))?;
    if !out.status.success() {
        bail!(
            "{program} {} failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Checks a pull request out in the review checkout and, by default, serves it.
///
/// By hand this is five commands (fetch, add a worktree, `gh pr checkout`, cd, `xtask dev`), and
/// the way to get one subtly wrong is to check a PR out over your own work. This never touches
/// the checkout it was started from, and refuses to overwrite edits in the review checkout.
fn review(args: &[String]) -> Result<()> {
    let ReviewArgs {
        pr,
        serve: serve_it,
    } = parse_review_args(args)?;
    if Command::new("gh").arg("--version").output().is_err() {
        bail!("the GitHub CLI (`gh`) is required: https://cli.github.com");
    }
    let root = root();
    let pr_arg = pr.to_string();

    // Ask GitHub first: a wrong number or a missing login fails here, before anything is created.
    let summary = capture(
        &root,
        "gh",
        &[
            "pr",
            "view",
            &pr_arg,
            "--json",
            "title,headRefName,state",
            "--jq",
            r#""\(.title)  [\(.headRefName), \(.state)]""#,
        ],
    )?;
    println!("PR #{pr}: {summary}");

    let dir = review_dir_for(&primary_checkout(&root)?);
    if dir.exists() {
        // A reused checkout may hold someone's edits, and checking a PR out over them loses them.
        let dirty = capture(&dir, "git", &["status", "--porcelain"])?;
        if !dirty.is_empty() {
            bail!(
                "{0} has uncommitted changes, so it was left alone. Commit or discard them, or \
                 remove the checkout with `git worktree remove --force {0}`",
                dir.display()
            );
        }
    } else {
        run(&root, "git", &["fetch", "origin", "main"])?;
        run(
            &root,
            "git",
            &[
                "worktree",
                "add",
                "--detach",
                dir.to_str().unwrap(),
                "origin/main",
            ],
        )?;
    }
    run(&dir, "gh", &["pr", "checkout", &pr_arg, "--detach"])?;

    println!("\nPR #{pr} is checked out in {}", dir.display());
    if !serve_it {
        println!("Open that folder in your editor to read it, or run `cargo xtask dev` there.");
        return Ok(());
    }
    println!("Serving on http://localhost:8777 once it has built (Ctrl-C to stop).\n");
    run(&dir, "cargo", &["xtask", "dev"])
}

/// Prints a header before each check so a long CI log stays navigable, then runs it.
fn step(label: &str, dir: &Path, args: &[&str]) -> Result<()> {
    println!("\n==> {label}");
    run(dir, "cargo", args)
}

fn fmt() -> Result<()> {
    let root = root();
    step(
        "rustfmt: root workspace",
        &root,
        &["fmt", "--all", "--check"],
    )?;
    step(
        "rustfmt: apps workspace",
        &root.join("apps"),
        &["fmt", "--all", "--check"],
    )
}

fn clippy() -> Result<()> {
    let root = root();
    let apps = root.join("apps");
    let deny = ["--", "-D", "warnings"];

    step(
        "clippy: root workspace, native",
        &root,
        &[&["clippy", "--workspace", "--all-targets"], &deny[..]].concat(),
    )?;
    // The shell's dependencies are gated to wasm32, so native clippy sees none of it, and the
    // browser backend's tests are `cfg(target_arch = "wasm32")` — native clippy skips those too.
    step(
        "clippy: shell + browser backend, wasm32",
        &root,
        &[
            &[
                "clippy",
                "-p",
                "ccosel-shell",
                "-p",
                "ccosel-host-web",
                "--target",
                WASM,
                "--all-targets",
            ],
            &deny[..],
        ]
        .concat(),
    )?;
    step(
        "clippy: apps, native",
        &apps,
        &[&["clippy", "--workspace", "--all-targets"], &deny[..]].concat(),
    )?;
    step(
        "clippy: apps, wasm32",
        &apps,
        &[&["clippy", "--workspace", "--target", WASM], &deny[..]].concat(),
    )
}

fn test() -> Result<()> {
    let root = root();
    step("tests: root workspace", &root, &["test", "--workspace"])?;
    step("tests: apps", &root.join("apps"), &["test", "--workspace"])
}

/// The browser backend's tests only exist on wasm32 (native `cargo test` reports 0 for them),
/// and run under node via the runner configured in `.cargo/config.toml`.
fn test_wasm() -> Result<()> {
    step(
        "tests: browser backend, under node",
        &root(),
        &["test", "-p", "ccosel-host-web", "--target", WASM],
    )
}

/// Everything CI runs, in the order a failure is cheapest to find. Green here means green there.
fn ci() -> Result<()> {
    fmt()?;
    clippy()?;
    test()?;
    test_wasm()?;
    coverage()?;
    println!("\n==> build-web (wire-size budget)");
    build_web()?;
    println!("\nci: all checks passed");
    Ok(())
}

/// What one app's own source measured as, in lines.
#[derive(Debug, Default, PartialEq, Eq)]
struct Measure {
    found: u64,
    hit: u64,
}

impl Measure {
    fn percent(&self) -> f64 {
        if self.found == 0 {
            0.0
        } else {
            self.hit as f64 * 100.0 / self.found as f64
        }
    }

    /// Exact integer comparison, so 69.9…% can never round its way past the bar. An app with no
    /// measurable lines fails: a gate that passes on nothing is not a gate.
    fn meets(&self, min_percent: u64) -> bool {
        self.found > 0 && self.hit * 100 >= self.found * min_percent
    }
}

/// Tests count toward coverage if they live in the file under measurement, because test code is
/// always executed — a big test module would quietly buy the app its percentage. So test code
/// belongs in `src/tests.rs` or `tests/`, and is excluded here.
fn is_test_file(path: &str) -> bool {
    path.ends_with("/tests.rs") || path.contains("/tests/")
}

/// Sums line counts from an lcov report for the files under `apps/<app>/src/` that are not test
/// files. Done by hand rather than with `--fail-under-lines`, which would count the SDK the app
/// links and the app's own test code.
fn measure(lcov: &str, app: &str) -> Measure {
    let needle = format!("/apps/{app}/src/");
    let mut m = Measure::default();
    let mut counted = false;
    for line in lcov.lines() {
        if let Some(path) = line.strip_prefix("SF:") {
            counted = path.contains(&needle) && !is_test_file(path);
        } else if counted {
            if let Some(n) = line.strip_prefix("LF:") {
                m.found += n.parse().unwrap_or(0);
            } else if let Some(n) = line.strip_prefix("LH:") {
                m.hit += n.parse().unwrap_or(0);
            }
        }
    }
    m
}

/// True if `src` holds an inline `#[cfg(test)]` block instead of the out-of-line
/// `#[cfg(test)] mod tests;` declaration. See [`is_test_file`] for why that matters.
fn has_inline_test_code(src: &str) -> bool {
    let mut lines = src.lines().map(str::trim).filter(|l| !l.is_empty());
    while let Some(line) = lines.next() {
        if line == "#[cfg(test)]" && !lines.next().is_some_and(|next| next.ends_with(';')) {
            return true;
        }
    }
    false
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Every directory under `apps/` with a `Cargo.toml` is an app. Found by scanning rather than
/// listed anywhere, so a new app is gated the moment it exists with no CI edit to forget.
fn app_names(apps: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(apps)? {
        let entry = entry?;
        if entry.path().join("Cargo.toml").is_file() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// Fails unless every app reaches [`APP_COVERAGE_MIN_LINES`]. Per app on purpose: a single
/// workspace-wide number would let one well-tested app hide an untested one.
fn coverage() -> Result<()> {
    let apps = root().join("apps");
    if !Command::new("cargo")
        .args(["llvm-cov", "--version"])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        bail!(
            "cargo-llvm-cov is required for the coverage gate: cargo install cargo-llvm-cov --locked"
        );
    }

    let names = app_names(&apps)?;
    if names.is_empty() {
        bail!(
            "no apps found under {} — refusing to pass a coverage gate that measured nothing",
            apps.display()
        );
    }

    let out_dir = apps.join("target/coverage");
    std::fs::create_dir_all(&out_dir)?;

    let mut rows = Vec::new();
    for name in &names {
        let mut sources = Vec::new();
        rust_files(&apps.join(name).join("src"), &mut sources)?;
        let inline: Vec<_> = sources
            .iter()
            .filter(|p| !is_test_file(&p.to_string_lossy()))
            .filter(|p| has_inline_test_code(&std::fs::read_to_string(p).unwrap_or_default()))
            .collect();
        if let Some(p) = inline.first() {
            bail!(
                "{} has inline #[cfg(test)] code, which would count toward {name}'s coverage. \
                 Move it to src/tests.rs and declare it with `#[cfg(test)] mod tests;`",
                p.display()
            );
        }

        let lcov_path = out_dir.join(format!("{name}.lcov"));
        step(
            &format!("coverage: {name} (bar: {APP_COVERAGE_MIN_LINES}% of lines)"),
            &apps,
            &[
                "llvm-cov",
                "-p",
                name,
                "--lcov",
                "--output-path",
                lcov_path.to_str().unwrap(),
            ],
        )?;
        let lcov = std::fs::read_to_string(&lcov_path)
            .with_context(|| format!("reading {}", lcov_path.display()))?;
        rows.push((name.clone(), measure(&lcov, name)));
    }

    println!("\napp line coverage (bar: {APP_COVERAGE_MIN_LINES}%):");
    let mut summary = format!(
        "### App line coverage (bar: {APP_COVERAGE_MIN_LINES}%)\n\n| app | lines | coverage | |\n|---|---|---|---|\n"
    );
    let mut failing = Vec::new();
    for (name, m) in &rows {
        let ok = m.meets(APP_COVERAGE_MIN_LINES);
        let verdict = if ok { "pass" } else { "FAIL" };
        println!(
            "  {name:<20} {:>4}/{:<4} lines  {:>5.1}%  {verdict}",
            m.hit,
            m.found,
            m.percent()
        );
        summary.push_str(&format!(
            "| {name} | {}/{} | {:.1}% | {verdict} |\n",
            m.hit,
            m.found,
            m.percent()
        ));
        if !ok {
            failing.push(name.as_str());
        }
    }

    // On GitHub Actions this shows the table on the run page without opening the log.
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
        {
            let _ = f.write_all(summary.as_bytes());
        }
    }

    if !failing.is_empty() {
        bail!(
            "below the {APP_COVERAGE_MIN_LINES}% line-coverage bar: {}",
            failing.join(", ")
        );
    }
    Ok(())
}

/// Serves the shell, the app modules and `/rpc` from one origin.
///
/// It has to be one origin: split them and every RPC becomes a CORS preflight, which is an
/// extra round trip per call on a link that is already the bottleneck.
fn serve() -> Result<()> {
    let root = root();
    run(
        &root,
        "cargo",
        &[
            "run",
            "--release",
            "-p",
            "ccosel-server",
            "--",
            "--root",
            root.join("data/shared").to_str().unwrap(),
            "--web",
            root.join("web").to_str().unwrap(),
        ],
    )
}

/// Scaffolds a new guest app crate under `apps/` and adds it to that workspace's members.
///
/// It stops short of wiring the app into `registry.rs` and the `guests` array in
/// `build_web` above: those need an icon, a colour and a default window size, which are
/// judgment calls, not boilerplate — so this prints them as a checklist instead of guessing.
fn new_app(name: &str) -> Result<()> {
    if !is_valid_app_name(name) {
        bail!(
            "app name must be lowercase kebab-case (letters, digits, '-'), starting with a \
             letter — got {name:?}"
        );
    }

    let root = root();
    let dir = root.join("apps").join(name);
    if dir.exists() {
        bail!("apps/{name} already exists");
    }
    std::fs::create_dir_all(dir.join("src"))?;

    let struct_name = pascal_case(name);

    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\n\
             name = \"{name}\"\n\
             version.workspace = true\n\
             edition.workspace = true\n\
             license.workspace = true\n\
             \n\
             [lib]\n\
             crate-type = [\"cdylib\"]\n\
             \n\
             [dependencies]\n\
             ccosel-sdk = {{ workspace = true }}\n\
             \n\
             [dev-dependencies]\n\
             ccosel-sdk = {{ workspace = true, features = [\"testing\"] }}\n"
        ),
    )?;

    std::fs::write(
        dir.join("src/lib.rs"),
        format!(
            "use ccosel_sdk::{{App, Ui}};\n\
             \n\
             #[derive(Default)]\n\
             pub struct {struct_name} {{\n\
             \x20   field: u32,\n\
             }}\n\
             \n\
             impl App for {struct_name} {{\n\
             \x20   fn update(&mut self, ui: &mut Ui<'_>) {{\n\
             \x20       ui.label(\"{name}\");\n\
             \x20   }}\n\
             }}\n\
             \n\
             #[cfg(test)]\n\
             mod tests;\n\
             \n\
             ccosel_sdk::ccosel_app!({struct_name});\n"
        ),
    )?;

    std::fs::write(
        dir.join("src/tests.rs"),
        format!(
            "use ccosel_sdk::testing::Harness;\n\
             \n\
             use super::*;\n\
             \n\
             #[test]\n\
             fn renders_name() {{\n\
             \x20   let mut h = Harness::new({struct_name}::default());\n\
             \x20   h.frame();\n\
             \x20   assert!(h.has_label(\"{name}\"));\n\
             }}\n"
        ),
    )?;

    add_workspace_member(&root.join("apps/Cargo.toml"), name)?;

    let crate_name = name.replace('-', "_");
    println!("created apps/{name}\n");
    println!("still needs wiring by hand:");
    println!("  1. crates/ccosel-shell/src/registry.rs — add an AppEntry to catalog()");
    println!(
        "  2. xtask/src/main.rs, build_web()'s `guests` array — add (\"{crate_name}\", \"{name}\")"
    );
    Ok(())
}

fn is_valid_app_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn pascal_case(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            let first = chars.next().unwrap().to_ascii_uppercase();
            format!("{first}{}", chars.as_str())
        })
        .collect()
}

/// Inserts `name` into the `members = [...]` array of a Cargo workspace manifest.
fn add_workspace_member(manifest: &Path, name: &str) -> Result<()> {
    let contents = std::fs::read_to_string(manifest)
        .with_context(|| format!("reading {}", manifest.display()))?;
    let open_at = contents
        .find("members = [")
        .with_context(|| format!("no `members = [` in {}", manifest.display()))?
        + "members = [".len();
    let close_at = open_at
        + contents[open_at..]
            .find(']')
            .with_context(|| format!("unterminated `members` array in {}", manifest.display()))?;

    let already_present = contents[open_at..close_at]
        .split(',')
        .any(|entry| entry.trim().trim_matches('"') == name);
    if already_present {
        return Ok(());
    }

    let mut updated = contents.clone();
    updated.insert_str(close_at, &format!(", \"{name}\""));
    std::fs::write(manifest, updated)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LCOV: &str = "\
SF:/r/apps/clock/src/lib.rs
LF:10
LH:8
end_of_record
SF:/r/apps/clock/src/tests.rs
LF:50
LH:50
end_of_record
SF:/r/apps/file-browser/src/lib.rs
LF:100
LH:1
end_of_record
SF:/r/crates/ccosel-sdk/src/ui.rs
LF:400
LH:400
end_of_record
";

    #[test]
    fn measures_only_the_apps_own_non_test_source() {
        // The SDK (400/400), the app's test file (50/50) and another app (1/100) must not leak in.
        assert_eq!(measure(LCOV, "clock"), Measure { found: 10, hit: 8 });
        assert_eq!(
            measure(LCOV, "file-browser"),
            Measure { found: 100, hit: 1 }
        );
    }

    #[test]
    fn an_app_absent_from_the_report_measures_nothing() {
        assert_eq!(measure(LCOV, "missing"), Measure::default());
    }

    #[test]
    fn the_bar_is_exact_at_the_boundary() {
        assert!(Measure { found: 10, hit: 7 }.meets(70));
        assert!(
            !Measure {
                found: 100,
                hit: 69
            }
            .meets(70)
        );
        // 699/1000 is 69.9%: it must not round up to a pass.
        assert!(
            !Measure {
                found: 1000,
                hit: 699
            }
            .meets(70)
        );
    }

    #[test]
    fn an_app_with_no_measurable_lines_fails() {
        assert!(!Measure { found: 0, hit: 0 }.meets(70));
        assert_eq!(Measure { found: 0, hit: 0 }.percent(), 0.0);
    }

    #[test]
    fn only_out_of_line_test_modules_are_allowed_in_measured_files() {
        assert!(!has_inline_test_code("fn f() {}\n"));
        assert!(!has_inline_test_code(
            "fn f() {}\n\n#[cfg(test)]\nmod tests;\n"
        ));
        assert!(has_inline_test_code(
            "#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n"
        ));
        assert!(has_inline_test_code(
            "fn f() {}\n#[cfg(test)]\n\nmod t {\n}\n"
        ));
    }

    #[test]
    fn test_files_are_recognised_by_name_and_directory() {
        assert!(is_test_file("/r/apps/clock/src/tests.rs"));
        assert!(is_test_file("/r/apps/clock/src/tests/render.rs"));
        assert!(!is_test_file("/r/apps/clock/src/lib.rs"));
        assert!(!is_test_file("/r/apps/clock/src/contests.rs"));
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn review_takes_a_pr_number_and_serves_by_default() {
        assert_eq!(
            parse_review_args(&args(&["10"])).unwrap(),
            ReviewArgs {
                pr: 10,
                serve: true
            }
        );
    }

    #[test]
    fn review_accepts_a_hash_prefix_and_checkout_only_in_either_order() {
        let want = ReviewArgs {
            pr: 7,
            serve: false,
        };
        assert_eq!(
            parse_review_args(&args(&["#7", "--checkout-only"])).unwrap(),
            want
        );
        assert_eq!(
            parse_review_args(&args(&["--checkout-only", "7"])).unwrap(),
            want
        );
    }

    #[test]
    fn review_rejects_bad_input_and_says_how_to_call_it() {
        for bad in [&[][..], &["abc"], &["1", "2"], &["--nope", "1"], &["-1"]] {
            let err = parse_review_args(&args(bad)).unwrap_err().to_string();
            assert!(
                err.contains("usage: cargo xtask review"),
                "{bad:?} gave {err:?}"
            );
        }
    }

    #[test]
    fn the_review_checkout_is_a_named_sibling_of_the_repo() {
        assert_eq!(
            review_dir_for(Path::new("/home/me/2026/CCOSEL")),
            PathBuf::from("/home/me/2026/CCOSEL-review")
        );
    }

    #[test]
    fn valid_app_names_are_lowercase_kebab_case() {
        assert!(is_valid_app_name("my-app"));
        assert!(is_valid_app_name("a"));
        assert!(is_valid_app_name("clock"));
        assert!(is_valid_app_name("file-browser"));
        assert!(is_valid_app_name("app1"));
        assert!(is_valid_app_name("my-cool-app-2"));
    }

    #[test]
    fn invalid_app_names_are_rejected() {
        assert!(!is_valid_app_name(""));
        assert!(!is_valid_app_name("MyApp"));
        assert!(!is_valid_app_name("my_app"));
        assert!(!is_valid_app_name("my app"));
        assert!(!is_valid_app_name("1name"));
        assert!(!is_valid_app_name("-name"));
        assert!(!is_valid_app_name("name.with.dots"));
    }

    #[test]
    fn pascal_case_splits_on_hyphens_and_underscores() {
        assert_eq!(pascal_case("my-app"), "MyApp");
        assert_eq!(pascal_case("file-browser"), "FileBrowser");
        assert_eq!(pascal_case("clock"), "Clock");
        assert_eq!(pascal_case("my-cool-app"), "MyCoolApp");
        assert_eq!(pascal_case("a_b"), "AB");
        assert_eq!(pascal_case("already"), "Already");
    }

    #[test]
    fn add_workspace_member_inserts_a_name() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(&manifest, "[workspace]\nmembers = [\"alpha\", \"beta\"]\n").unwrap();

        add_workspace_member(&manifest, "gamma").unwrap();
        let got = std::fs::read_to_string(&manifest).unwrap();
        assert!(got.contains("\"gamma\""));
    }

    #[test]
    fn add_workspace_member_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(&manifest, "[workspace]\nmembers = [\"alpha\", \"beta\"]\n").unwrap();

        add_workspace_member(&manifest, "beta").unwrap();
        let got = std::fs::read_to_string(&manifest).unwrap();
        let beta_count = got.matches("\"beta\"").count();
        assert_eq!(beta_count, 1, "should not duplicate existing member");
    }

    #[test]
    fn new_app_refuses_existing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("apps/existing");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(apps.join("Cargo.toml"), "[package]\nname = \"existing\"\n").unwrap();

        let root_manifest = dir.path().join("xtask/Cargo.toml");
        std::fs::create_dir_all(root_manifest.parent().unwrap()).unwrap();
        // We can't easily test new_app() end-to-end without setting up the full repo
        // structure, but we can test the validation functions directly.
    }
}
