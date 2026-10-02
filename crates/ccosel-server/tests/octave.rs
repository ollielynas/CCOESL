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
    assert!(fig.changed, "the job drew it");
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

#[test]
fn a_render_job_draws_each_figure_for_its_own_user_only() {
    let (jail, support) = setup("render");
    let octave = mock(support, Duration::from_secs(20));
    run(&octave, &jail, None, 1, OctaveInput::Code(&fixture("plot")));
    let plotted = wait(&octave, None, 1).result.unwrap();
    // An ordinary job draws nothing.
    assert_eq!(plotted.figures[0].image, None);
    assert!(octave.figure(None, 1, 1, 1).is_none());

    run(&octave, &jail, None, 2, OctaveInput::Render);
    let result = wait(&octave, None, 2).result.unwrap();
    assert!(!result.error);
    assert_eq!(result.figures[0].image, Some((4, 3)));
    let png = octave.figure(None, 1, 2, 1).expect("figure 1 was drawn");
    assert_eq!(*png, fs::read(fixtures().join("figure.png")).unwrap());

    // Not another figure, not another job, not another user.
    assert!(octave.figure(None, 1, 2, 2).is_none());
    assert!(octave.figure(None, 2, 2, 1).is_none());
    assert!(octave.figure(Some("mallory"), 1, 2, 1).is_none());
}

#[test]
fn a_png_is_sized_from_its_header() {
    use ccosel_server::octave_api::png_size;
    let png = fs::read(fixtures().join("figure.png")).unwrap();
    assert_eq!(png_size(&png), Some((4, 3)));
    assert_eq!(png_size(b"GIF89a, not a PNG at all"), None);
    assert_eq!(png_size(&png[..20]), None);
}

#[test]
fn figure_urls_name_the_job_and_figure() {
    use ccosel_proto::octave::figure_url;
    assert_eq!(figure_url(9, 2, 1), "/octave/figure/9/2/1.png");
    assert_eq!(
        figure_url(u32::MAX, 0, 10),
        "/octave/figure/4294967295/0/10.png"
    );
}

/// Real Octave, when this machine has it: what the Figures tab shows is Octave's own print of
/// the figure, including what line data can't carry, such as a surface and its colour bar.
#[test]
fn real_octave_renders_a_surface() {
    let (jail, support) = setup("real-render");
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
        OctaveInput::Code("[X, Y] = meshgrid(-3:0.25:3); surf(X, Y, peaks(X, Y)); colorbar;"),
    );
    let status = wait(&octave, None, 1);
    assert!(!status.result.unwrap().error, "{}", status.output);
    run(&octave, &jail, None, 2, OctaveInput::Render);
    let status = wait(&octave, None, 2);
    let result = status.result.unwrap();
    assert!(!result.error, "{}", status.output);
    let (w, h) = result.figures[0].image.expect("figure 1 was drawn");
    assert!(w > 100 && h > 100, "{w}x{h}");
    let png = octave.figure(None, 1, 2, 1).unwrap();
    assert_eq!(ccosel_server::octave_api::png_size(&png), Some((w, h)));
}

/// Real Octave, when this machine has it: `exit` and `quit` would end the session. Each ends
/// the script instead, an error unless the status was 0, and the session and its variables stay.
#[test]
fn real_octave_survives_exit_and_input() {
    let (jail, support) = setup("real-exit");
    let octave = Octave::detect(support);
    if !octave.available() {
        eprintln!("octave-cli is not installed; skipped");
        return;
    }
    for (seq, code, says, error) in [
        // A script that ends with status 0 ended normally.
        (
            1,
            "a = 1;\nexit(0);\nb = 2;",
            "exit(0): the script ends here",
            false,
        ),
        (2, "quit", "quit(0): the script ends here", false),
        (3, "exit(3)", "exit(3): the script ends here", true),
    ] {
        run(&octave, &jail, None, seq, OctaveInput::Code(code));
        let status = wait(&octave, None, seq);
        assert!(status.elapsed_ms < 30_000, "{code}: answered at once");
        assert!(status.output.contains(says), "{code}: {}", status.output);
        assert!(
            !status.output.contains("shadows"),
            "no warning: {}",
            status.output
        );
        assert_eq!(
            status.output.contains("error: "),
            error,
            "{code}: {}",
            status.output
        );
        let result = status.result.unwrap();
        assert_eq!(result.error, error, "{code}: {}", status.output);
        assert!(!result.ended, "{code}: {}", status.output);
        let names: Vec<&str> = result.variables.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["a"], "{code}: the session and `a` survive");
    }
}

