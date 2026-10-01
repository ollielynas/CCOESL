use ccosel_proto::fs::{DirEntry, DirListing, FileText};
use ccosel_proto::octave::{Series, Variable};
use ccosel_sdk::testing::{Harness, rpc_error};

use ccosel_sdk::icons;

use super::*;

fn var(name: &str, value: &str) -> Variable {
    Variable {
        name: name.to_owned(),
        class: "double".to_owned(),
        size: "1x1".to_owned(),
        value: value.to_owned(),
        attributes: String::new(),
    }
}

fn session(vars: Vec<Variable>, figures: Vec<Figure>) -> OctaveResult {
    OctaveResult {
        cwd: "/home/alice".to_owned(),
        variables: vars,
        figures,
        ..OctaveResult::default()
    }
}

fn done(output: &str, result: OctaveResult) -> OctaveStatus {
    OctaveStatus {
        finished: true,
        output: output.to_owned(),
        next: output.len() as u32,
        result: Some(result),
        ..OctaveStatus::default()
    }
}

/// An app with the prompt holding `code`, Run pressed, and the server having taken the job.
fn running(code: &str) -> Harness<Octave> {
    let mut h = Harness::new(Octave::default());
    h.frame();
    h.type_text(1, code);
    h.frame();
    h.click(&label(icons::PLAY, "Run"));
    h.frame();
    assert_eq!(h.outstanding::<OctaveRun>(), 1);
    h.reply::<OctaveRun>(&true);
    h.frame();
    h
}

#[test]
fn starts_with_an_empty_session() {
    let mut h = Harness::new(Octave::default());
    h.frame();
    assert!(h.has_text("Workspace"));
    assert!(h.has_text("No variables yet"));
    assert!(h.has_button(&label(icons::PLAY, "Run")));
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        ccosel_sdk::REPAINT_ON_INPUT_ONLY
    );
}

#[test]
fn a_command_runs_and_its_output_and_workspace_arrive() {
    let mut h = running("x = 3");
    assert_eq!(h.app.history, ["x = 3"]);
    assert!(h.has_text(">> x = 3"));
    assert!(h.has_label(&label(icons::HOURGLASS, "Running…")));
    assert_eq!(h.app.wants_repaint_after_ms(), POLL_MS);
    assert_eq!(h.outstanding::<OctavePoll>(), 1);

    // Part of a line first: it waits for the rest.
    h.reply::<OctavePoll>(&OctaveStatus {
        output: "x = ".to_owned(),
        next: 4,
        ..OctaveStatus::default()
    });
    h.frame();
    assert!(!h.has_text("x = "));
    h.frame();
    assert_eq!(h.outstanding::<OctavePoll>(), 1, "polls again");

    h.reply::<OctavePoll>(&done("3\n", session(vec![var("x", "3")], vec![])));
    h.frame();
    h.frame();
    assert!(h.has_text("x = 3"));
    assert!(h.app.job.is_none());
    assert!(h.has_label("/home/alice"));
    assert!(h.selectables().iter().any(|(t, _)| t == "x  1x1 double"));
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        ccosel_sdk::REPAINT_ON_INPUT_ONLY
    );
}

#[test]
fn errors_are_marked_as_errors() {
    let mut h = running("oops");
    let mut result = session(vec![], vec![]);
    result.error = true;
    h.reply::<OctavePoll>(&done("error: 'oops' undefined\n", result));
    h.frame();
    assert_eq!(
        h.app.log.last(),
        Some(&(LineKind::Error, "error: 'oops' undefined".to_owned()))
    );
}

#[test]
fn a_blank_prompt_runs_nothing() {
    let mut h = Harness::new(Octave::default());
    h.frame();
    h.click(&label(icons::PLAY, "Run"));
    h.frame();
    assert_eq!(h.outstanding::<OctaveRun>(), 0);
}

#[test]
fn one_job_at_a_time() {
    let mut h = running("pause(5)");
    h.type_text(1, "y = 1");
    h.frame();
    h.click(&label(icons::PLAY, "Run"));
    h.frame();
    assert_eq!(h.outstanding::<OctaveRun>(), 0);
    assert_eq!(h.app.history, ["pause(5)"]);
}

