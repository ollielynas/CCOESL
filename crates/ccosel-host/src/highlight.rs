//! Syntax colouring for code editors (`Cmd::TextEditCode`).
//!
//! A small hand-written lexer per language rather than a grammar engine: a grammar crate is
//! hundreds of KB of shell for every user, and colouring wants only to be right about the
//! common cases while being typed, not to parse. It never fails: anything it doesn't
//! recognise is plain text.

use std::ops::Range;

use ccosel_abi::CodeLang;
use egui::text::{LayoutJob, TextFormat};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Comment,
    Number,
}

/// The coloured runs of `text`, in order and not overlapping. What lies between them is plain.
pub fn spans(text: &str, lang: CodeLang) -> Vec<(Range<usize>, Token)> {
    match lang {
        CodeLang::Octave => octave(text),
    }
}

const OCTAVE_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "classdef",
    "continue",
    "do",
    "else",
    "elseif",
    "end",
    "end_try_catch",
    "end_unwind_protect",
    "endclassdef",
    "endenumeration",
    "endevents",
    "endfor",
    "endfunction",
    "endif",
    "endmethods",
    "endparfor",
    "endproperties",
    "endswitch",
    "endwhile",
    "enumeration",
    "events",
    "for",
    "function",
    "global",
    "if",
    "methods",
    "otherwise",
    "parfor",
    "persistent",
    "properties",
    "return",
    "switch",
    "try",
    "unwind_protect",
    "unwind_protect_cleanup",
    "until",
    "while",
];

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Octave, and most MATLAB: `%`/`#` line comments, `%{ … %}` block comments, `...`
/// continuations, `"…"` strings with backslash escapes, `'…'` strings with `''` for a quote,
/// told apart from the transpose `x'` by what comes just before the quote.
fn octave(text: &str) -> Vec<(Range<usize>, Token)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut in_block = false;
    while i < bytes.len() {
        // Block comments are whole lines: `%{` or `#{` alone on a line opens one, and the
        // matching closer alone on a line ends it.
        let at_line_start = i == 0 || bytes[i - 1] == b'\n';
        if at_line_start {
            let end = text[i..].find('\n').map_or(text.len(), |n| i + n);
            let line = text[i..end].trim();
            if in_block || line == "%{" || line == "#{" {
                in_block = !(line == "%}" || line == "#}");
                out.push((i..end, Token::Comment));
                i = end;
                continue;
            }
        }
        let b = bytes[i];
        let prev = (i > 0).then(|| bytes[i - 1]);
        let line_end = text[i..].find('\n').map_or(text.len(), |n| i + n);
        let start = i;
        match b {
            b'%' | b'#' => {
                out.push((i..line_end, Token::Comment));
                i = line_end;
            }
            b'.' if text[i..].starts_with("...") => {
                out.push((i..line_end, Token::Comment));
                i = line_end;
            }
            b'"' => {
                i += 1;
                while i < line_end {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                i = i.min(line_end);
                out.push((start..i, Token::String));
            }
            // After a name, a closing bracket, a dot or another quote it is a transpose.
            b'\'' if !prev.is_some_and(|p| is_ident(p) || b")]}.'".contains(&p)) => {
                i += 1;
                while i < line_end {
                    if bytes[i] == b'\'' {
                        if bytes.get(i + 1) == Some(&b'\'') {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                i = i.min(line_end);
                out.push((start..i, Token::String));
            }
            b'0'..=b'9' | b'.'
                if !prev.is_some_and(is_ident)
                    && (b != b'.' || bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) =>
            {
                i += 1;
                while i < bytes.len() {
                    let c = bytes[i];
                    let digit = c.is_ascii_hexdigit() || matches!(c, b'.' | b'x' | b'X');
                    let exponent_sign =
                        matches!(c, b'+' | b'-') && matches!(bytes[i - 1], b'e' | b'E');
                    if !(digit || exponent_sign) {
                        break;
                    }
                    i += 1;
                }
                // An imaginary unit: `3i`, `2.5j`.
                if i < bytes.len() && matches!(bytes[i], b'i' | b'j' | b'I' | b'J') {
                    i += 1;
                }
                out.push((start..i, Token::Number));
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                while i < bytes.len() && is_ident(bytes[i]) {
                    i += 1;
                }
                // `s.end` or `x.for` is a field, not a keyword.
                if prev != Some(b'.') && OCTAVE_KEYWORDS.contains(&&text[start..i]) {
                    out.push((start..i, Token::Keyword));
                }
            }
            _ => {
                // Step a whole character, so a range never splits one.
                i += text[i..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    out
}

/// Colours for each token, for light and dark themes.
fn colour(token: Token, dark: bool) -> egui::Color32 {
    let rgb = |r, g, b| egui::Color32::from_rgb(r, g, b);
    match (token, dark) {
        (Token::Keyword, false) => rgb(0x00, 0x33, 0xB3),
        (Token::Keyword, true) => rgb(0x56, 0x9C, 0xD6),
        (Token::String, false) => rgb(0x06, 0x7D, 0x17),
        (Token::String, true) => rgb(0xCE, 0x91, 0x78),
        (Token::Comment, false) => rgb(0x8C, 0x8C, 0x8C),
        (Token::Comment, true) => rgb(0x6A, 0x99, 0x55),
        (Token::Number, false) => rgb(0x17, 0x50, 0xEB),
        (Token::Number, true) => rgb(0xB5, 0xCE, 0xA8),
    }
}

/// `text` laid out for a code editor in `ui`'s monospace font, coloured by `spans`.
pub fn layout_job(
    ui: &egui::Ui,
    text: &str,
    spans: &[(Range<usize>, Token)],
    wrap_width: f32,
) -> LayoutJob {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let plain = ui.visuals().text_color();
    let dark = ui.visuals().dark_mode;
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let mut at = 0;
    let push = |job: &mut LayoutJob, range: Range<usize>, color, italics| {
        if !range.is_empty() {
            job.append(
                &text[range],
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color,
                    italics,
                    ..TextFormat::default()
                },
            );
        }
    };
    for (range, token) in spans {
        push(&mut job, at..range.start, plain, false);
        push(
            &mut job,
            range.clone(),
            colour(*token, dark),
            *token == Token::Comment,
        );
        at = range.end;
    }
    push(&mut job, at..text.len(), plain, false);
    job
}

/// Lexes each text once while it is unchanged, rather than every frame: egui's frame cache
/// keeps a result while it is asked for, and drops it the first frame it isn't.
#[derive(Default)]
pub struct Lexer;

impl egui::cache::ComputerMut<(&str, CodeLang), Vec<(Range<usize>, Token)>> for Lexer {
    fn compute(&mut self, (text, lang): (&str, CodeLang)) -> Vec<(Range<usize>, Token)> {
        spans(text, lang)
    }
}

pub type LexCache = egui::cache::FrameCache<Vec<(Range<usize>, Token)>, Lexer>;
