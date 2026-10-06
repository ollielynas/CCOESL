use ccosel_proto::fs::{AccessReply, DirEntry, DirListing, EntryKind};
use ccosel_proto::sheet::{RangeReply, SheetStatus};
use ccosel_sdk::testing::{Harness, rpc_error};

use super::*;

const SESSION: u64 = 77;

const START: &str =
    "Open a .xlsx, .ods, .gnumeric or .csv file to start, or press New for an empty one.";

fn view(row: u32, col: u32, raw: &str, shown: &str) -> CellView {
    CellView {
        row,
        col,
        raw: raw.into(),
        shown: shown.into(),
    }
}

fn done(version: u32, sheets: &[&str], changed: Vec<CellView>) -> SheetStatus {
    SheetStatus {
        finished: true,
        result: Some(Ok(SheetDone {
            session: SESSION,
            sheets: sheets.iter().map(|s| s.to_string()).collect(),
            version,
            changed,
        })),
    }
}

fn failed(why: &str) -> SheetStatus {
    SheetStatus {
        finished: true,
        result: Some(Err(why.into())),
    }
}

fn running() -> SheetStatus {
    SheetStatus {
        finished: false,
        result: None,
    }
}

fn window(cells: Vec<CellView>) -> RangeReply {
    RangeReply {
        cells,
        last_row: 3,
        last_col: 2,
    }
}

/// Clicks `label`, draws the frame that acts on it, then the one that shows what it did.
fn press(h: &mut Harness<Spreadsheet>, label: &str) {
    h.click(label);
    h.frame();
    h.frame();
}

/// The cells of the window as drawn, by text.
fn shown(h: &Harness<Spreadsheet>) -> Vec<String> {
    h.selectables()
        .into_iter()
        .map(|(text, _)| text)
        .filter(|t| t != " ")
        .collect()
}

/// A spreadsheet with `/book.xlsx` open on `sheets`, its window holding A1 `item`, B2 `=A2*2`
/// showing 4, and A2 `2`.
fn opened(sheets: &[&str]) -> Harness<Spreadsheet> {
    let mut h = Harness::new(Spreadsheet::default());
    h.frame();
    h.type_text(0, "/book.xlsx");
    press(&mut h, "📂 Open");
    assert_eq!(h.outstanding::<Sheet>(), 1);
    h.reply::<Sheet>(&done(1, sheets, vec![]));
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<SheetRange>(), 1);
    h.reply::<SheetRange>(&window(vec![
        view(0, 0, "item", "item"),
        view(1, 0, "2", "2"),
        view(1, 1, "=A2*2", "4"),
    ]));
    h.frame();
    h.frame();
    h
}

#[test]
fn before_opening_anything_it_says_how_to_start() {
    let mut h = Harness::new(Spreadsheet::default());
    h.frame();
    assert!(h.has_label(START));
    assert!(!h.has_button("💾 Save"));
    assert_eq!(h.outstanding::<SheetRange>(), 0);
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        ccosel_sdk::REPAINT_ON_INPUT_ONLY
    );
}

#[test]
fn opening_a_file_shows_a_window_of_its_cells() {
    let h = opened(&["Sheet1"]);
    assert!(h.has_label("Opened /book.xlsx"));
    assert_eq!(shown(&h), ["item", "2", "4"]);
    // Column letters and row numbers around exactly the window.
    assert!(h.has_label("A") && h.has_label("H") && !h.has_label("I"));
    assert!(h.has_label("1") && h.has_label("20") && !h.has_label("21"));
    assert_eq!(h.selectables().len(), (ROWS * COLS) as usize);
    // A1 is selected, and the formula bar holds it.
    assert!(h.has_label("A1"));
    assert_eq!(h.text_fields()[1], "item");
    assert!(h.has_button("💾 Save") && h.has_button("Save as"));
    // One sheet, so no tabs.
    assert!(!h.selectables().iter().any(|(t, _)| t == "Sheet1"));
}