#[test]
fn without_octave_the_app_says_so() {
    let mut h = Harness::new(Octave::default());
    h.frame();
    h.type_text(1, "1");
    h.frame();
    h.click(&label(icons::PLAY, "Run"));
    h.frame();
    h.reply::<OctaveRun>(&false);
    h.frame();
    assert!(h.app.unavailable);
    assert!(h.has_text("Octave isn't installed on this server."));
}

#[test]
fn a_run_that_fails_is_reported() {
    let mut h = Harness::new(Octave::default());
    h.frame();
    h.type_text(1, "1");
    h.frame();
    h.click(&label(icons::PLAY, "Run"));
    h.frame();
    h.fail::<OctaveRun>(rpc_error::TRANSPORT);
    h.frame();
    assert!(h.app.job.is_none());
    assert!(
        h.app
            .log
            .last()
            .unwrap()
            .1
            .starts_with("error: could not run it")
    );
}

#[test]
fn a_poll_that_fails_is_reported() {
    let mut h = running("1");
    h.fail::<OctavePoll>(rpc_error::SERVER);
    h.frame();
    assert!(h.app.job.is_none());
    assert!(h.app.log.last().unwrap().1.starts_with("error: lost track"));
}

#[test]
fn truncated_output_is_flagged() {
    let mut h = running("1");
    let mut status = done("", session(vec![], vec![]));
    status.output_truncated = true;
    h.reply::<OctavePoll>(&status);
    h.frame();
    assert!(h.app.log.last().unwrap().1.contains("limit"));
}

#[test]
fn history_recalls_older_and_newer_commands() {
    let mut h = Harness::new(Octave::default());
    h.app.history = vec!["a = 1".to_owned(), "b = 2".to_owned()];
    h.frame();
    h.click(icons::CARET_UP);
    h.frame();
    assert_eq!(h.app.input.as_str(), "b = 2");
    h.click(icons::CARET_UP);
    h.frame();
    h.click(icons::CARET_UP);
    h.frame();
    assert_eq!(h.app.input.as_str(), "a = 1", "stops at the oldest");
    h.click(icons::CARET_DOWN);
    h.frame();
    assert_eq!(h.app.input.as_str(), "b = 2");
    h.click(icons::CARET_DOWN);
    h.frame();
    assert_eq!(
        h.app.input.as_str(),
        "",
        "past the newest is a blank prompt"
    );
}

#[test]
fn history_can_be_searched_recalled_and_rerun() {
    let mut h = Harness::new(Octave::default());
    h.app.history = vec!["alpha = 1".to_owned(), "beta = 2".to_owned()];
    h.frame();
    h.type_text(0, "BET");
    h.frame();
    let rows: Vec<String> = h.selectables().into_iter().map(|(t, _)| t).collect();
    assert!(rows.contains(&"beta = 2".to_owned()));
    assert!(!rows.contains(&"alpha = 1".to_owned()));

    h.click("beta = 2");
    h.frame();
    assert_eq!(h.app.input.as_str(), "beta = 2");

    h.click(icons::PLAY);
    h.frame();
    assert_eq!(h.outstanding::<OctaveRun>(), 1);
    assert_eq!(
        h.app.history.len(),
        2,
        "the same command twice is one entry"
    );
}

#[test]
fn an_empty_history_says_so() {
    let mut h = Harness::new(Octave::default());
    h.frame();
    assert!(h.has_text("Nothing yet"));
}

#[test]
fn create_script_puts_the_history_in_the_editor() {
    let mut h = Harness::new(Octave::default());
    h.app.history = vec!["a = 1".to_owned(), "b = a + 1".to_owned()];
    h.frame();
    h.click(&label(icons::NOTE_PENCIL, "Create script"));
    h.frame();
    assert_eq!(h.app.tab, Tab::Editor);
    assert_eq!(h.app.editor.as_str(), "a = 1\nb = a + 1\n");
    h.frame();
    assert!(h.has_text("A new script from the history. Save it to keep it."));
}

