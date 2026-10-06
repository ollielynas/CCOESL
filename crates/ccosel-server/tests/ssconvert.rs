//! The Spreadsheet server against the real `ssconvert`, end to end: open a CSV with formulas,
//! edit a cell and see what it changed, save as `.xlsx`, `.ods` and `.gnumeric`, and open each
//! again.
//!
//! Gnumeric isn't installed on the CI runner, so this only runs when asked to:
//!
//! ```sh
//! CCOSEL_TEST_SSCONVERT=1 cargo test -p ccosel-server --test ssconvert
//! ```
//!
//! Without the variable it prints that it was skipped and passes. Once the runner has
//! `gnumeric`, CI can set the variable for this test and it becomes a real gate.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ccosel_proto::sheet::{CellView, RangeReq, SheetDone, SheetOp, SheetReq};
use ccosel_server::fs_api::Jail;
use ccosel_server::sheet_api::{Sheets, Ssconvert};

fn enabled() -> bool {
    let on = std::env::var_os("CCOSEL_TEST_SSCONVERT").is_some_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set CCOSEL_TEST_SSCONVERT=1 to run this against the real ssconvert");
    }
    on
}

fn wait(sheets: &Sheets, jail: &Arc<Jail>, req: &SheetReq) -> SheetDone {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(result) = sheets.run(jail, None, req).unwrap().result {
            return result.unwrap_or_else(|e| panic!("{req:?} failed: {e}"));
        }
        assert!(Instant::now() < deadline, "{req:?} never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn cells(sheets: &Sheets, session: u64) -> Vec<CellView> {
    sheets
        .range(
            None,
            &RangeReq {
                session,
                sheet: 0,
                version: 0,
                row: 0,
                col: 0,
                rows: 20,
                cols: 5,
            },
        )
        .unwrap()
        .cells
}

/// `(raw, shown)` of the cell at `name`'s row and column.
fn at(cells: &[CellView], row: u32, col: u32) -> (String, String) {
    cells
        .iter()
        .find(|c| (c.row, c.col) == (row, col))
        .map(|c| (c.raw.clone(), c.shown.clone()))
        .unwrap_or_default()
}

fn pair(raw: &str, shown: &str) -> (String, String) {
    (raw.to_owned(), shown.to_owned())
}

#[test]
fn open_edit_save_and_open_again_with_real_gnumeric() {
    if !enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("ccosel-ssconvert-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("t.csv"),
        "item,count,total\nA,1,=B2*2\nB,3,=B3*2\nsum,,=SUM(C2:C3)\nbad,0,=1/B5\nhalf,0.1,=B6*3\n",
    )
    .unwrap();
    let jail = Arc::new(Jail::new(&dir).unwrap());
    let sheets = Sheets::new(Arc::new(Ssconvert::default())).unwrap();

    let open = |seq, path: &str| {
        let op = SheetOp::Open { path: path.into() };
        wait(
            &sheets,
            &jail,
            &SheetReq {
                session: 0,
                seq,
                op,
            },
        )
    };
    let opened = open(1, "/t.csv");
    let id = opened.session;
    let c = cells(&sheets, id);
    assert_eq!(at(&c, 1, 2), pair("=B2*2", "2"));
    // Gnumeric stores this one as a repeat of the formula above; it is written out here.
    assert_eq!(at(&c, 2, 2), pair("=B3*2", "6"));
    // Gnumeric keeps function names in lower case.
    assert_eq!(at(&c, 3, 2), pair("=sum(C2:C3)", "8"));
    assert_eq!(at(&c, 4, 2).1, "#DIV/0!", "errors are shown");
    assert_eq!(at(&c, 5, 1), pair("0.1", "0.1"));
    assert_eq!(at(&c, 5, 2).1, "0.3", "shown as displayed, not every digit");

    // An edit changes the cell and everything that depends on it.
    let op = SheetOp::Edit {
        sheet: 0,
        row: 1,
        col: 1,
        raw: "10".into(),
    };
    let edited = wait(
        &sheets,
        &jail,
        &SheetReq {
            session: id,
            seq: 2,
            op,
        },
    );
    assert_eq!(edited.version, 2);
    let changed: Vec<_> = edited
        .changed
        .iter()
        .map(|c| (c.row, c.col, c.shown.as_str()))
        .collect();
    assert_eq!(changed, [(1, 1, "10"), (1, 2, "20"), (3, 2, "26")]);

    for (seq, path) in [(3, "/out.xlsx"), (4, "/out.ods"), (5, "/out.gnumeric")] {
        let op = SheetOp::Save { path: path.into() };
        wait(
            &sheets,
            &jail,
            &SheetReq {
                session: id,
                seq,
                op,
            },
        );
        let again = open(seq + 10, path);
        let c = cells(&sheets, again.session);
        assert_eq!(at(&c, 1, 1).1, "10", "{path} keeps the edit");
        assert_eq!(at(&c, 3, 2), pair("=sum(C2:C3)", "26"), "{path}");
        assert_eq!(at(&c, 2, 2).0, "=B3*2", "{path}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
