//! The engraver against the real `lilypond`, end to end: a score in, pages, a PDF and MIDI out,
//! and an error pointing at its line.
//!
//! LilyPond isn't installed on the CI runner, so this only runs when asked to:
//!
//! ```sh
//! CCOSEL_TEST_LILYPOND=1 cargo test -p ccosel-server --test lilypond
//! ```
//!
//! Without the variable it prints that it was skipped and passes. Once the runner has
//! `lilypond`, CI can set the variable for this test and it becomes a real gate.

use std::time::{Duration, Instant};

use ccosel_proto::score::EngraveResult;
use ccosel_server::score_api::Engraver;

fn enabled() -> bool {
    let on = std::env::var_os("CCOSEL_TEST_LILYPOND").is_some_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set CCOSEL_TEST_LILYPOND=1 to run this against the real lilypond");
    }
    on
}

fn engrave(engraver: &Engraver, source: &str) -> EngraveResult {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(result) = engraver.engrave(source).unwrap().result {
            return result;
        }
        assert!(Instant::now() < deadline, "LilyPond never finished");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_real_score_is_engraved_into_pages_a_pdf_and_midi() {
    if !enabled() {
        return;
    }
    let engraver = Engraver::new().unwrap();
    let result = engrave(
        &engraver,
        "\\version \"2.24.0\"\n\\score {\n  \\relative c' { c4 d e f | g1 }\n  \\layout { }\n  \\midi { }\n}\n",
    );
    assert!(result.success, "{}", result.output);
    assert_eq!(result.pages, 1);
    assert!(result.pdf && result.midi, "{result:?}");
    let (page, _) = engraver.file(&result.key, "page-1.png").unwrap();
    assert_eq!(&std::fs::read(page).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
    let (pdf, _) = engraver.file(&result.key, "score.pdf").unwrap();
    assert_eq!(&std::fs::read(pdf).unwrap()[..5], b"%PDF-");
}

#[test]
fn a_long_score_has_one_picture_per_page() {
    if !enabled() {
        return;
    }
    let engraver = Engraver::new().unwrap();
    let bars = "c'4 d' e' f' | ".repeat(400);
    let result = engrave(&engraver, &format!("\\version \"2.24.0\"\n{{ {bars} }}\n"));
    assert!(result.success, "{}", result.output);
    assert!(result.pages > 1, "{result:?}");
    assert!(!result.midi, "no \\midi, no MIDI file");
    for page in 1..=result.pages {
        assert!(
            engraver
                .file(&result.key, &format!("page-{page}.png"))
                .is_some()
        );
    }
}

#[test]
fn a_mistake_is_reported_at_its_line() {
    if !enabled() {
        return;
    }
    let engraver = Engraver::new().unwrap();
    let result = engrave(
        &engraver,
        "\\version \"2.24.0\"\n{\n  c4 d e\n  \\nosuchcommand\n}\n",
    );
    assert!(!result.success);
    let error = result
        .messages
        .iter()
        .find(|m| m.error)
        .unwrap_or_else(|| panic!("no error in {result:?}"));
    assert_eq!(error.line, 4, "{result:?}");
}
