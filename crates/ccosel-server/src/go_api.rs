//! The Go app's opponent: GNU Go, asked over GTP for its move or for the score.
//!
//! Each question is a job, polled like a build, keyed by the whole request (the game and the
//! question), so the same position is only ever worked out once. For each one the server
//! starts `gnugo --mode gtp`, sets up the board, replays the moves, asks, checks the answer
//! against the rules in `ccosel_proto::go`, and stops it. No engine outlives its question.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ccosel_proto::go::{
    Answer, Ask, EngineReq, EngineStatus, Game, MAX_LEVEL, Move, Stone, handicap_points, komi_text,
    parse_vertex, vertex,
};
use ccosel_proto::server_error;

use crate::engine::{EngineError, LineEngine};

/// How many engines may run at once, so a burst of games can't take over the machine.
const MAX_RUNNING: usize = 4;

/// Answers are kept this long for a client that comes back for one.
const RETENTION: Duration = Duration::from_secs(10 * 60);

struct Job {
    answer: Mutex<Option<(Result<Answer, String>, Instant)>>,
}

pub struct GoEngines {
    /// The engine and its fixed arguments; `--level N` is added for a move.
    command: Vec<String>,
    limit: Duration,
    jobs: Mutex<HashMap<EngineReq, Arc<Job>>>,
}

impl Default for GoEngines {
    fn default() -> Self {
        Self::new(
            ["gnugo", "--mode", "gtp"].map(String::from).to_vec(),
            Duration::from_secs(60),
        )
    }
}

impl GoEngines {
    /// Engines started as `command`, each given `limit` to answer.
    pub fn new(command: Vec<String>, limit: Duration) -> Self {
        Self {
            command,
            limit,
            jobs: Mutex::new(HashMap::new()),
        }
    }

    /// Start working out `req` if it is new, and say where it has got to. `MALFORMED` for a
    /// game that breaks the rules, or a question it can't be asked.
    pub fn ask(&self, req: &EngineReq) -> Result<EngineStatus, u32> {
        let position = req.game.replay().map_err(|_| server_error::MALFORMED)?;
        if let Ask::Move { level } = req.ask
            && (level == 0 || level > MAX_LEVEL || position.is_over())
        {
            return Err(server_error::MALFORMED);
        }
        let mut jobs = self.jobs.lock().unwrap();
        jobs.retain(|_, job| {
            job.answer
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|(_, at)| at.elapsed() < RETENTION)
        });
        let job = match jobs.get(req) {
            Some(job) => job.clone(),
            None => {
                let running = jobs
                    .values()
                    .filter(|j| j.answer.lock().unwrap().is_none())
                    .count();
                if running >= MAX_RUNNING {
                    return Ok(EngineStatus {
                        finished: true,
                        answer: Some(Err("the server is busy with other games; try again".into())),
                    });
                }
                let job = Arc::new(Job {
                    answer: Mutex::new(None),
                });
                jobs.insert(req.clone(), job.clone());
                let (worker, req) = (job.clone(), req.clone());
                let (command, limit) = (self.command.clone(), self.limit);
                std::thread::spawn(move || {
                    let answer = run(&command, limit, &req);
                    *worker.answer.lock().unwrap() = Some((answer, Instant::now()));
                });
                job
            }
        };
        let answer = job.answer.lock().unwrap().as_ref().map(|(a, _)| a.clone());
        Ok(EngineStatus {
            finished: answer.is_some(),
            answer,
        })
    }
}

fn colour(stone: Stone) -> &'static str {
    match stone {
        Stone::Black => "black",
        Stone::White => "white",
    }
}

