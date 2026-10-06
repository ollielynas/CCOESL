use std::os::unix::fs::PermissionsExt;

use super::*;

/// What LilyPond 2.24 prints for a score with a syntax error and no `\version`: progress lines,
/// messages, and the source lines it quotes under each one.
const LOG: &str = "\
GNU LilyPond 2.24.3 (running Guile 2.2)
Processing `score.ly'
Parsing...
score.ly:4:9: error: syntax error, unexpected '}'
  c4 d e
        }
score.ly:1: warning: no \\version statement found, please add

\\version \"2.24.3\"

for future compatibility
/usr/share/lilypond/2.24.3/ly/init.ly:65:2: error: error in #{ ... }#
fatal error: failed files: \"score.ly\"
";

#[test]
fn messages_about_the_source_keep_their_line_and_column() {
    assert_eq!(
        parse_messages(LOG),
        vec![
            ScoreMessage {
                line: 4,
                column: 9,
                error: true,
                text: "syntax error, unexpected '}'".into(),
            },
            ScoreMessage {
                line: 1,
                column: 0,
                error: false,
                text: "no \\version statement found, please add".into(),
            },
            // About LilyPond's own file, so it can't point at a line of the score.
            ScoreMessage {
                line: 0,
                column: 0,
                error: true,
                text: "error in #{ ... }#".into(),
            },
            ScoreMessage {
                line: 0,
                column: 0,
                error: true,
                text: "failed files: \"score.ly\"".into(),
            },
        ]
    );
}

#[test]
fn a_clean_log_has_no_messages() {
    let log = "GNU LilyPond 2.24.3\nProcessing `score.ly'\nParsing...\nInterpreting music...\n\
               Preprocessing graphical objects...\nSuccess: compilation successfully completed\n";
    assert!(parse_messages(log).is_empty());
    // A colon and a number that aren't a source position aren't a message.
    assert!(parse_messages("Layout output to `score.ps': error: no").is_empty());
}

