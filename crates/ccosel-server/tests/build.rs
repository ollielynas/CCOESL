//! `build_api::compile` starts a real `cargo build` as a job and reports progress, jailed
//! exactly like `ListDir`. These tests invoke the actual toolchain, which is the only way to
//! trust the two things most likely to be silently wrong: artifact discovery, and the promise
//! that polling a running build never starts a second one.

use std::fs;
use std::time::{Duration, Instant};

use ccosel_proto::build::{CompileReq, CompileStatus};
use ccosel_proto::server_error;
use ccosel_server::build_api::{Jobs, compile};
use ccosel_server::fs_api::Jail;

fn temp_root(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ccosel-build-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A zero-dependency binary crate, so the build never touches the network.
fn write_crate(dir: &std::path::Path, name: &str, body: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    )
    .unwrap();
    fs::write(dir.join("src/main.rs"), body).unwrap();
}

/// Polls exactly as the app does, until the job reports itself finished.
fn build_to_completion(jail: &Jail, jobs: &Jobs, path: &str, generation: u32) -> CompileStatus {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let status = compile(jail, jobs, &CompileReq { path, generation }).unwrap();
        if status.finished {
            return status;
        }
        assert!(Instant::now() < deadline, "build never finished");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn builds_a_real_crate_and_reports_its_binary() {
    let root = temp_root("builds_a_real_crate_and_reports_its_binary");
    write_crate(
        &root.join("hello"),
        "hello",
        "fn main() { println!(\"hi\"); }",
    );
    let jail = Jail::new(&root).unwrap();
    let jobs = Jobs::new();

    let status = build_to_completion(&jail, &jobs, "/hello", 1);
    let result = status
        .result
        .as_ref()
        .expect("a finished job carries its result");

    assert!(result.success, "build failed:\n{}", result.output);
    assert_eq!(result.binaries.len(), 1, "output:\n{}", result.output);
    assert_eq!(result.binaries[0].name, "hello");
    assert_eq!(result.binaries[0].path, "/hello/target/release/hello");
    assert!(result.binaries[0].size > 0);

    // The reported path is jail-relative and must resolve back through the jail — that is the
    // property the download route depends on.
    assert!(jail.resolve(&result.binaries[0].path).is_ok());

    // Progress has to have actually moved, or the bar would sit at zero for the whole build.
    assert!(status.units_done >= 1, "no units counted");
    assert_eq!(status.fraction(), 1.0, "a finished job is a full bar");
}

#[test]
fn the_first_call_starts_the_build_without_waiting_for_it() {
    // The whole reason this is a job: the call that starts a build must not block on it, or
    // there is nothing to render a progress bar *from*.
    let root = temp_root("the_first_call_starts_the_build_without_waiting_for_it");
    write_crate(&root.join("slow"), "slow", "fn main() {}");
    let jail = Jail::new(&root).unwrap();
    let jobs = Jobs::new();

    let started = Instant::now();
    let status = compile(
        &jail,
        &jobs,
        &CompileReq {
            path: "/slow",
            generation: 1,
        },
    )
    .unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "compile() waited for the build instead of starting it"
    );
    assert!(
        !status.finished,
        "should report a job in flight, not a result"
    );

    build_to_completion(&jail, &jobs, "/slow", 1);
}

#[test]
fn polling_the_same_generation_never_starts_a_second_build() {
    // `generation` is the idempotency key. If repeating a request re-ran cargo, the app's own
    // poll loop would spawn a build per frame.
    let root = temp_root("polling_the_same_generation_never_starts_a_second_build");
    write_crate(&root.join("once"), "once", "fn main() {}");
    let jail = Jail::new(&root).unwrap();
    let jobs = Jobs::new();

    let first = build_to_completion(&jail, &jobs, "/once", 7);
    let first_elapsed = first.elapsed_ms;

    // Poll well past completion: the job must stay finished rather than restarting.
    for _ in 0..5 {
        let again = compile(
            &jail,
            &jobs,
            &CompileReq {
                path: "/once",
                generation: 7,
            },
        )
        .unwrap();
        assert!(again.finished, "a settled job restarted under polling");
        assert!(
            again.elapsed_ms >= first_elapsed,
            "elapsed went backwards, so this is a different job"
        );
    }

    // A new generation is a new build, and must not disturb the old one's result.
    build_to_completion(&jail, &jobs, "/once", 8);
    let old = compile(
        &jail,
        &jobs,
        &CompileReq {
            path: "/once",
            generation: 7,
        },
    )
    .unwrap();
    assert!(old.finished, "the earlier generation's result was lost");
}