#[test]
fn clear_empties_the_command_window() {
    let mut h = Harness::new(Octave::default());
    h.app.log.push((LineKind::Output, "old".to_owned()));
    h.frame();
    h.click(&label(icons::BROOM, "Clear"));
    h.frame();
    assert!(h.app.log.is_empty());
}

#[test]
fn restart_clears_the_session() {
    let mut h = Harness::new(Octave::default());
    h.app.session = session(vec![var("x", "1")], vec![]);
    h.app.inspected = Some("x".to_owned());
    h.frame();
    h.click(&label(icons::ARROW_CLOCKWISE, "Restart"));
    h.frame();
    assert_eq!(h.outstanding::<OctaveRun>(), 1);
    assert!(h.app.inspected.is_none());
    h.reply::<OctaveRun>(&true);
    h.frame();
    h.reply::<OctavePoll>(&done("", session(vec![], vec![])));
    h.frame();
    h.frame();
    assert!(h.has_text("No variables yet"));
}

#[test]
fn picking_a_variable_shows_all_of_it() {
    let mut h = Harness::new(Octave::default());
    let mut m = var("m", "");
    m.size = "3x3".to_owned();
    m.attributes = "global".to_owned();
    h.app.session = session(vec![m.clone()], vec![]);
    h.frame();
    h.click("m  3x3 double");
    h.frame();
    assert_eq!(h.app.tab, Tab::Variable);
    assert_eq!(h.outstanding::<OctaveRun>(), 1);
    h.reply::<OctaveRun>(&true);
    h.frame();
    h.reply::<OctavePoll>(&done(
        "   1   2   3\n   4   5   6\n",
        session(vec![m], vec![]),
    ));
    h.frame();
    h.frame();
    assert!(h.has_text("   4   5   6"));
    assert!(h.has_label("3x3 double, global"));
    // Displaying it is not something the user typed.
    assert!(h.app.history.is_empty());
    assert!(!h.app.log.iter().any(|(_, l)| l.contains("4   5")));
}

#[test]
fn a_strange_name_is_never_sent_as_code() {
    let mut h = Harness::new(Octave::default());
    h.app.session = session(vec![var("x); system(\"rm", "")], vec![]);
    h.frame();
    h.click("x); system(\"rm  1x1 double");
    h.frame();
    assert_eq!(h.outstanding::<OctaveRun>(), 0);
    assert!(identifier("_ok1").is_some());
    assert!(identifier("1bad").is_none());
    assert!(identifier("").is_none());
}

#[test]
fn the_variable_view_explains_itself() {
    let mut h = Harness::new(Octave::default());
    h.frame();
    h.click("Variable");
    h.frame();
    h.frame();
    assert!(h.has_text("Pick a variable in the workspace to see all of it here."));
    h.app.inspected = Some("gone".to_owned());
    h.frame();
    assert!(h.has_text("(no longer in the workspace)"));
}

#[test]
fn values_and_caps_show_in_the_workspace() {
    let mut h = Harness::new(Octave::default());
    let mut r = session(vec![var("v", "[1 2 3]")], vec![]);
    r.variables_truncated = true;
    h.app.session = r;
    h.frame();
    assert!(h.has_text("[1 2 3]"));
    assert!(h.has_text("(more variables than shown)"));
}

fn waves() -> Figure {
    Figure {
        number: 2,
        name: "demo".to_owned(),
        axes: vec![
            Axes {
                title: "waves".to_owned(),
                x_min: "0".to_owned(),
                x_max: "6.2832".to_owned(),
                y_min: "-1".to_owned(),
                y_max: "1".to_owned(),
                series: vec![Series {
                    label: "sin".to_owned(),
                    samples: vec![128, 255, 128, 0],
                }],
            },
            Axes {
                title: String::new(),
                x_min: "0".to_owned(),
                x_max: "0".to_owned(),
                y_min: "0".to_owned(),
                y_max: "0".to_owned(),
                series: vec![],
            },
        ],
    }
}

