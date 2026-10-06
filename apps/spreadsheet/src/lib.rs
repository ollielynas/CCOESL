//! The Spreadsheet app: a grid of cells, with GNU Gnumeric on the server doing the work.
//!
//! The app draws the part of a sheet that is on screen and edits one cell at a time. Opening,
//! recalculating and saving are the server's (`ccosel-server`'s `sheet_api`), which runs
//! Gnumeric's `ssconvert` on a copy of the workbook it keeps for this window (a session).
//!
//! Each of those is a job, polled like the Compiler's build: `Sheet` with a request starts it,
//! asking again with the same request reports on it. The app runs one at a time and keeps
//! edits made meanwhile in a queue, so they reach the server in the order they were made.
//!
//! An edit shows in its cell straight away, as typed, and the computed values follow when the
//! job finishes: the cells it changed come back with it, and the window on screen is read again.
//!
//! Only the window on screen is ever asked for (`SheetRange`), so a sheet of a million rows
//! costs what twenty rows do.

use std::collections::{BTreeMap, VecDeque};

use ccosel_proto::sheet::{
    CellView, RangeReq, Sheet, SheetDone, SheetOp, SheetRange, SheetReq, can_save_as, cell_name,
    column_name,
};
use ccosel_sdk::{App, Poll, Text, Ui};

/// The window of cells on screen.
pub const ROWS: u32 = 20;
pub const COLS: u32 = 8;

/// Four times a second while the server works: an edit's result should follow it closely.
const POLL_MS: u32 = 250;

/// What the job in flight is doing, for what to say about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Open,
    Edit,
    Save,
}

pub struct Spreadsheet {
    /// The file Open reads and Save as writes.
    path: Text,
    /// The selected cell's content, as typed.
    formula: Text,
    /// What the formula bar was last filled with from the cell, to tell whether it has been
    /// typed in since.
    loaded: String,
    /// The cell to go to, as a name like `B20`.
    goto: Text,
    /// The file the workbook was opened from, which Save writes back to.
    file: Option<String>,
    session: u64,
    sheets: Vec<String>,
    sheet: u16,
    /// The workbook's version as of the last finished job: part of every range request, so the
    /// window is read again after an edit.
    version: u32,
    /// The top-left cell of the window on screen.
    top: u32,
    left: u32,
    selected: (u32, u32),
    /// What is known of the current sheet's cells, `(raw, shown)`, from ranges and edits.
    cells: BTreeMap<(u32, u32), (String, String)>,
    /// Edits not yet recalculated, shown as typed until they are.
    typed: BTreeMap<(u16, u32, u32), String>,
    /// Edits waiting their turn.
    queue: VecDeque<SheetOp>,
    /// The job in flight.
    job: Option<(Kind, SheetReq)>,
    /// Numbers each request, so the same edit made twice is two jobs.
    seq: u32,
    status: Option<String>,
    /// Something was acted on after this frame was drawn: draw the next one now.
    changed: bool,
}

impl Default for Spreadsheet {
    fn default() -> Self {
        Self {
            path: Text::new(""),
            formula: Text::new(""),
            loaded: String::new(),
            goto: Text::new(""),
            file: None,
            session: 0,
            sheets: Vec::new(),
            sheet: 0,
            version: 0,
            top: 0,
            left: 0,
            selected: (0, 0),
            cells: BTreeMap::new(),
            typed: BTreeMap::new(),
            queue: VecDeque::new(),
            job: None,
            seq: 0,
            status: None,
            changed: false,
        }
    }
}

/// The `(row, column)` a cell name such as `B20` stands for, from 0. Case doesn't matter.
pub fn parse_cell(name: &str) -> Option<(u32, u32)> {
    let name = name.trim();
    let digits = name.find(|c: char| c.is_ascii_digit())?;
    let (letters, number) = name.split_at(digits);
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let mut col: u32 = 0;
    for b in letters.bytes() {
        if !b.is_ascii_alphabetic() {
            return None;
        }
        col = col * 26 + u32::from(b.to_ascii_uppercase() - b'A') + 1;
    }
    let row: u32 = number.parse().ok()?;
    (row > 0).then(|| (row - 1, col - 1))
}

