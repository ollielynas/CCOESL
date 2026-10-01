//! Octave sessions, end to end: a process per user, jobs run in order, output and state
//! reported, and a session that dies or hangs replaced rather than wedging its user.
//!
//! CI has no Octave, so these run against `fixtures/octave/mock-octave.sh`, which replays
//! transcripts real Octave produced. `real_octave_matches_the_recordings` runs the same
//! scripts through real Octave when it is installed, so a transcript that no longer matches
//! what Octave prints fails there. `CCOSEL_RECORD_OCTAVE=1` rewrites the transcripts instead.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ccosel_proto::octave::{
    MAX_LIMIT_MS, OctaveAction, OctaveControlReq, OctaveInput, OctavePollReq, OctaveRunReq,
    OctaveStatus,
};
use ccosel_proto::server_error;
use ccosel_server::fs_api::Jail;
use ccosel_server::octave_api::Octave;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/octave")
}

fn fixture(name: &str) -> String {
    fs::read_to_string(fixtures().join(format!("{name}.m"))).unwrap()
}

/// A fresh jail and support folder per test: tests run in parallel in one process.
fn setup(name: &str) -> (Jail, PathBuf) {
    let dir = std::env::temp_dir().join(format!("ccosel-octave-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("jail")).unwrap();
    (Jail::new(dir.join("jail")).unwrap(), dir.join("support"))
}

fn mock(support: PathBuf, timeout: Duration) -> Octave {
    let script = fixtures().join("mock-octave.sh");
    let program = vec!["sh".to_owned(), script.display().to_string()];
    Octave::with_program(Some(program), support, timeout)
}

fn run(octave: &Octave, jail: &Jail, user: Option<&str>, seq: u32, input: OctaveInput<'_>) {
    let req = OctaveRunReq {
        client: 1,
        seq,
        input,
    };
    assert_eq!(octave.run(jail, user, &req), Ok(true));
}

/// Poll job `seq` until it finishes, the way the app does: asking only for output it hasn't
/// had yet. Returns the last status, with `output` holding everything.
fn wait(octave: &Octave, user: Option<&str>, seq: u32) -> OctaveStatus {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut output = String::new();
    loop {
        let req = OctavePollReq {
            client: 1,
            seq,
            from: output.len() as u32,
        };
        let mut status = octave.poll(user, &req).unwrap();
        output.push_str(&status.output);
        assert_eq!(status.next as usize, output.len());
        if status.finished {
            status.output = output;
            return status;
        }
        assert!(Instant::now() < deadline, "job {seq} never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn without_octave_run_says_so() {
    let (jail, support) = setup("without");
    let octave = Octave::with_program(None, support, Duration::from_secs(5));
    assert!(!octave.available());
    let req = OctaveRunReq {
        client: 1,
        seq: 1,
        input: OctaveInput::Code("1"),
    };
    assert_eq!(octave.run(&jail, None, &req), Ok(false));
}

#[test]
fn a_job_reports_output_and_variables() {
    let (jail, support) = setup("assign");
    let octave = mock(support, Duration::from_secs(20));
    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code(&fixture("assign")),
    );
    let status = wait(&octave, None, 1);

    assert!(status.output.contains("x ="), "{}", status.output);
    assert!(
        !status.output.contains("@ccosel"),
        "the report is not output"
    );
    let result = status.result.unwrap();
    assert!(!result.error && !result.ended);
    let x = result.variables.iter().find(|v| v.name == "x").unwrap();
    assert_eq!((x.class.as_str(), x.size.as_str()), ("double", "2x2"));
    assert_eq!(x.value, "[1 2;3 4]");
    let g = result.variables.iter().find(|v| v.name == "g").unwrap();
    assert_eq!(g.attributes, "global");
}

#[test]
fn a_plot_becomes_a_figure() {
    let (jail, support) = setup("plot");
    let octave = mock(support, Duration::from_secs(20));
    run(&octave, &jail, None, 1, OctaveInput::Code(&fixture("plot")));
    let result = wait(&octave, None, 1).result.unwrap();

    assert_eq!(result.figures.len(), 1);
    let fig = &result.figures[0];
    assert_eq!(fig.number, 1);
    assert_eq!(fig.axes.len(), 1, "the legend is not a second set of axes");
    let axes = &fig.axes[0];
    assert_eq!(axes.title, "waves");
    assert_eq!((axes.x_min.as_str(), axes.x_max.as_str()), ("0", "6.2832"));
    assert_eq!((axes.y_min.as_str(), axes.y_max.as_str()), ("-0.9995", "1"));
    let labels: Vec<&str> = axes.series.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, ["sin", "cos"]);
    // sin starts in the middle and cos at the top of the shared range.
    assert!((126..=129).contains(&axes.series[0].samples[0]));
    assert_eq!(axes.series[1].samples[0], 255);
}

#[test]
fn an_error_is_reported_and_the_session_carries_on() {
    let (jail, support) = setup("error");
    let octave = mock(support, Duration::from_secs(20));
    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code(&fixture("error")),
    );
    let status = wait(&octave, None, 1);
    assert!(status.output.contains("before"));
    assert!(status.output.contains("error: 'undefined_thing' undefined"));
    let result = status.result.unwrap();
    assert!(result.error && !result.ended);

    run(
        &octave,
        &jail,
        None,
        2,
        OctaveInput::Code(&fixture("assign")),
    );
    assert!(!wait(&octave, None, 2).result.unwrap().error);
}

#[test]
fn running_a_job_twice_starts_it_once() {
    let (jail, support) = setup("twice");
    let octave = mock(support, Duration::from_secs(20));
    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code(&fixture("assign")),
    );
    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code(&fixture("error")),
    );
    let status = wait(&octave, None, 1);
    assert!(!status.result.unwrap().error, "the second send was ignored");
}

