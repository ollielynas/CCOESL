use ccosel_proto::fs::FileText;
use ccosel_sdk::testing::{Harness, rpc_error};

use super::*;

fn started() -> Harness<Chess> {
    let mut h = Harness::new(Chess::default());
    h.frame();
    h
}

/// Clicks `label`, then draws the frame that acts on it and the one that shows what it did.
fn press(h: &mut Harness<Chess>, label: &str) {
    h.click(label);
    h.frame();
    h.frame();
}

/// Clicks the square named `sq`, as `press` does.
fn square(h: &mut Harness<Chess>, sq: &str) {
    h.click_tooltip(sq);
    h.frame();
    h.frame();
}

/// Moves a piece by clicking it and then where it goes.
fn play(h: &mut Harness<Chess>, from: &str, to: &str) {
    square(h, from);
    square(h, to);
}

/// Answers the engine's request with `m`.
fn engine(h: &mut Harness<Chess>, m: &str) {
    assert_eq!(
        h.outstanding::<EngineMove>(),
        1,
        "GNU Chess should be asked"
    );
    h.reply::<EngineMove>(&EngineReply::Move(m.into()));
    h.frame();
    h.frame();
}

/// Every square's text, buttons and rows together, for counting what the board shows.
fn squares(h: &Harness<Chess>) -> Vec<String> {
    let mut all = h.buttons();
    all.extend(h.selectables().into_iter().map(|(t, _)| t));
    all
}

fn count(h: &Harness<Chess>, text: &str) -> usize {
    squares(h).iter().filter(|t| *t == text).count()
}

fn load(h: &mut Harness<Chess>, pgn_text: &str) {
    h.app.start(pgn::read(pgn_text).unwrap().0);
    h.frame();
}

#[test]
fn a_new_game_shows_the_board_with_white_to_move() {
    let h = started();
    assert!(h.has_label("Your move (White)."));
    assert_eq!(count(&h, "♙"), 8);
    assert_eq!(count(&h, "♟"), 8);
    assert_eq!(count(&h, "♔") + count(&h, "♚"), 2);
    // White at the bottom: rank 8 is drawn first.
    let labels = h.labels();
    let eight = labels.iter().position(|l| l == "8").unwrap();
    let one = labels.iter().position(|l| l == "1").unwrap();
    assert!(eight < one);
    assert_eq!(h.outstanding::<EngineMove>(), 0);
}

#[test]
fn picking_up_a_piece_shows_where_it_can_go() {
    let mut h = started();
    square(&mut h, "e2");
    assert_eq!(count(&h, "[♙]"), 1);
    assert_eq!(count(&h, "•"), 2, "e3 and e4");
    // The same piece again puts it back.
    square(&mut h, "e2");
    assert_eq!(count(&h, "•"), 0);
    // Another of yours picks that one up instead; theirs can't be picked up.
    square(&mut h, "g1");
    square(&mut h, "b1");
    assert_eq!(count(&h, "[♘]"), 1);
    assert_eq!(count(&h, "•"), 2, "a3 and c3");
    square(&mut h, "e7");
    assert_eq!(h.app.picked, Some(1));
}

#[test]
fn an_illegal_move_is_refused() {
    let mut h = started();
    play(&mut h, "e2", "e5");
    assert!(h.has_label("e2 can't move to e5."));
    assert!(h.app.game.moves().is_empty());
    assert_eq!(h.outstanding::<EngineMove>(), 0);
    // The pawn is still in hand, to try somewhere else.
    square(&mut h, "e4");
    assert_eq!(h.app.game.san_moves(), ["e4"]);
}

#[test]
fn an_empty_square_with_nothing_picked_up_does_nothing() {
    let mut h = started();
    square(&mut h, "e4");
    assert_eq!(h.app.picked, None);
    assert!(h.app.game.moves().is_empty());
}

#[test]
fn the_engines_reply_is_played_and_listed() {
    let mut h = started();
    play(&mut h, "e2", "e4");
    assert!(h.has_label("GNU Chess is thinking…"));
    // Its turn: clicking the board does nothing.
    square(&mut h, "d2");
    assert_eq!(h.app.picked, None);
    engine(&mut h, "e7e5");
    assert!(h.has_label("1. e4 e5  "));
    assert!(h.has_label("Your move (White)."));
    play(&mut h, "g1", "f3");
    engine(&mut h, "b8c6");
    assert!(h.has_label("2. Nf3 Nc6  "));
}

#[test]
fn the_engine_is_asked_at_the_level_chosen() {
    let mut h = started();
    press(&mut h, "4");
    assert_eq!(h.app.level, 4);
    assert!(h.selectables().contains(&("4".to_owned(), true)));
    play(&mut h, "d2", "d4");
    assert_eq!(h.outstanding::<EngineMove>(), 1);
}

