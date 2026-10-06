//! The open, edit and save flow against a fake Gnumeric, so CI needs none. The real `ssconvert`
//! is exercised by `tests/ssconvert.rs`.

use std::sync::atomic::AtomicU32;

use super::*;
use crate::gnumeric::escape;

/// `fixture.xml`: three sheets as `ssconvert` writes them, with a formula repeated down a column
/// (C3 repeats C2's `=A2+B2`).
const FIXTURE: &str = include_str!("../gnumeric/fixture.xml");

/// Stands in for `ssconvert`. Opening "converts" by copying (the files it opens hold Gnumeric
/// XML already), saving copies the XML out, and a cell shows its own text, except that a formula
/// shows `f(<formula>)` and a probe shows the formula of the cell it names, written out for that
/// cell, as Gnumeric's `GET.FORMULA` does.
#[derive(Default)]
struct Fake {
    runs: AtomicU32,
    /// Fail every run with this.
    broken: Option<String>,
}

impl Fake {
    fn run(&self) -> Result<(), String> {
        self.runs.fetch_add(1, Ordering::Relaxed);
        match &self.broken {
            Some(why) => Err(why.clone()),
            None => Ok(()),
        }
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

impl Gnumeric for Fake {
    fn to_xml(&self, input: &Path, output: &Path) -> Result<(), String> {
        self.run()?;
        std::fs::copy(input, output).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn shown(&self, xml: &Path, _scratch: &Path, sheets: usize) -> Result<Vec<String>, String> {
        self.run()?;
        let book = Workbook::parse(&std::fs::read_to_string(xml).unwrap())?;
        assert_eq!(book.sheets.len(), sheets);
        Ok(book
            .sheets
            .iter()
            .map(|sheet| {
                let rows = sheet.cells.keys().map(|&(r, _)| r + 1).max().unwrap_or(0);
                let cols = sheet.cells.keys().map(|&(_, c)| c + 1).max().unwrap_or(0);
                let mut csv = String::new();
                for r in 0..rows {
                    let fields: Vec<String> = (0..cols)
                        .map(|c| match sheet.cells.get(&(r, c)) {
                            None => String::new(),
                            Some(cell) if cell.text == "=GET.FORMULA($C$3)" => "=A3+B3".into(),
                            Some(cell) if cell.text.starts_with('=') => {
                                format!("f({})", cell.text)
                            }
                            Some(cell) => cell.raw(),
                        })
                        .map(|f| csv_field(&f))
                        .collect();
                    csv.push_str(&fields.join(","));
                    csv.push('\n');
                }
                csv
            })
            .collect())
    }

    fn save(&self, xml: &Path, output: &Path) -> Result<(), String> {
        self.run()?;
        std::fs::copy(xml, output).map_err(|e| e.to_string())?;
        Ok(())
    }
}

struct Setup {
    dir: PathBuf,
    jail: Arc<Jail>,
    fake: Arc<Fake>,
    sheets: Sheets,
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A jail holding `book.xlsx` (the fixture) and a read-only folder `/ro`.
fn setup(name: &str, fake: Fake) -> Setup {
    let dir = std::env::temp_dir().join(format!("ccosel-sheet-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("ro")).unwrap();
    std::fs::write(dir.join("book.xlsx"), FIXTURE).unwrap();
    std::fs::write(dir.join("ro/.access"), "read: *\n").unwrap();
    std::fs::write(dir.join("ro/book.xlsx"), FIXTURE).unwrap();
    let fake = Arc::new(fake);
    let sheets = Sheets::new(fake.clone()).unwrap();
    Setup {
        jail: Arc::new(Jail::new(&dir).unwrap()),
        dir,
        fake,
        sheets,
    }
}

fn req(session: u64, seq: u32, op: SheetOp) -> SheetReq {
    SheetReq { session, seq, op }
}

fn open_op(path: &str) -> SheetOp {
    SheetOp::Open { path: path.into() }
}

fn edit_op(row: u32, col: u32, raw: &str) -> SheetOp {
    SheetOp::Edit {
        sheet: 0,
        row,
        col,
        raw: raw.into(),
    }
}

/// Polls `req` until its job finishes.
fn wait(s: &Setup, user: Option<&str>, req: &SheetReq) -> Result<SheetDone, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = s.sheets.run(&s.jail, user, req).unwrap();
        if let Some(result) = status.result {
            assert!(status.finished);
            return result;
        }
        assert!(Instant::now() < deadline, "the job never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn open(s: &Setup, user: Option<&str>, path: &str) -> SheetDone {
    wait(s, user, &req(0, 1, open_op(path))).unwrap()
}

fn range(s: &Setup, user: Option<&str>, session: u64, sheet: u16) -> Vec<CellView> {
    s.sheets
        .range(
            user,
            &RangeReq {
                session,
                sheet,
                version: 0,
                row: 0,
                col: 0,
                rows: 50,
                cols: 10,
            },
        )
        .unwrap()
        .cells
}

fn cell(cells: &[CellView], row: u32, col: u32) -> Option<&CellView> {
    cells.iter().find(|c| (c.row, c.col) == (row, col))
}

#[test]
fn opening_a_file_shows_its_sheets_and_every_cell_with_its_formula() {
    let s = setup("open", Fake::default());
    let done = open(&s, None, "/book.xlsx");
    assert_ne!(done.session, 0);
    assert_eq!(done.sheets, ["t.csv", "Second & more", "Empty"]);
    assert_eq!(done.version, 1);
    assert!(done.changed.is_empty());

    let cells = range(&s, None, done.session, 0);
    let a2 = cell(&cells, 1, 0).unwrap();
    assert_eq!((a2.raw.as_str(), a2.shown.as_str()), ("1", "1"));
    let c2 = cell(&cells, 1, 2).unwrap();
    assert_eq!(
        (c2.raw.as_str(), c2.shown.as_str()),
        ("=A2+B2", "f(=A2+B2)")
    );
    // The repeat of C2's formula has its own, written out for its row by the probe.
    let c3 = cell(&cells, 2, 2).unwrap();
    assert_eq!(c3.raw, "=A3+B3");
    // The probe itself is gone.
    assert!(cell(&cells, 5, 0).is_none());
    assert!(
        range(&s, None, done.session, 2).is_empty(),
        "the empty sheet"
    );
}

#[test]
fn a_range_holds_only_the_cells_asked_for() {
    let s = setup("range", Fake::default());
    let id = open(&s, None, "/book.xlsx").session;
    let reply = s
        .sheets
        .range(
            None,
            &RangeReq {
                session: id,
                sheet: 0,
                version: 1,
                row: 1,
                col: 1,
                rows: 2,
                cols: 1,
            },
        )
        .unwrap();
    let places: Vec<_> = reply.cells.iter().map(|c| (c.row, c.col)).collect();
    assert_eq!(places, [(1, 1), (2, 1)]);
    assert_eq!((reply.last_row, reply.last_col), (4, 2));

    let huge = RangeReq {
        session: id,
        sheet: 0,
        version: 1,
        row: 0,
        col: 0,
        rows: 1_000,
        cols: 1_000,
    };
    assert_eq!(
        s.sheets.range(None, &huge).err(),
        Some(server_error::TOO_LARGE)
    );
    let no_sheet = RangeReq { sheet: 9, ..huge };
    let no_sheet = RangeReq {
        rows: 1,
        cols: 1,
        ..no_sheet
    };
    assert_eq!(
        s.sheets.range(None, &no_sheet).err(),
        Some(server_error::NOT_FOUND)
    );
}

#[test]
fn an_edit_recalculates_and_reports_what_changed() {
    let s = setup("edit", Fake::default());
    let id = open(&s, None, "/book.xlsx").session;
    let done = wait(&s, None, &req(id, 2, edit_op(1, 1, "10"))).unwrap();
    assert_eq!(done.version, 2);
    assert_eq!(
        done.changed,
        [CellView {
            row: 1,
            col: 1,
            raw: "10".into(),
            shown: "10".into(),
        }]
    );
    // A new formula, and a cleared cell.
    let done = wait(&s, None, &req(id, 3, edit_op(9, 0, "=1/0"))).unwrap();
    assert_eq!(done.version, 3);
    assert_eq!(done.changed[0].shown, "f(=1/0)");
    let done = wait(&s, None, &req(id, 4, edit_op(0, 0, ""))).unwrap();
    assert_eq!(
        done.changed,
        [CellView {
            row: 0,
            col: 0,
            raw: String::new(),
            shown: String::new(),
        }]
    );
    let cells = range(&s, None, id, 0);
    assert!(cell(&cells, 0, 0).is_none());
    assert_eq!(cell(&cells, 9, 0).unwrap().raw, "=1/0");
}

#[test]
fn polling_an_edit_reports_on_it_without_making_it_again() {
    let s = setup("poll", Fake::default());
    let id = open(&s, None, "/book.xlsx").session;
    let edit = req(id, 2, edit_op(1, 1, "10"));
    wait(&s, None, &edit).unwrap();
    let runs = s.fake.runs.load(Ordering::Relaxed);
    let again = s.sheets.run(&s.jail, None, &edit).unwrap();
    assert_eq!(again.result.unwrap().unwrap().version, 2);
    assert_eq!(
        s.fake.runs.load(Ordering::Relaxed),
        runs,
        "nothing ran again"
    );
}

#[test]
fn an_edit_off_the_sheet_is_refused() {
    let s = setup("off", Fake::default());
    let id = open(&s, None, "/book.xlsx").session;
    let off = req(
        id,
        2,
        SheetOp::Edit {
            sheet: 7,
            row: 0,
            col: 0,
            raw: "x".into(),
        },
    );
    assert_eq!(
        s.sheets.run(&s.jail, None, &off).err(),
        Some(server_error::NOT_FOUND)
    );
}

#[test]
fn a_session_is_only_its_owners() {
    let s = setup("owner", Fake::default());
    let id = open(&s, Some("alice"), "/book.xlsx").session;
    assert!(!range(&s, Some("alice"), id, 0).is_empty());
    let range = RangeReq {
        session: id,
        sheet: 0,
        version: 1,
        row: 0,
        col: 0,
        rows: 1,
        cols: 1,
    };
    assert_eq!(
        s.sheets.range(Some("bob"), &range).err(),
        Some(server_error::NOT_FOUND)
    );
    assert_eq!(
        s.sheets.range(None, &range).err(),
        Some(server_error::NOT_FOUND)
    );
    assert_eq!(
        s.sheets
            .run(&s.jail, Some("bob"), &req(id, 2, edit_op(0, 0, "x")))
            .err(),
        Some(server_error::NOT_FOUND)
    );
}

#[test]
fn opening_checks_the_file_first() {
    let s = setup("checks", Fake::default());
    let run = |path: &str| s.sheets.run(&s.jail, None, &req(0, 1, open_op(path))).err();
    assert_eq!(run("/missing.xlsx"), Some(server_error::NOT_FOUND));
    assert_eq!(run("/ro"), Some(server_error::NOT_A_DIRECTORY));
    assert!(run("/../etc/passwd").is_some());
}

#[test]
fn what_gnumeric_says_when_it_fails_is_the_result() {
    let s = setup(
        "broken",
        Fake {
            broken: Some("Gnumeric couldn't do it: unsupported format".into()),
            ..Fake::default()
        },
    );
    let result = wait(&s, None, &req(0, 1, open_op("/book.xlsx")));
    assert_eq!(
        result.err().as_deref(),
        Some("Gnumeric couldn't do it: unsupported format")
    );
}

#[test]
fn a_file_that_is_not_a_workbook_fails_to_open() {
    let s = setup("garbage", Fake::default());
    std::fs::write(s.dir.join("notes.csv"), "not xml at all").unwrap();
    let result = wait(&s, None, &req(0, 1, open_op("/notes.csv")));
    assert!(result.unwrap_err().contains("couldn't be read"));
}

#[test]
fn saving_writes_the_workbook_where_asked() {
    let s = setup("save", Fake::default());
    let id = open(&s, None, "/book.xlsx").session;
    wait(&s, None, &req(id, 2, edit_op(1, 1, "a & b"))).unwrap();
    let done = wait(
        &s,
        None,
        &req(
            id,
            3,
            SheetOp::Save {
                path: "/copy.ods".into(),
            },
        ),
    )
    .unwrap();
    assert_eq!(done.session, id);
    let saved = std::fs::read_to_string(s.dir.join("copy.ods")).unwrap();
    assert!(saved.contains(&escape("a & b")), "the edit was saved");
}

#[test]
fn saving_needs_a_known_format_and_write_permission() {
    let s = setup("save-checks", Fake::default());
    let id = open(&s, None, "/ro/book.xlsx").session;
    let save = |seq, path: &str| {
        s.sheets
            .run(
                &s.jail,
                None,
                &req(id, seq, SheetOp::Save { path: path.into() }),
            )
            .err()
    };
    assert_eq!(save(2, "/copy.txt"), Some(server_error::MALFORMED));
    assert_eq!(save(3, "/ro/book.xlsx"), Some(server_error::DENIED));
    assert!(save(4, "/nowhere/book.xlsx").is_some());
    assert!(!s.dir.join("copy.txt").exists());
}

#[test]
fn a_person_has_only_a_few_workbooks_open_at_once() {
    let s = setup("limit", Fake::default());
    let first = open(&s, Some("alice"), "/book.xlsx").session;
    for seq in 2..=SESSIONS_PER_USER as u32 + 1 {
        std::thread::sleep(Duration::from_millis(2));
        wait(&s, Some("alice"), &req(0, seq, open_op("/book.xlsx"))).unwrap();
    }
    // Opening the fifth closed the least recently used: the first.
    let range = RangeReq {
        session: first,
        sheet: 0,
        version: 1,
        row: 0,
        col: 0,
        rows: 1,
        cols: 1,
    };
    assert_eq!(
        s.sheets.range(Some("alice"), &range).err(),
        Some(server_error::NOT_FOUND)
    );
    // Someone else's don't count.
    assert!(open(&s, Some("bob"), "/book.xlsx").session != 0);
    assert_eq!(
        s.sheets.sessions.lock().unwrap().len(),
        SESSIONS_PER_USER + 1
    );
}

#[test]
fn idle_sessions_are_closed_and_their_folders_removed() {
    let s = setup("idle", Fake::default());
    let id = open(&s, None, "/book.xlsx").session;
    let folder = s.sheets.root.join(id.to_string());
    assert!(folder.is_dir());
    s.sheets.sweep(Instant::now() + IDLE / 2);
    assert_eq!(s.sheets.sessions.lock().unwrap().len(), 1);
    s.sheets
        .sweep(Instant::now() + IDLE + Duration::from_secs(1));
    assert!(s.sheets.sessions.lock().unwrap().is_empty());
    assert!(!folder.exists());
}

#[test]
fn shown_cells_skip_the_empty_ones() {
    let shown = shown_cells("a,,c\n,\n\"x,y\",2\n");
    assert_eq!(shown.len(), 4);
    assert_eq!(shown[&(0, 2)], "c");
    assert_eq!(shown[&(2, 0)], "x,y");
    assert!(!shown.contains_key(&(1, 0)));
}
