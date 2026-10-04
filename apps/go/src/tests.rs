//! The Go app through the SDK's harness, playing the server's part: answering `GoEngine` as
//! GNU Go would, and the file calls.

use ccosel_proto::fs::FileText;
use ccosel_proto::go::{EngineStatus, parse_vertex};
use ccosel_sdk::testing::{Harness, rpc_error};

use super::*;

fn mv(v: &str) -> Move {
    parse_vertex(v, 9).unwrap()
}

fn running() -> EngineStatus {
    EngineStatus {
        finished: false,
        answer: None,
    }
}

fn answer(a: Result<Answer, String>) -> EngineStatus {
    EngineStatus {
        finished: true,
        answer: Some(a),
    }
}

fn started() -> Harness<GoApp> {
    let mut h = Harness::new(GoApp::default());
    h.frame();
    h
}

/// Plays `you` then answers GNU Go's turn with `reply`.
fn exchange(h: &mut Harness<GoApp>, you: &str, reply: &str) {
    h.app.play(mv(you));
    h.frame();
    assert_eq!(h.outstanding::<GoEngine>(), 1, "GNU Go was asked");
    h.reply::<GoEngine>(&answer(Ok(Answer::Move(mv(reply)))));
    h.frame();
}

fn cells(h: &Harness<GoApp>) -> Vec<(String, bool)> {
    h.selectables()
}

#[test]
fn a_new_app_shows_an_empty_9x9_board_and_waits_for_you() {
    let h = started();
    assert!(h.has_label("Your move (Black)."));
    assert_eq!(cells(&h).len(), 81);
    assert!(cells(&h).iter().all(|(g, sel)| g == EMPTY && !sel));
    assert!(h.has_label("A") && h.has_label("J") && !h.has_label("I"));
    assert_eq!(
        h.outstanding::<GoEngine>(),
        0,
        "nothing to ask on your turn"
    );
    assert!(!h.has_button(UNDO));
}

#[test]
fn clicking_a_point_plays_there_and_gnu_go_answers() {
    let mut h = started();
    h.click(EMPTY); // the top-left point, A9
    h.frame();
    assert_eq!(h.app.game.moves, [mv("A9")]);
    h.frame();
    assert!(h.has_label("GNU Go is thinking…"));
    assert_eq!(h.outstanding::<GoEngine>(), 1);
    assert_eq!(h.app.wants_repaint_after_ms(), POLL_MS);

    h.reply::<GoEngine>(&running());
    h.frame();
    h.frame();
    assert_eq!(
        h.outstanding::<GoEngine>(),
        1,
        "asked again while it thinks"
    );
    h.reply::<GoEngine>(&answer(Ok(Answer::Move(mv("E5")))));
    h.frame();
    h.frame();
    assert_eq!(h.app.game.moves, [mv("A9"), mv("E5")]);
    assert!(h.has_label("Your move (Black)."));
    // E5 is row 5 from the bottom (index 4), column E (4): the last move, highlighted.
    let board = cells(&h);
    assert_eq!(board[4 * 9 + 4], (WHITE.to_owned(), true));
    assert_eq!(board[0], (BLACK.to_owned(), false));
    assert!(h.has_text("Last move: E5 (highlighted)"));
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        ccosel_sdk::REPAINT_ON_INPUT_ONLY
    );
}

#[test]
fn a_capture_takes_the_stone_off_and_is_counted() {
    let mut h = started();
    exchange(&mut h, "D5", "E5");
    exchange(&mut h, "F5", "A1");
    exchange(&mut h, "E4", "A2");
    h.app.play(mv("E6"));
    h.frame();
    assert!(h.has_label("Captured: by you 1, by GNU Go 0."));
    assert_eq!(cells(&h)[4 * 9 + 4].0, EMPTY);
}

#[test]
fn ko_occupied_and_out_of_turn_moves_are_refused_with_a_reason() {
    let mut h = started();
    // Black has just taken at E5; you are White, and may not retake at D5 at once.
    h.app.you = Stone::White;
    h.app.game.moves = ["C5", "D5", "D4", "E4", "D6", "E6", "A1", "F5", "E5"]
        .map(mv)
        .to_vec();
    h.app.play(mv("D5"));
    h.frame();
    assert!(h.has_text(
        "You can't play there: ko: you can't take back straight away; play elsewhere first."
    ));
    assert_eq!(h.app.game.moves.len(), 9);
    h.app.play(mv("E5"));
    assert_eq!(
        h.app.notice.as_deref(),
        Some("You can't play there: there is already a stone there.")
    );
    h.app.play(mv("A9"));
    h.app.play(mv("B9"));
    assert_eq!(h.app.notice.as_deref(), Some("Wait for GNU Go to move."));
}