#[test]
fn figures_are_drawn_and_can_be_saved() {
    let mut h = Harness::new(Octave::default());
    h.app.session = session(
        vec![],
        vec![
            waves(),
            Figure {
                number: 3,
                name: String::new(),
                axes: vec![],
            },
        ],
    );
    h.frame();
    h.click("Figures (2)");
    h.frame();
    h.frame();
    assert!(h.has_text("Figure 2: demo"));
    assert!(h.has_text("waves"));
    assert!(h.has_text("sin"));
    assert!(h.has_text("y 1"));
    assert!(h.has_text("y -1    x 0 … 6.2832"));
    assert!(h.has_text("(nothing plotted)"));
    assert_eq!(h.plots(), [vec![128, 255, 128, 0]]);

    h.click(&label(icons::FLOPPY_DISK, "Save PNG"));
    h.frame();
    assert_eq!(h.outstanding::<OctaveRun>(), 1);
    assert!(h.app.history[0].starts_with("print(2, \"figure2.png\")"));
}

#[test]
fn no_figures_says_how_to_make_one() {
    let mut h = Harness::new(Octave::default());
    h.frame();
    h.click("Figures");
    h.frame();
    h.frame();
    assert!(h.has_text("No figures. Draw one with plot, for example plot(sin(0:0.1:6))."));
}

fn editor() -> Harness<Octave> {
    let mut h = Harness::new(Octave::default());
    h.frame();
    h.click("Editor");
    // One frame for the app to see the click, one to draw the tab it chose.
    h.frame();
    h.frame();
    h
}

#[test]
fn the_editor_opens_a_file() {
    let mut h = editor();
    // Fields: the history search, then the editor's path and its text.
    h.type_text(1, "/home/alice/fit.m");
    h.frame();
    h.click(&label(icons::FOLDER_OPEN, "Open"));
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ReadFile>(), 1);
    h.reply::<ReadFile>(&FileText {
        text: "y = 2 * x".to_owned(),
        writable: false,
    });
    h.frame();
    assert_eq!(h.app.editor.as_str(), "y = 2 * x");
    assert!(h.has_text("Opened, read-only: you can run it but not save over it."));
}

#[test]
fn the_editor_reports_a_file_it_cannot_open() {
    let mut h = editor();
    h.click(&label(icons::FOLDER_OPEN, "Open"));
    h.frame();
    h.frame();
    assert!(h.has_text("Type the file's path first."));
    h.type_text(1, "/nope.m");
    h.frame();
    h.click(&label(icons::FOLDER_OPEN, "Open"));
    h.frame();
    h.frame();
    h.fail::<ReadFile>(rpc_error::SERVER);
    h.frame();
    assert!(h.app.editor_status.starts_with("Could not open it"));
}

#[test]
fn the_editor_saves_and_runs_its_file() {
    let mut h = editor();
    h.type_text(1, "/home/alice/fit.m");
    h.type_text(2, "a = 1");
    h.frame();
    h.click(&label(icons::PLAY, "Run"));
    h.frame();
    assert_eq!(h.outstanding::<WriteFile>(), 1);
    h.reply::<WriteFile>(&());
    h.frame();
    assert_eq!(h.outstanding::<OctaveRun>(), 1, "saved, then run");
    assert_eq!(h.app.tab, Tab::Command);
    assert!(h.app.log.iter().any(|(_, l)| l == "run /home/alice/fit.m"));
}

#[test]
fn the_editor_without_a_file_runs_its_text_as_one_script() {
    let mut h = editor();
    h.type_text(2, "a = 1\nb = a + 1");
    h.frame();
    h.click(&label(icons::PLAY, "Run"));
    h.frame();
    assert_eq!(h.outstanding::<WriteFile>(), 0);
    assert_eq!(h.outstanding::<OctaveRun>(), 1);
    assert_eq!(h.app.tab, Tab::Command);
    // One line in the command window for the whole script, and nothing in the history.
    assert_eq!(h.app.log, [(LineKind::Input, UNSAVED_RUN.to_owned())]);
    assert!(h.app.history.is_empty());
}

