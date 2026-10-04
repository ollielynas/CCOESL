//! Against a fake GTP engine, a shell script, so CI needs no GNU Go. The real one is in
//! `tests/go_gnugo.rs`, run with `CCOSEL_TEST_GNUGO=1`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_proto::go::{Game, Stone, parse_vertex};

use super::*;

/// A fake GNU Go answering `genmove` with `genmove`, `final_score` with `score` and
/// `final_status_list` with `dead`. It writes its arguments and every command it receives to
/// `log`. `genmove` may also be `CRASH` (exit) or `HANG` (never answer).
struct Fake {
    dir: PathBuf,
}

impl Fake {
    fn new(genmove: &str, score: &str, dead: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ccosel-gtp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let genmove = match genmove {
            "CRASH" => "exit 3".to_owned(),
            "HANG" => "sleep 30".to_owned(),
            reply => format!("printf '= {reply}\\n\\n'"),
        };
        let script = format!(
            r#"echo "$@" > "{log}"
while read -r line; do
  echo "$line" >> "{log}"
  case "$line" in
    genmove*) {genmove} ;;
    final_score*) printf '= {score}\n\n' ;;
    final_status_list*) printf '= {dead}\n\n' ;;
    boardsize\ 4) printf '? unacceptable size\n\n' ;;
    quit*) printf '=\n\n'; exit 0 ;;
    *) printf '=\n\n' ;;
  esac
done
"#,
            log = dir.join("log").display()
        );
        std::fs::write(dir.join("gnugo.sh"), script).unwrap();
        Self { dir }
    }

    fn engines(&self, limit: Duration) -> GoEngines {
        GoEngines::new(
            vec!["sh".into(), self.dir.join("gnugo.sh").display().to_string()],
            limit,
        )
    }

    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn game(size: u8, moves: &[&str]) -> Game {
    let mut g = Game::new(size, 13, 0);
    g.moves = moves
        .iter()
        .map(|m| parse_vertex(m, size).unwrap())
        .collect();
    g
}

