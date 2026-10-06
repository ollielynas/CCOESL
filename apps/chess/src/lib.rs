//! The Chess app: play against GNU Chess, which runs on the server.
//!
//! The board, the rules and the game are all here, in the guest (`ccosel-chess`), so a move is
//! checked and shown without a round trip. Only the engine's move goes to the server:
//! `EngineMove` with the game so far and the level. The request names the position exactly, so
//! the request cache does the rest: the same position is asked about once, and a new move is
//! simply a new request.
//!
//! The board is a table of squares built from existing widgets: dark squares are buttons and
//! light ones selectable rows, which gives the two tones. Each square's tooltip is its name
//! (`e4`), which is also how tests click a particular one.

use ccosel_chess::pgn::{self, Tags};
use ccosel_chess::{Color, Game, Kind, Move, Outcome, Square, square_name};
use ccosel_proto::chess::{EngineMove, EngineMoveReq, EngineReply, MAX_LEVEL};
use ccosel_proto::fs::{PathReq, ReadFile, WriteFile, WriteFileReq};
use ccosel_sdk::{App, CallId, Poll, Text, Ui};

/// The level a new game starts at.
pub const DEFAULT_LEVEL: u8 = 2;

/// What GNU Chess is called in the move list and saved games.
const ENGINE: &str = "GNU Chess";

pub struct Chess {
    game: Game,
    /// The side you play.
    you: Color,
    level: u8,
    /// Black at the bottom instead of White.
    flipped: bool,
    /// The square of the piece you picked up.
    picked: Option<Square>,
    /// A pawn move to the last rank, waiting for you to choose what it becomes.
    promoting: Option<(Square, Square)>,
    /// The engine resigned.
    resigned: bool,
    /// The engine couldn't answer, and why. Cleared by Try again.
    engine_failed: Option<String>,
    /// The PGN file Open reads and Save writes.
    path: Text,
    /// A file being read.
    opening: Option<String>,
    saving: Option<CallId>,
    /// A one-line note on the last action: an illegal move, "Saved", or why something failed.
    status: Option<String>,
    /// Something was acted on after this frame was drawn: draw the next one now.
    changed: bool,
}

impl Default for Chess {
    fn default() -> Self {
        Self {
            game: Game::default(),
            you: Color::White,
            level: DEFAULT_LEVEL,
            flipped: false,
            picked: None,
            promoting: None,
            resigned: false,
            engine_failed: None,
            path: Text::new(""),
            opening: None,
            saving: None,
            status: None,
            changed: false,
        }
    }
}

/// What was clicked this frame, acted on once drawing is done.
#[derive(Default, PartialEq)]
struct Actions {
    new_game: Option<Color>,
    level: Option<u8>,
    undo: bool,
    flip: bool,
    square: Option<Square>,
    promote: Option<Kind>,
    cancel_promotion: bool,
    retry: bool,
    open: bool,
    save: bool,
}

/// The game's moves as numbered pairs: `1. e4 e5`, `2. Nf3`.
pub fn move_list(sans: &[String]) -> Vec<String> {
    sans.chunks(2)
        .enumerate()
        .map(|(i, pair)| format!("{}. {}", i + 1, pair.join(" ")))
        .collect()
}

fn side_name(c: Color) -> &'static str {
    match c {
        Color::White => "White",
        Color::Black => "Black",
    }
}

impl Chess {
    fn engine_to_move(&self) -> bool {
        self.game.position().side != self.you && self.game.outcome().is_none() && !self.resigned
    }