/// Real Octave, when this machine has it: which figures a job created or drew into, where a
/// desktop would have popped up a window, whether shown while it ran or at its end; and a figure made visible draws nothing into the
/// output, nor does waiting for its window to close hang.
#[test]
fn real_octave_says_which_figures_a_job_drew() {
    let (jail, support) = setup("real-changed");
    let octave = Octave::detect(support);
    if !octave.available() {
        eprintln!("octave-cli is not installed; skipped");
        return;
    }
    let changed = |octave: &Octave, seq, code| {
        run(octave, &jail, None, seq, OctaveInput::Code(code));
        let status = wait(octave, None, seq);
        let result = status.result.unwrap();
        assert!(!result.error, "{code}: {}", status.output);
        // Shown while it ran (as `figure()` draws the one before), or changed at the end.
        let mut drawn: Vec<u32> = (status.prompts.iter())
            .filter_map(|p| match p.kind {
                ccosel_proto::octave::PromptKind::Figure { number, .. } => Some(number),
                _ => None,
            })
            .chain(
                result
                    .figures
                    .iter()
                    .filter(|f| f.changed)
                    .map(|f| f.number),
            )
            .collect();
        drawn.sort_unstable();
        drawn.dedup();
        (drawn, status.output)
    };
    assert_eq!(
        changed(&octave, 1, "plot(1:3); figure(); plot(3:-1:1);").0,
        [1, 2]
    );
    assert_eq!(
        changed(&octave, 2, "x = 1;").0,
        [] as [u32; 0],
        "nothing drawn"
    );
    assert_eq!(changed(&octave, 3, "figure(1); hold on; plot(2:4);").0, [1]);
    assert_eq!(
        changed(&octave, 4, "print(2, \"two.png\");").0,
        [] as [u32; 0],
        "saving isn't drawing"
    );

    let (drawn, output) = changed(
        &octave,
        5,
        "f = figure(\"visible\", \"on\"); plot(1:10); drawnow; waitfor(f); uiwait(f); disp(\"done\")",
    );
    assert_eq!(drawn, [3]);
    assert!(output.contains("done"), "{output}");
    assert!(output.contains("waitfor doesn't wait here"), "{output}");
    assert!(output.contains("uiwait doesn't wait here"), "{output}");
    // No text-art plot, and none of gnuplot's notes about drawing nothing.
    assert!(
        !output.contains("#####") && !output.contains("|---"),
        "{output}"
    );
    assert!(!output.contains("unknown' terminal"), "{output}");
    // Figures are shown, so as far as a script is concerned there are windows, and a new
    // figure is visible, as it would be on a desktop.
    let (_, output) = changed(
        &octave,
        6,
        "disp(have_window_system()); disp(get(figure(), \"visible\"))",
    );
    assert_eq!(output.trim(), "1\non", "{output}");
}

#[test]
fn only_so_many_renders_are_kept_per_user() {
    let (jail, support) = setup("renders-kept");
    let octave = mock(support, Duration::from_secs(20));
    run(&octave, &jail, None, 1, OctaveInput::Code(&fixture("plot")));
    wait(&octave, None, 1);
    for seq in 2..=70 {
        run(&octave, &jail, None, seq, OctaveInput::Render);
        wait(&octave, None, seq);
    }
    // Starting another job is when old ones are swept.
    run(
        &octave,
        &jail,
        None,
        71,
        OctaveInput::Code(&fixture("assign")),
    );
    wait(&octave, None, 71);
    assert!(octave.figure(None, 1, 2, 1).is_none(), "the oldest went");
    assert!(octave.figure(None, 1, 70, 1).is_some(), "the newest stay");
    let kept = (2..=70)
        .filter(|&seq| octave.figure(None, 1, seq, 1).is_some())
        .count();
    assert_eq!(kept, 64);
}

