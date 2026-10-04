//! Go, shared by the Go app and the server: the rules (captures, no suicide, simple ko), the
//! standard handicap points, GTP's names for points, SGF, and the method that asks GNU Go for
//! a move or a score.
//!
//! Both sides replay a game with the same [`Position`], so the app refuses an illegal move as
//! it is clicked, and the server refuses a request holding one, or an engine reply that would
//! be one, by the very same rules.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Effect, Method, Query, Rpc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Stone {
    Black,
    White,
}

impl Stone {
    pub fn other(self) -> Self {
        match self {
            Self::Black => Self::White,
            Self::White => Self::Black,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Black => "Black",
            Self::White => "White",
        }
    }
}

/// A move: a stone at column `x` and row `y`, both from the top left starting at 0, or a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Move {
    Play(u8, u8),
    Pass,
}

/// The board sizes offered.
pub const SIZES: [u8; 3] = [9, 13, 19];

/// Komi is kept in half points, so 6.5 is 13: no floats on the wire or in the app.
pub const MAX_KOMI_X2: i16 = 200;

/// A game as it is played: the setup and every move since. The colour of each move follows
/// from the order: Black first, or White first after handicap stones.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Game {
    pub size: u8,
    pub komi_x2: i16,
    /// 0 for none, otherwise 2 or more stones for Black on the standard points.
    pub handicap: u8,
    pub moves: Vec<Move>,
}

impl Game {
    pub fn new(size: u8, komi_x2: i16, handicap: u8) -> Self {
        Self {
            size,
            komi_x2,
            handicap,
            moves: Vec::new(),
        }
    }

    pub fn first_to_move(&self) -> Stone {
        if self.handicap >= 2 {
            Stone::White
        } else {
            Stone::Black
        }
    }

    /// Who plays move number `n` (from 0).
    pub fn color_of(&self, n: usize) -> Stone {
        if n.is_multiple_of(2) {
            self.first_to_move()
        } else {
            self.first_to_move().other()
        }
    }

    /// The position after every move, or the first thing wrong with the game: a size, komi or
    /// handicap that isn't offered, or an illegal move (and its number).
    pub fn replay(&self) -> Result<Position, (Option<usize>, Illegal)> {
        if !SIZES.contains(&self.size)
            || self.komi_x2.abs() > MAX_KOMI_X2
            || self.handicap == 1
            || self.handicap > max_handicap(self.size)
        {
            return Err((None, Illegal::Setup));
        }
        let mut pos = Position::new(self.size);
        for (x, y) in handicap_points(self.size, self.handicap) {
            let i = pos.index(x, y);
            pos.cells[i] = Some(Stone::Black);
        }
        pos.to_move = self.first_to_move();
        for (n, &mv) in self.moves.iter().enumerate() {
            pos.play(mv).map_err(|e| (Some(n), e))?;
        }
        Ok(pos)
    }
}

/// Why a move can't be played.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Illegal {
    OffBoard,
    Occupied,
    /// It would leave its own stones with no liberties and capture nothing.
    Suicide,
    /// It would take back a single stone straight after it took one.
    Ko,
    /// The size, komi or handicap isn't one that is offered.
    Setup,
}

impl Illegal {
    pub fn describe(self) -> &'static str {
        match self {
            Self::OffBoard => "that point isn't on the board",
            Self::Occupied => "there is already a stone there",
            Self::Suicide => "that would leave your stones with no liberties",
            Self::Ko => "ko: you can't take back straight away; play elsewhere first",
            Self::Setup => "that board size, komi or handicap isn't offered",
        }
    }
}

/// The most handicap stones a board takes.
pub fn max_handicap(size: u8) -> u8 {
    if size == 9 { 5 } else { 9 }
}

