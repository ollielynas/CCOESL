//! The Modeller's back end: FreeCAD as a long-running worker process, rebuilds as jobs, and a
//! content-addressed store of the meshes they produce.
//!
//! **The worker.** `cad/worker.py` runs inside `freecadcmd`, reading one JSON request per line
//! and answering with one line. It is started on first use and kept, because FreeCAD takes a
//! moment to load and caches every model prefix it has built. One worker serves every user,
//! one request at a time: FreeCAD is single-threaded, and a LAN's worth of people drawing
//! boxes does not need more. If it dies or hangs it is killed and started afresh on the next
//! request.
//!
//! **Rebuilds are jobs**, like `build_api`'s compiles: `regenerate` starts one the first time
//! it sees an op list and reports a snapshot every time after. The op list is the key, so a
//! poll can never start a second rebuild.
//!
//! **Meshes** are stored by the BLAKE3 hash of their postcard bytes and served at
//! `/cad/mesh/<hash>`, immutable: the browser caches each one forever, and undoing back to a
//! model costs no download at all.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ccosel_proto::cad::{
    CadOp, ExportFormat, ExportReq, Exported, MAX_OPS, MAX_POINTS, MESH_PATH, MeshData, Model,
    RegenResult, RegenStatus, RenderReq,
};
use ccosel_proto::scene2d::Scene2D;
use ccosel_proto::server_error;
use ccosel_view3d::{Camera, Mesh, Style, render};
use serde_json::{Value, json};

use crate::fs_api::Jail;

/// The worker script, carried in the binary so a deployed server needs nothing beside it.
const WORKER_PY: &str = include_str!("../cad/worker.py");

/// Every answer line starts with this; anything else on stdout is FreeCAD talking to itself.
const ANSWER_PREFIX: &str = "@@CCOSEL ";

/// FreeCAD loading its modules on a cold start.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// One rebuild or export. Long enough for a model with a few hundred booleans; a request that
/// takes longer has hung, and the worker is replaced.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Finished jobs are kept this long, so a window that stopped polling can still collect one.
const JOB_RETENTION: Duration = Duration::from_secs(30 * 60);

/// The mesh store's budget. Meshes past it are dropped oldest first; a client that still
/// wants one has it in its HTTP cache.
const MESH_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Largest frame the server will render. Bigger than any screen, small enough that a request
/// cannot ask for a gigapixel.
const MAX_RENDER_PX: u16 = 4096;

/// How the server starts the worker: a program and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Launch {
    /// `freecadcmd` running the worker script. The command is `CCOSEL_FREECADCMD` if set (a
    /// path, or a command line such as `/opt/FreeCAD.AppImage freecadcmd`), else the first of
    /// `freecadcmd` or `FreeCADCmd` on `PATH`. `None` if there is no FreeCAD to be found.
    pub fn freecad(script: &Path) -> Option<Self> {
        let mut words: Vec<String> = match std::env::var("CCOSEL_FREECADCMD") {
            Ok(v) if !v.trim().is_empty() => v.split_whitespace().map(str::to_owned).collect(),
            _ => vec![find_on_path(&["freecadcmd", "FreeCADCmd"])?],
        };
        let program = PathBuf::from(words.remove(0));
        let run = format!(
            "import runpy; runpy.run_path({:?}, run_name='__main__')",
            script.display().to_string()
        );
        words.push("-c".to_owned());
        words.push(run);
        Some(Self {
            program,
            args: words,
        })
    }
}

fn find_on_path(names: &[&str]) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}

/// The running worker: its stdin, and a channel of its answer lines.
struct Worker {
    child: Child,
    stdin: ChildStdin,
    answers: Receiver<String>,
}

