//! The Spreadsheet app's server side: GNU Gnumeric's `ssconvert` reads and writes the files and
//! computes the formulas; this keeps each open workbook (a **session**) and answers the app.
//!
//! A session is a workbook held as Gnumeric XML ([`Workbook`]) in a folder of its own outside the
//! jail, with the value every cell shows. Opening converts the file to that XML and works out
//! every cell's formula; editing sets one cell and recalculates; saving converts the XML to the
//! format the destination's extension names. Each of these runs `ssconvert` with a time limit,
//! and each is a job, polled like a build, so one that takes a while never holds up a call.
//!
//! An edit is applied to the workbook when it arrives, in the order edits arrive; only the
//! recalculation that follows runs as the job. So two quick edits to one cell end with the
//! second, whichever recalculation finishes first.
//!
//! A session belongs to whoever opened it: anyone else is told it isn't there. Opening and
//! saving check the jail's permissions exactly as reading and writing a file do, and saving
//! checks again just before it writes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ccosel_proto::server_error;
use ccosel_proto::sheet::{
    CellView, MAX_CHANGED, MAX_RANGE_CELLS, RangeReply, RangeReq, SheetDone, SheetOp, SheetReq,
    SheetStatus, can_save_as,
};

use crate::fs_api::{Jail, Need};
use crate::gnumeric::{Workbook, parse_csv};

/// The largest file opened, and the most cells a workbook may hold.
pub const MAX_FILE_BYTES: u64 = 50 << 20;
pub const MAX_CELLS: usize = 500_000;
/// Sessions each person may have open; opening another closes their least recently used one.
pub const SESSIONS_PER_USER: usize = 4;
/// A session nobody has used for this long is closed.
pub const IDLE: Duration = Duration::from_secs(2 * 60 * 60);
/// Finished jobs are remembered this long, for an app that comes back for the answer.
const JOB_RETENTION: Duration = Duration::from_secs(30 * 60);
/// How long one `ssconvert` run may take.
const SSCONVERT_LIMIT: Duration = Duration::from_secs(60);

/// What runs Gnumeric. `ssconvert` in the server, a fake in the tests, so CI needs no Gnumeric.
pub trait Gnumeric: Send + Sync {
    /// Convert `input`, in any format Gnumeric reads, to uncompressed Gnumeric XML at `output`.
    fn to_xml(&self, input: &Path, output: &Path) -> Result<(), String>;
    /// Recalculate the workbook at `xml` and return what each of its `sheets` shows, as CSV
    /// starting at A1, values formatted as they are displayed. `scratch` is a folder it may
    /// write in.
    fn shown(&self, xml: &Path, scratch: &Path, sheets: usize) -> Result<Vec<String>, String>;
    /// Recalculate `xml` and write it to `output`, in the format `output`'s extension names.
    fn save(&self, xml: &Path, output: &Path) -> Result<(), String>;
}

/// The real thing: `ssconvert` (or the program `CCOSEL_SSCONVERT` names), each run stopped
/// after a time limit.
pub struct Ssconvert {
    program: std::ffi::OsString,
    limit: Duration,
}

impl Default for Ssconvert {
    fn default() -> Self {
        Self {
            program: std::env::var_os("CCOSEL_SSCONVERT").unwrap_or_else(|| "ssconvert".into()),
            limit: SSCONVERT_LIMIT,
        }
    }
}

impl Ssconvert {
    fn run(&self, args: &[&std::ffi::OsStr]) -> Result<(), String> {
        let mut child = Command::new(&self.program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Gnumeric couldn't be started ({e}); is it installed?"))?;
        let mut stderr = child.stderr.take().expect("piped");
        let said = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut stderr, &mut s);
            s
        });
        let deadline = Instant::now() + self.limit;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => break None,
            }
        };
        let said = said.join().unwrap_or_default();
        match status {
            Some(s) if s.success() => Ok(()),
            Some(_) => Err(format!(
                "Gnumeric couldn't do it: {}",
                said.lines().last().unwrap_or("").trim()
            )),
            None => Err("Gnumeric took too long and was stopped".into()),
        }
    }
}