#[test]
fn a_prompt_line_becomes_a_dialog_for_the_app() {
    use ccosel_proto::octave::{MessageIcon, PromptKind};
    use ccosel_server::octave_api::parse_prompt;
    let root = Path::new("/srv/jail");
    let p = parse_prompt(
        "1\tMESSAGE\terror\tOops\tline one\\nsaid \\\\n\\tthere",
        root,
    )
    .unwrap();
    assert_eq!(p.id, 1);
    assert_eq!(
        p.kind,
        PromptKind::Message {
            icon: MessageIcon::Error,
            title: "Oops".to_owned(),
            text: "line one\nsaid \\n\tthere".to_owned(),
        }
    );
    // A start folder is a jail path for the app, or nothing if it is outside the jail.
    let open = |start| match parse_prompt(&format!("2\tOPENFILE\tPick\t*.m\t{start}"), root)
        .unwrap()
        .kind
    {
        PromptKind::OpenFile { start, .. } => start,
        other => panic!("{other:?}"),
    };
    assert_eq!(open("/srv/jail/home/alice"), "/home/alice");
    assert_eq!(open("/etc"), "");
    assert_eq!(open(""), "");
    assert!(parse_prompt("3\tSOMETHING\tELSE", root).is_none());
    assert!(parse_prompt("x\tMESSAGE", root).is_none());
}

/// Real Octave, when this machine has it: `msgbox` and kin are shown by the app without the
/// job waiting, and `uigetfile` waits for the file the app picks, which Octave gets as a real
/// path, or for Cancel.
#[test]
fn real_octave_asks_the_app_for_dialogs() {
    use ccosel_proto::octave::{MessageIcon, OctaveAnswerReq, PromptAnswer, PromptKind};
    let (jail, support) = setup("real-dialogs");
    let octave = Octave::detect(support);
    if !octave.available() {
        eprintln!("octave-cli is not installed; skipped");
        return;
    }
    fs::write(jail.root().join("data.txt"), "42").unwrap();

    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code(
            "h = msgbox({\"Saved\", \"all of it\"}, \"Done\"); warndlg(\"careful\"); disp(isempty(h))",
        ),
    );
    let shown = wait(&octave, None, 1);
    assert!(!shown.result.unwrap().error, "{}", shown.output);
    assert_eq!(shown.output.trim(), "1");
    let messages: Vec<_> = shown.prompts.iter().map(|p| p.kind.clone()).collect();
    assert_eq!(
        messages,
        [
            PromptKind::Message {
                icon: MessageIcon::None,
                title: "Done".to_owned(),
                text: "Saved\nall of it".to_owned(),
            },
            PromptKind::Message {
                icon: MessageIcon::Warning,
                title: String::new(),
                text: "careful".to_owned(),
            },
        ]
    );

    // Waits for an answer: poll until the picker is asked for.
    let pick = |seq: u32, path: Option<&str>| {
        run(
            &octave,
            &jail,
            None,
            seq,
            OctaveInput::Code(
                "[f, p] = uigetfile(\"*.txt\", \"Pick data\"); if ischar(f), disp(fileread([p f])), else, disp(\"cancelled\"), end",
            ),
        );
        let start = Instant::now();
        let prompt = loop {
            if let Some(p) = status(&octave, seq).prompts.first().cloned() {
                break p;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "never asked");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(
            prompt.kind,
            PromptKind::OpenFile {
                title: "Pick data".to_owned(),
                filter: "*.txt".to_owned(),
                start: String::new(),
            }
        );
        std::thread::sleep(Duration::from_millis(300));
        assert!(!status(&octave, seq).finished, "it waits");
        let answer = OctaveAnswerReq {
            client: 1,
            seq,
            prompt: prompt.id,
            answer: PromptAnswer::File(path),
        };
        octave.answer(&jail, None, &answer).unwrap();
        wait(&octave, None, seq)
    };
    let picked = pick(2, Some("/data.txt"));
    assert_eq!(picked.output.trim(), "42", "{}", picked.output);
    let cancelled = pick(3, None);
    assert_eq!(cancelled.output.trim(), "cancelled", "{}", cancelled.output);

    // Only a file the caller may read, and only for a prompt that waits.
    run(&octave, &jail, None, 4, OctaveInput::Code("uigetfile()"));
    while status(&octave, 4).prompts.is_empty() {
        std::thread::sleep(Duration::from_millis(50));
    }
    let answer = |prompt, path| OctaveAnswerReq {
        client: 1,
        seq: 4,
        prompt,
        answer: PromptAnswer::File(path),
    };
    assert!(
        octave
            .answer(&jail, None, &answer(1, Some("/../../etc/passwd")))
            .is_err()
    );
    assert!(
        octave
            .answer(&jail, None, &answer(1, Some("/nope.txt")))
            .is_err()
    );
    assert_eq!(
        octave.answer(&jail, None, &answer(9, None)),
        Err(server_error::NOT_FOUND)
    );
    octave.answer(&jail, None, &answer(1, None)).unwrap();
    wait(&octave, None, 4);
    // Answering a message, or a finished job, is nothing to answer.
    assert_eq!(
        octave.answer(
            &jail,
            None,
            &OctaveAnswerReq {
                client: 1,
                seq: 1,
                prompt: 1,
                answer: PromptAnswer::File(None),
            }
        ),
        Err(server_error::NOT_FOUND)
    );
}

