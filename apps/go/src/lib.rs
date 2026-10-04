//! The Go app: play GNU Go on the server, on a 9×9, 13×13 or 19×19 board.
//!
//! The rules live in `ccosel_proto::go`, shared with the server: this app refuses an illegal
//! move the moment it is clicked, and the server checks the same rules again. GNU Go's move is a
//! `GoEngine` job keyed by the whole game, so asking about the same position twice costs one
//! engine run, and taking a move back and playing it again is answered from the cache.

use ccosel_proto::fs::{PathReq, ReadFile, WriteFile, WriteFileReq};
use ccosel_proto::go::{
    Answer, Ask, EngineReq, Game, GoEngine, MAX_KOMI_X2, MAX_LEVEL, Move, SIZES, Stone, column,
    from_sgf, komi_text, max_handicap, to_sgf, vertex,
};
use ccosel_sdk::{App, CallId, Poll, Text, TextStyle, Ui};

/// How often a move being worked out is asked about.
pub const POLL_MS: u32 = 250;

/// What a point shows: a stone, a stone counted dead, or nothing.
pub const BLACK: &str = "⚫";
pub const WHITE: &str = "⚪";
pub const DEAD: &str = "×";
pub const EMPTY: &str = "·";

pub const PASS: &str = "Pass";
pub const RESIGN: &str = "Resign";
pub const UNDO: &str = "Undo";
pub const NEW_GAME: &str = "New game";
pub const START: &str = "Start";
pub const RETRY: &str = "Ask again";
pub const SAVE: &str = "Save";
pub const OPEN: &str = "Open";
pub const FEWER: &str = "Fewer";
/// Said when Save or Open is pressed with no file named.
pub const NEED_PATH: &str = "Type the file's path first, such as /home/you/game.sgf.";
pub const MORE: &str = "More";
pub const WEAKER: &str = "Weaker";
pub const STRONGER: &str = "Stronger";

/// How a game ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Resigned {
        winner: Stone,
    },
    Scored {
        winner: Option<Stone>,
        margin_x2: u16,
        dead: Vec<(u8, u8)>,
    },
}

/// A file being read or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileCall {
    Save(CallId),
}

pub struct GoApp {
    pub game: Game,
    /// The colour you play in this game.
    pub you: Stone,
    pub level: u8,
    pub outcome: Option<Outcome>,
    /// Why GNU Go couldn't answer, until asked again.
    pub error: Option<String>,
    /// Why the last click wasn't a move.
    pub notice: Option<String>,
    /// Whether the New game settings are open, and what they are.
    pub setting_up: bool,
    pub new_size: u8,
    pub new_you: Stone,
    pub new_handicap: u8,
    pub new_komi_x2: i16,
    pub new_level: u8,
    pub file: Text,
    pub file_status: Option<String>,
    file_call: Option<FileCall>,
    /// The `.sgf` being opened, until it arrives.
    pub opening: Option<String>,
    pub polling: bool,
}

impl Default for GoApp {
    fn default() -> Self {
        Self {
            game: Game::new(9, 13, 0),
            you: Stone::Black,
            level: 5,
            outcome: None,
            error: None,
            notice: None,
            setting_up: false,
            new_size: 9,
            new_you: Stone::Black,
            new_handicap: 0,
            new_komi_x2: 13,
            new_level: 5,
            file: Text::new(""),
            file_status: None,
            file_call: None,
            opening: None,
            polling: false,
        }
    }
}

impl GoApp {
    fn engine_colour(&self) -> Stone {
        self.you.other()
    }

    /// What GNU Go should be asked now, if anything: its move on its turn, or the score once
    /// both have passed.
    pub fn question(&self) -> Option<EngineReq> {
        if self.outcome.is_some() || self.error.is_some() || self.setting_up {
            return None;
        }
        let position = self.game.replay().ok()?;
        let ask = if position.is_over() {
            Ask::Score
        } else if position.to_move == self.engine_colour() {
            Ask::Move { level: self.level }
        } else {
            return None;
        };
        Some(EngineReq {
            game: self.game.clone(),
            ask,
        })
    }