impl Gnumeric for Ssconvert {
    fn to_xml(&self, input: &Path, output: &Path) -> Result<(), String> {
        self.run(&[
            "-T".as_ref(),
            "Gnumeric_XmlIO:sax:0".as_ref(),
            input.as_os_str(),
            output.as_os_str(),
        ])
    }

    fn shown(&self, xml: &Path, scratch: &Path, sheets: usize) -> Result<Vec<String>, String> {
        let file = |i: usize| scratch.join(format!("shown.{i}.csv"));
        for i in 0..sheets {
            let _ = std::fs::remove_file(file(i));
        }
        let pattern = scratch.join("shown.%n.csv");
        // The configurable text exporter, not plain CSV: only it can write values as they are
        // displayed (`0.3`, not `0.30000000000000004`).
        self.run(&[
            "--recalc".as_ref(),
            "-S".as_ref(),
            "-O".as_ref(),
            "separator=, format=preserve quoting-mode=auto".as_ref(),
            "-T".as_ref(),
            "Gnumeric_stf:stf_assistant".as_ref(),
            xml.as_os_str(),
            pattern.as_os_str(),
        ])?;
        // A sheet with nothing on it may have no file at all.
        Ok((0..sheets)
            .map(|i| std::fs::read_to_string(file(i)).unwrap_or_default())
            .collect())
    }

    fn save(&self, xml: &Path, output: &Path) -> Result<(), String> {
        self.run(&["--recalc".as_ref(), xml.as_os_str(), output.as_os_str()])
    }
}

/// One open workbook.
struct Session {
    owner: Option<String>,
    dir: PathBuf,
    book: Workbook,
    /// What each cell shows, by sheet.
    shown: Vec<HashMap<(u32, u32), String>>,
    /// Bumped by every edit.
    version: u32,
    last_used: Instant,
}

impl Session {
    fn xml(&self) -> PathBuf {
        self.dir.join("book.gnumeric")
    }