/// Where `n` handicap stones go: the star points, corners first, then the sides, with the
/// centre for an odd number from five.
pub fn handicap_points(size: u8, n: u8) -> Vec<(u8, u8)> {
    let (lo, hi) = if size >= 13 {
        (3, size - 4)
    } else {
        (2, size - 3)
    };
    let mid = size / 2;
    let corners = [(lo, hi), (hi, lo), (hi, hi), (lo, lo)];
    let mut points: Vec<(u8, u8)> = match n {
        0 | 1 => return Vec::new(),
        2..=4 => corners[..usize::from(n)].to_vec(),
        5 => corners.to_vec(),
        6 | 7 => {
            let mut p = corners.to_vec();
            p.extend([(lo, mid), (hi, mid)]);
            p
        }
        _ => {
            let mut p = corners.to_vec();
            p.extend([(lo, mid), (hi, mid), (mid, lo), (mid, hi)]);
            p
        }
    };
    if n >= 5 && !n.is_multiple_of(2) {
        points.push((mid, mid));
    }
    points
}

/// The board after some moves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Position {
    pub size: u8,
    cells: Vec<Option<Stone>>,
    pub to_move: Stone,
    /// Stones captured by Black, and by White.
    pub captured_by: [u32; 2],
    /// The point the player to move may not play on, by the ko rule.
    ko: Option<usize>,
    pub last: Option<Move>,
    /// Passes in a row: two end the game.
    pub passes: u8,
}

impl Position {
    pub fn new(size: u8) -> Self {
        Self {
            size,
            cells: vec![None; usize::from(size) * usize::from(size)],
            to_move: Stone::Black,
            captured_by: [0, 0],
            ko: None,
            last: None,
            passes: 0,
        }
    }

    fn index(&self, x: u8, y: u8) -> usize {
        usize::from(y) * usize::from(self.size) + usize::from(x)
    }

    pub fn at(&self, x: u8, y: u8) -> Option<Stone> {
        if x < self.size && y < self.size {
            self.cells[self.index(x, y)]
        } else {
            None
        }
    }

    pub fn is_over(&self) -> bool {
        self.passes >= 2
    }

    pub fn captures(&self, by: Stone) -> u32 {
        self.captured_by[by as usize]
    }

    fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> + use<> {
        let size = usize::from(self.size);
        let (x, y) = (i % size, i / size);
        let mut out = [None; 4];
        if x > 0 {
            out[0] = Some(i - 1);
        }
        if x + 1 < size {
            out[1] = Some(i + 1);
        }
        if y > 0 {
            out[2] = Some(i - size);
        }
        if y + 1 < size {
            out[3] = Some(i + size);
        }
        out.into_iter().flatten()
    }

    /// The stones of the group at `i`, and how many liberties it has.
    fn group(&self, i: usize) -> (Vec<usize>, usize) {
        let colour = self.cells[i];
        let mut stones = vec![i];
        let mut seen = vec![false; self.cells.len()];
        seen[i] = true;
        let mut liberties = Vec::new();
        let mut k = 0;
        while k < stones.len() {
            for n in self.neighbours(stones[k]) {
                if seen[n] {
                    continue;
                }
                seen[n] = true;
                match self.cells[n] {
                    None => liberties.push(n),
                    c if c == colour => stones.push(n),
                    _ => {}
                }
            }
            k += 1;
        }
        (stones, liberties.len())
    }

    /// Whether `mv` is legal for the player to move.
    pub fn check(&self, mv: Move) -> Result<(), Illegal> {
        self.clone().play(mv)
    }