#[test]
fn you_can_checkmate_the_engine() {
    let mut h = started();
    play(&mut h, "e2", "e4");
    engine(&mut h, "e7e5");
    play(&mut h, "d1", "h5");
    engine(&mut h, "b8c6");
    play(&mut h, "f1", "c4");
    engine(&mut h, "g8f6");
    play(&mut h, "h5", "f7");
    assert!(h.has_label("Checkmate. You win!"));
    assert!(h.has_label("4. Qxf7#  "));
    assert_eq!(h.outstanding::<EngineMove>(), 0, "nothing left to ask");
    square(&mut h, "a2");
    assert_eq!(h.app.picked, None, "the game is over");
}

#[test]
fn playing_black_the_engine_moves_first_and_can_win() {
    let mut h = started();
    press(&mut h, "♟ New game as Black");
    assert!(h.app.flipped);
    let labels = h.labels();
    let one = labels.iter().position(|l| l == "1").unwrap();
    let eight = labels.iter().position(|l| l == "8").unwrap();
    assert!(one < eight, "Black at the bottom");
    engine(&mut h, "f2f3");
    play(&mut h, "e7", "e5");
    engine(&mut h, "g2g4");
    play(&mut h, "d8", "h4");
    assert!(h.has_label("Checkmate. You win!"));

    // The other way round: GNU Chess mates you.
    press(&mut h, "♟ New game as Black");
    engine(&mut h, "e2e4");
    play(&mut h, "f7", "f6");
    engine(&mut h, "d2d4");
    play(&mut h, "g7", "g5");
    engine(&mut h, "d1h5");
    assert!(h.has_label("Checkmate. GNU Chess wins."));
}

#[test]
fn check_is_announced() {
    let mut h = started();
    load(&mut h, "1. e4 f5 2. Qh5+");
    // It's Black's turn in that game; play it as Black.
    h.app.you = Color::Black;
    h.frame();
    assert!(h.has_label("Check! Your move."));
}

#[test]
fn a_pawn_reaching_the_end_becomes_the_piece_you_choose() {
    let mut h = started();
    load(&mut h, "1. a4 b5 2. axb5 a6 3. bxa6 Bb7 4. axb7 Nc6");
    play(&mut h, "b7", "b8");
    assert!(h.has_label("Promote to"));
    press(&mut h, "Cancel");
    assert!(!h.has_label("Promote to"));
    play(&mut h, "b7", "a8");
    press(&mut h, "♘ Knight");
    let last = *h.app.game.moves().last().unwrap();
    assert_eq!(last.coordinate(), "b7a8n");
    assert!(h.has_label("5. bxa8=N  "));
}

#[test]
fn undo_takes_back_your_move_and_the_reply() {
    let mut h = started();
    play(&mut h, "e2", "e4");
    engine(&mut h, "e7e5");
    play(&mut h, "g1", "f3");
    // While the engine thinks, only your move goes.
    press(&mut h, "↶ Undo");
    assert_eq!(h.app.game.moves().len(), 2);
    // After its reply, both go.
    press(&mut h, "↶ Undo");
    assert!(h.app.game.moves().is_empty());
    press(&mut h, "↶ Undo");
    assert!(h.app.game.moves().is_empty());
}

#[test]
fn the_board_can_be_flipped() {
    let mut h = started();
    press(&mut h, "⇅ Flip board");
    let labels = h.labels();
    let one = labels.iter().position(|l| l == "1").unwrap();
    let eight = labels.iter().position(|l| l == "8").unwrap();
    assert!(one < eight);
    press(&mut h, "⇅ Flip board");
    assert!(!h.app.flipped);
}

#[test]
fn when_the_engine_fails_it_says_why_and_can_try_again() {
    let mut h = started();
    play(&mut h, "e2", "e4");
    h.reply::<EngineMove>(&EngineReply::Failed(
        "GNU Chess took too long and was stopped".into(),
    ));
    h.frame();
    h.frame();
    assert!(h.has_label("GNU Chess couldn't move: GNU Chess took too long and was stopped"));
    assert_eq!(
        h.outstanding::<EngineMove>(),
        0,
        "not asked again by itself"
    );
    press(&mut h, "Try again");
    assert_eq!(h.outstanding::<EngineMove>(), 1);
    h.fail::<EngineMove>(rpc_error::TIMEOUT);
    h.frame();
    h.frame();
    assert!(h.has_label("GNU Chess couldn't move: timed out"));
    // An illegal answer is never played, even if the server passed one on.
    press(&mut h, "Try again");
    engine(&mut h, "e2e4");
    assert!(h.has_label("GNU Chess couldn't move: it answered e2e4, which isn't legal"));
    assert_eq!(h.app.game.moves().len(), 1);
}