    fn done(&self, id: u64, changed: Vec<CellView>) -> SheetDone {
        SheetDone {
            session: id,
            sheets: self.book.sheets.iter().map(|s| s.name.clone()).collect(),
            version: self.version,
            changed,
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A session behind two locks. `data` is held only briefly, so reading cells never waits for
/// Gnumeric. `work` is held while Gnumeric runs on the session's folder, so its runs take turns
/// and each one starts from the workbook as every earlier edit left it.
struct Open {
    work: Mutex<()>,
    data: Mutex<Session>,
}

impl Open {
    fn new(session: Session) -> Self {
        Self {
            work: Mutex::new(()),
            data: Mutex::new(session),
        }
    }

    /// Whether anything is using it right now.
    fn busy(&self) -> bool {
        self.work.try_lock().is_err() || self.data.try_lock().is_err()
    }
}

#[derive(Default)]
struct Job {
    result: Mutex<Option<(Result<SheetDone, String>, Instant)>>,
}

impl Job {
    fn finish(&self, result: Result<SheetDone, String>) {
        *self.result.lock().unwrap() = Some((result, Instant::now()));
    }

    fn status(&self) -> SheetStatus {
        let result = self.result.lock().unwrap().as_ref().map(|(r, _)| r.clone());
        SheetStatus {
            finished: result.is_some(),
            result,
        }
    }
}

type Sessions = Mutex<HashMap<u64, Arc<Open>>>;

/// A job is named by who asked and what they asked for, so one person's request never finds
/// another's job.
type JobKey = (Option<String>, SheetReq);

/// Every open workbook, and the jobs working on them.
pub struct Sheets {
    engine: Arc<dyn Gnumeric>,
    /// One folder per session inside. Removed when this is dropped.
    root: PathBuf,
    sessions: Arc<Sessions>,
    jobs: Mutex<HashMap<JobKey, Arc<Job>>>,
}

impl Drop for Sheets {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Sheets {
    /// Sessions kept in a fresh folder under the system's temporary directory, worked on by
    /// `engine`.
    pub fn new(engine: Arc<dyn Gnumeric>) -> std::io::Result<Self> {
        // Its own folder, so two servers on one machine (or two tests in one process) never
        // share sessions.
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ccosel-sheets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            engine,
            root,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            jobs: Mutex::new(HashMap::new()),
        })
    }

    /// The session `id`, if `user` opened it and it is still open.
    fn session(&self, id: u64, user: Option<&str>) -> Result<Arc<Open>, u32> {
        // Out of the table before locking it, so a busy session never holds up the others.
        let session = self
            .sessions
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or(server_error::NOT_FOUND)?;
        let mut s = session.data.lock().unwrap();
        if s.owner.as_deref() != user {
            return Err(server_error::NOT_FOUND);
        }
        s.last_used = Instant::now();
        drop(s);
        Ok(session)
    }

    /// Close sessions idle for [`IDLE`] as of `now`, and forget jobs finished long ago.
    fn sweep(&self, now: Instant) {
        self.sessions.lock().unwrap().retain(|_, s| {
            s.busy() || now.saturating_duration_since(s.data.lock().unwrap().last_used) < IDLE
        });
        self.jobs.lock().unwrap().retain(|_, j| {
            j.result
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|(_, at)| now.saturating_duration_since(*at) < JOB_RETENTION)
        });
    }

    /// Start `req` if it is new, and say where it has got to. What can be checked at once is,
    /// and fails the call: a session that isn't the caller's, a file they may not read,
    /// somewhere they may not write, a cell that isn't on the sheet.
    pub fn run(
        &self,
        jail: &Arc<Jail>,
        user: Option<&str>,
        req: &SheetReq,
    ) -> Result<SheetStatus, u32> {
        let key = (user.map(str::to_owned), req.clone());
        self.sweep(Instant::now());
        // Held until the job is in the table, so the same request twice at once is one job.
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.get(&key) {
            return Ok(job.status());
        }
        let job = Arc::new(Job::default());
        match &req.op {
            SheetOp::Open { path } => {
                let real = jail.authorize(path, user, Need::Read)?;
                let meta = std::fs::metadata(&real).map_err(|_| server_error::NOT_FOUND)?;
                if !meta.is_file() {
                    return Err(server_error::NOT_A_DIRECTORY);
                }
                if meta.len() > MAX_FILE_BYTES {
                    return Err(server_error::TOO_LARGE);
                }
                let id = self.new_session_id();
                let dir = self.root.join(id.to_string());
                let (engine, sessions, owner, worker) = (
                    self.engine.clone(),
                    self.sessions.clone(),
                    user.map(str::to_owned),
                    job.clone(),
                );
                std::thread::spawn(move || {
                    worker.finish(open(&*engine, &sessions, id, dir, owner, &real));
                });
            }
            SheetOp::Edit {
                sheet,
                row,
                col,
                raw,
            } => {
                let session = self.session(req.session, user)?;
                // Applied now, in the order edits arrive; only the recalculation waits.
                session
                    .data
                    .lock()
                    .unwrap()
                    .book
                    .set(*sheet, *row, *col, raw)
                    .map_err(|_| server_error::NOT_FOUND)?;
                let (engine, worker, id, sheet) =
                    (self.engine.clone(), job.clone(), req.session, *sheet);
                std::thread::spawn(move || {
                    worker.finish(recalculate(&*engine, &session, id, Some(sheet)));
                });
            }
            SheetOp::Save { path } => {
                if !can_save_as(path) {
                    return Err(server_error::MALFORMED);
                }
                jail.write_target(path, user)?;
                let session = self.session(req.session, user)?;
                let (engine, worker, id) = (self.engine.clone(), job.clone(), req.session);
                let (jail, path, user) = (jail.clone(), path.clone(), user.map(str::to_owned));
                std::thread::spawn(move || {
                    worker.finish(save(&*engine, &session, id, &jail, &path, user.as_deref()));
                });
            }
        }
        jobs.insert(key, job.clone());
        Ok(job.status())
    }

    fn new_session_id(&self) -> u64 {
        let sessions = self.sessions.lock().unwrap();
        loop {
            // Never 0, which a request uses to mean "no session yet".
            let id = rand::random::<u64>();
            if id != 0 && !sessions.contains_key(&id) {
                return id;
            }
        }
    }

    /// The cells of part of a sheet.
    pub fn range(&self, user: Option<&str>, req: &RangeReq) -> Result<RangeReply, u32> {
        if req.rows.saturating_mul(req.cols) > MAX_RANGE_CELLS {
            return Err(server_error::TOO_LARGE);
        }
        let session = self.session(req.session, user)?;
        let s = session.data.lock().unwrap();
        let sheet = s
            .book
            .sheets
            .get(usize::from(req.sheet))
            .ok_or(server_error::NOT_FOUND)?;
        let shown = &s.shown[usize::from(req.sheet)];
        let rows = req.row..req.row.saturating_add(req.rows);
        let cols = req.col..req.col.saturating_add(req.cols);
        let cells: Vec<CellView> = sheet
            .cells
            .range((rows.start, 0)..(rows.end, 0))
            .filter(|((_, c), _)| cols.contains(c))
            .map(|(&(row, col), cell)| CellView {
                row,
                col,
                raw: cell.raw(),
                shown: shown.get(&(row, col)).cloned().unwrap_or_default(),
            })
            .collect();
        let last_row = sheet.cells.keys().map(|&(r, _)| r).max().unwrap_or(0);
        let last_col = sheet.cells.keys().map(|&(_, c)| c).max().unwrap_or(0);
        Ok(RangeReply {
            cells,
            last_row,
            last_col,
        })
    }
}

/// Opens `real` as session `id`, owned by `owner`, and makes room for it among the owner's.
fn open(
    engine: &dyn Gnumeric,
    sessions: &Sessions,
    id: u64,
    dir: PathBuf,
    owner: Option<String>,
    real: &Path,
) -> Result<SheetDone, String> {
    std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't make room for it ({e})"))?;
    let book = match read_book(engine, real, &dir) {
        Ok(book) => book,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
    };
    let session = Arc::new(Open::new(Session {
        owner: owner.clone(),
        book,
        dir,
        shown: Vec::new(),
        version: 1,
        last_used: Instant::now(),
    }));
    let done = recalculate(engine, &session, id, None)?;

    let mut all = sessions.lock().unwrap();
    let mut theirs: Vec<(u64, Instant)> = all
        .iter()
        .filter_map(|(&k, s)| {
            let s = s.data.try_lock().ok()?;
            (s.owner == owner).then_some((k, s.last_used))
        })
        .collect();
    theirs.sort_by_key(|&(_, used)| used);
    while theirs.len() >= SESSIONS_PER_USER {
        all.remove(&theirs.remove(0).0);
    }
    all.insert(id, session);
    Ok(done)
}

/// Converts `real` to a workbook, with every repeated formula written out in each cell.
fn read_book(engine: &dyn Gnumeric, real: &Path, dir: &Path) -> Result<Workbook, String> {
    let xml = dir.join("book.gnumeric");
    engine.to_xml(real, &xml)?;
    let text = std::fs::read_to_string(&xml)
        .map_err(|_| "Gnumeric didn't write the workbook".to_owned())?;
    let mut book = Workbook::parse(&text)?;
    if book.cell_count() > MAX_CELLS {
        return Err(format!(
            "it has more than {MAX_CELLS} cells, which is more than this app opens"
        ));
    }
    // Gnumeric writes a formula repeated down a column once. Ask it to write each one out: a
    // probe cell per repeat, holding `=GET.FORMULA(..)` of it, shows its formula.
    let shared = book.shared_cells();
    if !shared.is_empty() {
        let (probed, rows) = book
            .with_probes(&shared)
            .ok_or("its sheets are too full to work out every formula")?;
        let probe = dir.join("probe.gnumeric");
        std::fs::write(&probe, probed.to_xml())
            .map_err(|e| format!("couldn't write a working copy ({e})"))?;
        let shown: Vec<Vec<Vec<String>>> = engine
            .shown(&probe, dir, book.sheets.len())?
            .iter()
            .map(|csv| parse_csv(csv))
            .collect();
        for (&(sheet, row, col), &probe_row) in shared.iter().zip(&rows) {
            let formula = shown[sheet]
                .get(probe_row as usize)
                .and_then(|r| r.first())
                .filter(|f| f.starts_with('='))
                .ok_or("a repeated formula couldn't be worked out")?;
            book.resolve(sheet, row, col, formula);
        }
        let _ = std::fs::remove_file(probe);
    }
    book.unshare();
    Ok(book)
}

/// Writes the session's workbook, recalculates it, and keeps what every cell now shows. With
/// `edited`, the cells of that sheet that show something new are the result's `changed`.
fn recalculate(
    engine: &dyn Gnumeric,
    session: &Open,
    id: u64,
    edited: Option<u16>,
) -> Result<SheetDone, String> {
    let _turn = session.work.lock().unwrap();
    let (xml, dir, sheets) = {
        let s = session.data.lock().unwrap();
        std::fs::write(s.xml(), s.book.to_xml())
            .map_err(|e| format!("couldn't write a working copy ({e})"))?;
        (s.xml(), s.dir.clone(), s.book.sheets.len())
    };
    let shown: Vec<HashMap<(u32, u32), String>> = engine
        .shown(&xml, &dir, sheets)?
        .iter()
        .map(|csv| shown_cells(csv))
        .collect();
    let mut s = session.data.lock().unwrap();
    let mut changed = Vec::new();
    if let Some(sheet) = edited.map(usize::from)
        && let (Some(new), Some(old)) = (shown.get(sheet), s.shown.get(sheet))
    {
        let book_sheet = &s.book.sheets[sheet];
        let mut places: Vec<&(u32, u32)> = new
            .keys()
            .chain(old.keys())
            .filter(|p| new.get(p) != old.get(p))
            .collect();
        places.sort();
        places.dedup();
        for &(row, col) in places.into_iter().take(MAX_CHANGED) {
            changed.push(CellView {
                row,
                col,
                raw: book_sheet
                    .cells
                    .get(&(row, col))
                    .map(|c| c.raw())
                    .unwrap_or_default(),
                shown: new.get(&(row, col)).cloned().unwrap_or_default(),
            });
        }
    }
    s.shown = shown;
    if edited.is_some() {
        s.version += 1;
    }
    Ok(s.done(id, changed))
}

/// The non-empty cells of a sheet's CSV, by `(row, column)`.
fn shown_cells(csv: &str) -> HashMap<(u32, u32), String> {
    let mut out = HashMap::new();
    for (row, fields) in parse_csv(csv).into_iter().enumerate() {
        for (col, value) in fields.into_iter().enumerate() {
            if !value.is_empty() {
                out.insert((row as u32, col as u32), value);
            }
        }
    }
    out
}

/// Writes the session's workbook to `path`, in the format its extension names, checking again
/// that `user` may write there.
fn save(
    engine: &dyn Gnumeric,
    session: &Open,
    id: u64,
    jail: &Jail,
    path: &str,
    user: Option<&str>,
) -> Result<SheetDone, String> {
    let _turn = session.work.lock().unwrap();
    let (xml, dir) = {
        let s = session.data.lock().unwrap();
        std::fs::write(s.xml(), s.book.to_xml())
            .map_err(|e| format!("couldn't write a working copy ({e})"))?;
        (s.xml(), s.dir.clone())
    };
    // Gnumeric writes into the session's own folder; only the copy below touches the jail, after
    // checking again, so nothing is written anywhere the caller may not write.
    let ext = path
        .rsplit_once('.')
        .map_or("", |(_, e)| e)
        .to_ascii_lowercase();
    let out = dir.join(format!("save.{ext}"));
    engine.save(&xml, &out)?;
    let target = jail
        .write_target(path, user)
        .map_err(|_| "you may no longer save there".to_owned())?;
    std::fs::copy(&out, &target).map_err(|e| format!("couldn't write the file ({e})"))?;
    let _ = std::fs::remove_file(out);
    let s = session.data.lock().unwrap();
    Ok(s.done(id, Vec::new()))
}

#[cfg(test)]
mod tests;
