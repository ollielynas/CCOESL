//! C and C++ builds, for `build_api`'s job: `make` for a folder with a `Makefile`, or GCC
//! straight over a folder of sources. The job, its time limit and its output cap are
//! `build_api`'s; this is only how each kind runs.
//!
//! Like `cargo build`, this runs whatever the project says: a `Makefile` can run any command.
//! See "building is running untrusted code" in `ARCHITECTURE.md`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use ccosel_proto::build::{CompileResult, is_c_source, is_cpp_source};

use crate::build_api::{BUILD_TIMEOUT, Job, MAX_OUTPUT_BYTES, built_binary, cap_tail, kill};

/// Where a sources build puts its objects and program, inside the project, as cargo does.
const TARGET: &str = "target";

/// How many files `make`'s before/after comparison looks at, so a huge tree can't stall it.
const MAX_SCANNED: usize = 20_000;

/// Run `make` in `dir`. The programs it built are the executable files under `dir` that are
/// new or changed since it started: a `Makefile` names its outputs however it likes.
pub(crate) fn run_make(dir: &Path, jail_root: &Path, job: &Job) -> CompileResult {
    let before = executables(dir);
    let deadline = Instant::now() + BUILD_TIMEOUT;
    let mut make = Command::new("make");
    make.current_dir(dir);
    let run = run_logged(make, deadline, |line| {
        let line = line.trim();
        // Each compiler run is one step done; the last line is what it is doing now.
        let mut progress = job.progress.lock().unwrap();
        if ["gcc ", "g++ ", "cc ", "c++ ", "clang "]
            .iter()
            .any(|c| line.starts_with(c))
        {
            progress.units_done += 1;
        }
        if !line.is_empty() {
            progress.current = shorten(line);
        }
    });
    job.progress.lock().unwrap().current.clear();
    let (success, log) = match run {
        Ok(Some((status, log))) => (status.success(), log),
        Ok(None) => return timed_out(),
        Err(e) => return could_not_run("make", &e),
    };
    let after = executables(dir);
    let mut built: Vec<PathBuf> = after
        .into_iter()
        .filter(|(path, mtime)| before.get(path) != Some(mtime))
        .map(|(path, _)| path)
        .collect();
    built.sort();
    result(success, log, &built, jail_root)
}

/// Compile each C/C++ source directly in `dir` to an object in `target/obj/`, then link them
/// into one program, `target/<folder name>`. Every file is compiled even after one fails, so
/// all the errors show at once; a failure skips linking.
pub(crate) fn run_sources(dir: &Path, jail_root: &Path, job: &Job) -> CompileResult {
    let mut sources: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| is_c_source(n) || is_cpp_source(n))
                .collect()
        })
        .unwrap_or_default();
    sources.sort();
    let objects_dir = dir.join(TARGET).join("obj");
    if let Err(e) = std::fs::create_dir_all(&objects_dir) {
        return could_not_run("the build folder", &e);
    }
    job.progress.lock().unwrap().units_total = sources.len() as u32 + 1;

    let deadline = Instant::now() + BUILD_TIMEOUT;
    let mut log = String::new();
    let mut ok = true;
    let mut objects = Vec::new();
    for source in &sources {
        job.progress.lock().unwrap().current = source.clone();
        let object = objects_dir.join(format!("{source}.o"));
        let mut cc = Command::new(compiler_for(source));
        cc.current_dir(dir)
            .args(["-O2", "-Wall", "-c"])
            .arg(source)
            .arg("-o")
            .arg(&object);
        match run_logged(cc, deadline, |_| {}) {
            Ok(Some((status, out))) => {
                log.push_str(&out);
                ok &= status.success();
            }
            Ok(None) => return timed_out(),
            Err(e) => return could_not_run(compiler_for(source), &e),
        }
        objects.push(object);
        job.progress.lock().unwrap().units_done += 1;
    }

    let program = dir.join(TARGET).join(program_name(dir));
    if ok {
        job.progress.lock().unwrap().current = "linking".to_owned();
        let linker = if sources.iter().any(|s| is_cpp_source(s)) {
            "g++"
        } else {
            "gcc"
        };
        let mut link = Command::new(linker);
        link.current_dir(dir)
            .args(&objects)
            .arg("-o")
            .arg(&program)
            .arg("-lm");
        match run_logged(link, deadline, |_| {}) {
            Ok(Some((status, out))) => {
                log.push_str(&out);
                ok = status.success();
            }
            Ok(None) => return timed_out(),
            Err(e) => return could_not_run(linker, &e),
        }
        job.progress.lock().unwrap().units_done += 1;
    }
    job.progress.lock().unwrap().current.clear();
    let built = if ok { vec![program] } else { Vec::new() };
    result(ok, log, &built, jail_root)
}