#[test]
fn a_compile_error_is_a_reported_failure_not_an_rpc_error() {
    let root = temp_root("a_compile_error_is_a_reported_failure_not_an_rpc_error");
    write_crate(
        &root.join("broken"),
        "broken",
        "fn main() { this is not rust }",
    );
    let jail = Jail::new(&root).unwrap();
    let jobs = Jobs::new();

    let status = build_to_completion(&jail, &jobs, "/broken", 1);
    let result = status.result.expect("a finished job carries its result");

    assert!(!result.success);
    assert!(result.binaries.is_empty());
    assert!(!result.output.is_empty(), "a failure should explain itself");
}

#[test]
fn refuses_a_directory_with_no_cargo_toml() {
    let root = temp_root("refuses_a_directory_with_no_cargo_toml");
    fs::create_dir_all(root.join("not-a-crate")).unwrap();
    fs::write(root.join("not-a-crate/notes.txt"), b"just a file").unwrap();
    let jail = Jail::new(&root).unwrap();
    let jobs = Jobs::new();

    let err = compile(
        &jail,
        &jobs,
        &CompileReq {
            path: "/not-a-crate",
            generation: 1,
        },
    )
    .unwrap_err();
    assert_eq!(err, server_error::NOT_A_CARGO_PROJECT);
}

#[test]
fn refuses_to_climb_out_of_the_jail() {
    let root = temp_root("refuses_to_climb_out_of_the_jail");
    fs::create_dir_all(root.join("inside")).unwrap();
    let jail = Jail::new(root.join("inside")).unwrap();
    let jobs = Jobs::new();

    let err = compile(
        &jail,
        &jobs,
        &CompileReq {
            path: "/../outside",
            generation: 1,
        },
    )
    .unwrap_err();
    assert!(err == server_error::DENIED || err == server_error::NOT_FOUND);
}

/// What `cargo xtask serve` and `cargo xtask review` actually do: serve `data/shared` from
/// inside this repository, whose own `Cargo.toml` is a workspace. A project put there (by an
/// upload, say) must build as its own package, not be claimed by that workspace. This failed
/// with "current package believes it's in a workspace when it's not" until `data` was added
/// to the workspace's `exclude`.
#[test]
fn builds_a_project_inside_this_repositorys_data_folder() {
    let shared = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/shared");
    fs::create_dir_all(&shared).unwrap();
    let name = format!(".build-test-{}", std::process::id());
    let dir = shared.join(&name);
    let _ = fs::remove_dir_all(&dir);
    write_crate(&dir.join("proj"), "proj", "fn main() {}");

    let jail = Jail::new(&shared).unwrap();
    let status = build_to_completion(&jail, &Jobs::new(), &format!("/{name}/proj"), 1);
    let _ = fs::remove_dir_all(&dir);

    let result = status.result.unwrap();
    assert!(result.success, "build failed:\n{}", result.output);
    assert_eq!(result.binaries.len(), 1);
}

// C and C++. `gcc`, `g++` and `make` are on the CI runner, so these use the real ones.

/// Builds `dir` (jail-relative) to completion and runs the one program it made, returning
/// what the program printed.
fn build_and_run(jail: &Jail, root: &std::path::Path, path: &str) -> (CompileStatus, String) {
    let status = build_to_completion(jail, &Jobs::new(), path, 1);
    let result = status.result.as_ref().unwrap();
    assert!(result.success, "build failed:\n{}", result.output);
    assert_eq!(result.binaries.len(), 1, "output:\n{}", result.output);
    let exe = root.join(result.binaries[0].path.trim_start_matches('/'));
    let out = std::process::Command::new(&exe).output().unwrap();
    assert!(out.status.success());
    (status, String::from_utf8(out.stdout).unwrap())
}