/// Ask GNU Go about `req`: start it, set up the game, ask, check the answer.
fn run(command: &[String], limit: Duration, req: &EngineReq) -> Result<Answer, String> {
    let mut argv = command.to_vec();
    if let Ask::Move { level } = req.ask {
        argv.extend(["--level".into(), level.to_string()]);
    }
    let failed = |e: EngineError| format!("GNU Go {e}");
    let mut engine = LineEngine::start(&argv, limit).map_err(failed)?;
    let game = &req.game;
    setup(&mut engine, game)?;
    let answer = match req.ask {
        Ask::Move { .. } => {
            let to_move = game.color_of(game.moves.len());
            let said = gtp(&mut engine, &format!("genmove {}", colour(to_move)))?;
            parse_move(&said, game)?
        }
        Ask::Score => {
            let said = gtp(&mut engine, "final_score")?;
            let (winner, margin_x2) = parse_score(&said)
                .ok_or_else(|| format!("GNU Go gave a score that couldn't be read: “{said}”"))?;
            let said = gtp(&mut engine, "final_status_list dead")?;
            let dead = said
                .split_whitespace()
                .map(|v| match parse_vertex(v, game.size) {
                    Some(Move::Play(x, y)) => Ok((x, y)),
                    _ => Err(format!(
                        "GNU Go named a point that isn't on the board: “{v}”"
                    )),
                })
                .collect::<Result<_, _>>()?;
            Answer::Score {
                winner,
                margin_x2,
                dead,
            }
        }
    };
    let _ = engine.send("quit");
    Ok(answer)
}

/// Board size, komi, handicap stones, then every move so far.
fn setup(engine: &mut LineEngine, game: &Game) -> Result<(), String> {
    gtp(engine, &format!("boardsize {}", game.size))?;
    gtp(engine, "clear_board")?;
    gtp(engine, &format!("komi {}", komi_text(game.komi_x2)))?;
    if game.handicap >= 2 {
        let stones: Vec<String> = handicap_points(game.size, game.handicap)
            .into_iter()
            .map(|(x, y)| vertex(Move::Play(x, y), game.size))
            .collect();
        gtp(engine, &format!("set_free_handicap {}", stones.join(" ")))?;
    }
    for (n, &mv) in game.moves.iter().enumerate() {
        let who = colour(game.color_of(n));
        gtp(engine, &format!("play {who} {}", vertex(mv, game.size)))?;
    }
    Ok(())
}

/// Send one GTP command and return its answer: the text after `=`, up to the blank line that
/// ends it. A `?` answer is GNU Go refusing the command.
fn gtp(engine: &mut LineEngine, command: &str) -> Result<String, String> {
    let failed = |e: EngineError| format!("GNU Go {e}");
    engine.send(command).map_err(failed)?;
    let mut first = engine.read_line().map_err(failed)?;
    while first.trim().is_empty() {
        first = engine.read_line().map_err(failed)?;
    }
    // Everything after the `=` or `?`, which may be followed by a command id.
    let rest = first.char_indices().nth(1).map_or("", |(i, _)| &first[i..]);
    let mut text = rest
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim()
        .to_owned();
    loop {
        let line = engine.read_line().map_err(failed)?;
        if line.trim().is_empty() {
            break;
        }
        text.push('\n');
        text.push_str(line.trim());
    }
    match first.chars().next() {
        Some('=') => Ok(text),
        Some('?') => Err(format!("GNU Go refused “{command}”: {text}")),
        _ => Err(format!(
            "GNU Go answered something that isn't GTP: “{first}”"
        )),
    }
}

/// GNU Go's move, if it is one the rules allow.
fn parse_move(said: &str, game: &Game) -> Result<Answer, String> {
    if said.eq_ignore_ascii_case("resign") {
        return Ok(Answer::Resign);
    }
    let illegal = || format!("GNU Go answered “{said}”, which isn't a legal move here");
    let mv = parse_vertex(said, game.size).ok_or_else(illegal)?;
    let position = game.replay().map_err(|_| illegal())?;
    position.check(mv).map_err(|_| illegal())?;
    Ok(Answer::Move(mv))
}

/// `W+7.5`, `B+3` or `0`: the winner and the margin in half points.
fn parse_score(said: &str) -> Option<(Option<Stone>, u16)> {
    let said = said.trim();
    if said == "0" {
        return Some((None, 0));
    }
    let (who, margin) = said.split_once('+')?;
    let winner = match who {
        "B" | "b" => Stone::Black,
        "W" | "w" => Stone::White,
        _ => return None,
    };
    let margin_x2 = ccosel_proto::go::parse_komi(margin)?;
    Some((Some(winner), u16::try_from(margin_x2).ok()?))
}

#[cfg(test)]
mod tests;