#[test]
fn an_empty_editor_runs_nothing() {
    let mut h = editor();
    h.click(&label(icons::PLAY, "Run"));
    h.frame();
    assert_eq!(h.outstanding::<OctaveRun>(), 0);
    assert_eq!(h.app.tab, Tab::Editor);
}

#[test]
fn what_would_start_a_job_is_greyed_out_while_one_runs() {
    let mut h = running("pause(5)");
    h.app.history.push("x = 1".to_owned());
    h.app.session = session(vec![var("x", "1")], vec![waves()]);
    h.frame();
    h.reply::<ListDir>(&listing(vec![entry("data", EntryKind::Dir)]));
    h.frame();
    let run = label(icons::PLAY, "Run");
    for button in [
        run.as_str(),
        icons::PLAY,
        icons::ARROW_UP,
        "cd",
        &label(icons::ARROW_CLOCKWISE, "Restart"),
        "x  1x1 double",
    ] {
        assert!(h.is_disabled(button), "{button} is greyed out");
    }
    // Browsing and recalling still work, and the prompt still takes typing.
    assert!(!h.is_disabled(icons::CARET_UP));
    assert!(!h.is_disabled(&label(icons::FOLDER, "data")));
    h.type_text(1, "y = 2");
    h.frame();

    // Clicking Run does nothing, and Enter keeps what was typed for later.
    h.click(&run);
    h.press_enter(1);
    h.frame();
    assert_eq!(h.app.history, ["pause(5)", "x = 1"]);
    assert_eq!(h.app.input.as_str(), "y = 2");

    // The figure's Save PNG waits too.
    h.click("Figures (1)");
    h.frame();
    h.frame();
    assert!(h.is_disabled(&label(icons::FLOPPY_DISK, "Save PNG")));

    // Once the job ends, everything is back.
    h.reply::<OctavePoll>(&done("", session(vec![], vec![])));
    h.frame();
    h.click("Command Window");
    h.frame();
    h.frame();
    assert!(!h.is_disabled(&run));
}

#[test]
fn the_editor_needs_a_path_to_save_and_reports_failures() {
    let mut h = editor();
    h.click(&label(icons::FLOPPY_DISK, "Save"));
    h.frame();
    assert_eq!(h.app.editor_status, "Type a path to save to first.");
    h.type_text(1, "/Docs/x.m");
    h.frame();
    h.click(&label(icons::FLOPPY_DISK, "Save"));
    h.frame();
    h.fail::<WriteFile>(rpc_error::DENIED);
    h.frame();
    assert_eq!(h.app.editor_status, "Could not save: permission denied");
}

#[test]
fn new_clears_the_editor() {
    let mut h = editor();
    h.type_text(2, "junk");
    h.frame();
    h.click("New");
    h.frame();
    assert_eq!(h.app.editor.as_str(), "");
}

#[test]
fn the_log_keeps_only_recent_lines() {
    let mut h = Harness::new(Octave::default());
    for i in 0..MAX_LOG + 10 {
        h.app.push_log(LineKind::Output, &itoa(i as u64));
    }
    assert_eq!(h.app.log.len(), MAX_LOG);
    assert_eq!(h.app.log[0].1, "10");
    h.frame();
}

#[test]
fn outside_the_jail_the_folder_says_so() {
    let mut h = Harness::new(Octave::default());
    h.frame();
    assert!(h.has_label("(outside the shared files)"));
}

#[test]
fn enter_at_the_prompt_runs_the_command() {
    let mut h = Harness::new(Octave::default());
    h.frame();
    assert!(h.has_text(
        "Type Octave code at the >> prompt below and press Enter to run it. \
         Variables are kept between commands."
    ));
    h.type_text(1, "y = 4");
    h.press_enter(1);
    h.frame();
    assert_eq!(h.outstanding::<OctaveRun>(), 1);
    assert_eq!(h.app.history, ["y = 4"]);
    assert_eq!(h.app.input.as_str(), "");
    h.frame();
    assert!(h.has_text(">> y = 4"));
}