#[test]
fn a_key_is_a_stable_hash_of_the_source() {
    let key = key_of("{ c4 }");
    assert_eq!(key.len(), 32);
    assert!(
        key.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
    assert_eq!(key, key_of("{ c4 }"));
    assert_ne!(key, key_of("{ c4 d }"));
}

#[test]
fn only_the_files_an_engraving_makes_are_served() {
    assert_eq!(served_type("page-1.png"), Some("image/png"));
    assert_eq!(served_type("page-12.png"), Some("image/png"));
    assert_eq!(served_type("score.pdf"), Some("application/pdf"));
    assert_eq!(served_type("score.midi"), Some("audio/midi"));
    for name in [
        "score.ly",
        "page-.png",
        "page-01.png",
        "page-1.png.ly",
        "../score.pdf",
        "page-1/../score.ly",
        "",
    ] {
        assert_eq!(served_type(name), None, "{name:?}");
    }
}

#[test]
fn page_pictures_are_put_in_page_order() {
    let names: Vec<String> = [
        "score-page10.png",
        "score-page2.png",
        "score.ly",
        "score-page1.png",
    ]
    .map(String::from)
    .into();
    assert_eq!(
        page_files(&names),
        ["score-page1.png", "score-page2.png", "score-page10.png"]
    );
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ccosel-score-{name}-{}", std::process::id()))
}

fn temp(name: &str) -> PathBuf {
    let dir = temp_path(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn touch(dir: &Path, names: &[&str]) {
    for name in names {
        std::fs::write(dir.join(name), name.as_bytes()).unwrap();
    }
}

#[test]
fn a_multi_page_result_is_renamed_to_what_it_is_served_as() {
    let dir = temp("multi");
    touch(
        &dir,
        &[
            "score.ly",
            "score-page1.png",
            "score-page2.png",
            "score.pdf",
            "score.midi",
        ],
    );
    assert_eq!(
        collect(&dir),
        Outputs {
            pages: 2,
            pdf: true,
            midi: true
        }
    );
    assert_eq!(
        std::fs::read(dir.join("page-2.png")).unwrap(),
        b"score-page2.png"
    );
    assert!(dir.join("score.pdf").is_file() && dir.join("score.midi").is_file());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_one_page_result_and_numbered_or_old_style_files_are_found_too() {
    let dir = temp("single");
    touch(
        &dir,
        &[
            "score.ly",
            "score.png",
            "score-1.pdf",
            "score-2.pdf",
            "score.mid",
        ],
    );
    assert_eq!(
        collect(&dir),
        Outputs {
            pages: 1,
            pdf: true,
            midi: true
        }
    );
    assert_eq!(std::fs::read(dir.join("page-1.png")).unwrap(), b"score.png");
    assert_eq!(
        std::fs::read(dir.join("score.pdf")).unwrap(),
        b"score-1.pdf"
    );
    assert_eq!(std::fs::read(dir.join("score.midi")).unwrap(), b"score.mid");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn nothing_made_is_nothing_served() {
    let dir = temp("empty");
    touch(&dir, &["score.ly"]);
    assert_eq!(collect(&dir), Outputs::default());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_tail_of_a_long_log_is_kept() {
    assert_eq!(cap_tail("short".into(), 10), ("short".into(), false));
    let (kept, cut) = cap_tail("ab€cd".into(), 3);
    assert!(cut);
    // Cut at a character boundary, never inside the euro sign.
    assert_eq!(kept, "cd");
}

/// A stand-in for `lilypond`: a shell script that writes what LilyPond would, and whose log says
/// what it was asked to do. It runs in the job's directory, as LilyPond would. Returns the
/// directory it is in (`temp_path(name)`) and the script.
fn fake_lilypond(name: &str, body: &str) -> (PathBuf, PathBuf) {
    let dir = temp(name);
    let script = dir.join("lilypond");
    std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, script)
}

fn wait(engraver: &Engraver, source: &str) -> EngraveResult {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let status = engraver.engrave(source).unwrap();
        if let Some(result) = status.result {
            assert!(status.finished);
            return result;
        }
        assert!(Instant::now() < deadline, "the engraving never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_engraving_runs_once_per_source_and_serves_what_it_made() {
    let (dir, script) = fake_lilypond(
        "runs",
        &format!(
            "echo \"$@\" >> {runs}\n\
             printf 'one' > score-page1.png; printf 'two' > score-page2.png; \
             printf 'pdf' > score.pdf\n\
             echo 'score.ly:1: warning: no \\version statement found, please add' >&2",
            runs = temp_path("runs").join("runs").display()
        ),
    );
    let engraver = Engraver::with_program(&script).unwrap();
    let result = wait(&engraver, "{ c4 }");
    assert!(result.success, "{}", result.output);
    assert_eq!(result.key, key_of("{ c4 }"));
    assert_eq!((result.pages, result.pdf, result.midi), (2, true, false));
    assert_eq!(result.messages.len(), 1);
    assert!(!result.messages[0].error);

    // LilyPond was asked for pictures and a PDF of the source, under the name it's served as.
    let runs = std::fs::read_to_string(dir.join("runs")).unwrap();
    assert_eq!(
        runs,
        "--pdf --png -dresolution=110 -dno-point-and-click --output=score score.ly\n"
    );

    let (page, kind) = engraver.file(&result.key, "page-2.png").unwrap();
    assert_eq!(kind, "image/png");
    assert_eq!(std::fs::read(page).unwrap(), b"two");
    assert!(engraver.file(&result.key, "score.pdf").is_some());
    // Not made, not a served name, and not a result there is.
    assert!(engraver.file(&result.key, "score.midi").is_none());
    assert!(engraver.file(&result.key, "score.ly").is_none());
    assert!(engraver.file(&key_of("other"), "page-1.png").is_none());

    // The same text again finds the finished job: LilyPond doesn't run a second time.
    let again = engraver.engrave("{ c4 }").unwrap();
    assert!(again.finished);
    assert_eq!(std::fs::read_to_string(dir.join("runs")).unwrap(), runs);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_error_fails_the_engraving_even_when_pages_were_made() {
    let (dir, script) = fake_lilypond(
        "error",
        "printf 'x' > score.png\necho \"score.ly:2:3: error: unknown escaped string: \\\\foo\" >&2\nexit 1",
    );
    let engraver = Engraver::with_program(&script).unwrap();
    let result = wait(&engraver, "{ \\foo }");
    assert!(!result.success);
    assert_eq!(result.messages[0].line, 2);
    assert_eq!(result.messages[0].column, 3);
    assert!(result.messages[0].error);
    assert!(result.output.contains("unknown escaped string"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_lilypond_is_a_failed_engraving_that_says_so() {
    let engraver = Engraver::with_program("/nonexistent/lilypond").unwrap();
    let result = wait(&engraver, "{ c4 }");
    assert!(!result.success);
    assert_eq!(result.pages, 0);
    assert!(
        result.output.starts_with("could not run LilyPond"),
        "{}",
        result.output
    );
}

#[test]
fn an_oversized_source_is_refused() {
    let engraver = Engraver::with_program("/nonexistent/lilypond").unwrap();
    let big = "c".repeat(MAX_TEXT_BYTES + 1);
    assert_eq!(engraver.engrave(&big).err(), Some(server_error::TOO_LARGE));
}

#[test]
fn only_a_few_engravings_run_at_once() {
    let (dir, script) = fake_lilypond("busy", "sleep 5");
    let engraver = Engraver::with_program(&script).unwrap();
    for i in 0..MAX_RUNNING {
        assert!(engraver.engrave(&format!("{{ c{i} }}")).is_ok());
    }
    assert_eq!(
        engraver.engrave("{ one too many }").err(),
        Some(server_error::LIMIT)
    );
    // Polling one that is already running is still fine.
    assert!(!engraver.engrave("{ c0 }").unwrap().finished);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn results_unused_for_a_while_are_deleted() {
    let (dir, script) = fake_lilypond("sweep", "printf 'x' > score.png");
    let engraver = Engraver::with_program(&script).unwrap();
    let result = wait(&engraver, "{ c4 }");
    assert!(engraver.file(&result.key, "page-1.png").is_some());

    let mut jobs = engraver.jobs.lock().unwrap();
    engraver.sweep(&mut jobs, Instant::now() + RETENTION / 2);
    assert_eq!(jobs.len(), 1, "still in use");
    engraver.sweep(
        &mut jobs,
        Instant::now() + RETENTION + Duration::from_secs(1),
    );
    assert!(jobs.is_empty());
    drop(jobs);
    assert!(!engraver.dir.join(&result.key).exists());
    assert!(engraver.file(&result.key, "page-1.png").is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dropping_the_engraver_deletes_its_results() {
    let engraver = Engraver::with_program("/nonexistent/lilypond").unwrap();
    let dir = engraver.dir.clone();
    assert!(dir.is_dir());
    drop(engraver);
    assert!(!dir.exists());
}