impl Worker {
    fn start(launch: &Launch) -> Result<Self, String> {
        let mut child = Command::new(&launch.program)
            .args(&launch.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", launch.program.display()))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let (tx, answers) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(answer) = line.strip_prefix(ANSWER_PREFIX)
                    && tx.send(answer.to_owned()).is_err()
                {
                    return;
                }
            }
        });
        let mut worker = Self {
            child,
            stdin,
            answers,
        };
        let ready = worker.answer(START_TIMEOUT, |v| v.get("ready").is_some());
        match ready {
            Ok(_) => Ok(worker),
            Err(e) => {
                worker.kill();
                Err(format!("FreeCAD did not start: {e}"))
            }
        }
    }

    /// The next answer that `wanted` accepts, skipping stale ones from a request that timed
    /// out earlier.
    fn answer(
        &mut self,
        timeout: Duration,
        wanted: impl Fn(&Value) -> bool,
    ) -> Result<Value, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.answers.recv_timeout(left) {
                Ok(line) => {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if wanted(&v) {
                        return Ok(v);
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Err("timed out".to_owned()),
                Err(RecvTimeoutError::Disconnected) => return Err("the worker exited".to_owned()),
            }
        }
    }

    fn request(&mut self, id: u64, body: &Value) -> Result<Value, String> {
        let mut line = body.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("the worker is not listening: {e}"))?;
        self.answer(REQUEST_TIMEOUT, |v| {
            v.get("id").and_then(Value::as_u64) == Some(id)
        })
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.kill();
    }
}

struct Job {
    started: Instant,
    result: Mutex<Option<RegenResult>>,
    finished_at: Mutex<Option<Instant>>,
}

#[derive(Default)]
struct MeshStore {
    bytes: HashMap<String, Arc<Vec<u8>>>,
    order: VecDeque<String>,
    total: usize,
    /// Decoded meshes for server rendering, so orbiting does not re-decode per frame.
    decoded: HashMap<String, Arc<Mesh>>,
}

impl MeshStore {
    fn insert(&mut self, hash: String, bytes: Vec<u8>) {
        if self.bytes.contains_key(&hash) {
            return;
        }
        self.total += bytes.len();
        self.bytes.insert(hash.clone(), Arc::new(bytes));
        self.order.push_back(hash);
        while self.total > MESH_BUDGET_BYTES && self.order.len() > 1 {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(b) = self.bytes.remove(&old) {
                self.total -= b.len();
            }
            self.decoded.remove(&old);
        }
    }
}

/// Everything CAD on the server. One per server, in `AppState`.
pub struct Cad {
    launch: Option<Launch>,
    /// Why there is no `launch`, for the error an app shows.
    missing: String,
    worker: Mutex<Option<Worker>>,
    next_id: AtomicU64,
    jobs: Mutex<HashMap<[u8; 32], Arc<Job>>>,
    meshes: Mutex<MeshStore>,
    temp: PathBuf,
}

impl Cad {
    /// The CAD service using the FreeCAD found by [`Launch::freecad`], with its worker script
    /// and export scratch files under `temp`.
    pub fn from_env(temp: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(temp)?;
        let script = temp.join("worker.py");
        std::fs::write(&script, WORKER_PY)?;
        let launch = Launch::freecad(&script);
        Ok(Self::new(launch, temp))
    }

    /// The CAD service with an explicit worker, or none. For tests, which stand a small fake
    /// in for FreeCAD.
    pub fn new(launch: Option<Launch>, temp: &Path) -> Self {
        Self {
            launch,
            missing: "FreeCAD is not installed on the server: install it, or set \
                      CCOSEL_FREECADCMD to its freecadcmd"
                .to_owned(),
            worker: Mutex::new(None),
            next_id: AtomicU64::new(1),
            jobs: Mutex::new(HashMap::new()),
            meshes: Mutex::new(MeshStore::default()),
            temp: temp.to_path_buf(),
        }
    }

    pub fn available(&self) -> bool {
        self.launch.is_some()
    }