fn entry(name: &str, kind: EntryKind) -> DirEntry {
    DirEntry {
        name: name.to_owned(),
        kind,
        size: 0,
        mtime_s: 0,
    }
}

fn listing(entries: Vec<DirEntry>) -> DirListing {
    DirListing {
        entries,
        truncated: false,
    }
}

/// An app whose session is in `/home/alice`, with that folder listed.
fn in_folder() -> Harness<Octave> {
    let mut h = Harness::new(Octave::default());
    h.app.session = session(vec![], vec![]);
    h.frame();
    assert!(h.has_text("Current Folder"));
    assert!(h.has_text("Loading…"));
    h.reply::<ListDir>(&listing(vec![
        entry("notes.txt", EntryKind::File),
        entry("fit.m", EntryKind::File),
        entry(".hidden.m", EntryKind::File),
        entry("data", EntryKind::Dir),
    ]));
    h.frame();
    h
}

#[test]
fn the_current_folder_lists_folders_then_files() {
    let h = in_folder();
    let rows: Vec<String> = h.selectables().into_iter().map(|(t, _)| t).collect();
    let data = rows
        .iter()
        .position(|r| *r == label(icons::FOLDER, "data"))
        .unwrap();
    let fit = rows
        .iter()
        .position(|r| *r == label(icons::FILE_CODE, "fit.m"))
        .unwrap();
    assert!(data < fit, "folders first: {rows:?}");
    // Other files are shown but do nothing; hidden ones are left out.
    assert!(h.has_text(&label(icons::FILE, "notes.txt")));
    assert!(!rows.iter().any(|r| r.contains(".hidden")));
}

#[test]
fn a_script_in_the_tree_opens_in_the_editor() {
    let mut h = in_folder();
    h.click(&label(icons::FILE_CODE, "fit.m"));
    h.frame();
    assert_eq!(h.app.tab, Tab::Editor);
    assert_eq!(h.app.editor_path.as_str(), "/home/alice/fit.m");
    h.frame();
    assert_eq!(h.outstanding::<ReadFile>(), 1);
    h.reply::<ReadFile>(&FileText {
        text: "p = polyfit(x, y, 1)".to_owned(),
        writable: true,
    });
    h.frame();
    assert_eq!(h.app.editor.as_str(), "p = polyfit(x, y, 1)");
    assert!(
        h.selectables()
            .contains(&(label(icons::FILE_CODE, "fit.m"), true))
    );
}

#[test]
fn a_folder_in_the_tree_opens_and_closes() {
    let mut h = in_folder();
    h.click(&label(icons::FOLDER, "data"));
    h.frame();
    h.frame();
    assert!(h.app.expanded.contains("/home/alice/data"));
    assert_eq!(h.outstanding::<ListDir>(), 1);
    h.reply::<ListDir>(&listing(vec![]));
    h.frame();
    assert!(h.has_text("Empty"));
    h.click(&label(icons::FOLDER_OPEN, "data"));
    h.frame();
    assert!(h.app.expanded.is_empty());
}

#[test]
fn the_tree_changes_octaves_folder() {
    let mut h = in_folder();
    h.click("cd");
    h.frame();
    assert_eq!(h.app.history, ["cd 'data'"]);
    h.reply::<OctaveRun>(&true);
    h.frame();
    let mut moved = session(vec![], vec![]);
    moved.cwd = "/home/alice/data".to_owned();
    h.reply::<OctavePoll>(&done("", moved));
    h.frame();
    // The new folder is listed, and the old one's listing is dropped.
    h.frame();
    assert_eq!(h.outstanding::<ListDir>(), 1);
    h.reply::<ListDir>(&listing(vec![]));
    h.frame();

    h.click(icons::ARROW_UP);
    h.frame();
    assert_eq!(h.app.history, ["cd 'data'", "cd '..'"]);
}

#[test]
fn quotes_in_a_folder_name_are_doubled() {
    assert_eq!(quoted("it's"), "'it''s'");
}
