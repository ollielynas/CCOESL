//! Games as PGN, the text format every chess program reads and writes.
//!
//! Written with the seven tags PGN asks for and the moves in SAN. Read leniently: tags,
//! comments, variations, move numbers, annotation glyphs and the result are skipped, and each
//! move is matched against the legal moves of its position, so a game is only ever read as
//! legal moves. A game that starts anywhere but the usual position (a `FEN` tag) is refused.

use alloc::format;
use alloc::string::String;

use crate::{Game, Outcome};

/// The tags a saved game carries besides its moves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tags {
    pub white: String,
    pub black: String,
    /// `YYYY.MM.DD`, or `????.??.??` when unknown.
    pub date: String,
}

/// `game` as PGN text. Its result is the game's outcome, or `*` while it is still going.
pub fn write(game: &Game, tags: &Tags) -> String {
    write_with_result(game, tags, game.outcome().map_or("*", Outcome::result))
}

/// [`write`], with the result given: for a game that ended some way the moves don't show,
/// such as a resignation.
pub fn write_with_result(game: &Game, tags: &Tags, result: &str) -> String {
    let or_unknown = |s: &str, unknown: &'static str| {
        if s.is_empty() {
            String::from(unknown)
        } else {
            s.replace(['"', '\\'], "")
        }
    };
    let mut out = format!(
        "[Event \"Casual game\"]\n[Site \"CCOSEL\"]\n[Date \"{}\"]\n[Round \"-\"]\n\
         [White \"{}\"]\n[Black \"{}\"]\n[Result \"{result}\"]\n\n",
        or_unknown(&tags.date, "????.??.??"),
        or_unknown(&tags.white, "?"),
        or_unknown(&tags.black, "?"),
    );
    // Movetext, wrapped before 80 columns as the standard asks.
    let mut line = String::new();
    let mut push = |word: &str, out: &mut String| {
        if !line.is_empty() && line.len() + 1 + word.len() > 79 {
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    };
    for (i, san) in game.san_moves().iter().enumerate() {
        if i % 2 == 0 {
            push(&format!("{}.", i / 2 + 1), &mut out);
        }
        push(san, &mut out);
    }
    push(result, &mut out);
    out.push_str(&line);
    out.push('\n');
    out
}

/// The first game in `text`, and its tags.
pub fn read(text: &str) -> Result<(Game, Tags), String> {
    let mut tags = Tags::default();
    let mut game = Game::default();
    let mut rest = text;
    // Tags: `[Name "value"]` lines before the moves.
    loop {
        rest = rest.trim_start();
        let Some(tag) = rest.strip_prefix('[') else {
            break;
        };
        let end = tag.find(']').ok_or("a tag is not closed")?;
        let (name, value) = tag[..end].split_once(' ').unwrap_or((&tag[..end], ""));
        let value = value.trim().trim_matches('"');
        match name {
            "White" => tags.white = value.into(),
            "Black" => tags.black = value.into(),
            "Date" => tags.date = value.into(),
            "FEN" | "SetUp" if value != "0" => {
                return Err("games that start from a set-up position can't be opened".into());
            }
            _ => {}
        }
        rest = &tag[end + 1..];
    }

    let mut depth = 0usize; // inside `(...)` variations
    let mut chars = rest.char_indices().peekable();
    let mut word = String::new();
    let mut words = alloc::vec::Vec::new();
    while let Some((_, c)) = chars.next() {
        match c {
            '{' => {
                // A comment, to its closing brace.
                for (_, c) in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                }
            }
            ';' => {
                for (_, c) in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() => {
                if depth == 0 && !word.is_empty() {
                    words.push(core::mem::take(&mut word));
                }
                word.clear();
            }
            c if depth == 0 => word.push(c),
            _ => {}
        }
    }
    if !word.is_empty() {
        words.push(word);
    }

    for word in words {
        if matches!(word.as_str(), "1-0" | "0-1" | "1/2-1/2" | "*") {
            break;
        }
        // `12.`, `12...` and `12.e4` all carry a move number to drop.
        let san = word.rsplit('.').next().unwrap_or("");
        if san.is_empty() || san.starts_with('$') {
            continue;
        }
        let pos = game.position().clone();
        let m = pos
            .parse_san(san)
            .ok_or_else(|| format!("{san} isn't a legal move in move {}", pos.fullmove))?;
        game.play(m);
    }
    Ok((game, tags))
}
