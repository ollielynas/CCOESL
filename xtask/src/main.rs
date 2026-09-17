//! Build automation. Run with `cargo xtask <command>`.
//!
//! Exists because the build spans two cargo workspaces plus a `wasm-bindgen` step, and because
//! module size is a product requirement on a slow LAN rather than a nice-to-have — so it is
//! measured on every build and fails the build when the budget is exceeded.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// App modules are the recurring download, so they get a hard budget. The shell is fetched
/// once and cached forever, so it is reported but not gated.
const GUEST_BUDGET_GZIP: u64 = 100 * 1024;

fn main() -> Result<()> {
    match std::env::args().nth(1).unwrap_or_else(|| "help".into()).as_str() {
        "build-web" => build_web(),
        "serve" => serve(),
        _ => {
            eprintln!("usage: cargo xtask <build-web|serve>");
            Ok(())
        }
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
    Ok(String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0))
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
            "build", "--profile", "web-release", "-p", "ccosel-shell",
            "--target", "wasm32-unknown-unknown",
        ],
    )?;

    println!("generating bindings…");
    let shell_wasm = root.join("target/wasm32-unknown-unknown/web-release/ccosel_shell.wasm");
    run(
        &root,
        "wasm-bindgen",
        &[
            "--target", "web", "--no-typescript",
            "--out-dir", dist.to_str().unwrap(),
            "--out-name", "ccosel-shell",
            shell_wasm.to_str().unwrap(),
        ],
    )?;

    // Guest crate names use underscores; the served names use hyphens, matching the registry.
    let guests = [("file_browser", "file-browser"), ("clock", "clock")];
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
        let gz = report(&format!("{served}.wasm"), &dist.join(format!("{served}.wasm")))?;
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

fn serve() -> Result<()> {
    println!("http://127.0.0.1:8777/");
    run(
        &root().join("web"),
        "python3",
        &["-m", "http.server", "8777", "--bind", "127.0.0.1"],
    )
}