/// `gcc` for C, `g++` for C++.
fn compiler_for(source: &str) -> &'static str {
    if is_cpp_source(source) { "g++" } else { "gcc" }
}

/// The program's file name: the project folder's, with anything but letters, digits, `-`,
/// `_` and `.` made `_`.
pub(crate) fn program_name(dir: &Path) -> String {
    let name: String = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() || name.starts_with('.') {
        "program".to_owned()
    } else {
        name
    }
}

/// A status line short enough for the app's one line of "what it's doing".
fn shorten(line: &str) -> String {
    const MAX: usize = 120;
    if line.len() <= MAX {
        return line.to_owned();
    }
    let mut end = MAX;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

fn result(success: bool, log: String, built: &[PathBuf], jail_root: &Path) -> CompileResult {
    let (output, output_truncated) = cap_tail(log, MAX_OUTPUT_BYTES);
    CompileResult {
        success,
        output,
        output_truncated,
        binaries: built
            .iter()
            .filter_map(|p| built_binary(jail_root, p))
            .collect(),
    }
}

fn timed_out() -> CompileResult {
    CompileResult {
        success: false,
        output: "build exceeded the server's time limit and was stopped".to_owned(),
        output_truncated: false,
        binaries: Vec::new(),
    }
}

fn could_not_run(what: &str, e: &std::io::Error) -> CompileResult {
    CompileResult {
        success: false,
        output: format!("could not run {what}: {e}"),
        output_truncated: false,
        binaries: Vec::new(),
    }
}

/// Run `cmd` to completion or `deadline`, with stdout and stderr gathered into one log in the
/// order lines arrive, and each line handed to `on_line` as it does. `Ok(None)` means the
/// deadline hit and it was killed.
fn run_logged(
    mut cmd: Command,
    deadline: Instant,
    on_line: impl Fn(&str) + Sync,
) -> std::io::Result<Option<(ExitStatus, String)>> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let log = Mutex::new(String::new());
    let drain = |pipe: Box<dyn Read + Send>| {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            on_line(&line);
            let mut log = log.lock().unwrap();
            log.push_str(&line);
            log.push('\n');
        }
    };
    let status = std::thread::scope(|scope| {
        scope.spawn(|| drain(Box::new(stdout)));
        scope.spawn(|| drain(Box::new(stderr)));
        loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(Some(status)),
                Ok(None) => {}
                Err(e) => break Err(e),
            }
            if Instant::now() >= deadline {
                kill(&mut child);
                break Ok(None);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })?;
    Ok(status.map(|s| (s, log.into_inner().unwrap())))
}

/// Every executable file under `dir`, with when it last changed. `.git` is skipped, and links
/// are not followed, so nothing outside the project is looked at.
fn executables(dir: &Path) -> HashMap<PathBuf, SystemTime> {
    let mut out = HashMap::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut seen = 0;
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in rd.filter_map(Result::ok) {
            seen += 1;
            if seen > MAX_SCANNED {
                return out;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let path = entry.path();
            if meta.is_dir() {
                if entry.file_name() != ".git" {
                    stack.push(path);
                }
            } else if meta.is_file()
                && is_executable(&path, &meta)
                && let Ok(mtime) = meta.modified()
            {
                out.insert(path, mtime);
            }
        }
    }
    out
}

#[cfg(unix)]
fn is_executable(_: &Path, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(path: &Path, _: &std::fs::Metadata) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}
