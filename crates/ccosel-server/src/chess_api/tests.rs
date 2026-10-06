//! Against a fake engine that speaks enough of the xboard protocol: it logs what it is told and,
//! when told `go`, answers however the test says. So CI needs no GNU Chess; the real one is
//! exercised by `tests/gnuchess.rs`.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::*;

struct Fake {
    dir: PathBuf,
    engine: Engine,
}

impl Fake {
    /// What the engine was told, one command per line.
    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("log")).unwrap_or_default()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A fake engine whose answer to `go` is the shell command `on_go`.
fn fake(name: &str, on_go: &str) -> Fake {
    let dir = std::env::temp_dir().join(format!("ccosel-chess-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("gnuchess");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ \"$1\" = --xboard ] || exit 3\n\
             while read -r line; do\n  echo \"$line\" >> '{log}'\n  case \"$line\" in\n    \
             go) {on_go} ;;\n    quit) exit 0 ;;\n  esac\ndone\n",
            log = dir.join("log").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    Fake {
        engine: Engine::with_program(&script, Duration::from_secs(2)),
        dir,
    }
}

fn ask(engine: &Engine, moves: &str, level: u8) -> Result<EngineReply, u32> {
    engine.best_move(&EngineMoveReq { moves, level })
}

#[test]
fn the_engine_is_told_the_position_and_level_and_its_move_comes_back() {
    let f = fake("move", "echo 'move e7e5'");
    assert_eq!(
        ask(&f.engine, "e2e4", 1),
        Ok(EngineReply::Move("e7e5".into()))
    );
    let log = f.log();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        lines,
        [
            "xboard",
            "protover 2",
            "new",
            "force",
            "setboard rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1",
            "sd 1",
            "st 2",
            "go",
            "quit",
        ]
    );
}

#[test]
fn each_level_searches_deeper_and_the_top_one_only_by_time() {
    assert_eq!(level_commands(1), ["sd 1", "st 2"]);
    assert_eq!(level_commands(3), ["sd 4", "st 2"]);
    assert_eq!(level_commands(4), ["sd 6", "st 2"]);
    assert_eq!(level_commands(MAX_LEVEL), ["st 2"]);
}

#[test]
fn older_engines_and_moves_in_algebraic_notation_are_understood() {
    let f = fake("older", "echo 'My move is : e7e5'");
    assert_eq!(
        ask(&f.engine, "e2e4", 2),
        Ok(EngineReply::Move("e7e5".into()))
    );
    let f = fake("san", "echo 'feature done=1'; echo 'move Nf6'");
    assert_eq!(
        ask(&f.engine, "e2e4", 2),
        Ok(EngineReply::Move("g8f6".into()))
    );
}

#[test]
fn an_illegal_answer_is_never_passed_on() {
    // White's move, when it is Black's turn.
    let f = fake("illegal", "echo 'move d2d4'");
    assert_eq!(
        ask(&f.engine, "e2e4", 2),
        Ok(EngineReply::Failed(
            "GNU Chess answered d2d4, which isn't a legal move here".into()
        ))
    );
}

#[test]
fn a_crash_a_hang_and_a_missing_engine_are_failures_to_show() {
    let f = fake("crash", "exit 1");
    assert_eq!(
        ask(&f.engine, "", 2),
        Ok(EngineReply::Failed(
            "GNU Chess stopped without moving".into()
        ))
    );
    let mut f = fake("hang", "sleep 30");
    f.engine.limit = Duration::from_millis(300);
    let started = Instant::now();
    assert_eq!(
        ask(&f.engine, "", 2),
        Ok(EngineReply::Failed(
            "GNU Chess took too long and was stopped".into()
        ))
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "it was stopped, not waited for"
    );
    let missing = Engine::with_program("/nonexistent/gnuchess", Duration::from_secs(1));
    let Ok(EngineReply::Failed(why)) = ask(&missing, "", 2) else {
        panic!("a missing engine should fail");
    };
    assert!(why.starts_with("GNU Chess couldn't be started"), "{why}");
}

#[test]
fn the_engine_may_resign() {
    let f = fake("resign", "echo resign");
    assert_eq!(ask(&f.engine, "e2e4 e7e5", 3), Ok(EngineReply::Resign));
}

#[test]
fn only_a_legal_game_in_progress_is_asked_about() {
    let f = fake("refuse", "echo 'move e7e5'");
    // Not a level, not a move, not legal, and over already.
    assert_eq!(ask(&f.engine, "", 0), Err(server_error::MALFORMED));
    assert_eq!(
        ask(&f.engine, "", MAX_LEVEL + 1),
        Err(server_error::MALFORMED)
    );
    assert_eq!(ask(&f.engine, "e2", 1), Err(server_error::MALFORMED));
    assert_eq!(ask(&f.engine, "e2e5", 1), Err(server_error::MALFORMED));
    assert_eq!(
        ask(&f.engine, "f2f3 e7e5 g2g4 d8h4", 1),
        Err(server_error::MALFORMED),
        "checkmate: there is no move to make"
    );
    let endless = "g1f3 g8f6 f3g1 f6g8 ".repeat(MAX_PLIES / 4 + 1);
    assert_eq!(ask(&f.engine, &endless, 1), Err(server_error::TOO_LARGE));
    assert_eq!(f.log(), "", "the engine was never started");
}
