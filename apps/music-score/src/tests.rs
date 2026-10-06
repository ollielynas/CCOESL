use ccosel_proto::fs::FileText;
use ccosel_proto::score::EngraveStatus;
use ccosel_sdk::testing::{Harness, rpc_error};

use super::*;

const KEY: &str = "0123456789abcdef0123456789abcdef";

fn started() -> Harness<MusicScore> {
    let mut h = Harness::new(MusicScore::default());
    h.frame();
    h
}

fn running(elapsed_ms: u64) -> EngraveStatus {
    EngraveStatus {
        finished: false,
        elapsed_ms,
        result: None,
    }
}

fn result(pages: u32, messages: Vec<ScoreMessage>) -> EngraveResult {
    EngraveResult {
        success: !messages.iter().any(|m| m.error) && pages > 0,
        key: KEY.to_owned(),
        pages,
        pdf: pages > 0,
        midi: pages > 0,
        messages,
        output: String::new(),
        output_truncated: false,
    }
}

fn done(result: EngraveResult) -> EngraveStatus {
    EngraveStatus {
        finished: true,
        elapsed_ms: 2_000,
        result: Some(result),
    }
}

fn message(line: u32, column: u32, error: bool, text: &str) -> ScoreMessage {
    ScoreMessage {
        line,
        column,
        error,
        text: text.to_owned(),
    }
}

/// Presses Engrave and lets the app send its first poll.
fn engrave(h: &mut Harness<MusicScore>) {
    press(h, "🎼 Engrave");
    assert_eq!(h.outstanding::<Engrave>(), 1);
}

/// Answers the poll in flight with `status` and draws what the app makes of it.
fn answer(h: &mut Harness<MusicScore>, status: &EngraveStatus) {
    h.reply::<Engrave>(status);
    h.frame();
}

/// Clicks `label` and draws the frame that acts on it, then the one that shows what it did,
/// which the app asks for straight away.
fn press(h: &mut Harness<MusicScore>, label: &str) {
    h.click(label);
    h.frame();
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        0,
        "{label} asks to be redrawn"
    );
    h.frame();
}

#[test]
fn a_new_score_starts_from_the_starter_and_can_be_engraved_straight_away() {
    let h = started();
    assert_eq!(h.text_fields(), ["", STARTER]);
    assert!(h.has_button("📂 Open") && h.has_button("💾 Save") && h.has_button("🎼 Engrave"));
    assert!(!h.has_label("(unsaved)"));
    assert!(h.images().is_empty());
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        ccosel_sdk::REPAINT_ON_INPUT_ONLY
    );
}

#[test]
fn engraving_shows_progress_then_the_pages_and_downloads() {
    let mut h = started();
    engrave(&mut h);
    assert!(h.has_label("Engraving… 0 s"));
    assert!(!h.has_button("🎼 Engrave"), "one engraving at a time");
    assert_eq!(h.app.wants_repaint_after_ms(), POLL_MS);

    answer(&mut h, &running(3_400));
    assert!(h.has_label("Engraving… 3 s"));
    // Asked again, for the result, when the poll timer runs the app next.
    h.frame();
    assert_eq!(h.outstanding::<Engrave>(), 1);

    answer(&mut h, &done(result(3, vec![])));
    assert_eq!(
        h.outstanding::<Engrave>(),
        0,
        "the result is kept, not asked for again"
    );
    assert_eq!(h.images(), [page_url(KEY, 1)]);
    assert!(h.has_label("Page 1 of 3"));
    assert_eq!(
        h.open_urls(),
        [
            ("Download PDF".to_owned(), pdf_url(KEY)),
            ("Download MIDI".to_owned(), midi_url(KEY)),
        ]
    );
    assert!(h.has_button("🎼 Engrave"));
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        ccosel_sdk::REPAINT_ON_INPUT_ONLY
    );
}