    /// Play `mv` for the player to move, capturing what it captures.
    pub fn play(&mut self, mv: Move) -> Result<(), Illegal> {
        let Move::Play(x, y) = mv else {
            self.to_move = self.to_move.other();
            self.ko = None;
            self.passes += 1;
            self.last = Some(Move::Pass);
            return Ok(());
        };
        if x >= self.size || y >= self.size {
            return Err(Illegal::OffBoard);
        }
        let i = self.index(x, y);
        if self.cells[i].is_some() {
            return Err(Illegal::Occupied);
        }
        if self.ko == Some(i) {
            return Err(Illegal::Ko);
        }
        let me = self.to_move;
        self.cells[i] = Some(me);
        let mut taken = Vec::new();
        for n in self.neighbours(i) {
            if self.cells[n] == Some(me.other()) && !taken.contains(&n) {
                let (stones, liberties) = self.group(n);
                if liberties == 0 {
                    taken.extend(stones);
                }
            }
        }
        for &t in &taken {
            self.cells[t] = None;
        }
        let (mine, liberties) = self.group(i);
        if liberties == 0 {
            self.cells[i] = None;
            return Err(Illegal::Suicide);
        }
        // A single stone that took a single stone, and is left in atari, can be taken back at
        // once: that recapture is what the ko rule forbids, for one move.
        self.ko = (taken.len() == 1 && mine.len() == 1 && liberties == 1).then(|| taken[0]);
        self.captured_by[me as usize] += taken.len() as u32;
        self.to_move = me.other();
        self.last = Some(mv);
        self.passes = 0;
        Ok(())
    }
}

/// GTP's name for a point: a column letter (skipping I) and a row number from the bottom.
pub fn vertex(mv: Move, size: u8) -> String {
    match mv {
        Move::Pass => "pass".into(),
        Move::Play(x, y) => format!("{}{}", column(x), size - y),
    }
}

/// The letter of column `x`, as on a board: A to T, with no I.
pub fn column(x: u8) -> char {
    let c = b'A' + x;
    char::from(if c >= b'I' { c + 1 } else { c })
}

/// A GTP point (`D4`, `q16`, `PASS`) as a move, or `None` if it isn't one on this board.
pub fn parse_vertex(text: &str, size: u8) -> Option<Move> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("pass") {
        return Some(Move::Pass);
    }
    let mut chars = text.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    if !letter.is_ascii_uppercase() || letter == 'I' {
        return None;
    }
    let x = letter as u8 - b'A' - u8::from(letter > 'I');
    let row: u8 = chars.as_str().parse().ok()?;
    (x < size && (1..=size).contains(&row)).then(|| Move::Play(x, size - row))
}

/// Komi as people write it: `6.5`, `0`, `-0.5`.
pub fn komi_text(komi_x2: i16) -> String {
    let sign = if komi_x2 < 0 { "-" } else { "" };
    let whole = komi_x2.unsigned_abs() / 2;
    if komi_x2.unsigned_abs().is_multiple_of(2) {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.5")
    }
}

/// `6.5` as half points, rounded down to a half. `None` if it isn't a number.
pub fn parse_komi(text: &str) -> Option<i16> {
    let text = text.trim();
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    let whole: i16 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    if !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let half = i16::from(frac.as_bytes().first().is_some_and(|&d| d >= b'5'));
    let x2 = whole.checked_mul(2)?.checked_add(half)?;
    Some(if negative { -x2 } else { x2 })
}

/// A game as SGF, the file format Go programs share.
pub fn to_sgf(game: &Game) -> String {
    let point = |x: u8, y: u8| format!("{}{}", char::from(b'a' + x), char::from(b'a' + y));
    let mut out = format!(
        "(;GM[1]FF[4]CA[UTF-8]AP[CCOSEL]SZ[{}]KM[{}]",
        game.size,
        komi_text(game.komi_x2)
    );
    if game.handicap >= 2 {
        out.push_str(&format!("HA[{}]AB", game.handicap));
        for (x, y) in handicap_points(game.size, game.handicap) {
            out.push_str(&format!("[{}]", point(x, y)));
        }
    }
    for (n, mv) in game.moves.iter().enumerate() {
        let colour = if game.color_of(n) == Stone::Black {
            'B'
        } else {
            'W'
        };
        let at = match *mv {
            Move::Pass => String::new(),
            Move::Play(x, y) => point(x, y),
        };
        out.push_str(&format!("\n;{colour}[{at}]"));
    }
    out.push_str(")\n");
    out
}