/// What was clicked this frame, acted on once drawing is done.
#[derive(Default, PartialEq)]
struct Actions {
    open: bool,
    save: bool,
    save_as: bool,
    set: bool,
    goto: bool,
    /// Rows and columns to move the window by.
    scroll: (i64, i64),
    select: Option<(u32, u32)>,
    tab: Option<u16>,
}

fn moved(at: u32, by: i64) -> u32 {
    u32::try_from((i64::from(at) + by).max(0)).unwrap_or(u32::MAX)
}

impl Spreadsheet {
    fn toolbar(&mut self, ui: &mut Ui<'_>, act: &mut Actions) {
        ui.horizontal(|ui| {
            ui.label("File");
            ui.text_edit(&mut self.path);
            ui.tooltip("A .xlsx, .ods, .gnumeric or .csv file on the server");
            act.open = ui.button("📂 Open").clicked();
            if self.session != 0 {
                act.save = ui.button("💾 Save").clicked();
                ui.tooltip("Save to the file it was opened from");
                act.save_as = ui.button("Save as").clicked();
                ui.tooltip("Save to the file named in the box, in the format its ending names");
            }
        });
        if let Some((kind, _)) = &self.job {
            ui.label(match kind {
                Kind::Open => "Opening…",
                Kind::Edit => "Calculating…",
                Kind::Save => "Saving…",
            });
        }
        if let Some(status) = &self.status {
            ui.label(status);
        }
    }

    fn formula_bar(&mut self, ui: &mut Ui<'_>, act: &mut Actions) {
        ui.horizontal(|ui| {
            ui.label(&cell_name(self.selected.0, self.selected.1));
            ui.text_edit(&mut self.formula);
            ui.tooltip("A value, or a formula starting with =, such as =SUM(A1:A3)");
            act.set = ui.button("✔ Set").clicked();
            ui.label("Go to");
            ui.text_edit(&mut self.goto);
            act.goto = ui.button("Go").clicked();
        });
    }

    fn tabs(&self, ui: &mut Ui<'_>, act: &mut Actions) {
        if self.sheets.len() < 2 {
            return;
        }
        ui.horizontal(|ui| {
            for (i, name) in self.sheets.iter().enumerate() {
                ui.push_id(name, |ui| {
                    if ui.selectable(i == usize::from(self.sheet), name).clicked() {
                        act.tab = u16::try_from(i).ok();
                    }
                });
            }
        });
    }

    /// What a cell shows: an edit as typed until it is recalculated, otherwise its value.
    fn shown(&self, row: u32, col: u32) -> &str {
        if let Some(raw) = self.typed.get(&(self.sheet, row, col)) {
            return raw;
        }
        self.cells.get(&(row, col)).map_or(
            "",
            |(raw, shown)| if shown.is_empty() { raw } else { shown },
        )
    }

    fn grid(&self, ui: &mut Ui<'_>, act: &mut Actions) {
        ui.horizontal(|ui| {
            for (label, by) in [
                ("▲", (-i64::from(ROWS), 0)),
                ("▼", (i64::from(ROWS), 0)),
                ("◀", (0, -i64::from(COLS))),
                ("▶", (0, i64::from(COLS))),
            ] {
                if ui.button(label).clicked() {
                    act.scroll = by;
                }
            }
        });
        ui.table(|ui| {
            ui.row(|ui| {
                ui.label("");
                for col in self.left..self.left + COLS {
                    ui.label(&column_name(col));
                }
            });
            for row in self.top..self.top + ROWS {
                ui.push_id(&format!("r{row}"), |ui| {
                    ui.row(|ui| {
                        ui.label(&format!("{}", row + 1));
                        for col in self.left..self.left + COLS {
                            let text = self.shown(row, col);
                            // A space, so an empty cell is still something to click.
                            let text = if text.is_empty() { " " } else { text };
                            if ui.selectable(self.selected == (row, col), text).clicked() {
                                act.select = Some((row, col));
                            }
                        }
                    });
                });
            }
        });
    }

    fn range_req(&self) -> RangeReq {
        RangeReq {
            session: self.session,
            sheet: self.sheet,
            version: self.version,
            row: self.top,
            col: self.left,
            rows: ROWS,
            cols: COLS,
        }
    }