#[test]
fn opening_is_polled_until_it_finishes() {
    let mut h = Harness::new(Spreadsheet::default());
    h.frame();
    h.type_text(0, "/big.ods");
    press(&mut h, "📂 Open");
    assert!(h.has_label("Opening…"));
    assert_eq!(h.app.wants_repaint_after_ms(), POLL_MS);
    h.reply::<Sheet>(&running());
    h.frame();
    // Asked again when the poll timer runs the app next.
    h.frame();
    assert_eq!(h.outstanding::<Sheet>(), 1, "asked again");
    h.reply::<Sheet>(&failed("Gnumeric couldn't do it: not a spreadsheet"));
    h.frame();
    h.frame();
    assert!(h.has_label("Could not open it: Gnumeric couldn't do it: not a spreadsheet"));
    assert!(h.has_label(START));
}

#[test]
fn a_refused_open_says_why() {
    let mut h = Harness::new(Spreadsheet::default());
    h.frame();
    h.type_text(0, "/private.xlsx");
    press(&mut h, "📂 Open");
    h.fail::<Sheet>(rpc_error::DENIED);
    h.frame();
    h.frame();
    assert!(h.has_label("Could not open it: permission denied"));
}

#[test]
fn open_needs_a_path() {
    let mut h = Harness::new(Spreadsheet::default());
    h.frame();
    press(&mut h, "📂 Open");
    assert!(h.has_label("Type the path of a spreadsheet to open."));
    assert_eq!(h.outstanding::<Sheet>(), 0);
}

#[test]
fn an_app_opened_on_a_file_opens_it() {
    let mut h = Harness::new(Spreadsheet::default());
    h.launch("/Shared/budget.xlsx");
    h.frame();
    assert_eq!(h.outstanding::<Sheet>(), 1);
    assert_eq!(h.text_fields()[0], "/Shared/budget.xlsx");
}

#[test]
fn selecting_a_cell_puts_what_it_holds_in_the_formula_bar() {
    let mut h = opened(&["Sheet1"]);
    press(&mut h, "4");
    assert!(h.has_label("B2"));
    assert_eq!(h.text_fields()[1], "=A2*2");
    assert!(h.selectables().contains(&("4".to_owned(), true)));
}

#[test]
fn an_edit_shows_at_once_and_its_results_follow() {
    let mut h = opened(&["Sheet1"]);
    press(&mut h, "2");
    h.type_text(1, "5");
    h.frame();
    press(&mut h, "✔ Set");
    // As typed, before the server has answered.
    assert!(shown(&h).contains(&"5".to_owned()));
    assert!(h.has_label("Calculating…"));
    assert_eq!(h.outstanding::<Sheet>(), 1);

    h.reply::<Sheet>(&done(
        2,
        &["Sheet1"],
        vec![view(1, 0, "5", "5"), view(1, 1, "=A2*2", "10")],
    ));
    h.frame();
    h.frame();
    assert_eq!(shown(&h), ["item", "5", "10"]);
    assert!(!h.has_label("Calculating…"));
    // The window is read again for the new version.
    assert_eq!(h.outstanding::<SheetRange>(), 1);
    assert_eq!(h.app.version, 2);
}

#[test]
fn errors_from_formulas_are_shown_in_their_cells() {
    let mut h = opened(&["Sheet1"]);
    press(&mut h, "item");
    h.type_text(1, "=1/0");
    h.frame();
    press(&mut h, "✔ Set");
    h.reply::<Sheet>(&done(2, &["Sheet1"], vec![view(0, 0, "=1/0", "#DIV/0!")]));
    h.frame();
    assert!(shown(&h).contains(&"#DIV/0!".to_owned()));
}

#[test]
fn edits_made_while_one_is_calculating_wait_their_turn() {
    let mut h = opened(&["Sheet1"]);
    press(&mut h, "item");
    h.type_text(1, "first");
    h.frame();
    press(&mut h, "✔ Set");
    press(&mut h, "2");
    h.type_text(1, "second");
    h.frame();
    press(&mut h, "✔ Set");
    assert_eq!(h.outstanding::<Sheet>(), 1, "one at a time");
    assert_eq!(h.app.queue.len(), 1);
    assert!(shown(&h).contains(&"first".to_owned()) && shown(&h).contains(&"second".to_owned()));

    h.reply::<Sheet>(&done(2, &["Sheet1"], vec![view(0, 0, "first", "first")]));
    h.frame();
    h.frame();
    // The second goes now, after the first.
    assert_eq!(h.outstanding::<Sheet>(), 1);
    let Some((_, req)) = &h.app.job else {
        panic!("the second edit should be in flight");
    };
    assert!(matches!(&req.op, SheetOp::Edit { raw, .. } if raw == "second"));
}

