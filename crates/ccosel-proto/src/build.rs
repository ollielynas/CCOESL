//! The Compiler app's method: build a jailed project directory, report what it produced.
//!
//! What kind of project a folder is comes from [`detect`], which the server uses to choose how
//! to build it and the app uses to say what it found, so the two can never disagree.
//!
//! A build runs for minutes, so this is **not** a call that waits for one. `Compile` returns a
//! snapshot immediately: the first call for a given `(path, generation)` starts the job, every
//! later call reports how far it has got. The app polls, and `ARCHITECTURE.md`'s "anything over
//! ~2s is a Job" holds without needing the push channel that does not exist yet.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Effect, Method, Query, Rpc};

/// How a folder gets built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectKind {
    /// A `Cargo.toml`: `cargo build --release`.
    Cargo,
    /// A `Makefile` (or `makefile`, `GNUmakefile`) and no `Cargo.toml`: `make`.
    Make,
    /// No build file, but C or C++ sources at the top level: each `.c` through `gcc`, each
    /// C++ file through `g++`, linked into one program.
    Sources,
}

impl ProjectKind {
    /// What the app calls it.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Cargo => "Rust project (Cargo.toml)",
            Self::Make => "C/C++ project (Makefile)",
            Self::Sources => "C/C++ sources",
        }
    }
}

/// The names `make` looks for, in the order it looks.
pub const MAKEFILES: [&str; 3] = ["GNUmakefile", "makefile", "Makefile"];

/// Whether `name` is a C source file.
pub fn is_c_source(name: &str) -> bool {
    name.ends_with(".c")
}

/// Whether `name` is a C++ source file.
pub fn is_cpp_source(name: &str) -> bool {
    [".cpp", ".cc", ".cxx"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// What kind of project a folder holding `files` (the names of the plain files directly in it)
/// is, or `None` if it isn't one. A build file wins over loose sources: a folder with a
/// `Makefile` is built the way its `Makefile` says.
pub fn detect<'a>(files: impl IntoIterator<Item = &'a str>) -> Option<ProjectKind> {
    let mut kind = None;
    for name in files {
        let this = if name == "Cargo.toml" {
            ProjectKind::Cargo
        } else if MAKEFILES.contains(&name) {
            ProjectKind::Make
        } else if is_c_source(name) || is_cpp_source(name) {
            ProjectKind::Sources
        } else {
            continue;
        };
        kind = Some(match (kind, this) {
            (Some(ProjectKind::Cargo), _) | (_, ProjectKind::Cargo) => ProjectKind::Cargo,
            (Some(ProjectKind::Make), _) | (_, ProjectKind::Make) => ProjectKind::Make,
            _ => ProjectKind::Sources,
        });
    }
    kind
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompileReq<'a> {
    /// A jail-relative directory that [`detect`] says is a project, same convention as
    /// `ListDirReq`.
    #[serde(borrow)]
    pub path: &'a str,
    /// Bumped by the app once per Build press. Repeating a `(path, generation)` means "the job
    /// I already started", so polling can never kick off a second build — this is the
    /// idempotency key a job is supposed to carry, expressed through the request cache's key.
    pub generation: u32,
}

/// One executable the build produced. `path` is jail-relative, so the app can turn it
/// straight into a download link without ever seeing a filesystem-absolute path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuiltBinary {
    pub name: String,
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompileResult {
    pub success: bool,
    /// Rendered compiler diagnostics. Tail-capped like a directory listing is entry-capped —
    /// see `output_truncated` — because a guest must never be handed an unbounded reply.
    pub output: String,
    pub output_truncated: bool,
    pub binaries: Vec<BuiltBinary>,
}

/// Where a build has got to. Cheap enough to re-send at a few hertz, which is the whole point.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompileStatus {
    pub finished: bool,
    /// Crates for a Rust build, source files for a C/C++ one, compiler runs seen for `make`.
    pub units_done: u32,
    /// Cargo's resolved package count, or the number of steps (each source, then linking) for
    /// C/C++ sources, or 0 while that is unknown, which it always is for `make`. An estimate on
    /// purpose: cargo does not publish its unit graph on stable, so this is the honest
    /// denominator available rather than a precise one that would need nightly.
    pub units_total: u32,
    /// What the build is working on right now, e.g. `serde v1.0.229` or `main.c`.
    pub current: String,
    pub elapsed_ms: u64,
    /// Present exactly when `finished`.
    pub result: Option<CompileResult>,
}

impl CompileStatus {
    /// Progress in 0..=1, or negative when there is no denominator to divide by. Lives here so
    /// the app and any future progress UI agree on what the numbers mean.
    pub fn fraction(&self) -> f32 {
        if self.finished {
            return 1.0;
        }
        if self.units_total == 0 {
            return -1.0;
        }
        let f = self.units_done as f32 / self.units_total as f32;
        // Never show a full bar before the build says it is done, or the last stretch looks
        // like a hang instead of linking.
        if f > 0.99 { 0.99 } else { f }
    }
}

pub struct Compile;

impl Rpc for Compile {
    const METHOD: Method = Method::Compile;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    // Honest, not a convenience: repeating a `(path, generation)` returns the same job's
    // snapshot and starts nothing, so this really is safe to retry and safe to cache.
    const EFFECT: Effect = Effect::Idempotent;
    // A poll, not a build: the server answers out of its job table without waiting.
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = CompileReq<'a>;
    type Reply = CompileStatus;
}

impl Query for Compile {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_prefers_a_build_file_over_loose_sources() {
        assert_eq!(
            detect(["Cargo.toml", "Makefile", "a.c"]),
            Some(ProjectKind::Cargo)
        );
        assert_eq!(detect(["a.c", "Makefile"]), Some(ProjectKind::Make));
        assert_eq!(detect(["GNUmakefile"]), Some(ProjectKind::Make));
        assert_eq!(detect(["makefile", "a.cpp"]), Some(ProjectKind::Make));
        assert_eq!(detect(["main.c", "util.cc"]), Some(ProjectKind::Sources));
        assert_eq!(detect(["x.cxx"]), Some(ProjectKind::Sources));
        assert_eq!(detect(["README.md", "a.h", "notes.c.txt"]), None);
        assert_eq!(detect([]), None);
    }

    #[test]
    fn source_kinds() {
        assert!(is_c_source("main.c"));
        assert!(!is_c_source("main.cc"));
        assert!(is_cpp_source("main.cc") && is_cpp_source("a.cpp") && is_cpp_source("b.cxx"));
        assert!(!is_cpp_source("main.c") && !is_cpp_source("main.h"));
    }
}