/// A game from SGF: its size, komi, handicap and main line of moves. Variations are left out,
/// and so is anything that doesn't affect the game. Refused if its handicap stones aren't on
/// the standard points, or its moves don't take turns or aren't legal.
pub fn from_sgf(text: &str) -> Result<Game, String> {
    let mut props: Vec<(String, Vec<String>)> = Vec::new();
    let mut chars = text.chars().peekable();
    let mut ident = String::new();
    while let Some(c) = chars.next() {
        match c {
            // The main line is the first branch all the way down, so it ends at the first `)`:
            // whatever follows is another variation.
            ')' => break,
            '(' => {}
            '[' => {
                let mut value = String::new();
                while let Some(v) = chars.next() {
                    match v {
                        '\\' => {
                            if let Some(e) = chars.next() {
                                value.push(e);
                            }
                        }
                        ']' => break,
                        v => value.push(v),
                    }
                }
                if !ident.is_empty() {
                    props.push((core::mem::take(&mut ident), vec![value]));
                } else if let Some(last) = props.last_mut() {
                    last.1.push(value);
                }
            }
            c if c.is_ascii_uppercase() => ident.push(c),
            _ => ident.clear(),
        }
    }
    finish_sgf(props)
}

fn finish_sgf(props: Vec<(String, Vec<String>)>) -> Result<Game, String> {
    let get = |name: &str| props.iter().find(|(n, _)| n == name).map(|(_, v)| v);
    let size: u8 = match get("SZ") {
        Some(v) => v[0]
            .trim()
            .parse()
            .map_err(|_| "its board size isn't a number")?,
        None => 19,
    };
    if !SIZES.contains(&size) {
        return Err(format!("{size}×{size} boards aren't offered here"));
    }
    let komi_x2 = match get("KM") {
        Some(v) => parse_komi(&v[0]).ok_or("its komi isn't a number")?,
        None => 0,
    };
    let point = |v: &str| -> Result<Move, String> {
        let b = v.trim().as_bytes();
        if b.is_empty() || (size <= 19 && b == b"tt") {
            return Ok(Move::Pass);
        }
        if b.len() != 2 || !b[0].is_ascii_lowercase() || !b[1].is_ascii_lowercase() {
            return Err(format!("“{v}” isn't a point"));
        }
        Ok(Move::Play(b[0] - b'a', b[1] - b'a'))
    };
    let mut stones: Vec<(u8, u8)> = Vec::new();
    for v in get("AB").into_iter().flatten() {
        if let Move::Play(x, y) = point(v)? {
            stones.push((x, y));
        }
    }
    let handicap = stones.len() as u8;
    let mut expected = handicap_points(size, handicap);
    expected.sort_unstable();
    stones.sort_unstable();
    if handicap == 1 || handicap > max_handicap(size) || expected != stones {
        return Err("its handicap stones aren't on the standard points".into());
    }
    let mut game = Game::new(size, komi_x2, handicap);
    for (name, values) in &props {
        let colour = match name.as_str() {
            "B" => Stone::Black,
            "W" => Stone::White,
            _ => continue,
        };
        if colour != game.color_of(game.moves.len()) {
            return Err("its moves don't take turns".into());
        }
        game.moves.push(point(&values[0])?);
    }
    game.replay().map_err(|(n, e)| match n {
        Some(n) => format!("move {} is illegal: {}", n + 1, e.describe()),
        None => e.describe().into(),
    })?;
    Ok(game)
}

/// What to ask GNU Go about a game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ask {
    /// Its move, for the player to move, at strength `level` (1 to 10).
    Move { level: u8 },
    /// Who won, once both have passed.
    Score,
}

pub const MAX_LEVEL: u8 = 10;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EngineReq {
    pub game: Game,
    pub ask: Ask,
}

/// What GNU Go answered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Answer {
    Move(Move),
    Resign,
    Score {
        /// `None` for a draw.
        winner: Option<Stone>,
        margin_x2: u16,
        /// Stones GNU Go counted as dead, taken off before counting.
        dead: Vec<(u8, u8)>,
    },
}

/// Where a request to the engine has got to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineStatus {
    pub finished: bool,
    /// Present when `finished`: the answer, or why there is none.
    pub answer: Option<Result<Answer, String>>,
}