    /// Read the window on screen, and keep what it holds.
    fn read_window(&mut self, ui: &mut Ui<'_>) {
        if self.session == 0 {
            return;
        }
        let req = self.range_req();
        match ui.rpc().get::<SheetRange>(&req) {
            Poll::Ready(reply) => {
                let window = |&(r, c): &(u32, u32)| {
                    (req.row..req.row + ROWS).contains(&r) && (req.col..req.col + COLS).contains(&c)
                };
                self.cells.retain(|place, _| !window(place));
                for cell in &reply.cells {
                    self.cells
                        .insert((cell.row, cell.col), (cell.raw.clone(), cell.shown.clone()));
                }
                // The selected cell's content arrived after it was selected (just opened, say):
                // show it, unless something has been typed in the formula bar meanwhile.
                if self.formula.as_str() == self.loaded && self.raw(self.selected) != self.loaded {
                    self.load_formula();
                    self.changed = true;
                }
            }
            Poll::Pending => {}
            Poll::Failed(e) => {
                self.status = Some(format!("Could not read the sheet: {}", e.message()));
            }
        }
    }

    /// Start the next queued edit if nothing is in flight, then check on what is.
    fn poll_job(&mut self, ui: &mut Ui<'_>) {
        if self.job.is_none()
            && let Some(op) = self.queue.pop_front()
        {
            self.job = Some((Kind::Edit, self.request(op)));
        }
        let Some((kind, req)) = self.job.clone() else {
            return;
        };
        let rpc = ui.rpc();
        let result = match rpc.get::<Sheet>(&req) {
            Poll::Pending => return,
            Poll::Failed(e) => Err(e.message().to_owned()),
            Poll::Ready(status) => match &status.result {
                None => {
                    // Not done: ask again next time.
                    rpc.invalidate::<Sheet>(&req);
                    return;
                }
                Some(result) => result.clone(),
            },
        };
        rpc.invalidate::<Sheet>(&req);
        self.job = None;
        self.changed = true;
        match result {
            Ok(done) => self.finished(kind, &req, done),
            Err(why) => {
                let what = match kind {
                    Kind::Open => "open it",
                    Kind::Edit => "change the cell",
                    Kind::Save => "save it",
                };
                self.status = Some(format!("Could not {what}: {why}"));
                if let SheetOp::Edit {
                    sheet, row, col, ..
                } = req.op
                {
                    self.typed.remove(&(sheet, row, col));
                }
            }
        }
    }

    fn finished(&mut self, kind: Kind, req: &SheetReq, done: SheetDone) {
        match (kind, &req.op) {
            (Kind::Open, SheetOp::Open { path }) => {
                self.session = done.session;
                self.file = Some(path.clone());
                self.sheet = 0;
                self.top = 0;
                self.left = 0;
                self.cells.clear();
                self.typed.clear();
                self.queue.clear();
                self.select((0, 0));
                self.status = Some(format!("Opened {path}"));
            }
            (
                Kind::Edit,
                SheetOp::Edit {
                    sheet, row, col, ..
                },
            ) => {
                self.typed.remove(&(*sheet, *row, *col));
                if *sheet == self.sheet {
                    self.patch(&done.changed);
                }
            }
            (Kind::Save, SheetOp::Save { path }) => {
                self.status = Some(format!("Saved {path}"));
            }
            _ => {}
        }
        self.sheets = done.sheets;
        self.version = done.version;
    }

    /// Show what an edit changed straight away, before the window is read again.
    fn patch(&mut self, changed: &[CellView]) {
        for cell in changed {
            let place = (cell.row, cell.col);
            if cell.raw.is_empty() && cell.shown.is_empty() {
                self.cells.remove(&place);
            } else {
                self.cells
                    .insert(place, (cell.raw.clone(), cell.shown.clone()));
            }
        }
    }

    fn raw(&self, place: (u32, u32)) -> String {
        if let Some(raw) = self.typed.get(&(self.sheet, place.0, place.1)) {
            return raw.clone();
        }
        self.cells
            .get(&place)
            .map(|(raw, _)| raw.clone())
            .unwrap_or_default()
    }

    fn select(&mut self, place: (u32, u32)) {
        self.selected = place;
        self.load_formula();
    }

    fn load_formula(&mut self) {
        let raw = self.raw(self.selected);
        self.formula.set(&raw);
        self.loaded = raw;
    }