fn wait(engines: &GoEngines, req: &EngineReq) -> Result<Answer, String> {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let status = engines.ask(req).unwrap();
        if let Some(answer) = status.answer {
            assert!(status.finished);
            return answer;
        }
        assert!(Instant::now() < until, "never answered");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn move_req(g: Game, level: u8) -> EngineReq {
    EngineReq {
        game: g,
        ask: Ask::Move { level },
    }
}

#[test]
fn asks_for_a_move_after_replaying_the_game() {
    let fake = Fake::new("C3", "", "");
    let engines = fake.engines(Duration::from_secs(10));
    let req = move_req(game(9, &["E5", "D4"]), 7);
    assert_eq!(wait(&engines, &req), Ok(Answer::Move(Move::Play(2, 6))));
    let log = fake.log();
    assert_eq!(
        log[..7],
        [
            // The fake is started as `sh gnugo.sh`; the real command adds `--mode gtp`.
            "--level 7",
            "boardsize 9",
            "clear_board",
            "komi 6.5",
            "play black E5",
            "play white D4",
            "genmove black",
        ],
        "and then `quit`, which may or may not be read before it is stopped"
    );
}

#[test]
fn handicap_stones_are_placed_and_white_moves_first() {
    let fake = Fake::new("pass", "", "");
    let engines = fake.engines(Duration::from_secs(10));
    let mut g = Game::new(19, 1, 2);
    g.moves.push(Move::Play(9, 9));
    assert_eq!(
        wait(&engines, &move_req(g, 1)),
        Ok(Answer::Move(Move::Pass))
    );
    let log = fake.log();
    assert!(
        log.contains(&"set_free_handicap D4 Q16".to_owned()),
        "{log:?}"
    );
    assert!(log.contains(&"komi 0.5".to_owned()));
    assert!(log.contains(&"play white K10".to_owned()));
    assert!(log.contains(&"genmove black".to_owned()));
}

#[test]
fn a_resignation_is_an_answer() {
    let fake = Fake::new("resign", "", "");
    let engines = fake.engines(Duration::from_secs(10));
    assert_eq!(
        wait(&engines, &move_req(game(9, &[]), 1)),
        Ok(Answer::Resign)
    );
}

#[test]
fn an_illegal_or_unreadable_engine_move_is_an_error_not_a_move() {
    for reply in ["E5", "Z99", "K1", "nonsense"] {
        let fake = Fake::new(reply, "", "");
        let engines = fake.engines(Duration::from_secs(10));
        let answer = wait(&engines, &move_req(game(9, &["E5"]), 1));
        assert_eq!(
            answer,
            Err(format!(
                "GNU Go answered “{reply}”, which isn't a legal move here"
            )),
            "{reply}"
        );
    }
}

#[test]
fn a_crash_or_a_hang_is_an_error_the_app_can_show() {
    let fake = Fake::new("CRASH", "", "");
    let answer = wait(
        &fake.engines(Duration::from_secs(10)),
        &move_req(game(9, &[]), 1),
    );
    assert_eq!(answer, Err("GNU Go it stopped unexpectedly".to_owned()));

    let fake = Fake::new("HANG", "", "");
    let started = Instant::now();
    let answer = wait(
        &fake.engines(Duration::from_millis(300)),
        &move_req(game(9, &[]), 1),
    );
    assert_eq!(
        answer,
        Err("GNU Go it took too long and was stopped".to_owned())
    );
    assert!(started.elapsed() < Duration::from_secs(5));

    let engines = GoEngines::new(vec!["/no/such/gnugo".into()], Duration::from_secs(1));
    let answer = wait(&engines, &move_req(game(9, &[]), 1)).unwrap_err();
    assert!(answer.contains("is it installed?"), "{answer}");
}

#[test]
fn the_score_and_dead_stones_are_read() {
    let fake = Fake::new("", "W+7.5", "C3 D4");
    let engines = fake.engines(Duration::from_secs(10));
    let req = EngineReq {
        game: game(9, &["pass", "pass"]),
        ask: Ask::Score,
    };
    assert_eq!(
        wait(&engines, &req),
        Ok(Answer::Score {
            winner: Some(Stone::White),
            margin_x2: 15,
            dead: vec![(2, 6), (3, 5)]
        })
    );
    assert!(fake.log().contains(&"final_status_list dead".to_owned()));

    let fake = Fake::new("", "0", "");
    let answer = wait(&fake.engines(Duration::from_secs(10)), &req);
    assert_eq!(
        answer,
        Ok(Answer::Score {
            winner: None,
            margin_x2: 0,
            dead: vec![]
        })
    );

    let fake = Fake::new("", "who knows", "");
    let answer = wait(&fake.engines(Duration::from_secs(10)), &req).unwrap_err();
    assert!(answer.contains("couldn't be read"), "{answer}");
}

#[test]
fn bad_requests_are_refused_before_any_engine_starts() {
    let fake = Fake::new("C3", "", "");
    let engines = fake.engines(Duration::from_secs(10));
    for req in [
        move_req(game(9, &["E5", "E5"]), 1),
        move_req(game(9, &[]), 0),
        move_req(game(9, &[]), 11),
        move_req(game(9, &["pass", "pass"]), 5),
        move_req(Game::new(4, 13, 0), 1),
    ] {
        assert_eq!(engines.ask(&req), Err(server_error::MALFORMED), "{req:?}");
    }
    assert!(fake.log().is_empty(), "no engine was started");
}

#[test]
fn polling_the_same_position_starts_one_engine() {
    let fake = Fake::new("C3", "", "");
    let engines = fake.engines(Duration::from_secs(10));
    let req = move_req(game(9, &[]), 3);
    wait(&engines, &req).unwrap();
    std::fs::remove_file(fake.dir.join("log")).unwrap();
    assert_eq!(wait(&engines, &req), Ok(Answer::Move(Move::Play(2, 6))));
    assert!(fake.log().is_empty(), "answered from the finished job");
}

#[test]
fn a_refused_command_is_reported() {
    // Not reachable through `ask`, which refuses a 4×4 board first: driven directly.
    let fake = Fake::new("C3", "", "");
    let argv = vec!["sh".into(), fake.dir.join("gnugo.sh").display().to_string()];
    let mut engine = LineEngine::start(&argv, Duration::from_secs(5)).unwrap();
    assert_eq!(
        gtp(&mut engine, "boardsize 4"),
        Err("GNU Go refused “boardsize 4”: unacceptable size".to_owned())
    );
}

#[test]
fn scores_read_as_gnu_go_writes_them() {
    assert_eq!(parse_score("B+3"), Some((Some(Stone::Black), 6)));
    assert_eq!(parse_score("w+0.5"), Some((Some(Stone::White), 1)));
    assert_eq!(parse_score("0"), Some((None, 0)));
    assert_eq!(parse_score("X+1"), None);
    assert_eq!(parse_score("B+-1"), None);
    assert_eq!(parse_score("B3"), None);
}