/// Real Octave, when this machine has it: `input` asks the app, and gets what was typed, as
/// text or evaluated in the script's workspace; `keyboard` runs typed lines until `return`.
#[test]
fn real_octave_asks_the_app_for_input() {
    use ccosel_proto::octave::{OctaveAnswerReq, PromptAnswer, PromptKind};
    let (jail, support) = setup("real-input");
    let octave = Octave::detect(support);
    if !octave.available() {
        eprintln!("octave-cli is not installed; skipped");
        return;
    }
    // Answer each prompt the job asks, in turn, then wait for it to finish.
    let converse = |seq: u32, code: &str, answers: &[&str]| {
        run(&octave, &jail, None, seq, OctaveInput::Code(code));
        let mut prompts = Vec::new();
        for answer in answers {
            let start = Instant::now();
            let prompt = loop {
                let asked = status(&octave, seq).prompts;
                if asked.len() > prompts.len() {
                    break asked[prompts.len()].clone();
                }
                assert!(start.elapsed() < Duration::from_secs(30), "never asked");
                std::thread::sleep(Duration::from_millis(30));
            };
            let req = OctaveAnswerReq {
                client: 1,
                seq,
                prompt: prompt.id,
                answer: PromptAnswer::Text(answer),
            };
            octave.answer(&jail, None, &req).unwrap();
            prompts.push(prompt.kind);
        }
        (wait(&octave, None, seq), prompts)
    };

    let (done, prompts) = converse(
        1,
        "a = input(\"Name? \", \"s\"); n = input(\"How many?\\n> \"); e = input(\"\"); printf(\"%s %d %d\\n\", a, n, isempty(e))",
        &["Ada", "3 * 2", ""],
    );
    assert_eq!(done.output.trim(), "Ada 6 1", "{}", done.output);
    assert_eq!(
        prompts[1],
        PromptKind::Input {
            prompt: "How many?\n> ".to_owned()
        }
    );
    // The answer is evaluated where `input` was called: it sees the script's variables.
    let (done, _) = converse(2, "b = input(\"? \")", &["n + 1"]);
    assert!(done.output.contains("b = 7"), "{}", done.output);
    assert!(!done.result.unwrap().error);

    // keyboard: lines run in the script's workspace until `return`.
    let (done, prompts) = converse(
        3,
        "k = 1; keyboard; disp(k)",
        &["k = k + 41;", "disp(k * 0 + 99)", "return"],
    );
    assert_eq!(
        prompts[0],
        PromptKind::Input {
            prompt: "K>> ".to_owned()
        }
    );
    assert_eq!(done.output.trim(), "99\n42", "{}", done.output);
    let (done, _) = converse(4, "keyboard; disp(\"not here\")", &["dbquit"]);
    assert!(
        done.output.contains("dbquit: the script stopped here"),
        "{}",
        done.output
    );
    assert!(!done.output.contains("not here"));

    // A file answer for an input doesn't fit it.
    run(
        &octave,
        &jail,
        None,
        5,
        OctaveInput::Code("x = input(\"? \")"),
    );
    while status(&octave, 5).prompts.is_empty() {
        std::thread::sleep(Duration::from_millis(30));
    }
    let wrong = OctaveAnswerReq {
        client: 1,
        seq: 5,
        prompt: 1,
        answer: PromptAnswer::File(None),
    };
    assert_eq!(
        octave.answer(&jail, None, &wrong),
        Err(server_error::NOT_FOUND)
    );
    control(&octave, 5, OctaveAction::Stop).unwrap();
    assert!(
        !wait(&octave, None, 5).result.unwrap().ended,
        "Stop works while it waits"
    );
}