    /// Play `mv` for you, if it's your turn and legal.
    pub fn play(&mut self, mv: Move) {
        self.notice = None;
        if self.outcome.is_some() {
            return;
        }
        let Ok(position) = self.game.replay() else {
            return;
        };
        if position.to_move != self.you || position.is_over() {
            self.notice = Some("Wait for GNU Go to move.".into());
            return;
        }
        match position.check(mv) {
            Ok(()) => self.game.moves.push(mv),
            Err(why) => self.notice = Some(format!("You can't play there: {}.", why.describe())),
        }
    }

    /// Take back your last move, and GNU Go's reply to it.
    pub fn undo(&mut self) {
        let Some(last_mine) = (0..self.game.moves.len())
            .rev()
            .find(|&n| self.game.color_of(n) == self.you)
        else {
            return;
        };
        self.game.moves.truncate(last_mine);
        self.outcome = None;
        self.error = None;
        self.notice = None;
    }

    fn can_undo(&self) -> bool {
        (0..self.game.moves.len()).any(|n| self.game.color_of(n) == self.you)
    }

    pub fn start(&mut self) {
        self.game = Game::new(self.new_size, self.new_komi_x2, self.new_handicap);
        self.you = self.new_you;
        self.level = self.new_level;
        self.outcome = None;
        self.error = None;
        self.notice = None;
        self.setting_up = false;
    }

    /// Act on GNU Go's answer.
    fn answered(&mut self, answer: &Result<Answer, String>) {
        match answer {
            Ok(Answer::Move(mv)) => {
                // The server checked it against the rules already; a move that doesn't fit this
                // game any more (it changed meanwhile) is simply not played.
                if self.game.replay().is_ok_and(|p| p.check(*mv).is_ok()) {
                    self.game.moves.push(*mv);
                }
            }
            Ok(Answer::Resign) => {
                self.outcome = Some(Outcome::Resigned { winner: self.you });
            }
            Ok(Answer::Score {
                winner,
                margin_x2,
                dead,
            }) => {
                self.outcome = Some(Outcome::Scored {
                    winner: *winner,
                    margin_x2: *margin_x2,
                    dead: dead.clone(),
                });
            }
            Err(why) => self.error = Some(why.clone()),
        }
    }

    /// Ask GNU Go whatever needs asking, and act on its answer once it comes.
    fn consult(&mut self, ui: &mut Ui<'_>) {
        self.polling = false;
        let Some(req) = self.question() else {
            return;
        };
        match ui.rpc().get::<GoEngine>(&req) {
            Poll::Ready(status) if status.finished => {
                if let Some(answer) = &status.answer {
                    self.answered(answer);
                }
            }
            Poll::Ready(_) => {
                // Read the job again next time; this never starts a second engine.
                ui.rpc().invalidate::<GoEngine>(&req);
                self.polling = true;
            }
            Poll::Pending => self.polling = true,
            Poll::Failed(e) => self.error = Some(e.message().to_owned()),
        }
    }

    /// What is going on, in a sentence.
    pub fn status(&self) -> String {
        let you = self.you.name();
        if let Some(outcome) = &self.outcome {
            return match outcome {
                Outcome::Resigned { winner } if *winner == self.you => {
                    "GNU Go resigned. You win!".into()
                }
                Outcome::Resigned { .. } => "You resigned. GNU Go wins.".into(),
                Outcome::Scored { winner: None, .. } => "The game is a draw.".into(),
                Outcome::Scored {
                    winner: Some(w),
                    margin_x2,
                    ..
                } => format!(
                    "{} wins by {} points ({}).",
                    w.name(),
                    komi_text(*margin_x2 as i16),
                    if *w == self.you { "you" } else { "GNU Go" }
                ),
            };
        }
        if let Some(why) = &self.error {
            return format!("GNU Go couldn't answer: {why}");
        }
        let Ok(position) = self.game.replay() else {
            return String::new();
        };
        if position.is_over() {
            return "Both passed. Counting…".into();
        }
        let engine_passed = self.game.moves.last() == Some(&Move::Pass)
            && self.game.color_of(self.game.moves.len() - 1) != self.you;
        if position.to_move == self.you {
            if engine_passed {
                format!("GNU Go passed. Your move ({you}); pass too to end the game.")
            } else {
                format!("Your move ({you}).")
            }
        } else {
            "GNU Go is thinking…".into()
        }
    }