#[test]
fn a_session_that_exits_is_replaced() {
    let (jail, support) = setup("exit");
    let octave = mock(support, Duration::from_secs(20));
    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code("% fixture: exit\nexit"),
    );
    let status = wait(&octave, None, 1);
    let result = status.result.unwrap();
    assert!(result.ended);
    assert!(status.output.contains("Octave has stopped"));

    run(
        &octave,
        &jail,
        None,
        2,
        OctaveInput::Code(&fixture("assign")),
    );
    let result = wait(&octave, None, 2).result.unwrap();
    assert!(!result.error && !result.ended);
}

#[test]
fn a_job_past_the_time_limit_is_stopped() {
    let (jail, support) = setup("hang");
    let octave = mock(support, Duration::from_millis(500));
    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code("% fixture: hang"),
    );
    let status = wait(&octave, None, 1);
    assert!(status.result.unwrap().ended);
    assert!(status.output.contains("time limit"));
}

#[test]
fn restart_reports_the_fresh_session() {
    let (jail, support) = setup("restart");
    let octave = mock(support, Duration::from_secs(20));
    run(&octave, &jail, None, 1, OctaveInput::Restart);
    let result = wait(&octave, None, 1).result.unwrap();
    assert!(!result.error);
    assert!(result.variables.is_empty());
    assert_eq!(result.cwd, "/");
}

#[test]
fn a_signed_in_user_starts_in_their_home_folder() {
    let (jail, support) = setup("home");
    let octave = mock(support, Duration::from_secs(20));
    run(&octave, &jail, Some("alice"), 1, OctaveInput::Restart);
    let result = wait(&octave, Some("alice"), 1).result.unwrap();
    assert_eq!(result.cwd, "/home/alice");
}

#[test]
fn users_cannot_see_each_others_jobs() {
    let (jail, support) = setup("users");
    let octave = mock(support, Duration::from_secs(20));
    run(
        &octave,
        &jail,
        Some("alice"),
        1,
        OctaveInput::Code(&fixture("assign")),
    );
    let req = OctavePollReq {
        client: 1,
        seq: 1,
        from: 0,
    };
    assert_eq!(
        octave.poll(Some("bob"), &req).unwrap_err(),
        server_error::NOT_FOUND
    );
}

