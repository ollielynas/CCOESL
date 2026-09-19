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
        "dev" => {
            build_web()?;
            serve()
        }
        "new-app" => {
            let name = std::env::args()
                .nth(2)
                .unwrap_or_else(|| { help(); std::process::exit(1) });
            new_app(&name)
        }
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
         \x20 cargo xtask serve       serve web/ on :8777\n\n\
         \x20 cargo xtask new-app <name>   scaffold a new app crate under apps/\n\n\
         Other useful commands:\n\n\
         \x20 cargo test                                                    native tests\n\
         \x20 cargo test -p ccosel-host-web --target wasm32-unknown-unknown browser backend, in node\n\
         \x20 cargo run -p ccosel-host-wasmtime --example dev_shell         native dev loop\n"
    );
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
    let guests = [
        ("file_browser", "file-browser"),
        ("clock", "clock"),
        ("rust_compiler", "rust-compiler"),
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
            "run", "--release", "-p", "ccosel-server", "--",
            "--root", root.join("data/shared").to_str().unwrap(),
            "--web", root.join("web").to_str().unwrap(),
        ],
    )
}

/// Scaffolds a new guest app crate under `apps/` and adds it to that workspace's members.
///
/// It stops short of wiring the app into `registry.rs` and the `guests` array in
/// `build_web` above: those need an icon, a colour and a default window size, which are
/// judgment calls, not boilerplate — so this prints them as a checklist instead of guessing.
fn new_app(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        bail!("app name must be lowercase kebab-case (letters, digits, '-'), starting with a letter — got {name:?}");
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
             ccosel-sdk = {{ workspace = true }}\n"
        ),
    )?;

    std::fs::write(
        dir.join("src/lib.rs"),
        format!(
            "use ccosel_sdk::{{App, Ui}};\n\
             \n\
             #[derive(Default)]\n\
             pub struct {struct_name};\n\
             \n\
             impl App for {struct_name} {{\n\
             \x20   fn update(&mut self, ui: &mut Ui<'_>) {{\n\
             \x20       ui.label(\"{name}\");\n\
             \x20   }}\n\
             }}\n\
             \n\
             ccosel_sdk::ccosel_app!({struct_name});\n"
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