    fn setup(&mut self, ui: &mut Ui<'_>) {
        ui.styled("New game", TextStyle::STRONG);
        ui.horizontal(|ui| {
            ui.label("Board");
            for size in SIZES {
                ui.push_id(&format!("size{size}"), |ui| {
                    if ui
                        .selectable(self.new_size == size, &format!("{size}×{size}"))
                        .clicked()
                    {
                        self.new_size = size;
                        self.new_handicap = self.new_handicap.min(max_handicap(size));
                    }
                });
            }
        });
        ui.horizontal(|ui| {
            ui.label("You play");
            for stone in [Stone::Black, Stone::White] {
                ui.push_id(stone.name(), |ui| {
                    if ui.selectable(self.new_you == stone, stone.name()).clicked() {
                        self.new_you = stone;
                    }
                });
            }
        });
        stepper(
            ui,
            "Handicap",
            &self.new_handicap.to_string(),
            FEWER,
            MORE,
            |up| {
                self.new_handicap = match (self.new_handicap, up) {
                    (0, true) => 2,
                    (2, false) => 0,
                    (n, true) => (n + 1).min(max_handicap(self.new_size)),
                    (n, false) => n.saturating_sub(1),
                };
            },
        );
        stepper(
            ui,
            "Komi",
            &komi_text(self.new_komi_x2),
            "−0.5",
            "+0.5",
            |up| {
                let step = if up { 1 } else { -1 };
                self.new_komi_x2 = (self.new_komi_x2 + step).clamp(-MAX_KOMI_X2, MAX_KOMI_X2);
            },
        );
        stepper(
            ui,
            "GNU Go's strength",
            &self.new_level.to_string(),
            WEAKER,
            STRONGER,
            |up| {
                self.new_level = if up {
                    (self.new_level + 1).min(MAX_LEVEL)
                } else {
                    self.new_level.saturating_sub(1).max(1)
                };
            },
        );
        ui.horizontal(|ui| {
            if ui.button(START).clicked() {
                self.start();
            }
            if ui.button("Cancel").clicked() {
                self.setting_up = false;
            }
        });
    }

    fn board(&mut self, ui: &mut Ui<'_>) {
        let Ok(position) = self.game.replay() else {
            ui.label("This game can't be shown.");
            return;
        };
        let size = self.game.size;
        let dead: &[(u8, u8)] = match &self.outcome {
            Some(Outcome::Scored { dead, .. }) => dead,
            _ => &[],
        };
        let mut clicked = None;
        ui.table(|ui| {
            ui.row(|ui| {
                ui.label("");
                for x in 0..size {
                    ui.label(&column(x).to_string());
                }
            });
            for y in 0..size {
                ui.push_id(&format!("row{y}"), |ui| {
                    ui.row(|ui| {
                        ui.label(&(size - y).to_string());
                        for x in 0..size {
                            let glyph = match position.at(x, y) {
                                Some(_) if dead.contains(&(x, y)) => DEAD,
                                Some(Stone::Black) => BLACK,
                                Some(Stone::White) => WHITE,
                                None => EMPTY,
                            };
                            let last = position.last == Some(Move::Play(x, y));
                            ui.push_id(&format!("{x}"), |ui| {
                                if ui.selectable(last, glyph).clicked() {
                                    clicked = Some(Move::Play(x, y));
                                }
                            });
                        }
                    });
                });
            }
        });
        if let Some(mv) = clicked {
            self.play(mv);
        }
        ui.label(&format!(
            "Captured: by you {}, by GNU Go {}.",
            position.captures(self.you),
            position.captures(self.engine_colour())
        ));
        if let Some(Move::Play(..)) = position.last {
            let last = position.last.unwrap_or(Move::Pass);
            ui.styled(
                &format!("Last move: {} (highlighted)", vertex(last, size)),
                TextStyle::WEAK,
            );
        }
    }