#[test]
fn the_engine_may_resign() {
    let mut h = started();
    play(&mut h, "e2", "e4");
    h.reply::<EngineMove>(&EngineReply::Resign);
    h.frame();
    h.frame();
    assert!(h.has_label("GNU Chess resigns. You win!"));
    square(&mut h, "d2");
    assert_eq!(h.app.picked, None, "the game is over");
}

#[test]
fn draws_are_recognised() {
    let mut h = started();
    // Sam Loyd's ten-move stalemate.
    load(
        &mut h,
        "1. e3 a5 2. Qh5 Ra6 3. Qxa5 h5 4. h4 Rah6 5. Qxc7 f6 6. Qxd7+ Kf7 7. Qxb7 Qd3 \
         8. Qxb8 Qh7 9. Qxc8 Kg6 10. Qe6",
    );
    assert!(h.has_label("Stalemate: a draw."));
    load(&mut h, "1. Nf3 Nf6 2. Ng1 Ng8 3. Nf3 Nf6 4. Ng1 Ng8");
    assert!(h.has_label("A draw: the same position three times."));
}

#[test]
fn games_are_saved_as_pgn() {
    let mut h = started();
    press(&mut h, "💾 Save");
    assert!(h.has_label("Type where to save the game first, such as /home/you/game.pgn."));
    assert_eq!(h.outstanding::<WriteFile>(), 0);

    play(&mut h, "e2", "e4");
    engine(&mut h, "e7e5");
    h.type_text(0, "/home/me/game.pgn");
    h.frame();
    press(&mut h, "💾 Save");
    assert_eq!(h.outstanding::<WriteFile>(), 1);
    h.reply::<WriteFile>(&());
    h.frame();
    h.frame();
    assert!(h.has_label("Saved"));

    press(&mut h, "💾 Save");
    h.fail::<WriteFile>(rpc_error::DENIED);
    h.frame();
    h.frame();
    assert!(h.has_label("Could not save: permission denied"));
}

#[test]
fn a_resigned_game_is_saved_as_won() {
    let mut h = started();
    play(&mut h, "e2", "e4");
    h.reply::<EngineMove>(&EngineReply::Resign);
    h.frame();
    h.type_text(0, "/g.pgn");
    h.frame();
    press(&mut h, "💾 Save");
    assert_eq!(h.outstanding::<WriteFile>(), 1);
}

#[test]
fn games_are_opened_from_pgn() {
    let mut h = started();
    press(&mut h, "📂 Open");
    assert!(h.has_label("Type the path of a .pgn file to open."));

    h.type_text(0, "/games/one.pgn");
    h.frame();
    press(&mut h, "📂 Open");
    assert_eq!(h.outstanding::<ReadFile>(), 1);
    h.reply::<ReadFile>(&FileText {
        text: "[White \"GNU Chess\"]\n[Black \"You\"]\n\n1. d4 d5 2. c4 *\n".into(),
        writable: true,
    });
    h.frame();
    h.frame();
    assert!(h.has_label("Opened /games/one.pgn"));
    assert_eq!(
        h.app.you,
        Color::Black,
        "you play the side GNU Chess didn't"
    );
    assert!(h.has_label("1. d4 d5  ") && h.has_label("2. c4  "));
    assert!(h.has_label("Your move (Black)."));
}

#[test]
fn a_file_that_isnt_a_game_or_cant_be_read_says_so() {
    let mut h = started();
    h.type_text(0, "/notes.pgn");
    h.frame();
    press(&mut h, "📂 Open");
    h.reply::<ReadFile>(&FileText {
        text: "1. e5".into(),
        writable: true,
    });
    h.frame();
    h.frame();
    assert!(h.has_label("Could not read /notes.pgn: e5 isn't a legal move in move 1"));

    press(&mut h, "📂 Open");
    h.fail::<ReadFile>(rpc_error::DENIED);
    h.frame();
    h.frame();
    assert!(h.has_label("Could not open /notes.pgn: permission denied"));
}

#[test]
fn an_app_opened_on_a_game_opens_it() {
    let mut h = Harness::new(Chess::default());
    h.launch("/Games/opera.pgn");
    h.frame();
    assert_eq!(h.outstanding::<ReadFile>(), 1);
    assert_eq!(h.text_fields(), ["/Games/opera.pgn"]);
}

#[test]
fn moves_are_listed_in_numbered_pairs() {
    let sans: Vec<String> = ["e4", "e5", "Nf3"].map(String::from).into();
    assert_eq!(move_list(&sans), ["1. e4 e5", "2. Nf3"]);
    assert!(move_list(&[]).is_empty());
}
