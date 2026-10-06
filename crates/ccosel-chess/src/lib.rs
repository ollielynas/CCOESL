//! The rules of chess: legal moves, check, mate, the draws, and the notations people and
//! engines use (FEN, SAN, PGN and the coordinate moves of the xboard protocol).
//!
//! Shared by the Chess app, which must only let you make legal moves, and the server, which
//! checks that GNU Chess's reply is one before passing it on. One implementation means the two
//! can never disagree about what is legal.
//!
//! `no_std` with `alloc`, and small on purpose: it ships in the app's module. Positions are
//! plain 64-square arrays copied on every move, which is slower than bitboards but plenty for
//! one human game, and a fraction of the code.

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub mod pgn;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    White,
    Black,
}

impl Color {
    pub fn other(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl Kind {
    /// The letter SAN and FEN use, upper case.
    pub fn letter(self) -> char {
        match self {
            Self::Pawn => 'P',
            Self::Knight => 'N',
            Self::Bishop => 'B',
            Self::Rook => 'R',
            Self::Queen => 'Q',
            Self::King => 'K',
        }
    }

    pub fn from_letter(c: char) -> Option<Self> {
        Some(match c.to_ascii_uppercase() {
            'P' => Self::Pawn,
            'N' => Self::Knight,
            'B' => Self::Bishop,
            'R' => Self::Rook,
            'Q' => Self::Queen,
            'K' => Self::King,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Piece {
    pub color: Color,
    pub kind: Kind,
}

impl Piece {
    /// The Unicode chess symbol for it.
    pub fn glyph(self) -> char {
        let white = ['♙', '♘', '♗', '♖', '♕', '♔'];
        let black = ['♟', '♞', '♝', '♜', '♛', '♚'];
        let i = self.kind as usize;
        match self.color {
            Color::White => white[i],
            Color::Black => black[i],
        }
    }
}

/// A square, 0 (a1) to 63 (h8): `rank * 8 + file`.
pub type Square = u8;

pub fn file_of(sq: Square) -> u8 {
    sq % 8
}

pub fn rank_of(sq: Square) -> u8 {
    sq / 8
}

/// `e4` and the like.
pub fn square_name(sq: Square) -> String {
    let mut s = String::new();
    s.push((b'a' + file_of(sq)) as char);
    s.push((b'1' + rank_of(sq)) as char);
    s
}

pub fn parse_square(s: &str) -> Option<Square> {
    let b = s.as_bytes();
    if b.len() != 2 || !(b'a'..=b'h').contains(&b[0]) || !(b'1'..=b'8').contains(&b[1]) {
        return None;
    }
    Some((b[1] - b'1') * 8 + (b[0] - b'a'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Move {
    pub from: Square,
    pub to: Square,
    /// What a pawn reaching the last rank becomes.
    pub promotion: Option<Kind>,
}

impl Move {
    /// The coordinate notation engines speak: `e2e4`, `e7e8q`.
    pub fn coordinate(self) -> String {
        let mut s = square_name(self.from);
        s.push_str(&square_name(self.to));
        if let Some(k) = self.promotion {
            s.push(k.letter().to_ascii_lowercase());
        }
        s
    }

    /// Read a coordinate move. Only its shape is checked: whether it is legal is
    /// [`Position::legal_coordinate`]'s business.
    pub fn parse_coordinate(s: &str) -> Option<Self> {
        let s = s.trim();
        if !(4..=5).contains(&s.len()) || !s.is_ascii() {
            return None;
        }
        let promotion = match s.as_bytes().get(4) {
            None => None,
            Some(&c) => match Kind::from_letter(c as char)? {
                Kind::Pawn | Kind::King => return None,
                k => Some(k),
            },
        };
        Some(Self {
            from: parse_square(&s[0..2])?,
            to: parse_square(&s[2..4])?,
            promotion,
        })
    }
}

/// Which castling moves are still possible, as far as the kings and rooks having moved goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Castling {
    pub white_king: bool,
    pub white_queen: bool,
    pub black_king: bool,
    pub black_queen: bool,
}

/// How a game stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The side to move is checkmated.
    Checkmate(Color),
    Stalemate,
    /// Neither side has enough left to mate.
    InsufficientMaterial,
    /// Fifty moves each without a capture or a pawn move.
    FiftyMoves,
    /// The same position for the third time.
    Repetition,
}

impl Outcome {
    /// The PGN result: `1-0`, `0-1` or `1/2-1/2`.
    pub fn result(self) -> &'static str {
        match self {
            Self::Checkmate(Color::Black) => "1-0",
            Self::Checkmate(Color::White) => "0-1",
            _ => "1/2-1/2",
        }
    }
}

/// Everything about a position that decides what can happen next.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    pub board: [Option<Piece>; 64],
    pub side: Color,
    pub castling: Castling,
    /// The square a pawn passed over on the last move, which an en-passant capture lands on.
    pub en_passant: Option<Square>,
    /// Half-moves since the last capture or pawn move.
    pub halfmove: u32,
    pub fullmove: u32,
}

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

const KNIGHT: [(i8, i8); 8] = [
    (1, 2),
    (2, 1),
    (2, -1),
    (1, -2),
    (-1, -2),
    (-2, -1),
    (-2, 1),
    (-1, 2),
];
const KING: [(i8, i8); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];
const ROOK_DIRS: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const BISHOP_DIRS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

/// `sq` moved by `(files, ranks)`, if that is still on the board.
fn step(sq: Square, (df, dr): (i8, i8)) -> Option<Square> {
    let f = file_of(sq) as i8 + df;
    let r = rank_of(sq) as i8 + dr;
    ((0..8).contains(&f) && (0..8).contains(&r)).then(|| (r * 8 + f) as Square)
}

impl Default for Position {
    fn default() -> Self {
        Self::from_fen(START_FEN).expect("the starting position is valid")
    }
}

impl Position {
    pub fn from_fen(fen: &str) -> Result<Self, String> {
        let mut parts = fen.split_whitespace();
        let placement = parts.next().ok_or("an empty FEN")?;
        let mut board = [None; 64];
        let ranks: Vec<&str> = placement.split('/').collect();
        if ranks.len() != 8 {
            return Err("a FEN needs eight ranks".into());
        }
        for (i, row) in ranks.iter().enumerate() {
            let rank = 7 - i as u8;
            let mut file = 0u8;
            for c in row.chars() {
                if let Some(n) = c.to_digit(10) {
                    file += n as u8;
                } else {
                    let kind = Kind::from_letter(c).ok_or("an unknown piece in the FEN")?;
                    let color = if c.is_ascii_uppercase() {
                        Color::White
                    } else {
                        Color::Black
                    };
                    if file >= 8 {
                        return Err("a FEN rank is too long".into());
                    }
                    board[(rank * 8 + file) as usize] = Some(Piece { color, kind });
                    file += 1;
                }
                if file > 8 {
                    return Err("a FEN rank is too long".into());
                }
            }
            if file != 8 {
                return Err("a FEN rank is too short".into());
            }
        }
        let side = match parts.next().unwrap_or("w") {
            "w" => Color::White,
            "b" => Color::Black,
            _ => return Err("the side to move must be w or b".into()),
        };
        let rights = parts.next().unwrap_or("-");
        let castling = Castling {
            white_king: rights.contains('K'),
            white_queen: rights.contains('Q'),
            black_king: rights.contains('k'),
            black_queen: rights.contains('q'),
        };
        let en_passant = match parts.next().unwrap_or("-") {
            "-" => None,
            s => Some(parse_square(s).ok_or("a bad en-passant square")?),
        };
        let halfmove = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let fullmove = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
        let pos = Self {
            board,
            side,
            castling,
            en_passant,
            halfmove,
            fullmove,
        };
        for color in [Color::White, Color::Black] {
            let kings = pos
                .board
                .iter()
                .filter(|p| {
                    **p == Some(Piece {
                        color,
                        kind: Kind::King,
                    })
                })
                .count();
            if kings != 1 {
                return Err("each side needs exactly one king".into());
            }
        }
        if pos.attacked(pos.king(side.other()), side) {
            return Err("the side not to move is in check".into());
        }
        Ok(pos)
    }

    pub fn fen(&self) -> String {
        let mut s = String::new();
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                match self.board[rank * 8 + file] {
                    None => empty += 1,
                    Some(p) => {
                        if empty > 0 {
                            s.push_str(&empty.to_string());
                            empty = 0;
                        }
                        let c = p.kind.letter();
                        s.push(if p.color == Color::White {
                            c
                        } else {
                            c.to_ascii_lowercase()
                        });
                    }
                }
            }
            if empty > 0 {
                s.push_str(&empty.to_string());
            }
            if rank > 0 {
                s.push('/');
            }
        }
        s.push_str(if self.side == Color::White {
            " w "
        } else {
            " b "
        });
        let c = self.castling;
        let mut rights = String::new();
        for (on, ch) in [
            (c.white_king, 'K'),
            (c.white_queen, 'Q'),
            (c.black_king, 'k'),
            (c.black_queen, 'q'),
        ] {
            if on {
                rights.push(ch);
            }
        }
        s.push_str(if rights.is_empty() { "-" } else { &rights });
        s.push(' ');
        match self.en_passant {
            Some(sq) => s.push_str(&square_name(sq)),
            None => s.push('-'),
        }
        s.push_str(&format!(" {} {}", self.halfmove, self.fullmove));
        s
    }

    pub fn at(&self, sq: Square) -> Option<Piece> {
        self.board[sq as usize]
    }

    fn king(&self, color: Color) -> Square {
        self.board
            .iter()
            .position(|p| {
                *p == Some(Piece {
                    color,
                    kind: Kind::King,
                })
            })
            .expect("a position always has both kings") as Square
    }

    /// Whether `by` attacks `sq`.
    pub fn attacked(&self, sq: Square, by: Color) -> bool {
        let is = |s: Option<Square>, kinds: &[Kind]| {
            s.and_then(|s| self.at(s))
                .is_some_and(|p| p.color == by && kinds.contains(&p.kind))
        };
        // A pawn of `by` attacks from one rank behind, as `by` sees it.
        let behind = if by == Color::White { -1 } else { 1 };
        if is(step(sq, (-1, behind)), &[Kind::Pawn]) || is(step(sq, (1, behind)), &[Kind::Pawn]) {
            return true;
        }
        if KNIGHT.iter().any(|&d| is(step(sq, d), &[Kind::Knight])) {
            return true;
        }
        if KING.iter().any(|&d| is(step(sq, d), &[Kind::King])) {
            return true;
        }
        for (dirs, kinds) in [
            (ROOK_DIRS, [Kind::Rook, Kind::Queen]),
            (BISHOP_DIRS, [Kind::Bishop, Kind::Queen]),
        ] {
            for d in dirs {
                let mut at = sq;
                while let Some(next) = step(at, d) {
                    if let Some(p) = self.at(next) {
                        if p.color == by && kinds.contains(&p.kind) {
                            return true;
                        }
                        break;
                    }
                    at = next;
                }
            }
        }
        false
    }

    pub fn in_check(&self) -> bool {
        self.attacked(self.king(self.side), self.side.other())
    }

    /// Every move the side to move could make, ignoring whether it leaves its king in check.
    fn pseudo_moves(&self) -> Vec<Move> {
        let mut out = Vec::new();
        let us = self.side;
        for from in 0..64u8 {
            let Some(piece) = self.at(from) else { continue };
            if piece.color != us {
                continue;
            }
            let plain = |to: Square| Move {
                from,
                to,
                promotion: None,
            };
            match piece.kind {
                Kind::Pawn => {
                    let (dir, start, last) = if us == Color::White {
                        (1, 1, 7)
                    } else {
                        (-1, 6, 0)
                    };
                    let mut pawn = Vec::new();
                    if let Some(one) = step(from, (0, dir))
                        && self.at(one).is_none()
                    {
                        pawn.push(one);
                        if rank_of(from) == start
                            && let Some(two) = step(one, (0, dir))
                            && self.at(two).is_none()
                        {
                            pawn.push(two);
                        }
                    }
                    for df in [-1, 1] {
                        if let Some(to) = step(from, (df, dir))
                            && (self.at(to).is_some_and(|p| p.color != us)
                                || self.en_passant == Some(to))
                        {
                            pawn.push(to);
                        }
                    }
                    for to in pawn {
                        if rank_of(to) == last {
                            for k in [Kind::Queen, Kind::Rook, Kind::Bishop, Kind::Knight] {
                                out.push(Move {
                                    from,
                                    to,
                                    promotion: Some(k),
                                });
                            }
                        } else {
                            out.push(Move {
                                from,
                                to,
                                promotion: None,
                            });
                        }
                    }
                }
                Kind::Knight | Kind::King => {
                    let steps = if piece.kind == Kind::Knight {
                        KNIGHT
                    } else {
                        KING
                    };
                    for d in steps {
                        if let Some(to) = step(from, d)
                            && self.at(to).is_none_or(|p| p.color != us)
                        {
                            out.push(plain(to));
                        }
                    }
                    if piece.kind == Kind::King {
                        self.castles(from, &mut out);
                    }
                }
                Kind::Bishop | Kind::Rook | Kind::Queen => {
                    let dirs: &[(i8, i8)] = match piece.kind {
                        Kind::Bishop => &BISHOP_DIRS,
                        Kind::Rook => &ROOK_DIRS,
                        _ => &[
                            (1, 0),
                            (-1, 0),
                            (0, 1),
                            (0, -1),
                            (1, 1),
                            (1, -1),
                            (-1, 1),
                            (-1, -1),
                        ],
                    };
                    for &d in dirs {
                        let mut at = from;
                        while let Some(to) = step(at, d) {
                            match self.at(to) {
                                None => out.push(plain(to)),
                                Some(p) => {
                                    if p.color != us {
                                        out.push(plain(to));
                                    }
                                    break;
                                }
                            }
                            at = to;
                        }
                    }
                }
            }
        }
        out
    }

    /// Castling from `king`, when the rights, the empty squares and the attacks allow it.
    fn castles(&self, king: Square, out: &mut Vec<Move>) {
        let us = self.side;
        let home = if us == Color::White { 4 } else { 60 };
        if king != home || self.in_check() {
            return;
        }
        let c = self.castling;
        let (short, long) = if us == Color::White {
            (c.white_king, c.white_queen)
        } else {
            (c.black_king, c.black_queen)
        };
        let rook = Some(Piece {
            color: us,
            kind: Kind::Rook,
        });
        let empty = |squares: &[u8]| squares.iter().all(|&s| self.at(home - 4 + s).is_none());
        let safe = |squares: &[u8]| {
            squares
                .iter()
                .all(|&s| !self.attacked(home - 4 + s, us.other()))
        };
        if short && self.at(home + 3) == rook && empty(&[5, 6]) && safe(&[5, 6]) {
            out.push(Move {
                from: king,
                to: home + 2,
                promotion: None,
            });
        }
        if long && self.at(home - 4) == rook && empty(&[1, 2, 3]) && safe(&[2, 3]) {
            out.push(Move {
                from: king,
                to: home - 2,
                promotion: None,
            });
        }
    }

    /// The position after `m`, which must be one of [`Position::legal_moves`].
    pub fn play(&self, m: Move) -> Self {
        let mut next = self.clone();
        let piece = self.at(m.from).expect("a move starts on a piece");
        let captured = self.at(m.to);
        next.board[m.from as usize] = None;
        next.board[m.to as usize] = Some(match m.promotion {
            Some(kind) => Piece {
                color: piece.color,
                kind,
            },
            None => piece,
        });
        next.en_passant = None;
        if piece.kind == Kind::Pawn {
            if Some(m.to) == self.en_passant {
                // The captured pawn is beside the mover, not on the square it lands on.
                let behind = if piece.color == Color::White {
                    m.to - 8
                } else {
                    m.to + 8
                };
                next.board[behind as usize] = None;
            }
            if m.to.abs_diff(m.from) == 16 {
                next.en_passant = Some((m.from + m.to) / 2);
            }
        }
        if piece.kind == Kind::King && m.to.abs_diff(m.from) == 2 {
            // Castling: the rook jumps over the king.
            let (rook_from, rook_to) = if m.to > m.from {
                (m.from + 3, m.from + 1)
            } else {
                (m.from - 4, m.from - 1)
            };
            next.board[rook_to as usize] = next.board[rook_from as usize].take();
        }
        // A king or rook moving, or a rook being taken, ends the castling it could make.
        for sq in [m.from, m.to] {
            match sq {
                4 => {
                    next.castling.white_king = false;
                    next.castling.white_queen = false;
                }
                60 => {
                    next.castling.black_king = false;
                    next.castling.black_queen = false;
                }
                0 => next.castling.white_queen = false,
                7 => next.castling.white_king = false,
                56 => next.castling.black_queen = false,
                63 => next.castling.black_king = false,
                _ => {}
            }
        }
        next.halfmove = if piece.kind == Kind::Pawn || captured.is_some() {
            0
        } else {
            self.halfmove + 1
        };
        if self.side == Color::Black {
            next.fullmove += 1;
        }
        next.side = self.side.other();
        next
    }

    pub fn legal_moves(&self) -> Vec<Move> {
        self.pseudo_moves()
            .into_iter()
            .filter(|&m| {
                let next = self.play(m);
                !next.attacked(next.king(self.side), next.side)
            })
            .collect()
    }

    /// `m`, if it is legal here.
    pub fn legal(&self, m: Move) -> Option<Move> {
        self.legal_moves().into_iter().find(|&l| l == m)
    }

    /// A coordinate move (`e2e4`), if it is legal here.
    pub fn legal_coordinate(&self, s: &str) -> Option<Move> {
        self.legal(Move::parse_coordinate(s)?)
    }

    /// Whether neither side can ever mate: kings alone, or with one bishop or knight between
    /// them.
    pub fn insufficient_material(&self) -> bool {
        let others: Vec<Kind> = self
            .board
            .iter()
            .flatten()
            .map(|p| p.kind)
            .filter(|&k| k != Kind::King)
            .collect();
        match others.as_slice() {
            [] => true,
            [k] => matches!(k, Kind::Bishop | Kind::Knight),
            _ => false,
        }
    }

    /// `m` in standard algebraic notation (`Nf3`, `exd5`, `O-O`, `e8=Q#`). `m` must be legal.
    pub fn san(&self, m: Move) -> String {
        let piece = self.at(m.from).expect("a move starts on a piece");
        let mut s = String::new();
        if piece.kind == Kind::King && m.to.abs_diff(m.from) == 2 {
            s.push_str(if m.to > m.from { "O-O" } else { "O-O-O" });
        } else {
            let capture = self.at(m.to).is_some()
                || (piece.kind == Kind::Pawn && Some(m.to) == self.en_passant);
            if piece.kind == Kind::Pawn {
                if capture {
                    s.push((b'a' + file_of(m.from)) as char);
                }
            } else {
                s.push(piece.kind.letter());
                // Name the file, the rank or both of where it came from, if another piece of
                // the same kind could go there too.
                let rivals: Vec<Square> = self
                    .legal_moves()
                    .into_iter()
                    .filter(|o| {
                        o.to == m.to
                            && o.from != m.from
                            && self.at(o.from).map(|p| p.kind) == Some(piece.kind)
                    })
                    .map(|o| o.from)
                    .collect();
                if !rivals.is_empty() {
                    let file_alone = rivals.iter().all(|&r| file_of(r) != file_of(m.from));
                    let rank_alone = rivals.iter().all(|&r| rank_of(r) != rank_of(m.from));
                    let name = square_name(m.from);
                    if file_alone {
                        s.push_str(&name[..1]);
                    } else if rank_alone {
                        s.push_str(&name[1..]);
                    } else {
                        s.push_str(&name);
                    }
                }
            }
            if capture {
                s.push('x');
            }
            s.push_str(&square_name(m.to));
            if let Some(k) = m.promotion {
                s.push('=');
                s.push(k.letter());
            }
        }
        let next = self.play(m);
        if next.in_check() {
            s.push(if next.legal_moves().is_empty() {
                '#'
            } else {
                '+'
            });
        }
        s
    }

    /// The legal move written `san`, ignoring the `+`, `#`, `!` and `?` that may follow it.
    pub fn parse_san(&self, san: &str) -> Option<Move> {
        let bare = |s: &str| {
            s.trim_end_matches(['+', '#', '!', '?'])
                .replace('0', "O")
                .to_string()
        };
        let wanted = bare(san.trim());
        self.legal_moves()
            .into_iter()
            .find(|&m| bare(&self.san(m)) == wanted)
    }
}

/// A game from the starting position: its moves and every position along the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    positions: Vec<Position>,
    moves: Vec<Move>,
}

impl Default for Game {
    fn default() -> Self {
        Self {
            positions: alloc::vec![Position::default()],
            moves: Vec::new(),
        }
    }
}

impl Game {
    pub fn position(&self) -> &Position {
        self.positions.last().expect("a game always has a position")
    }

    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    /// Make `m` if it is legal; returns whether it was.
    pub fn play(&mut self, m: Move) -> bool {
        if self.outcome().is_some() {
            return false;
        }
        let Some(m) = self.position().legal(m) else {
            return false;
        };
        let next = self.position().play(m);
        self.positions.push(next);
        self.moves.push(m);
        true
    }

    /// Take back the last move, if there is one.
    pub fn undo(&mut self) -> Option<Move> {
        let m = self.moves.pop()?;
        self.positions.pop();
        Some(m)
    }

    /// The moves in standard algebraic notation, in order.
    pub fn san_moves(&self) -> Vec<String> {
        self.moves
            .iter()
            .zip(&self.positions)
            .map(|(&m, pos)| pos.san(m))
            .collect()
    }

    /// How the game has ended, if it has.
    pub fn outcome(&self) -> Option<Outcome> {
        let pos = self.position();
        if pos.legal_moves().is_empty() {
            return Some(if pos.in_check() {
                Outcome::Checkmate(pos.side)
            } else {
                Outcome::Stalemate
            });
        }
        if pos.insufficient_material() {
            return Some(Outcome::InsufficientMaterial);
        }
        if pos.halfmove >= 100 {
            return Some(Outcome::FiftyMoves);
        }
        // The same position: the same pieces, side, castling and en passant, the counters aside.
        let key = |p: &Position| (p.board, p.side, p.castling, p.en_passant);
        let now = key(pos);
        if self.positions.iter().filter(|p| key(p) == now).count() >= 3 {
            return Some(Outcome::Repetition);
        }
        None
    }
}

#[cfg(test)]
mod tests;