    fn files(&mut self, ui: &mut Ui<'_>) {
        let rpc = ui.rpc();
        if let Some(FileCall::Save(id)) = self.file_call {
            match rpc.outcome::<WriteFile>(id) {
                Poll::Pending => {}
                Poll::Ready(_) => {
                    self.file_status = Some(format!("Saved to {}.", self.file.as_str().trim()));
                    self.file_call = None;
                }
                Poll::Failed(e) => {
                    self.file_status = Some(format!("Couldn't save: {}.", e.message()));
                    self.file_call = None;
                }
            }
        }
        if let Some(path) = self.opening.clone() {
            match rpc.get::<ReadFile>(&PathReq { path: &path }) {
                Poll::Pending => {}
                Poll::Ready(file) => {
                    match from_sgf(&file.text) {
                        Ok(game) => {
                            self.game = game;
                            // You take whoever is to move, so the game goes on from there.
                            self.you = self.game.color_of(self.game.moves.len());
                            self.outcome = None;
                            self.error = None;
                            self.notice = None;
                            self.setting_up = false;
                            self.file_status = Some(format!("Opened {path}."));
                        }
                        Err(why) => {
                            self.file_status = Some(format!("Couldn't open {path}: {why}."));
                        }
                    }
                    self.opening = None;
                }
                Poll::Failed(e) => {
                    self.file_status = Some(format!("Couldn't open {path}: {}.", e.message()));
                    self.opening = None;
                }
            }
        }
        ui.horizontal(|ui| {
            ui.label("File");
            ui.text_edit(&mut self.file);
            let path = self.file.as_str().trim().to_owned();
            let save = ui.button(SAVE).clicked();
            ui.tooltip("Save the game as an .sgf file, which other Go programs open too");
            let open = ui.button(OPEN).clicked();
            if (save || open) && path.is_empty() {
                self.file_status = Some(NEED_PATH.into());
            } else if save && self.file_call.is_none() {
                let text = to_sgf(&self.game);
                let id = ui.rpc().send::<WriteFile>(&WriteFileReq {
                    path: &path,
                    text: &text,
                    create_only: false,
                });
                self.file_call = Some(FileCall::Save(id));
                self.file_status = Some("Saving…".into());
            } else if open {
                ui.rpc().invalidate::<ReadFile>(&PathReq { path: &path });
                self.opening = Some(path);
                self.file_status = Some("Opening…".into());
            }
        });
        if let Some(status) = &self.file_status {
            ui.push_id("file-status", |ui| ui.styled(status, TextStyle::WEAK));
        }
    }
}

/// `Label  [down] value [up]`, calling `change(false)` for `down` and `change(true)` for `up`.
fn stepper(
    ui: &mut Ui<'_>,
    label: &str,
    value: &str,
    down: &str,
    up: &str,
    mut change: impl FnMut(bool),
) {
    ui.push_id(label, |ui| {
        ui.horizontal(|ui| {
            ui.label(label);
            if ui.button(down).clicked() {
                change(false);
            }
            ui.label(value);
            if ui.button(up).clicked() {
                change(true);
            }
        });
    });
}

impl App for GoApp {
    fn wants_repaint_after_ms(&self) -> u32 {
        if self.polling {
            POLL_MS
        } else {
            ccosel_sdk::REPAINT_ON_INPUT_ONLY
        }
    }

    fn update(&mut self, ui: &mut Ui<'_>) {
        self.consult(ui);
        if self.setting_up {
            self.setup(ui);
            ui.separator();
        }
        ui.label(&self.status());
        if let Some(notice) = &self.notice {
            ui.push_id("notice", |ui| ui.styled(notice, TextStyle::WEAK));
        }
        let my_turn = self.outcome.is_none()
            && self.error.is_none()
            && self
                .game
                .replay()
                .is_ok_and(|p| p.to_move == self.you && !p.is_over());
        ui.horizontal(|ui| {
            if my_turn && ui.button(PASS).clicked() {
                self.play(Move::Pass);
            }
            if self.outcome.is_none() && ui.button(RESIGN).clicked() {
                self.outcome = Some(Outcome::Resigned {
                    winner: self.engine_colour(),
                });
            }
            if self.can_undo() && ui.button(UNDO).clicked() {
                self.undo();
            }
            ui.tooltip("Take back your last move and GNU Go's reply");
            if self.error.is_some() && ui.button(RETRY).clicked() {
                // Forget the failed answer, so the question is asked afresh.
                self.error = None;
                if let Some(req) = self.question() {
                    ui.rpc().invalidate::<GoEngine>(&req);
                }
            }
            if !self.setting_up && ui.button(NEW_GAME).clicked() {
                self.setting_up = true;
            }
        });
        self.board(ui);
        ui.separator();
        self.files(ui);
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(GoApp);