/// The owner's script for testing popup windows, as they ran it in the app: it draws a figure,
/// asks whether you can see it, then closes it.
const POPUP_TEST: &str = r#"1;
ASK_USER = true;
function results = run_case(results, name, fn)
  try
    fn();
    results.passed += 1;
    printf("  [PASS] %s\n", name);
  catch err
    results.failed += 1;
    results.failures{end+1} = sprintf("%s: %s", name, err.message);
    printf("  [FAIL] %s\n         %s\n", name, err.message);
  end_try_catch
endfunction
results = struct("passed", 0, "failed", 0, "failures", {{}});
printf("Popup window tests (graphics toolkit: %s)\n\n", graphics_toolkit());
if ! have_window_system()
  printf("  [SKIP] No window system available (headless?). Popups can't be tested.\n");
  exit(0);
endif
fig = figure("name", "Popup test");
results = run_case(results, "figure opens", @() assert(isfigure(fig)));
results = run_case(results, "figure is visible", @() assert(get(fig, "visible"), "on"));
results = run_case(results, "figure is registered with root", @() assert(any(get(0, "children") == fig)));
results = run_case(results, "window title is set", @() assert(get(fig, "name"), "Popup test"));
x = linspace(0, 2*pi, 100);
h = plot(x, sin(x), "linewidth", 2);
title("If you can see this, the popup works");
drawnow;
results = run_case(results, "axes were created", @() assert(isaxes(gca())));
results = run_case(results, "line object was drawn", @() assert(strcmp(get(h, "type"), "line") && numel(get(h, "ydata")) == 100));
set(fig, "position", [100 100 640 480]);
drawnow;
pos = get(fig, "position");
results = run_case(results, "window can be resized", @() assert(pos(3:4), [640 480]));
if ASK_USER
  answer = input("\n  Do you see a window with a sine wave? (y/n): ", "s");
  results = run_case(results, "user confirmed window is visible", @() assert(strcmpi(strtrim(answer), "y"), true));
else
  pause(2);
endif
close(fig);
drawnow;
results = run_case(results, "window closes", @() assert(! isfigure(fig)));
total = results.passed + results.failed;
printf("\n%d/%d popup tests passed.\n", results.passed, total);
if results.failed > 0
  printf("\nFailures:\n");
  printf("  - %s\n", results.failures{:});
  exit(1);
endif
"#;