    /// The engine request for the position now.
    fn moves(&self) -> String {
        self.game
            .moves()
            .iter()
            .map(|m| m.coordinate())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Where `picked` may go.
    fn targets(&self) -> Vec<Square> {
        let Some(from) = self.picked else {
            return Vec::new();
        };
        self.game
            .position()
            .legal_moves()
            .into_iter()
            .filter(|m| m.from == from)
            .map(|m| m.to)
            .collect()
    }

    fn state_line(&self) -> String {
        if self.resigned {
            return format!("{ENGINE} resigns. You win!");
        }
        match self.game.outcome() {
            Some(Outcome::Checkmate(loser)) if loser == self.you => {
                "Checkmate. GNU Chess wins.".into()
            }
            Some(Outcome::Checkmate(_)) => "Checkmate. You win!".into(),
            Some(Outcome::Stalemate) => "Stalemate: a draw.".into(),
            Some(Outcome::InsufficientMaterial) => {
                "A draw: neither side can checkmate with what is left.".into()
            }
            Some(Outcome::FiftyMoves) => {
                "A draw: fifty moves without a capture or pawn move.".into()
            }
            Some(Outcome::Repetition) => "A draw: the same position three times.".into(),
            None if self.engine_to_move() => format!("{ENGINE} is thinking…"),
            None if self.game.position().in_check() => "Check! Your move.".into(),
            None => format!("Your move ({}).", side_name(self.you)),
        }
    }

    fn board(&self, ui: &mut Ui<'_>, act: &mut Actions) {
        let targets = self.targets();
        let pos = self.game.position();
        let ranks: Vec<u8> = if self.flipped {
            (0..8).collect()
        } else {
            (0..8).rev().collect()
        };
        let files: Vec<u8> = if self.flipped {
            (0..8).rev().collect()
        } else {
            (0..8).collect()
        };
        ui.table(|ui| {
            for &rank in &ranks {
                ui.row(|ui| {
                    ui.label(&format!("{}", rank + 1));
                    for &file in &files {
                        let sq = rank * 8 + file;
                        let piece = pos.at(sq).map(|p| p.glyph());
                        let target = targets.contains(&sq);
                        let text = match (piece, target) {
                            (Some(g), true) => format!("×{g}"),
                            (None, true) => "•".to_owned(),
                            (Some(g), false) if self.picked == Some(sq) => format!("[{g}]"),
                            (Some(g), false) => g.to_string(),
                            (None, false) => " ".to_owned(),
                        };
                        let dark = (rank + file) % 2 == 0;
                        let clicked = if dark {
                            ui.button(&text).clicked()
                        } else {
                            ui.selectable(target || self.picked == Some(sq), &text)
                                .clicked()
                        };
                        ui.tooltip(&square_name(sq));
                        if clicked {
                            act.square = Some(sq);
                        }
                    }
                });
            }
            ui.row(|ui| {
                ui.label("");
                for &file in &files {
                    ui.label(&((b'a' + file) as char).to_string());
                }
            });
        });
    }

    fn promotion(&self, ui: &mut Ui<'_>, act: &mut Actions) {
        if self.promoting.is_none() {
            return;
        }
        ui.horizontal(|ui| {
            ui.label("Promote to");
            for (label, kind) in [
                ("♕ Queen", Kind::Queen),
                ("♖ Rook", Kind::Rook),
                ("♗ Bishop", Kind::Bishop),
                ("♘ Knight", Kind::Knight),
            ] {
                if ui.button(label).clicked() {
                    act.promote = Some(kind);
                }
            }
            act.cancel_promotion = ui.button("Cancel").clicked();
        });
    }

    fn moves_panel(&self, ui: &mut Ui<'_>) {
        let sans = self.game.san_moves();
        if sans.is_empty() {
            return;
        }
        ui.separator();
        ui.label("Moves");
        ui.wrapped(|ui| {
            for line in move_list(&sans) {
                ui.label(&format!("{line}  "));
            }
        });
    }

    /// Ask the engine for its move if it is its turn, and make it when it comes.
    fn poll_engine(&mut self, ui: &mut Ui<'_>) {
        if !self.engine_to_move() || self.engine_failed.is_some() {
            return;
        }
        let moves = self.moves();
        let req = EngineMoveReq {
            moves: &moves,
            level: self.level,
        };
        let reply = match ui.rpc().get::<EngineMove>(&req) {
            Poll::Pending => return,
            Poll::Failed(e) => EngineReply::Failed(e.message().to_owned()),
            Poll::Ready(reply) => (*reply).clone(),
        };
        // Asked afresh if this position comes round again (after an undo, say).
        ui.rpc().invalidate::<EngineMove>(&req);
        self.changed = true;
        match reply {
            EngineReply::Move(m) => {
                // Checked by the server, and again here: the board only ever holds legal moves.
                match Move::parse_coordinate(&m).filter(|&m| self.game.play(m)) {
                    Some(_) => {}
                    None => {
                        self.engine_failed = Some(format!("it answered {m}, which isn't legal"));
                    }
                }
            }
            EngineReply::Resign => self.resigned = true,
            EngineReply::Failed(why) => self.engine_failed = Some(why),
        }
    }

    fn poll_files(&mut self, ui: &mut Ui<'_>) {
        if let Some(path) = self.opening.clone() {
            let rpc = ui.rpc();
            match rpc.get::<ReadFile>(&PathReq { path: &path }) {
                Poll::Pending => {}
                Poll::Ready(file) => {
                    rpc.invalidate::<ReadFile>(&PathReq { path: &path });
                    self.opening = None;
                    self.changed = true;
                    match pgn::read(&file.text) {
                        Ok((game, tags)) => {
                            // You play whichever side GNU Chess didn't, if the file says.
                            self.you = if tags.white == ENGINE {
                                Color::Black
                            } else {
                                Color::White
                            };
                            self.flipped = self.you == Color::Black;
                            self.start(game);
                            self.status = Some(format!("Opened {path}"));
                        }
                        Err(why) => self.status = Some(format!("Could not read {path}: {why}")),
                    }
                }
                Poll::Failed(e) => {
                    rpc.invalidate::<ReadFile>(&PathReq { path: &path });
                    self.opening = None;
                    self.changed = true;
                    self.status = Some(format!("Could not open {path}: {}", e.message()));
                }
            }
        }
        if let Some(id) = self.saving {
            match ui.rpc().outcome::<WriteFile>(id) {
                Poll::Pending => {}
                Poll::Ready(_) => {
                    self.saving = None;
                    self.changed = true;
                    self.status = Some("Saved".to_owned());
                }
                Poll::Failed(e) => {
                    self.saving = None;
                    self.changed = true;
                    self.status = Some(format!("Could not save: {}", e.message()));
                }
            }
        }
    }

    fn start(&mut self, game: Game) {
        self.game = game;
        self.picked = None;
        self.promoting = None;
        self.resigned = false;
        self.engine_failed = None;
    }

    /// You clicked `sq`: pick up a piece, put it down, or pick up another.
    fn click_square(&mut self, sq: Square) {
        if self.engine_to_move() || self.game.outcome().is_some() || self.resigned {
            return;
        }
        let pos = self.game.position();
        let mine = pos.at(sq).is_some_and(|p| p.color == self.you);
        let Some(from) = self.picked else {
            if mine {
                self.picked = Some(sq);
                self.status = None;
            }
            return;
        };
        if mine {
            // Another of your pieces, or the same one again to put it back.
            self.picked = (from != sq).then_some(sq);
            return;
        }
        let candidates: Vec<Move> = pos
            .legal_moves()
            .into_iter()
            .filter(|m| m.from == from && m.to == sq)
            .collect();
        match candidates.as_slice() {
            [] => {
                self.status = Some(format!(
                    "{} can't move to {}.",
                    square_name(from),
                    square_name(sq)
                ));
            }
            [m] => {
                self.game.play(*m);
                self.picked = None;
                self.status = None;
            }
            // Several moves from and to the same squares: a promotion, so ask what to.
            _ => {
                self.promoting = Some((from, sq));
                self.picked = None;
            }
        }
    }

    fn apply(&mut self, ui: &mut Ui<'_>, act: Actions) {
        self.changed |= act != Actions::default();
        if let Some(color) = act.new_game {
            self.you = color;
            self.flipped = color == Color::Black;
            self.start(Game::default());
            self.status = None;
        }
        if let Some(level) = act.level {
            self.level = level;
        }
        if act.flip {
            self.flipped = !self.flipped;
        }
        if act.undo {
            // Back to your move: your last move goes, and the engine's reply to it if it came.
            self.game.undo();
            if self.game.position().side != self.you {
                self.game.undo();
            }
            self.picked = None;
            self.promoting = None;
            self.resigned = false;
            self.engine_failed = None;
            self.status = None;
        }
        if act.retry {
            self.engine_failed = None;
        }
        if let Some(sq) = act.square {
            self.click_square(sq);
        }
        if let (Some(kind), Some((from, to))) = (act.promote, self.promoting) {
            self.game.play(Move {
                from,
                to,
                promotion: Some(kind),
            });
            self.promoting = None;
        }
        if act.cancel_promotion {
            self.promoting = None;
        }
        if act.open {
            let path = self.path.as_str().trim().to_owned();
            if path.is_empty() {
                self.status = Some("Type the path of a .pgn file to open.".to_owned());
            } else {
                self.opening = Some(path);
            }
        }
        if act.save && self.saving.is_none() {
            let path = self.path.as_str().trim();
            if path.is_empty() {
                self.status =
                    Some("Type where to save the game first, such as /home/you/game.pgn.".into());
            } else {
                let (white, black) = match self.you {
                    Color::White => ("You", ENGINE),
                    Color::Black => (ENGINE, "You"),
                };
                let tags = Tags {
                    white: white.into(),
                    black: black.into(),
                    date: String::new(),
                };
                let text = if self.resigned {
                    // PGN has no resignation of its own: the result says who won.
                    let result = if self.you == Color::White {
                        "1-0"
                    } else {
                        "0-1"
                    };
                    pgn::write_with_result(&self.game, &tags, result)
                } else {
                    pgn::write(&self.game, &tags)
                };
                self.saving = Some(ui.rpc().send::<WriteFile>(&WriteFileReq {
                    path,
                    text: &text,
                    create_only: false,
                }));
                self.status = None;
            }
        }
    }
}

impl App for Chess {
    fn update(&mut self, ui: &mut Ui<'_>) {
        self.changed = false;
        self.poll_engine(ui);
        self.poll_files(ui);

        // Each part under its own id, so a status line appearing above the board doesn't change
        // the ids of the squares below it.
        let mut act = Actions::default();
        ui.push_id("toolbar", |ui| {
            ui.horizontal(|ui| {
                if ui.button("♙ New game as White").clicked() {
                    act.new_game = Some(Color::White);
                }
                if ui.button("♟ New game as Black").clicked() {
                    act.new_game = Some(Color::Black);
                }
                act.undo = ui.button("↶ Undo").clicked();
                ui.tooltip("Take back your last move and GNU Chess's reply");
                act.flip = ui.button("⇅ Flip board").clicked();
            });
            ui.horizontal(|ui| {
                ui.label("Level");
                for level in 1..=MAX_LEVEL {
                    if ui
                        .selectable(level == self.level, &format!("{level}"))
                        .clicked()
                    {
                        act.level = Some(level);
                    }
                }
                ui.tooltip("1 is the easiest; 5 thinks for two seconds a move");
            });
            ui.horizontal(|ui| {
                ui.label("Game file");
                ui.text_edit(&mut self.path);
                ui.tooltip("A .pgn file on the server, such as /home/you/game.pgn");
                act.open = ui.button("📂 Open").clicked();
                act.save = ui.button("💾 Save").clicked();
            });
        });
        ui.push_id("state", |ui| {
            ui.label(&self.state_line());
            if let Some(why) = &self.engine_failed {
                ui.horizontal(|ui| {
                    ui.label(&format!("{ENGINE} couldn't move: {why}"));
                    act.retry = ui.button("Try again").clicked();
                });
            }
            if let Some(status) = &self.status {
                ui.label(status);
            }
        });
        ui.push_id("board", |ui| self.board(ui, &mut act));
        ui.push_id("promotion", |ui| self.promotion(ui, &mut act));
        ui.push_id("moves", |ui| self.moves_panel(ui));
        self.apply(ui, act);
    }

    /// Opened on a `.pgn` file: open that game.
    fn open(&mut self, arg: &str) {
        self.path.set(arg);
        self.opening = Some(arg.to_owned());
    }

    fn wants_repaint_after_ms(&self) -> u32 {
        if self.changed {
            0
        } else {
            // Waiting for the engine is waiting on a reply, which wakes the app by itself.
            ccosel_sdk::REPAINT_ON_INPUT_ONLY
        }
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(Chess);