#[test]
fn two_passes_end_the_game_and_it_is_scored() {
    let mut h = started();
    exchange(&mut h, "E5", "C3");
    h.click(PASS);
    h.frame();
    h.frame();
    h.reply::<GoEngine>(&answer(Ok(Answer::Move(Move::Pass))));
    h.frame();
    h.frame();
    assert!(h.has_label("Both passed. Counting…"));
    assert_eq!(h.outstanding::<GoEngine>(), 1, "the score is asked for");
    h.reply::<GoEngine>(&answer(Ok(Answer::Score {
        winner: Some(Stone::White),
        margin_x2: 15,
        dead: vec![(4, 4)],
    })));
    h.frame();
    h.frame();
    assert!(h.has_label("White wins by 7.5 points (GNU Go)."));
    assert_eq!(cells(&h)[4 * 9 + 4].0, DEAD, "dead stones are marked");
    assert!(!h.has_button(PASS) && !h.has_button(RESIGN));
}

#[test]
fn gnu_go_passing_says_so_and_a_draw_reads_as_one() {
    let mut h = started();
    h.app.play(mv("E5"));
    h.frame();
    h.reply::<GoEngine>(&answer(Ok(Answer::Move(Move::Pass))));
    h.frame();
    h.frame();
    assert!(h.has_label("GNU Go passed. Your move (Black); pass too to end the game."));
    h.app.outcome = Some(Outcome::Scored {
        winner: None,
        margin_x2: 0,
        dead: vec![],
    });
    assert_eq!(h.app.status(), "The game is a draw.");
    h.app.outcome = Some(Outcome::Scored {
        winner: Some(Stone::Black),
        margin_x2: 4,
        dead: vec![],
    });
    assert_eq!(h.app.status(), "Black wins by 2 points (you).");
}

#[test]
fn an_engine_error_is_shown_and_can_be_asked_again() {
    let mut h = started();
    h.app.play(mv("E5"));
    h.frame();
    h.reply::<GoEngine>(&answer(Err("GNU Go it stopped unexpectedly".into())));
    h.frame();
    h.frame();
    assert!(h.has_label("GNU Go couldn't answer: GNU Go it stopped unexpectedly"));
    assert_eq!(h.outstanding::<GoEngine>(), 0, "not asked again by itself");
    h.click(RETRY);
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<GoEngine>(), 1);
    h.fail::<GoEngine>(rpc_error::TIMEOUT);
    h.frame();
    h.frame();
    assert!(
        h.has_label("GNU Go couldn't answer: timed out"),
        "{:?}",
        h.labels()
    );
}

#[test]
fn resigning_either_way() {
    let mut h = started();
    h.app.play(mv("E5"));
    h.frame();
    h.reply::<GoEngine>(&answer(Ok(Answer::Resign)));
    h.frame();
    h.frame();
    assert!(h.has_label("GNU Go resigned. You win!"));

    let mut h = started();
    h.click(RESIGN);
    h.frame();
    h.frame();
    assert!(h.has_label("You resigned. GNU Go wins."));
}

#[test]
fn undo_takes_back_your_move_and_the_reply() {
    let mut h = started();
    exchange(&mut h, "E5", "C3");
    exchange(&mut h, "D4", "F6");
    h.click(UNDO);
    h.frame();
    assert_eq!(h.app.game.moves, [mv("E5"), mv("C3")]);
    // Undo while GNU Go is still thinking takes back the move it is thinking about.
    h.app.play(mv("G7"));
    h.app.undo();
    assert_eq!(h.app.game.moves.len(), 2);
}

#[test]
fn a_stale_answer_is_not_played() {
    let mut h = started();
    h.app.game.moves = vec![mv("E5")];
    h.app.answered(&Ok(Answer::Move(mv("E5"))));
    assert_eq!(h.app.game.moves, [mv("E5")], "occupied: not played");
}

#[test]
fn a_new_game_with_settings() {
    let mut h = started();
    h.click(NEW_GAME);
    h.frame();
    h.frame();
    h.click("13×13");
    h.frame();
    h.click("White");
    h.frame();
    for _ in 0..2 {
        h.click(MORE); // handicap: 0 → 2 → 3
        h.frame();
    }
    h.frame();
    assert_eq!(h.app.new_handicap, 3);
    assert!(h.has_label("6.5"));
    h.frame();
    h.click(START);
    h.frame();
    h.frame();
    assert_eq!(h.app.game, Game::new(13, 13, 3));
    assert_eq!(h.app.you, Stone::White);
    // White moves first after handicap stones: your turn.
    assert!(h.has_label("Your move (White)."));
    assert_eq!(cells(&h).iter().filter(|(g, _)| g == BLACK).count(), 3);
}