    fn request(&mut self, op: SheetOp) -> SheetReq {
        self.seq = self.seq.wrapping_add(1);
        SheetReq {
            session: if matches!(op, SheetOp::Open { .. }) {
                0
            } else {
                self.session
            },
            seq: self.seq,
            op,
        }
    }

    fn apply(&mut self, act: Actions) {
        self.changed |= act != Actions::default();
        if act.open {
            let path = self.path.as_str().trim().to_owned();
            if path.is_empty() {
                self.status = Some("Type the path of a spreadsheet to open.".to_owned());
            } else if self.job.is_some() || !self.queue.is_empty() {
                self.status = Some("Wait for the last change to finish first.".to_owned());
            } else {
                self.status = None;
                let req = self.request(SheetOp::Open { path });
                self.job = Some((Kind::Open, req));
            }
        }
        if act.save || act.save_as {
            let path = if act.save {
                self.file.clone().unwrap_or_default()
            } else {
                self.path.as_str().trim().to_owned()
            };
            if !can_save_as(&path) {
                self.status = Some(
                    "Spreadsheets save as .xlsx, .ods, .gnumeric or .csv: type a name ending in \
                     one of those."
                        .to_owned(),
                );
            } else if self.job.is_some() || !self.queue.is_empty() {
                self.status = Some("Wait for the last change to finish first.".to_owned());
            } else {
                self.status = None;
                let req = self.request(SheetOp::Save { path });
                self.job = Some((Kind::Save, req));
            }
        }
        if act.set && self.session != 0 {
            let (row, col) = self.selected;
            let raw = self.formula.as_str().to_owned();
            if raw != self.raw((row, col)) {
                self.typed.insert((self.sheet, row, col), raw.clone());
                self.queue.push_back(SheetOp::Edit {
                    sheet: self.sheet,
                    row,
                    col,
                    raw,
                });
                self.status = None;
            }
        }
        if act.goto {
            match parse_cell(self.goto.as_str()) {
                Some((row, col)) => {
                    // The cell becomes the window's top-left, unless it is on screen already.
                    if !(self.top..self.top + ROWS).contains(&row) {
                        self.top = row;
                    }
                    if !(self.left..self.left + COLS).contains(&col) {
                        self.left = col;
                    }
                    self.select((row, col));
                    self.goto.set("");
                    self.status = None;
                }
                None => {
                    self.status = Some("Type a cell to go to, such as B20.".to_owned());
                }
            }
        }
        if act.scroll != (0, 0) {
            self.top = moved(self.top, act.scroll.0);
            self.left = moved(self.left, act.scroll.1);
        }
        if let Some(place) = act.select {
            self.select(place);
        }
        if let Some(tab) = act.tab
            && tab != self.sheet
        {
            self.sheet = tab;
            self.cells.clear();
            self.top = 0;
            self.left = 0;
            self.select((0, 0));
        }
    }
}

impl App for Spreadsheet {
    fn update(&mut self, ui: &mut Ui<'_>) {
        self.changed = false;
        self.poll_job(ui);
        self.read_window(ui);

        // Each part under its own id, so a status line appearing above the grid doesn't change
        // the ids of the fields and cells below it.
        let mut act = Actions::default();
        ui.push_id("toolbar", |ui| self.toolbar(ui, &mut act));
        if self.session == 0 {
            ui.separator();
            ui.label("Open a .xlsx, .ods, .gnumeric or .csv file to start.");
        } else {
            ui.push_id("formula", |ui| self.formula_bar(ui, &mut act));
            ui.push_id("tabs", |ui| self.tabs(ui, &mut act));
            ui.push_id("grid", |ui| self.grid(ui, &mut act));
        }
        self.apply(act);
    }

    /// Opened on a spreadsheet file: open it.
    fn open(&mut self, arg: &str) {
        self.path.set(arg);
        let req = self.request(SheetOp::Open {
            path: arg.to_owned(),
        });
        self.job = Some((Kind::Open, req));
    }

    fn wants_repaint_after_ms(&self) -> u32 {
        if self.changed {
            0
        } else if self.job.is_some() || !self.queue.is_empty() {
            POLL_MS
        } else {
            ccosel_sdk::REPAINT_ON_INPUT_ONLY
        }
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(Spreadsheet);