#[test]
fn a_file_is_run_only_if_the_caller_may_read_it() {
    let (jail, support) = setup("file");
    let octave = mock(support, Duration::from_secs(20));
    fs::write(jail.root().join("fit.m"), fixture("assign")).unwrap();
    run(&octave, &jail, None, 1, OctaveInput::File("/fit.m"));
    assert!(!wait(&octave, None, 1).result.unwrap().error);

    let req = OctaveRunReq {
        client: 1,
        seq: 2,
        input: OctaveInput::File("/../outside.m"),
    };
    assert!(octave.run(&jail, None, &req).is_err());
}

/// Real Octave, when this machine has it: the recorded transcripts must still be what Octave
/// prints for their scripts. With `CCOSEL_RECORD_OCTAVE=1`, writes them afresh instead.
#[test]
fn real_octave_matches_the_recordings() {
    let Some(octave) = std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join("octave-cli"))
            .find(|p| p.is_file())
    }) else {
        eprintln!("octave-cli is not installed; checked against the recordings only");
        return;
    };
    let record = std::env::var_os("CCOSEL_RECORD_OCTAVE").is_some();
    let dir = std::env::temp_dir().join(format!("ccosel-octave-rec-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("__ccosel_run__.m"),
        include_str!("../src/octave/__ccosel_run__.m"),
    )
    .unwrap();

    for name in ["assign", "plot", "error", "empty"] {
        let script = fixtures().join(format!("{name}.m"));
        let input = format!(
            "more off\naddpath(\"{}\")\nset(0, \"defaultfigurevisible\", \"off\")\n\
             try, close(figure()), end\n\
             __ccosel_run__(\"code\", \"{}\", \"@TAG\")\n",
            dir.display(),
            script.display()
        );
        let out = std::process::Command::new(&octave)
            .args(["--quiet", "--norc", "--no-history", "--no-window-system"])
            .current_dir(&dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(input.as_bytes())?;
                child.wait_with_output()
            })
            .unwrap();
        let got = String::from_utf8(out.stdout).unwrap();
        // The folder it ran in becomes `@CWD`, which the mock fills in with its own.
        let got = got.replace(&format!("\t{}\n", dir.display()), "\t@CWD\n");
        let path = fixtures().join(format!("{name}.out"));
        if record {
            fs::write(&path, &got).unwrap();
        } else {
            assert_eq!(
                got,
                fs::read_to_string(&path).unwrap(),
                "{name}.out no longer matches Octave; re-record with CCOSEL_RECORD_OCTAVE=1"
            );
        }
    }
}

/// Real Octave, when this machine has it: a variable made by one job is there for the next,
/// which is the whole point of a session.
#[test]
fn real_octave_keeps_variables_between_jobs() {
    let (jail, support) = setup("real");
    let octave = Octave::detect(support);
    if !octave.available() {
        eprintln!("octave-cli is not installed; skipped");
        return;
    }
    run(&octave, &jail, None, 1, OctaveInput::Code("a = 6;"));
    let status = wait(&octave, None, 1);
    assert!(!status.result.unwrap().error, "{}", status.output);
    run(&octave, &jail, None, 2, OctaveInput::Code("b = a * 7"));
    let status = wait(&octave, None, 2);
    assert!(status.output.contains("b = 42"), "{}", status.output);
    let result = status.result.unwrap();
    assert_eq!(result.cwd, "/");
    let names: Vec<&str> = result.variables.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
}

fn control(octave: &Octave, seq: u32, action: OctaveAction) -> Result<u64, u32> {
    octave.control(
        None,
        &OctaveControlReq {
            client: 1,
            seq,
            action,
        },
    )
}

/// Poll job `seq` once.
fn status(octave: &Octave, seq: u32) -> OctaveStatus {
    let req = OctavePollReq {
        client: 1,
        seq,
        from: 0,
    };
    octave.poll(None, &req).unwrap()
}