#[test]
fn setting_what_a_cell_already_holds_does_nothing() {
    let mut h = opened(&["Sheet1"]);
    press(&mut h, "item");
    press(&mut h, "✔ Set");
    assert_eq!(h.outstanding::<Sheet>(), 0);
    assert!(h.app.queue.is_empty());
}

#[test]
fn the_formula_bar_waits_for_the_cell_unless_typed_in() {
    let mut h = opened(&["Sheet1"]);
    press(&mut h, "▼");
    // Selected before its window has arrived: empty for now.
    h.type_text(2, "A21");
    h.frame();
    press(&mut h, "Go");
    assert_eq!(h.text_fields()[1], "");
    h.type_text(1, "mine");
    h.frame();
    // A21 is in the window already asked for, which now arrives.
    h.reply::<SheetRange>(&window(vec![view(20, 0, "theirs", "theirs")]));
    h.frame();
    h.frame();
    assert_eq!(h.text_fields()[1], "mine", "what was typed is kept");
}

#[test]
fn a_failed_edit_goes_back_to_what_the_cell_showed() {
    let mut h = opened(&["Sheet1"]);
    press(&mut h, "item");
    h.type_text(1, "oops");
    h.frame();
    press(&mut h, "✔ Set");
    h.reply::<Sheet>(&failed("that cell is outside the sheet"));
    h.frame();
    h.frame();
    assert!(h.has_label("Could not change the cell: that cell is outside the sheet"));
    assert!(shown(&h).contains(&"item".to_owned()));
}

#[test]
fn saving_writes_back_to_the_file_it_came_from() {
    let mut h = opened(&["Sheet1"]);
    press(&mut h, "💾 Save");
    assert!(h.has_label("Saving…"));
    let Some((Kind::Save, req)) = &h.app.job else {
        panic!("a save should be in flight");
    };
    assert_eq!(
        req.op,
        SheetOp::Save {
            path: "/book.xlsx".into()
        }
    );
    assert_eq!(req.session, SESSION);
    h.reply::<Sheet>(&done(1, &["Sheet1"], vec![]));
    h.frame();
    h.frame();
    assert!(h.has_label("Saved /book.xlsx"));
}

#[test]
fn save_as_writes_where_the_box_says_in_a_known_format() {
    let mut h = opened(&["Sheet1"]);
    h.type_text(0, "/notes.txt");
    h.frame();
    press(&mut h, "Save as");
    assert!(h.has_label(
        "Spreadsheets save as .xlsx, .ods, .gnumeric or .csv: type a name ending in one of those."
    ));
    assert_eq!(h.outstanding::<Sheet>(), 0);

    h.type_text(0, "/copy.ods");
    h.frame();
    press(&mut h, "Save as");
    h.fail::<Sheet>(rpc_error::DENIED);
    h.frame();
    h.frame();
    assert!(h.has_label("Could not save it: permission denied"));
}

#[test]
fn nothing_else_starts_while_a_change_is_in_flight() {
    let mut h = opened(&["Sheet1"]);
    press(&mut h, "item");
    h.type_text(1, "x");
    h.frame();
    press(&mut h, "✔ Set");
    press(&mut h, "💾 Save");
    assert!(h.has_label("Wait for the last change to finish first."));
    press(&mut h, "📂 Open");
    assert!(h.has_label("Wait for the last change to finish first."));
    assert_eq!(h.outstanding::<Sheet>(), 1);
}

#[test]
fn the_window_moves_a_page_at_a_time_and_never_before_a1() {
    let mut h = opened(&["Sheet1"]);
    press(&mut h, "▼");
    assert!(h.has_label("21") && h.has_label("40") && !h.has_label("20"));
    press(&mut h, "▶");
    assert!(h.has_label("I") && h.has_label("P") && !h.has_label("H"));
    assert_eq!(
        h.outstanding::<SheetRange>(),
        2,
        "each window shown is asked for"
    );
    press(&mut h, "▲");
    press(&mut h, "▲");
    press(&mut h, "◀");
    press(&mut h, "◀");
    assert_eq!((h.app.top, h.app.left), (0, 0));
}

