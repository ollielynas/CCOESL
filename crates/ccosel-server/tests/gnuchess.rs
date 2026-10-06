//! The engine against the real `gnuchess`: it answers with a legal move at every level, from
//! the start and from the middle of a game, and finds a mate in one.
//!
//! GNU Chess isn't installed on the CI runner, so this only runs when asked to:
//!
//! ```sh
//! CCOSEL_TEST_GNUCHESS=1 cargo test -p ccosel-server --test gnuchess
//! ```
//!
//! Without the variable it prints that it was skipped and passes. Once the runner has
//! `gnuchess`, CI can set the variable for this test and it becomes a real gate.

use ccosel_chess::{Game, Move};
use ccosel_proto::chess::{EngineMoveReq, EngineReply, MAX_LEVEL};
use ccosel_server::chess_api::Engine;

fn enabled() -> bool {
    let on = std::env::var_os("CCOSEL_TEST_GNUCHESS").is_some_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set CCOSEL_TEST_GNUCHESS=1 to run this against the real gnuchess");
    }
    on
}

fn legal_after(moves: &str, reply: &EngineReply) -> bool {
    let EngineReply::Move(m) = reply else {
        return false;
    };
    let mut game = Game::default();
    for w in moves.split_whitespace() {
        assert!(game.play(Move::parse_coordinate(w).unwrap()));
    }
    game.position().legal_coordinate(m).is_some()
}

#[test]
fn every_level_answers_with_a_legal_move() {
    if !enabled() {
        return;
    }
    let engine = Engine::default();
    for moves in ["", "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3"] {
        for level in 1..=MAX_LEVEL {
            let reply = engine.best_move(&EngineMoveReq { moves, level }).unwrap();
            assert!(
                legal_after(moves, &reply),
                "level {level} after {moves:?}: {reply:?}"
            );
        }
    }
}

#[test]
fn it_finds_a_mate_in_one() {
    if !enabled() {
        return;
    }
    // 1. e4 e5 2. Qh5 Nc6 3. Bc4 Nf6??: Qxf7 is mate.
    let moves = "e2e4 e7e5 d1h5 b8c6 f1c4 g8f6";
    let reply = Engine::default()
        .best_move(&EngineMoveReq { moves, level: 3 })
        .unwrap();
    assert_eq!(reply, EngineReply::Move("h5f7".into()));
}