#[test]
fn a_running_job_can_be_stopped_and_its_session_carries_on() {
    let (jail, support) = setup("stop");
    let octave = mock(support, Duration::from_secs(20));
    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code("% fixture: hang"),
    );
    // Give it time to be running, not just queued.
    std::thread::sleep(Duration::from_millis(300));
    assert!(!status(&octave, 1).finished);
    assert_eq!(control(&octave, 1, OctaveAction::Stop), Ok(20_000));

    let status = wait(&octave, None, 1);
    let result = status.result.unwrap();
    assert!(result.error, "a stopped job is an error");
    assert!(!result.ended, "but the session survives it");
    assert!(status.output.contains("Stopped."), "{}", status.output);
    // Well before the limit.
    assert!(status.elapsed_ms < 10_000, "{}", status.elapsed_ms);

    // The same session runs the next job.
    run(
        &octave,
        &jail,
        None,
        2,
        OctaveInput::Code(&fixture("assign")),
    );
    let result = wait(&octave, None, 2).result.unwrap();
    assert!(!result.error && !result.ended);
}

#[test]
fn a_jobs_time_limit_shows_and_can_be_extended() {
    let (jail, support) = setup("extend");
    let octave = mock(support, Duration::from_millis(500));
    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code("% fixture: hang"),
    );
    assert_eq!(status(&octave, 1).limit_ms, 500);
    assert_eq!(
        control(&octave, 1, OctaveAction::Extend { minutes: 1 }),
        Ok(60_500)
    );
    assert_eq!(status(&octave, 1).limit_ms, 60_500);
    // Past the old limit, it is still running.
    std::thread::sleep(Duration::from_millis(1_000));
    assert!(!status(&octave, 1).finished);

    // However much is asked for, it stops at the cap.
    assert_eq!(
        control(&octave, 1, OctaveAction::Extend { minutes: u32::MAX }),
        Ok(MAX_LIMIT_MS)
    );
    control(&octave, 1, OctaveAction::Stop).unwrap();
    assert!(!wait(&octave, None, 1).result.unwrap().ended);
}

#[test]
fn only_a_running_job_of_yours_can_be_changed() {
    let (jail, support) = setup("control");
    let octave = mock(support, Duration::from_secs(20));
    assert_eq!(
        control(&octave, 9, OctaveAction::Stop),
        Err(server_error::NOT_FOUND)
    );
    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code(&fixture("assign")),
    );
    wait(&octave, None, 1);
    // Finished: nothing changes.
    assert_eq!(
        control(&octave, 1, OctaveAction::Extend { minutes: 5 }),
        Ok(20_000)
    );
    // Someone else's job of the same number isn't there for them.
    let req = OctaveControlReq {
        client: 1,
        seq: 1,
        action: OctaveAction::Stop,
    };
    assert_eq!(
        octave.control(Some("mallory"), &req),
        Err(server_error::NOT_FOUND)
    );
}

/// Real Octave, when this machine has it: a script waiting forever (here, for figure windows
/// nobody can close) is interrupted, and what it set before is kept.
#[test]
fn real_octave_stops_a_job_and_keeps_its_variables() {
    let (jail, support) = setup("real-stop");
    let octave = Octave::detect(support);
    if !octave.available() {
        eprintln!("octave-cli is not installed; skipped");
        return;
    }
    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code(
            "a = 5;\nfigure();\ntry\n  while ! isempty(get(0, \"children\"))\n    pause(0.2);\n  endwhile\ncatch\n  disp(\"caught\");\nend_try_catch\nb = 6;",
        ),
    );
    // Real Octave takes a moment to start.
    let start = Instant::now();
    while status(&octave, 1).elapsed_ms < 1_500 && start.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(1_500));
    assert!(
        !status(&octave, 1).finished,
        "{}",
        status(&octave, 1).output
    );
    control(&octave, 1, OctaveAction::Stop).unwrap();

    let status = wait(&octave, None, 1);
    assert!(
        !status.output.contains("caught"),
        "try doesn't catch it: {}",
        status.output
    );
    let result = status.result.unwrap();
    assert!(result.error && !result.ended, "{}", status.output);
    let names: Vec<&str> = result.variables.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["a"], "{}", status.output);
    assert_eq!(result.figures.len(), 1);

    // And the session takes the next command.
    run(&octave, &jail, None, 2, OctaveInput::Code("c = a + 1"));
    let status = wait(&octave, None, 2);
    assert!(status.output.contains("c = 6"), "{}", status.output);
}