#[test]
fn the_settings_stay_in_range() {
    let mut app = GoApp {
        setting_up: true,
        ..GoApp::default()
    };
    app.new_size = 9;
    app.new_handicap = 5;
    let mut h = Harness::new(app);
    h.frame();
    // More stays at the 9×9 maximum of 5; fewer from 2 goes to none.
    h.click(MORE);
    h.frame();
    assert_eq!(h.app.new_handicap, 5);
    h.app.new_handicap = 2;
    h.frame();
    h.click(FEWER);
    h.frame();
    assert_eq!(h.app.new_handicap, 0);
    h.app.new_handicap = 4;
    h.frame();
    h.click(FEWER);
    h.frame();
    assert_eq!(h.app.new_handicap, 3);
    h.app.new_handicap = 9;
    h.app.new_size = 19;
    h.frame();
    h.click("9×9");
    h.frame();
    assert_eq!(h.app.new_handicap, 5, "clamped to the smaller board");
    h.click("Cancel");
    h.frame();
    assert!(!h.app.setting_up);
}

#[test]
fn komi_and_strength_steppers_stay_in_range() {
    let mut h = Harness::new(GoApp {
        setting_up: true,
        ..GoApp::default()
    });
    h.frame();
    h.click("+0.5");
    h.frame();
    assert_eq!(h.app.new_komi_x2, 14);
    assert!(h.has_label("7"));
    h.app.new_komi_x2 = -MAX_KOMI_X2;
    h.frame();
    h.click("−0.5");
    h.frame();
    assert_eq!(h.app.new_komi_x2, -MAX_KOMI_X2);

    h.click(STRONGER);
    h.frame();
    assert_eq!(h.app.new_level, 6);
    h.app.new_level = MAX_LEVEL;
    h.frame();
    h.click(STRONGER);
    h.frame();
    assert_eq!(h.app.new_level, MAX_LEVEL);
    h.app.new_level = 1;
    h.frame();
    h.click(WEAKER);
    h.frame();
    assert_eq!(h.app.new_level, 1);
}

#[test]
fn a_game_saves_as_sgf_and_opens_again() {
    let mut h = started();
    exchange(&mut h, "E5", "C3");
    h.type_text(0, "/home/me/fight.sgf");
    h.frame();
    h.click(SAVE);
    h.frame();
    assert_eq!(h.outstanding::<WriteFile>(), 1);
    h.reply::<WriteFile>(&());
    h.frame();
    h.frame();
    assert!(h.has_text("Saved to /home/me/fight.sgf."));

    let saved = to_sgf(&h.app.game);
    let mut h = started();
    h.type_text(0, "/home/me/fight.sgf");
    h.frame();
    h.click(OPEN);
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ReadFile>(), 1);
    h.reply::<ReadFile>(&FileText {
        text: saved,
        writable: true,
    });
    h.frame();
    h.frame();
    assert!(h.has_text("Opened /home/me/fight.sgf."));
    assert_eq!(h.app.game.moves, [mv("E5"), mv("C3")]);
    assert_eq!(h.app.you, Stone::Black, "you take whoever is to move");
}

#[test]
fn file_failures_say_why() {
    let mut h = started();
    h.click(SAVE);
    h.frame();
    assert!(h.has_text(NEED_PATH));
    h.type_text(0, "/game.sgf");
    h.frame();
    h.click(SAVE);
    h.frame();
    h.fail::<WriteFile>(rpc_error::DENIED);
    h.frame();
    h.frame();
    assert!(h.has_text("Couldn't save: permission denied."));

    h.click(OPEN);
    h.frame();
    h.frame();
    h.reply::<ReadFile>(&FileText {
        text: "(;SZ[10])".into(),
        writable: true,
    });
    h.frame();
    h.frame();
    assert!(
        h.has_text("Couldn't open /game.sgf: 10×10 boards aren't offered here."),
        "{:?}",
        h.styled()
    );

    h.click(OPEN);
    h.frame();
    h.frame();
    h.fail::<ReadFile>(rpc_error::DENIED);
    h.frame();
    h.frame();
    assert!(h.has_text("Couldn't open /game.sgf: permission denied."));
}