    /// Send one request to the worker, starting it first if need be. A worker that fails is
    /// dropped, so the next request gets a fresh one.
    fn call(&self, mut body: Value) -> Result<Value, String> {
        let launch = self.launch.as_ref().ok_or_else(|| self.missing.clone())?;
        let mut slot = self.worker.lock().unwrap();
        if slot.is_none() {
            *slot = Some(Worker::start(launch)?);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        body["id"] = json!(id);
        let worker = slot.as_mut().expect("started above");
        match worker.request(id, &body) {
            Ok(v) => Ok(v),
            Err(e) => {
                *slot = None;
                Err(format!("the CAD worker failed: {e}"))
            }
        }
    }

    /// Start rebuilding `ops` if this list has not been seen, then say how it is going.
    pub fn regenerate(self: &Arc<Self>, ops: &[CadOp]) -> Result<RegenStatus, u32> {
        check_ops(ops)?;
        if self.launch.is_none() {
            return Err(server_error::UNAVAILABLE);
        }
        let key = *blake3::hash(&postcard::to_allocvec(ops).map_err(|_| server_error::MALFORMED)?)
            .as_bytes();
        let job = {
            let mut jobs = self.jobs.lock().unwrap();
            jobs.retain(|_, j| {
                j.finished_at
                    .lock()
                    .unwrap()
                    .is_none_or(|at| at.elapsed() < JOB_RETENTION)
            });
            jobs.entry(key)
                .or_insert_with(|| {
                    let job = Arc::new(Job {
                        started: Instant::now(),
                        result: Mutex::new(None),
                        finished_at: Mutex::new(None),
                    });
                    let (cad, worker_job, ops) = (self.clone(), job.clone(), ops.to_vec());
                    std::thread::spawn(move || {
                        let result = cad.rebuild(&ops);
                        *worker_job.result.lock().unwrap() = Some(result);
                        *worker_job.finished_at.lock().unwrap() = Some(Instant::now());
                    });
                    job
                })
                .clone()
        };
        let result = job.result.lock().unwrap().clone();
        Ok(RegenStatus {
            finished: result.is_some(),
            elapsed_ms: job.started.elapsed().as_millis() as u64,
            result,
        })
    }

    /// Rebuild `ops` now, waiting for the worker. Store the mesh and say where it is.
    pub fn rebuild(&self, ops: &[CadOp]) -> RegenResult {
        let failed = |op: u32, message: String| RegenResult::Failed { op, message };
        let answer = match self.call(json!({"cmd": "regen", "ops": ops_json(ops)})) {
            Ok(a) => a,
            Err(e) => return failed(u32::MAX, e),
        };
        if answer.get("ok").and_then(Value::as_bool) != Some(true) {
            let op = answer
                .get("op")
                .and_then(Value::as_u64)
                .and_then(|o| u32::try_from(o).ok())
                .unwrap_or(u32::MAX);
            let msg = answer
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("FreeCAD could not build the model");
            return failed(op, msg.to_owned());
        }
        let mesh: MeshData = match answer
            .get("mesh")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
        {
            Ok(Some(m)) => m,
            _ => return failed(u32::MAX, "the worker's mesh did not parse".to_owned()),
        };
        if !mesh.is_consistent() {
            return failed(u32::MAX, "the worker's mesh is inconsistent".to_owned());
        }
        let triangles = mesh.triangles.len() as u32;
        let Ok(bytes) = postcard::to_allocvec(&mesh) else {
            return failed(u32::MAX, "the mesh did not encode".to_owned());
        };
        let hash = blake3::hash(&bytes).to_hex().to_string();
        self.meshes.lock().unwrap().insert(hash.clone(), bytes);
        let count = |k: &str| answer.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
        RegenResult::Ok(Model {
            mesh: format!("{MESH_PATH}/{hash}"),
            triangles,
            faces: count("faces"),
            solids: count("solids"),
            volume_mm3: answer
                .get("volume")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .round()
                .max(0.0) as u64,
        })
    }

    /// A stored mesh's bytes, by hash.
    pub fn mesh_bytes(&self, hash: &str) -> Option<Arc<Vec<u8>>> {
        self.meshes.lock().unwrap().bytes.get(hash).cloned()
    }

    /// Render a stored mesh on the server: the `POST /cad/render` half of a viewport in
    /// server mode. `None` for a mesh the store does not have.
    pub fn render(&self, req: &RenderReq) -> Option<Scene2D> {
        let hash = req.mesh.strip_prefix(MESH_PATH)?.strip_prefix('/')?;
        let mesh = {
            let mut store = self.meshes.lock().unwrap();
            match store.decoded.get(hash) {
                Some(m) => m.clone(),
                None => {
                    let bytes = store.bytes.get(hash)?.clone();
                    let data: MeshData = postcard::from_bytes(&bytes).ok()?;
                    let mesh = Arc::new(Mesh::new(data).ok()?);
                    store.decoded.insert(hash.to_owned(), mesh.clone());
                    mesh
                }
            }
        };
        let (w, h) = (
            req.width.clamp(1, MAX_RENDER_PX),
            req.height.clamp(1, MAX_RENDER_PX),
        );
        Some(render(
            Some(&mesh),
            &Camera::new(req.camera),
            w,
            h,
            &Style::default(),
        ))
    }

    /// Write the model `req.ops` builds to `req.path`, as `user`. The worker writes a scratch
    /// file the server owns; only this function writes into the jail, under the same checks
    /// as `WriteFile`.
    pub fn export(
        &self,
        jail: &Jail,
        user: Option<&str>,
        req: &ExportReq,
    ) -> Result<Exported, (u32, String)> {
        check_ops(&req.ops).map_err(|c| (c, String::new()))?;
        let target = jail
            .authorize_file_write(&req.path, user, false)
            .map_err(|c| (c, req.path.clone()))?;
        if self.launch.is_none() {
            return Err((server_error::UNAVAILABLE, self.missing.clone()));
        }
        let scratch = self.temp.join(format!(
            "export-{}.{}",
            self.next_id.fetch_add(1, Ordering::Relaxed),
            req.format.extension()
        ));
        let format = match req.format {
            ExportFormat::Step => "step",
            ExportFormat::Stl => "stl",
            ExportFormat::FreeCad => "fcstd",
        };
        let answer = self
            .call(json!({
                "cmd": "export",
                "ops": ops_json(&req.ops),
                "format": format,
                "path": scratch.display().to_string(),
            }))
            .map_err(|e| (server_error::IO, e))?;
        let outcome = if answer.get("ok").and_then(Value::as_bool) == Some(true) {
            std::fs::copy(&scratch, &target)
                .map(|bytes| Exported {
                    path: req.path.clone(),
                    bytes,
                })
                .map_err(|e| (server_error::IO, format!("{}: {e}", req.path)))
        } else {
            let msg = answer
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("FreeCAD could not export the model");
            Err((server_error::IO, msg.to_owned()))
        };
        let _ = std::fs::remove_file(&scratch);
        outcome
    }
}

/// Refuse a request no person drew: too many operations, too many corners, or coordinates
/// that are not numbers. Cheap, and it keeps garbage away from FreeCAD.
fn check_ops(ops: &[CadOp]) -> Result<(), u32> {
    if ops.len() > MAX_OPS {
        return Err(server_error::TOO_LARGE);
    }
    let finite = |p: &[f32; 3]| p.iter().all(|c| c.is_finite());
    for op in ops {
        match op {
            CadOp::Polygon { points, normal } => {
                if points.len() > MAX_POINTS {
                    return Err(server_error::TOO_LARGE);
                }
                if !points.iter().all(finite) || !finite(normal) {
                    return Err(server_error::MALFORMED);
                }
            }
            CadOp::PushPull { distance, .. } => {
                if !distance.is_finite() {
                    return Err(server_error::MALFORMED);
                }
            }
        }
    }
    Ok(())
}

/// The op list in the worker's JSON shape.
fn ops_json(ops: &[CadOp]) -> Value {
    Value::Array(
        ops.iter()
            .map(|op| match op {
                CadOp::Polygon { points, normal } => json!({
                    "kind": "polygon",
                    "points": points,
                    "normal": normal,
                }),
                CadOp::PushPull {
                    after,
                    face,
                    distance,
                } => json!({
                    "kind": "pushpull",
                    "after": after,
                    "face": face,
                    "distance": distance,
                }),
            })
            .collect(),
    )
}