#[test]
fn go_to_jumps_to_a_cell_and_selects_it() {
    let mut h = opened(&["Sheet1"]);
    h.type_text(2, "c30");
    h.frame();
    press(&mut h, "Go");
    assert!(h.has_label("C30"));
    assert_eq!((h.app.top, h.app.left), (29, 0), "C is on screen already");
    assert_eq!(h.text_fields()[2], "");

    h.type_text(2, "nowhere");
    h.frame();
    press(&mut h, "Go");
    assert!(h.has_label("Type a cell to go to, such as B20."));
}

#[test]
fn several_sheets_show_as_tabs() {
    let mut h = opened(&["Costs", "Totals"]);
    assert!(h.selectables().contains(&("Costs".to_owned(), true)));
    press(&mut h, "Totals");
    assert_eq!(h.app.sheet, 1);
    assert!(h.selectables().contains(&("Totals".to_owned(), true)));
    // The other sheet's cells are gone until its window is read.
    assert!(shown(&h).iter().all(|t| t == "Costs" || t == "Totals"));
    assert_eq!(h.outstanding::<SheetRange>(), 1);
}

#[test]
fn a_window_that_cannot_be_read_says_so() {
    let mut h = opened(&["Sheet1"]);
    press(&mut h, "▼");
    h.fail::<SheetRange>(rpc_error::TIMEOUT);
    h.frame();
    assert!(h.has_label("Could not read the sheet: timed out"));
}

#[test]
fn cell_names_are_read_like_a_spreadsheet() {
    assert_eq!(parse_cell("A1"), Some((0, 0)));
    assert_eq!(parse_cell(" b20 "), Some((19, 1)));
    assert_eq!(parse_cell("AA3"), Some((2, 26)));
    assert_eq!(parse_cell("ZZ1"), Some((0, 701)));
    for bad in ["", "A", "12", "A0", "1A", "A-1", "ABCD1", "A1B"] {
        assert_eq!(parse_cell(bad), None, "{bad:?}");
    }
}

fn entry(name: &str, kind: EntryKind) -> DirEntry {
    DirEntry {
        name: name.into(),
        kind,
        size: 10,
        mtime_s: 0,
        writable: true,
    }
}

fn listing(entries: Vec<DirEntry>) -> DirListing {
    DirListing {
        entries,
        truncated: false,
    }
}

/// Answers the sidebar's questions: who this is, and the listings of the shared folder and theirs.
fn sidebar_for(
    h: &mut Harness<Spreadsheet>,
    user: Option<&str>,
    home: DirListing,
    shared: DirListing,
) {
    h.reply::<Access>(&AccessReply {
        read: true,
        write: true,
        user: user.map(String::from),
    });
    h.frame();
    // The shared folder was asked for first, before it was known whose folder is theirs.
    h.reply::<ListDir>(&shared);
    if user.is_some() {
        h.reply::<ListDir>(&home);
    }
    h.frame();
}

#[test]
fn the_sidebar_lists_your_spreadsheets_and_the_shared_ones() {
    let mut h = Harness::new(Spreadsheet::default());
    h.frame();
    assert!(h.has_text("Loading…"));
    sidebar_for(
        &mut h,
        Some("ann"),
        listing(vec![
            entry("budget.xlsx", EntryKind::File),
            entry("notes.md", EntryKind::File),
        ]),
        listing(vec![
            entry("Reports", EntryKind::Dir),
            entry("home", EntryKind::Dir),
            entry("team.ods", EntryKind::File),
        ]),
    );
    assert!(h.has_text("🏠 My files") && h.has_text("👥 Shared"));
    let rows: Vec<String> = h.selectables().into_iter().map(|(t, _)| t).collect();
    assert_eq!(rows, ["📄 budget.xlsx", "📁 Reports", "📄 team.ods"]);

    // A folder opens to show what is in it, and closes again.
    press(&mut h, "📁 Reports");
    assert_eq!(h.outstanding::<ListDir>(), 1);
    h.reply::<ListDir>(&listing(vec![]));
    h.frame();
    assert!(h.has_button("📂 Reports") || h.selectables().iter().any(|(t, _)| t == "📂 Reports"));
    assert!(h.has_text("No spreadsheets"));
    press(&mut h, "📂 Reports");
    assert!(h.selectables().iter().any(|(t, _)| t == "📁 Reports"));
}