/// Ask GNU Go for a move or a score. A job, polled like `Compile`: the request is the whole
/// game, so the request cache works unchanged, and the same position is only ever asked once.
/// The server keeps no engine running between calls.
pub struct GoEngine;

impl Rpc for GoEngine {
    const METHOD: Method = Method::GoEngine;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    // The same game asked again reports on the same job: safe to retry and to cache.
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = EngineReq;
    type Reply = EngineStatus;
}

impl Query for GoEngine {}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(size: u8, moves: &[&str]) -> Game {
        let mut g = Game::new(size, 13, 0);
        for m in moves {
            g.moves.push(parse_vertex(m, size).unwrap());
        }
        g
    }

    #[test]
    fn vertices_skip_i_and_count_rows_from_the_bottom() {
        assert_eq!(vertex(Move::Play(0, 18), 19), "A1");
        assert_eq!(vertex(Move::Play(8, 0), 19), "J19");
        assert_eq!(vertex(Move::Play(18, 0), 19), "T19");
        assert_eq!(vertex(Move::Pass, 9), "pass");
        assert_eq!(parse_vertex("j19", 19), Some(Move::Play(8, 0)));
        assert_eq!(parse_vertex("PASS", 9), Some(Move::Pass));
        assert_eq!(parse_vertex("I5", 19), None);
        assert_eq!(parse_vertex("K10", 9), None);
        assert_eq!(parse_vertex("A0", 9), None);
        assert_eq!(parse_vertex("Z", 9), None);
        assert_eq!(parse_vertex("", 9), None);
        for x in 0..19 {
            for y in 0..19 {
                let mv = Move::Play(x, y);
                assert_eq!(parse_vertex(&vertex(mv, 19), 19), Some(mv));
            }
        }
    }

    #[test]
    fn a_surrounded_stone_is_captured_and_counted() {
        // White's E5 surrounded by Black.
        let g = game(9, &["D5", "E5", "F5", "A1", "E4", "A2", "E6"]);
        let p = g.replay().unwrap();
        assert_eq!(p.at(4, 4), None);
        assert_eq!(p.captures(Stone::Black), 1);
        assert_eq!(p.captures(Stone::White), 0);
        assert_eq!(p.to_move, Stone::White);
    }

    #[test]
    fn suicide_is_refused_but_a_capturing_move_is_not() {
        // Black surrounds A1's neighbours: White playing A1 has no liberties.
        let p = game(9, &["A2", "E5", "B1"]).replay().unwrap();
        assert_eq!(p.check(Move::Play(0, 8)), Err(Illegal::Suicide));
        // Filling the last liberty is fine if it captures: B1 and A2 are taken.
        let g = game(9, &["A2", "B2", "B1", "C1", "E5", "A3", "E6"]);
        let p = g.replay().unwrap();
        assert_eq!(p.check(Move::Play(0, 8)), Ok(()));
        let mut p = p;
        p.play(Move::Play(0, 8)).unwrap();
        assert_eq!(p.captures(Stone::White), 2);
    }

    #[test]
    fn ko_forbids_retaking_at_once_but_not_after_a_move_elsewhere() {
        // White's D5 is surrounded on three sides; Black E5 takes it, leaving E5 itself in
        // atari, so White retaking at D5 would repeat the position.
        let g = game(9, &["C5", "D5", "D4", "E4", "D6", "E6", "A1", "F5", "E5"]);
        let mut p = g.replay().unwrap();
        // Black E5 captured White D5.
        assert_eq!(p.at(3, 4), None);
        assert_eq!(p.captures(Stone::Black), 1);
        // White may not retake at D5 at once.
        assert_eq!(p.check(Move::Play(3, 4)), Err(Illegal::Ko));
        p.play(parse_vertex("A9", 9).unwrap()).unwrap();
        p.play(parse_vertex("A8", 9).unwrap()).unwrap();
        assert_eq!(p.check(Move::Play(3, 4)), Ok(()), "after a move elsewhere");
    }

    #[test]
    fn occupied_off_board_and_passes() {
        let mut p = game(9, &["E5"]).replay().unwrap();
        assert_eq!(p.check(Move::Play(4, 4)), Err(Illegal::Occupied));
        assert_eq!(p.check(Move::Play(9, 0)), Err(Illegal::OffBoard));
        p.play(Move::Pass).unwrap();
        assert!(!p.is_over());
        p.play(Move::Pass).unwrap();
        assert!(p.is_over());
        assert_eq!(p.last, Some(Move::Pass));
    }

    #[test]
    fn handicap_stones_and_who_moves_first() {
        for size in SIZES {
            for n in 2..=max_handicap(size) {
                let pts = handicap_points(size, n);
                assert_eq!(pts.len(), usize::from(n));
                let mut sorted = pts.clone();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(sorted.len(), pts.len(), "{size} {n}");
            }
        }
        assert_eq!(handicap_points(19, 2), [(3, 15), (15, 3)]);
        assert_eq!(handicap_points(9, 5).last(), Some(&(4, 4)));
        let g = Game::new(19, 1, 4);
        let p = g.replay().unwrap();
        assert_eq!(p.to_move, Stone::White);
        assert_eq!(p.at(3, 3), Some(Stone::Black));
        assert_eq!(g.color_of(1), Stone::Black);
        assert!(Game::new(19, 13, 1).replay().is_err());
        assert!(Game::new(9, 13, 6).replay().is_err());
        assert!(Game::new(10, 13, 0).replay().is_err());
        assert!(Game::new(9, 999, 0).replay().is_err());
    }

    #[test]
    fn komi_reads_and_writes_in_half_points() {
        assert_eq!(komi_text(13), "6.5");
        assert_eq!(komi_text(0), "0");
        assert_eq!(komi_text(-1), "-0.5");
        assert_eq!(komi_text(-14), "-7");
        assert_eq!(parse_komi("6.5"), Some(13));
        assert_eq!(parse_komi("7"), Some(14));
        assert_eq!(parse_komi("-0.5"), Some(-1));
        assert_eq!(parse_komi("0.75"), Some(1));
        assert_eq!(parse_komi("x"), None);
        assert_eq!(parse_komi("1.x"), None);
    }

    #[test]
    fn sgf_round_trips_and_refuses_what_it_cant_hold() {
        let mut g = Game::new(13, 13, 3);
        g.moves = alloc::vec![Move::Play(6, 6), Move::Pass, Move::Play(0, 0)];
        let text = to_sgf(&g);
        assert!(text.contains("SZ[13]KM[6.5]HA[3]AB"), "{text}");
        assert_eq!(from_sgf(&text), Ok(g));

        // A typical file from elsewhere, with comments and a variation.
        let other =
            "(;FF[4]GM[1]SZ[9]KM[7]C[a comment [with\\] brackets]];B[ee];W[cc](;B[gg])(;B[aa]))";
        let g = from_sgf(other).unwrap();
        assert_eq!(g.size, 9);
        assert_eq!(g.komi_x2, 14);
        assert_eq!(
            g.moves,
            [Move::Play(4, 4), Move::Play(2, 2), Move::Play(6, 6)]
        );

        assert!(from_sgf("(;SZ[10])").is_err());
        assert!(from_sgf("(;SZ[9];B[ee];B[cc])").is_err(), "turns");
        assert!(from_sgf("(;SZ[9];B[ee];W[ee])").is_err(), "illegal");
        assert!(from_sgf("(;SZ[9]AB[aa][bb])").is_err(), "free handicap");
        assert!(from_sgf("(;SZ[x])").is_err());
        assert!(from_sgf("(;SZ[9]KM[lots])").is_err());
        assert!(from_sgf("(;SZ[9];B[e])").is_err());
        assert_eq!(
            from_sgf("(;SZ[9];B[];W[tt])").unwrap().moves,
            [Move::Pass, Move::Pass]
        );
        assert_eq!(from_sgf("(;B[dd])").unwrap().size, 19);
    }
}