#[test]
fn builds_a_folder_of_c_sources_into_one_program() {
    let root = temp_root("builds_a_folder_of_c_sources_into_one_program");
    let dir = root.join("hello c");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("greet.h"), "const char *greeting(void);\n").unwrap();
    fs::write(
        dir.join("greet.c"),
        "#include \"greet.h\"\nconst char *greeting(void) { return \"hi from c\"; }\n",
    )
    .unwrap();
    fs::write(
        dir.join("main.c"),
        "#include <stdio.h>\n#include <math.h>\n#include \"greet.h\"\n\
         int main(void) { printf(\"%s %d\\n\", greeting(), (int)sqrt(16.0)); return 0; }\n",
    )
    .unwrap();
    let jail = Jail::new(&root).unwrap();

    let (status, printed) = build_and_run(&jail, &root, "/hello c");
    assert_eq!(printed, "hi from c 4\n", "linked with libm");
    let binary = &status.result.as_ref().unwrap().binaries[0];
    assert_eq!(binary.name, "hello_c");
    assert_eq!(binary.path, "/hello c/target/hello_c");
    // Two sources and the link.
    assert_eq!((status.units_done, status.units_total), (3, 3));
}

#[test]
fn builds_c_and_cpp_sources_together_linked_as_cpp() {
    let root = temp_root("builds_c_and_cpp_sources_together_linked_as_cpp");
    let dir = root.join("mixed");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("add.c"),
        "int add(int a, int b) { return a + b; }\n",
    )
    .unwrap();
    fs::write(
        dir.join("main.cpp"),
        "#include <iostream>\n#include <vector>\nextern \"C\" int add(int, int);\n\
         int main() { std::vector<int> v{1, 2}; std::cout << add(v[0], v[1]) << std::endl; }\n",
    )
    .unwrap();
    let jail = Jail::new(&root).unwrap();

    let (_, printed) = build_and_run(&jail, &root, "/mixed");
    assert_eq!(printed, "3\n");
}

#[test]
fn a_c_compile_error_is_reported_with_every_file_tried_and_nothing_linked() {
    let root = temp_root("a_c_compile_error_is_reported_with_every_file_tried");
    let dir = root.join("broken");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("a.c"), "int main(void) { this is not c }\n").unwrap();
    fs::write(dir.join("b.cc"), "int helper() { also not c++ }\n").unwrap();
    let jail = Jail::new(&root).unwrap();

    let status = build_to_completion(&jail, &Jobs::new(), "/broken", 1);
    let result = status.result.unwrap();
    assert!(!result.success);
    assert!(result.binaries.is_empty());
    assert!(result.output.contains("a.c:1"), "{}", result.output);
    assert!(result.output.contains("b.cc:1"), "{}", result.output);
    assert!(!dir.join("target/broken").exists(), "nothing was linked");
}

#[test]
fn builds_a_makefile_project_and_finds_what_it_made() {
    let root = temp_root("builds_a_makefile_project_and_finds_what_it_made");
    let dir = root.join("made");
    fs::create_dir_all(dir.join("tools")).unwrap();
    // An executable that was there before the build is not something it made.
    fs::write(dir.join("tools/old.sh"), "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.join("tools/old.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        dir.join("main.c"),
        "#include <stdio.h>\nint main(void) { puts(\"made\"); return 0; }\n",
    )
    .unwrap();
    // Also a loose source, so this proves the Makefile wins over compiling sources directly.
    fs::write(
        dir.join("Makefile"),
        "bin/app: main.c\n\tmkdir -p bin\n\tgcc -O2 -o bin/app main.c\n",
    )
    .unwrap();
    let jail = Jail::new(&root).unwrap();

    let (status, printed) = build_and_run(&jail, &root, "/made");
    assert_eq!(printed, "made\n");
    let result = status.result.as_ref().unwrap();
    assert_eq!(result.binaries[0].path, "/made/bin/app");
    assert!(
        result.output.contains("gcc -O2"),
        "make's own output is shown"
    );
    assert_eq!(status.units_done, 1, "one compiler run counted");
}

#[test]
fn a_failing_makefile_is_a_reported_failure() {
    let root = temp_root("a_failing_makefile_is_a_reported_failure");
    let dir = root.join("bad");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Makefile"), "all:\n\t@echo going wrong; exit 3\n").unwrap();
    let jail = Jail::new(&root).unwrap();

    let status = build_to_completion(&jail, &Jobs::new(), "/bad", 1);
    let result = status.result.unwrap();
    assert!(!result.success);
    assert!(result.output.contains("going wrong"), "{}", result.output);
}
