//! The Spreadsheet app's methods. GNU Gnumeric on the server does the work: it reads and writes
//! the file formats and computes every formula. The app only draws a grid and edits cells.
//!
//! An open workbook is a **session** on the server, held as Gnumeric's own file so nothing the
//! app doesn't show (styles, names, charts) is lost when it is saved. `Sheet` runs one operation
//! on it (open, edit a cell, save) as a job, polled like `Compile`: the first call for a request
//! starts it, later ones report on it. `SheetRange` reads the cells of the window on screen.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Effect, Method, Query, Rpc};

/// What to do to a workbook.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SheetOp {
    /// Open a `.gnumeric`, `.xlsx`, `.ods`, `.csv` (or anything else Gnumeric reads) file as a
    /// new session.
    Open { path: String },
    /// Set one cell's content: a value, or a formula starting with `=`. Empty clears it.
    Edit {
        sheet: u16,
        row: u32,
        col: u32,
        raw: String,
    },
    /// Write the workbook to `path`, in the format its extension names.
    Save { path: String },
    /// Start a new, empty workbook of one sheet as a new session. It has no file until it is
    /// saved.
    New,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SheetReq {
    /// The session to act on; 0 for `Open`, which makes one.
    pub session: u64,
    /// Bumped by the app once per operation: what makes the same edit made twice two jobs.
    pub seq: u32,
    pub op: SheetOp,
}

/// One cell as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellView {
    pub row: u32,
    pub col: u32,
    /// What was typed: a value, or a formula such as `=SUM(A1:A3)`.
    pub raw: String,
    /// What it shows: the computed value, formatted, or an error such as `#DIV/0!`.
    pub shown: String,
}

/// What a finished operation left.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetDone {
    pub session: u64,
    pub sheets: Vec<String>,
    /// Bumped by every edit: part of `RangeReq`, so a range is read again after one.
    pub version: u32,
    /// For an edit, every cell whose shown value changed, on the sheet edited (at most
    /// [`MAX_CHANGED`]).
    pub changed: Vec<CellView>,
}

pub const MAX_CHANGED: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetStatus {
    pub finished: bool,
    /// Present when `finished`: what it left, or why it failed.
    pub result: Option<Result<SheetDone, String>>,
}

pub struct Sheet;

impl Rpc for Sheet {
    const METHOD: Method = Method::Sheet;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    // Repeating a request reports on the job it started, which is what makes polling safe.
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = SheetReq;
    type Reply = SheetStatus;
}

impl Query for Sheet {}

/// The most cells one range may ask for.
pub const MAX_RANGE_CELLS: u32 = 4_000;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RangeReq {
    pub session: u64,
    pub sheet: u16,
    /// The session's version the app has seen, so a new one is a new request.
    pub version: u32,
    pub row: u32,
    pub col: u32,
    pub rows: u32,
    pub cols: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeReply {
    /// The cells in the range that hold something.
    pub cells: Vec<CellView>,
    /// The last row and column holding anything on the sheet, if any does.
    pub last_row: u32,
    pub last_col: u32,
}

/// Read the cells of part of a sheet.
pub struct SheetRange;

impl Rpc for SheetRange {
    const METHOD: Method = Method::SheetRange;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = RangeReq;
    type Reply = RangeReply;
}

impl Query for SheetRange {}

/// A column's letters: A … Z, AA … ZZ, AAA …
pub fn column_name(mut col: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// A cell's name: `A1` for row 0, column 0.
pub fn cell_name(row: u32, col: u32) -> String {
    let mut s = column_name(col);
    s.push_str(&itoa(u64::from(row) + 1));
    s
}

fn itoa(mut n: u64) -> String {
    let mut buf = Vec::new();
    loop {
        buf.push(b'0' + (n % 10) as u8);
        n /= 10;
        if n == 0 {
            break;
        }
    }
    buf.reverse();
    String::from_utf8(buf).unwrap_or_default()
}

/// The file extensions `Save` writes.
pub const SAVE_FORMATS: [&str; 4] = ["gnumeric", "xlsx", "ods", "csv"];

/// Whether `path` names a format `Save` writes.
pub fn can_save_as(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && SAVE_FORMATS.contains(&&*ext.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_and_cells_are_named_like_a_spreadsheet() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(27), "AB");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
        assert_eq!(cell_name(0, 0), "A1");
        assert_eq!(cell_name(9, 2), "C10");
    }

    #[test]
    fn saving_needs_a_known_extension() {
        assert!(can_save_as("/a/b.xlsx"));
        assert!(can_save_as("b.ODS"));
        assert!(can_save_as("/x/data.csv"));
        assert!(!can_save_as("/a/b.txt"));
        assert!(!can_save_as("/a/.xlsx"));
        assert!(!can_save_as("/a/noext"));
    }
}