#[test]
fn pages_are_turned_one_at_a_time_and_stop_at_either_end() {
    let mut h = started();
    engrave(&mut h);
    answer(&mut h, &done(result(2, vec![])));

    press(&mut h, "◀ Previous");
    assert!(h.has_label("Page 1 of 2"));
    press(&mut h, "Next ▶");
    assert!(h.has_label("Page 2 of 2"));
    assert_eq!(h.images(), [page_url(KEY, 2)]);
    press(&mut h, "Next ▶");
    assert!(h.has_label("Page 2 of 2"));
    press(&mut h, "◀ Previous");
    assert_eq!(h.images(), [page_url(KEY, 1)]);
}

#[test]
fn a_one_page_score_has_no_page_buttons() {
    let mut h = started();
    engrave(&mut h);
    answer(&mut h, &done(result(1, vec![])));
    assert_eq!(h.images().len(), 1);
    assert!(!h.has_button("Next ▶") && !h.has_label("Page 1 of 1"));
}

#[test]
fn mistakes_are_listed_with_their_lines() {
    let mut h = started();
    engrave(&mut h);
    let mut failed = result(
        0,
        vec![
            message(4, 9, true, "syntax error, unexpected '}'"),
            message(1, 0, false, "no \\version statement found"),
            message(0, 0, true, "failed files: \"score.ly\""),
        ],
    );
    failed.success = false;
    answer(&mut h, &done(failed));
    assert!(h.has_label("❌ LilyPond could not engrave this score"));
    assert!(h.has_label("❌ Line 4, column 9: syntax error, unexpected '}'"));
    assert!(h.has_label("⚠ Line 1: no \\version statement found"));
    assert!(h.has_label("❌ failed files: \"score.ly\""));
    assert!(h.images().is_empty() && h.open_urls().is_empty());
}

#[test]
fn warnings_on_a_good_score_are_shown_beside_its_pages() {
    let mut h = started();
    engrave(&mut h);
    answer(
        &mut h,
        &done(result(1, vec![message(2, 0, false, "barcheck failed")])),
    );
    assert!(h.has_label("⚠ Line 2: barcheck failed"));
    assert!(!h.has_label("❌ LilyPond could not engrave this score"));
    assert_eq!(h.images().len(), 1);
}

#[test]
fn a_failure_with_nothing_to_point_at_shows_the_end_of_the_log() {
    let mut h = started();
    engrave(&mut h);
    let mut failed = result(0, vec![]);
    failed.output = (1..=20).map(|i| format!("log line {i}\n")).collect();
    failed.output_truncated = true;
    answer(&mut h, &done(failed));
    assert!(h.has_label("(earlier output trimmed)"));
    assert!(h.has_label("log line 20") && h.has_label("log line 9"));
    assert!(
        !h.has_label("log line 8"),
        "only the last {LOG_LINES} lines"
    );
}

#[test]
fn an_engraving_the_server_refuses_says_why() {
    let mut h = started();
    engrave(&mut h);
    h.fail::<Engrave>(rpc_error::SERVER);
    h.frame();
    assert!(h.has_label("Could not engrave: server error"));
    assert!(h.has_button("🎼 Engrave"));
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        ccosel_sdk::REPAINT_ON_INPUT_ONLY
    );
}

#[test]
fn what_is_engraved_is_what_is_in_the_editor() {
    let mut h = started();
    h.type_text(1, "{ c'1 }");
    h.frame();
    // Typing reaches the app as the editor is drawn, after the toolbar: it redraws at once.
    assert_eq!(h.app.wants_repaint_after_ms(), 0);
    h.frame();
    assert!(h.has_label("(unsaved)"));
    engrave(&mut h);
    assert_eq!(h.app.engraving.as_deref(), Some("{ c'1 }"));
}

#[test]
fn opening_a_file_puts_it_in_the_editor() {
    let mut h = started();
    h.type_text(0, "/home/me/song.ly");
    h.frame();
    press(&mut h, "📂 Open");
    assert!(h.has_label("Opening /home/me/song.ly…"));
    assert_eq!(h.outstanding::<ReadFile>(), 1);
    h.reply::<ReadFile>(&FileText {
        text: "{ e'1 }".to_owned(),
        writable: true,
    });
    h.frame();
    h.frame();
    assert_eq!(h.text_fields(), ["/home/me/song.ly", "{ e'1 }"]);
    assert!(h.has_label("Opened /home/me/song.ly"));
    assert!(!h.has_label("(unsaved)"));
}