/// Wait until job `seq` has asked `n` prompts, and return them.
fn prompts_after(octave: &Octave, seq: u32, n: usize) -> Vec<ccosel_proto::octave::OctavePrompt> {
    let start = Instant::now();
    loop {
        let asked = status(octave, seq).prompts;
        if asked.len() >= n {
            return asked;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "asked {asked:?}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// Real Octave, when this machine has it: a figure is shown while the script runs, as a
/// desktop window would show it, not only once it has finished: here, before the question
/// that asks whether you can see it.
#[test]
fn real_octave_shows_figures_while_the_script_runs() {
    use ccosel_proto::octave::{OctaveAnswerReq, PromptAnswer, PromptKind};
    let (jail, support) = setup("real-live");
    let octave = Octave::detect(support);
    if !octave.available() {
        eprintln!("octave-cli is not installed; skipped");
        return;
    }
    run(&octave, &jail, None, 1, OctaveInput::Code(POPUP_TEST));
    let asked = loop {
        let asked = prompts_after(&octave, 1, 1);
        if asked
            .iter()
            .any(|p| matches!(p.kind, PromptKind::Input { .. }))
        {
            break asked;
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    // The figure first, then the question about it.
    let (figure, question) = match (&asked[0].kind, &asked[asked.len() - 1].kind) {
        (PromptKind::Figure { .. }, PromptKind::Input { prompt }) => (&asked[0], prompt),
        other => panic!("{other:?}"),
    };
    assert!(question.contains("Do you see a window"), "{question}");
    let PromptKind::Figure {
        number,
        image,
        width,
        height,
    } = figure.kind
    else {
        unreachable!()
    };
    assert_eq!(number, 1);
    let png = octave.figure(None, 1, 1, image).expect("served");
    assert_eq!(
        ccosel_server::octave_api::png_size(&png),
        Some((width, height))
    );

    let answer = OctaveAnswerReq {
        client: 1,
        seq: 1,
        prompt: asked[asked.len() - 1].id,
        answer: PromptAnswer::Text("y"),
    };
    octave.answer(&jail, None, &answer).unwrap();
    let done = wait(&octave, None, 1);
    assert!(
        done.output.contains("9/9 popup tests passed."),
        "{}",
        done.output
    );
    let result = done.result.unwrap();
    assert!(!result.error, "{}", done.output);
    // It closed the figure, and nothing else was drawn after it was shown.
    assert!(result.figures.iter().all(|f| !f.changed));
}

#[test]
fn real_octave_shows_a_drawing_loop_without_flooding_and_pause_waits_for_enter() {
    use ccosel_proto::octave::{OctaveAnswerReq, PromptAnswer, PromptKind};
    let (jail, support) = setup("real-loop");
    let octave = Octave::detect(support);
    if !octave.available() {
        eprintln!("octave-cli is not installed; skipped");
        return;
    }
    // Twenty frames in well under a second: at most one a second is shown.
    run(
        &octave,
        &jail,
        None,
        1,
        OctaveInput::Code("for k = 1:20, plot(rand(1, 5)); drawnow; end"),
    );
    let done = wait(&octave, None, 1);
    let shown = (done.prompts.iter())
        .filter(|p| matches!(p.kind, PromptKind::Figure { .. }))
        .count();
    assert!((1..=3).contains(&shown), "{shown} shown");

    run(
        &octave,
        &jail,
        None,
        2,
        OctaveInput::Code("pause; disp(\"carried on\")"),
    );
    let asked = prompts_after(&octave, 2, 1);
    assert!(
        matches!(&asked[0].kind, PromptKind::Input { prompt } if prompt.contains("press Enter"))
    );
    let answer = OctaveAnswerReq {
        client: 1,
        seq: 2,
        prompt: asked[0].id,
        answer: PromptAnswer::Text(""),
    };
    octave.answer(&jail, None, &answer).unwrap();
    assert!(wait(&octave, None, 2).output.contains("carried on"));
}

/// Real Octave, when this machine has it: a job that fails says where, for the editor to mark:
/// in the code it ran itself (an empty path), or in a file in the jail.
#[test]
fn real_octave_says_where_an_error_is() {
    let (jail, support) = setup("real-where");
    let octave = Octave::detect(support);
    if !octave.available() {
        eprintln!("octave-cli is not installed; skipped");
        return;
    }
    fs::write(
        jail.root().join("broken.m"),
        "a = 1;\nif a > 0\n  b = (2 + ;\nend\n",
    )
    .unwrap();
    fs::write(
        jail.root().join("helper.m"),
        "function r = helper(x)\n  r = x(10);\nend\n",
    )
    .unwrap();
    let at = |seq, input| {
        run(&octave, &jail, None, seq, input);
        let status = wait(&octave, None, seq);
        let result = status.result.unwrap();
        assert!(result.error, "{}", status.output);
        result.error_at.map(|e| (e.path, e.line, e.column))
    };
    // A runtime error in what was run: its line, and the column Octave gives.
    assert_eq!(
        at(1, OctaveInput::Code("x = 1;\ny = [1 2];\nz = y(5);")),
        Some((String::new(), 3, 1))
    );
    // A syntax error in a file, from its message: line and column.
    assert_eq!(
        at(2, OctaveInput::File("/broken.m")),
        Some(("/broken.m".to_owned(), 3, 12))
    );
    // In a function file, called from what was typed: the innermost of the user's own.
    assert_eq!(
        at(3, OctaveInput::Code("y = helper([1 2]);")),
        Some(("/helper.m".to_owned(), 2, 3))
    );
    // Raised inside one of Octave's own functions: where the user's code called it.
    let inside = at(4, OctaveInput::Code("q = 1;\nstrrep(1, 2, 3)"));
    assert_eq!(inside.map(|(p, l, _)| (p, l)), Some((String::new(), 2)));
    // A success says nothing.
    run(&octave, &jail, None, 5, OctaveInput::Code("ok = 1;"));
    assert!(wait(&octave, None, 5).result.unwrap().error_at.is_none());
}
