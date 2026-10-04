//! The Go app's engine against the real GNU Go. CI has no `gnugo` yet, so this runs only with
//! `CCOSEL_TEST_GNUGO=1`; without it, it says so and checks nothing:
//!
//! ```sh
//! CCOSEL_TEST_GNUGO=1 cargo test -p ccosel-server --test go_gnugo -- --nocapture
//! ```

use std::time::{Duration, Instant};

use ccosel_proto::go::{Answer, Ask, EngineReq, Game, Move};
use ccosel_server::go_api::GoEngines;

fn enabled() -> bool {
    let on = std::env::var("CCOSEL_TEST_GNUGO").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set CCOSEL_TEST_GNUGO=1 to run against the real GNU Go");
    }
    on
}

fn ask(engines: &GoEngines, req: &EngineReq) -> Answer {
    let until = Instant::now() + Duration::from_secs(120);
    loop {
        let status = engines.ask(req).unwrap();
        if let Some(answer) = status.answer {
            return answer.unwrap();
        }
        assert!(Instant::now() < until, "GNU Go never answered");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn gnu_go_plays_a_legal_move_with_and_without_handicap() {
    if !enabled() {
        return;
    }
    let engines = GoEngines::default();
    for (size, handicap) in [(9, 0), (13, 3), (19, 9)] {
        let mut game = Game::new(size, 13, handicap);
        game.moves.push(Move::Play(size / 2 - 1, size / 2 + 1));
        let answer = ask(
            &engines,
            &EngineReq {
                game: game.clone(),
                ask: Ask::Move { level: 1 },
            },
        );
        let Answer::Move(mv) = answer else {
            panic!("{size}: {answer:?}");
        };
        // Already checked against the rules by the server; check it again here.
        game.replay().unwrap().check(mv).unwrap();
    }
}

#[test]
fn gnu_go_scores_a_finished_game() {
    if !enabled() {
        return;
    }
    let mut game = Game::new(9, 13, 0);
    game.moves = vec![Move::Play(4, 4), Move::Pass, Move::Pass];
    let answer = ask(
        &GoEngines::default(),
        &EngineReq {
            game,
            ask: Ask::Score,
        },
    );
    // One black stone on an empty board: Black owns it all, 81 - 6.5.
    assert!(
        matches!(
            answer,
            Answer::Score {
                winner: Some(ccosel_proto::go::Stone::Black),
                ..
            }
        ),
        "{answer:?}"
    );
}
