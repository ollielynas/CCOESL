//! GNU Chess's move, for the Chess app.
//!
//! Each call starts `gnuchess --xboard`, sets up the position the request describes, asks for a
//! move, checks it is legal, and stops the engine. Nothing is kept between calls, so there is
//! no process per player to leak, and a crash costs one move, not a game.
//!
//! The position is replayed here with `ccosel-chess` first, so a request whose moves aren't a
//! legal game never reaches the engine, and the engine's answer is checked with the same rules
//! the app plays by.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ccosel_chess::{Game, Move};
use ccosel_proto::chess::{EngineMoveReq, EngineReply, MAX_LEVEL, MAX_THINK_SECONDS};
use ccosel_proto::server_error;

/// How long the engine gets to start and answer, beyond its thinking time, before it is stopped.
const SLACK: Duration = Duration::from_secs(5);

/// The longest game a request may describe, in half-moves. Far longer than real games; what a
/// request costs to replay stays small.
pub const MAX_PLIES: usize = 1_000;

/// How to run GNU Chess.
pub struct Engine {
    program: OsString,
    limit: Duration,
}

impl Default for Engine {
    /// `gnuchess` from `PATH`, or the program `CCOSEL_GNUCHESS` names.
    fn default() -> Self {
        Self::with_program(
            std::env::var_os("CCOSEL_GNUCHESS").unwrap_or_else(|| "gnuchess".into()),
            Duration::from_secs(MAX_THINK_SECONDS.into()) + SLACK,
        )
    }
}

/// The xboard commands that make a level: a search depth (none at the top level), and always
/// the time limit.
fn level_commands(level: u8) -> Vec<String> {
    let depth = match level {
        1 => Some(1),
        2 => Some(2),
        3 => Some(4),
        4 => Some(6),
        _ => None,
    };
    let mut out: Vec<String> = depth.map(|d| format!("sd {d}")).into_iter().collect();
    out.push(format!("st {MAX_THINK_SECONDS}"));
    out
}

impl Engine {
    /// Runs `program` instead of GNU Chess, stopping it after `limit`. For tests.
    pub fn with_program(program: impl Into<OsString>, limit: Duration) -> Self {
        Self {
            program: program.into(),
            limit,
        }
    }

    /// The engine's move in the position `req` describes. A request that isn't a legal game in
    /// progress is refused; anything that goes wrong with the engine is a [`EngineReply::Failed`]
    /// for the app to show.
    pub fn best_move(&self, req: &EngineMoveReq<'_>) -> Result<EngineReply, u32> {
        if !(1..=MAX_LEVEL).contains(&req.level) {
            return Err(server_error::MALFORMED);
        }
        if req.moves.split_whitespace().count() > MAX_PLIES {
            return Err(server_error::TOO_LARGE);
        }
        let mut game = Game::default();
        for word in req.moves.split_whitespace() {
            let m = Move::parse_coordinate(word).ok_or(server_error::MALFORMED)?;
            if !game.play(m) {
                return Err(server_error::MALFORMED);
            }
        }
        if game.outcome().is_some() {
            return Err(server_error::MALFORMED);
        }
        Ok(self.ask(&game, req.level))
    }

    fn ask(&self, game: &Game, level: u8) -> EngineReply {
        let mut child = match Command::new(&self.program)
            .arg("--xboard")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                return EngineReply::Failed(format!(
                    "GNU Chess couldn't be started ({e}); is it installed?"
                ));
            }
        };
        let reply = self.converse(&mut child, game, level);
        let _ = child.kill();
        let _ = child.wait();
        reply
    }

    fn converse(&self, child: &mut Child, game: &Game, level: u8) -> EngineReply {
        let pos = game.position();
        let mut script = vec![
            "xboard".to_owned(),
            "protover 2".to_owned(),
            "new".to_owned(),
            // Don't move until asked, whatever the position.
            "force".to_owned(),
            format!("setboard {}", pos.fen()),
        ];
        script.extend(level_commands(level));
        script.push("go".to_owned());
        let mut stdin = child.stdin.take().expect("piped");
        for line in &script {
            if writeln!(stdin, "{line}").is_err() {
                return EngineReply::Failed("GNU Chess stopped before it was asked".into());
            }
        }
        let _ = stdin.flush();

        // Lines arrive on a thread of their own, so waiting for one can time out.
        let stdout = child.stdout.take().expect("piped");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let deadline = Instant::now() + self.limit;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = match rx.recv_timeout(left) {
                Ok(line) => line,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return EngineReply::Failed("GNU Chess took too long and was stopped".into());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return EngineReply::Failed("GNU Chess stopped without moving".into());
                }
            };
            let line = line.trim();
            if line == "resign" || line.starts_with("tellics resign") {
                let _ = writeln!(stdin, "quit");
                return EngineReply::Resign;
            }
            // `move e7e5`; older versions also say `My move is : e7e5`.
            let said = line
                .strip_prefix("move ")
                .or_else(|| line.strip_prefix("My move is : "))
                .or_else(|| line.strip_prefix("My move is: "));
            if let Some(said) = said {
                let _ = writeln!(stdin, "quit");
                let said = said.trim();
                return match pos.legal_coordinate(said).or_else(|| pos.parse_san(said)) {
                    Some(m) => EngineReply::Move(m.coordinate()),
                    None => EngineReply::Failed(format!(
                        "GNU Chess answered {said}, which isn't a legal move here"
                    )),
                };
            }
        }
    }
}

#[cfg(test)]
mod tests;