#[test]
fn clicking_a_spreadsheet_in_the_sidebar_opens_it() {
    let mut h = Harness::new(Spreadsheet::default());
    h.frame();
    sidebar_for(
        &mut h,
        None,
        listing(vec![]),
        listing(vec![entry("team.ods", EntryKind::File)]),
    );
    assert!(
        !h.has_text("🏠 My files"),
        "signed out: no folder of your own"
    );
    press(&mut h, "📄 team.ods");
    assert_eq!(h.text_fields()[0], "/team.ods");
    assert_eq!(h.outstanding::<Sheet>(), 1);
    h.reply::<Sheet>(&done(1, &["Sheet1"], vec![]));
    h.frame();
    h.frame();
    assert!(h.has_label("Opened /team.ods"));
    assert!(
        h.selectables().contains(&("📄 team.ods".to_owned(), true)),
        "the open one is marked"
    );
}

#[test]
fn a_folder_that_cannot_be_listed_says_so() {
    let mut h = Harness::new(Spreadsheet::default());
    h.frame();
    h.reply::<Access>(&AccessReply {
        read: true,
        write: true,
        user: None,
    });
    h.frame();
    h.fail::<ListDir>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_text("Can't be listed"));
}

#[test]
fn the_sidebar_folds_away() {
    let mut h = Harness::new(Spreadsheet::default());
    h.frame();
    assert!(h.has_text("👥 Shared"));
    press(&mut h, "◀ Files");
    assert!(!h.has_text("👥 Shared"));
    assert!(h.has_button("▶ Files"));
    press(&mut h, "▶ Files");
    assert!(h.has_text("👥 Shared"));
}

#[test]
fn a_new_spreadsheet_has_to_be_saved_as_something() {
    let mut h = Harness::new(Spreadsheet::default());
    h.frame();
    press(&mut h, "📄 New");
    assert!(h.has_label("Starting a new spreadsheet…"));
    let Some((_, req)) = &h.app.job else {
        panic!("a new spreadsheet should be on its way");
    };
    assert_eq!((req.session, &req.op), (0, &SheetOp::New));
    h.reply::<Sheet>(&done(1, &["Sheet1"], vec![]));
    h.frame();
    h.frame();
    assert!(h.has_label("A new spreadsheet. Type a name in File and press Save as to keep it."));
    assert!(h.has_label("A1"));

    press(&mut h, "💾 Save");
    assert!(
        h.has_label("This spreadsheet has no file yet: type a name in File and press Save as.")
    );
    assert_eq!(h.outstanding::<Sheet>(), 0);

    h.type_text(0, "/home/ann/new.xlsx");
    h.frame();
    press(&mut h, "Save as");
    h.reply::<Sheet>(&done(1, &["Sheet1"], vec![]));
    h.frame();
    h.frame();
    assert!(h.has_label("Saved /home/ann/new.xlsx"));
    // From now on Save writes there.
    press(&mut h, "💾 Save");
    let Some((_, req)) = &h.app.job else {
        panic!("a save should be in flight");
    };
    assert_eq!(
        req.op,
        SheetOp::Save {
            path: "/home/ann/new.xlsx".into()
        }
    );
}

#[test]
fn a_new_spreadsheet_that_fails_says_why() {
    let mut h = Harness::new(Spreadsheet::default());
    h.frame();
    press(&mut h, "📄 New");
    h.reply::<Sheet>(&failed("couldn't make room for it"));
    h.frame();
    h.frame();
    assert!(h.has_label("Could not start a new spreadsheet: couldn't make room for it"));
}

#[test]
fn spreadsheet_files_are_recognised_by_their_ending() {
    for yes in ["a.xlsx", "B.ODS", "c.gnumeric", "d.csv", "e.xls", "f.tsv"] {
        assert!(is_sheet(yes), "{yes}");
    }
    for no in ["a.md", "b.txt", "xlsx", "c.pdf"] {
        assert!(!is_sheet(no), "{no}");
    }
}