#[test]
fn opening_an_unreadable_file_says_so() {
    let mut h = started();
    h.type_text(0, "/secret.ly");
    h.frame();
    press(&mut h, "📂 Open");
    h.fail::<ReadFile>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("Could not open /secret.ly: permission denied"));
    assert_eq!(h.text_fields()[1], STARTER);
}

#[test]
fn open_needs_a_path() {
    let mut h = started();
    press(&mut h, "📂 Open");
    assert!(h.has_label("Type the path of a .ly file to open."));
    assert_eq!(h.outstanding::<ReadFile>(), 0);
}

#[test]
fn opening_over_unsaved_changes_asks_first() {
    let mut h = started();
    h.type_text(0, "/song.ly");
    h.type_text(1, "{ changed }");
    h.frame();
    press(&mut h, "📂 Open");
    assert!(h.has_label("You have unsaved changes. Press Open again to discard them."));
    h.frame();
    assert_eq!(h.outstanding::<ReadFile>(), 0);
    press(&mut h, "📂 Open");
    assert_eq!(h.outstanding::<ReadFile>(), 1);
}

#[test]
fn an_app_opened_on_a_file_reads_it() {
    let mut h = Harness::new(MusicScore::default());
    h.launch("/Music/waltz.ly");
    h.frame();
    assert_eq!(h.outstanding::<ReadFile>(), 1);
    h.reply::<ReadFile>(&FileText {
        text: "{ g'2. }".to_owned(),
        writable: false,
    });
    h.frame();
    h.frame();
    assert_eq!(h.text_fields(), ["/Music/waltz.ly", "{ g'2. }"]);
}

#[test]
fn opening_a_new_file_clears_the_old_pages() {
    let mut h = started();
    engrave(&mut h);
    answer(&mut h, &done(result(1, vec![])));
    h.app.open("/other.ly");
    h.frame();
    h.reply::<ReadFile>(&FileText {
        text: "{ a1 }".to_owned(),
        writable: true,
    });
    h.frame();
    assert!(h.images().is_empty());
}

#[test]
fn saving_writes_the_editor_to_the_path() {
    let mut h = started();
    h.type_text(0, "/home/me/song.ly");
    h.type_text(1, "{ d'1 }");
    h.frame();
    press(&mut h, "💾 Save");
    assert_eq!(h.outstanding::<WriteFile>(), 1);
    h.frame();
    assert!(h.has_button("Saving…"));
    h.reply::<WriteFile>(&());
    h.frame();
    h.frame();
    assert!(h.has_label("Saved"));
    assert!(!h.has_label("(unsaved)"));
}

#[test]
fn a_failed_save_keeps_the_changes_and_says_why() {
    let mut h = started();
    h.type_text(0, "/Docs/song.ly");
    h.type_text(1, "{ d'1 }");
    h.frame();
    press(&mut h, "💾 Save");
    h.fail::<WriteFile>(rpc_error::DENIED);
    h.frame();
    h.frame();
    assert!(h.has_label("Could not save: permission denied"));
    assert!(h.has_label("(unsaved)"));
}

#[test]
fn save_needs_a_path() {
    let mut h = started();
    press(&mut h, "💾 Save");
    assert!(h.has_label("Type where to save it first, such as /home/you/song.ly."));
    assert_eq!(h.outstanding::<WriteFile>(), 0);
}

#[test]
fn a_place_reads_as_a_line_and_column() {
    assert_eq!(place(&message(3, 7, true, "")), "Line 3, column 7: ");
    assert_eq!(place(&message(3, 0, true, "")), "Line 3: ");
    assert_eq!(place(&message(0, 5, true, "")), "");
}

/// The user documentation shows the starter score, so it must be the one a new score starts as.
#[test]
fn the_docs_show_the_starter_score_as_it_is() {
    let page = include_str!("../../../data/shared/Docs/Apps/music-score.md");
    assert!(
        page.contains(STARTER),
        "update the starter score in the docs page"
    );
}
