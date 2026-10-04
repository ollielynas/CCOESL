//! The File Browser's Compress, Gzip and Extract, run on the server as **jobs** polled like
//! `build_api`'s builds: the first `Archive` call for a `(path, action, generation)` starts one,
//! later ones report on it.
//!
//! Every job works into a temporary entry beside where its result goes, and renames it into
//! place only once it has fully succeeded, so a failed or refused one leaves nothing behind.
//!
//! Extracting is where the care is. An archive is a list of paths and links chosen by whoever
//! made it, so each entry is checked before anything is written: no absolute paths, no `..`,
//! no `.access` file (which would hand its maker the folder's permissions), no link leading
//! out of the folder being unpacked into, and nothing written *through* a link. Any of those
//! refuses the whole archive. Total size and entry count are capped, so a small file that
//! unpacks to terabytes can't fill the disk.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ccosel_proto::archive::{
    ArchiveAction, ArchiveKind, ArchiveReq, ArchiveResult, ArchiveStatus, archive_kind,
    extracted_name,
};
use ccosel_proto::server_error;
use flate2::Compression;
use flate2::read::{GzDecoder, MultiGzDecoder};
use flate2::write::GzEncoder;

use crate::access::ACCESS_FILE;
use crate::fs_api::{Jail, Need};

/// How much an extraction may write, and how many entries it may hold.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_bytes: u64,
    pub max_entries: u64,
}

/// The limits the server runs with.
pub const LIMITS: Limits = Limits {
    max_bytes: 4 << 30,
    max_entries: 100_000,
};

/// Finished jobs older than this are dropped, as in `build_api`.
const JOB_RETENTION: Duration = Duration::from_secs(30 * 60);

struct Job {
    done: AtomicU64,
    total: AtomicU64,
    result: Mutex<Option<ArchiveResult>>,
    finished_at: Mutex<Option<Instant>>,
}

/// Who asked, for what, and which press: a job belongs to the caller who started it.
type Key = (Option<String>, String, ArchiveAction, u32);

pub struct Jobs {
    map: Mutex<HashMap<Key, Arc<Job>>>,
    limits: Limits,
}

impl Default for Jobs {
    fn default() -> Self {
        Self::with_limits(LIMITS)
    }
}

impl Jobs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Jobs that extract under `limits` rather than [`LIMITS`]. For tests.
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            limits,
        }
    }
}

/// What a job will do, settled (and permission-checked) before it starts.
struct Plan {
    action: ArchiveAction,
    /// The real file or folder it reads.
    source: PathBuf,
    /// The real folder its result goes in, and that folder as the caller named it.
    dir: PathBuf,
    dir_requested: String,
    /// What the result is called, before a suffix makes it unique.
    name: String,
}

/// Start the job if this caller hasn't asked for `(path, action, generation)` before, then
/// report where it has got to. Never waits for it.
pub fn archive(
    jail: &Jail,
    jobs: &Jobs,
    req: &ArchiveReq<'_>,
    user: Option<&str>,
) -> Result<ArchiveStatus, u32> {
    let key = (
        user.map(str::to_owned),
        req.path.to_owned(),
        req.action,
        req.generation,
    );
    let mut map = jobs.map.lock().unwrap();
    map.retain(|_, job| {
        job.finished_at
            .lock()
            .unwrap()
            .is_none_or(|at| at.elapsed() < JOB_RETENTION)
    });
    if let Some(job) = map.get(&key) {
        return Ok(snapshot(job));
    }

    let plan = plan(jail, req, user)?;
    let job = Arc::new(Job {
        done: AtomicU64::new(0),
        total: AtomicU64::new(0),
        result: Mutex::new(None),
        finished_at: Mutex::new(None),
    });
    map.insert(key, job.clone());
    drop(map);

    let worker = job.clone();
    let limits = jobs.limits;
    std::thread::spawn(move || {
        let result = run(&plan, &worker, limits);
        *worker.result.lock().unwrap() = Some(result);
        *worker.finished_at.lock().unwrap() = Some(Instant::now());
    });
    Ok(snapshot(&job))
}

fn snapshot(job: &Job) -> ArchiveStatus {
    let result = job.result.lock().unwrap().clone();
    ArchiveStatus {
        finished: result.is_some(),
        done_bytes: job.done.load(Ordering::Relaxed),
        total_bytes: job.total.load(Ordering::Relaxed),
        result,
    }
}

/// Check the caller may read what the job reads and write where its result goes, and work out
/// what that result is called.
fn plan(jail: &Jail, req: &ArchiveReq<'_>, user: Option<&str>) -> Result<Plan, u32> {
    let trimmed = req.path.trim_end_matches('/');
    let (dir_requested, file_name) = match trimmed.rfind('/') {
        Some(i) => (&trimmed[..i], &trimmed[i + 1..]),
        None => ("", trimmed),
    };
    // The top of the server has nowhere beside it to put anything.
    if file_name.is_empty() {
        return Err(server_error::DENIED);
    }
    let source = match req.action {
        ArchiveAction::Compress => {
            jail.check_readable_tree(req.path, user)?;
            jail.authorize(req.path, user, Need::Read)?
        }
        ArchiveAction::Gzip | ArchiveAction::Extract => {
            let real = jail.authorize(req.path, user, Need::Read)?;
            if !real.is_file() {
                return Err(server_error::DENIED);
            }
            real
        }
    };
    let name = match req.action {
        ArchiveAction::Compress => format!("{file_name}.tar.gz"),
        ArchiveAction::Gzip => format!("{file_name}.gz"),
        ArchiveAction::Extract => {
            archive_kind(file_name).ok_or(server_error::MALFORMED)?;
            extracted_name(file_name).to_owned()
        }
    };
    // Write where the result goes, under the name it would have. A suffix added later to make
    // it unique doesn't change that: a new entry has no rules of its own.
    let target = jail.authorize_new(&format!("{dir_requested}/{name}"), user)?;
    let dir = target.parent().ok_or(server_error::DENIED)?.to_path_buf();
    Ok(Plan {
        action: req.action,
        source,
        dir,
        dir_requested: dir_requested.to_owned(),
        name,
    })
}

/// Do the work into a temporary entry, then move it into place under a free name.
fn run(plan: &Plan, job: &Job, limits: Limits) -> ArchiveResult {
    let temp = plan
        .dir
        .join(format!(".ccosel-{:016x}.part", rand::random::<u64>()));
    let outcome = match plan.action {
        ArchiveAction::Compress => compress(&plan.source, &temp, job),
        ArchiveAction::Gzip => gzip(&plan.source, &temp, job),
        ArchiveAction::Extract => extract(&plan.source, &temp, job, limits),
    };
    if let Err(why) = outcome {
        let _ = fs::remove_dir_all(&temp);
        let _ = fs::remove_file(&temp);
        return ArchiveResult::Failed(why);
    }
    let name = free_name(&plan.dir, &plan.name);
    match fs::rename(&temp, plan.dir.join(&name)) {
        Ok(()) => ArchiveResult::Made(format!("{}/{name}", plan.dir_requested)),
        Err(e) => {
            let _ = fs::remove_dir_all(&temp);
            let _ = fs::remove_file(&temp);
            ArchiveResult::Failed(format!("couldn't put it in place: {e}"))
        }
    }
}

/// `name`, or `name (2)`, `name (3)`, … with the number before the extension, whichever is
/// free in `dir`. Nothing already there is ever overwritten.
pub fn free_name(dir: &Path, name: &str) -> String {
    if fs::symlink_metadata(dir.join(name)).is_err() {
        return name.to_owned();
    }
    // The extension is everything from the first dot that isn't the name's first character,
    // so `photos.tar.gz` becomes `photos (2).tar.gz`.
    let split = name[1..].find('.').map_or(name.len(), |i| i + 1);
    let (stem, ext) = name.split_at(split);
    (2u32..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|candidate| fs::symlink_metadata(dir.join(candidate)).is_err())
        .expect("some number is free")
}

/// A reader that adds what passes through it to a job's progress.
struct Counting<'a, R> {
    inner: R,
    done: &'a AtomicU64,
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

fn io_err(what: &str) -> impl Fn(io::Error) -> String + '_ {
    move |e| format!("{what}: {e}")
}

/// `source` (a file or folder) into a gzipped tar at `out`. Links are kept as links, not
/// followed, and `.access` files are left out: what a folder's rules are is not for an archive
/// to carry away.
fn compress(source: &Path, out: &Path, job: &Job) -> Result<(), String> {
    let name = source
        .file_name()
        .ok_or("nothing to name it by")?
        .to_owned();
    let mut entries = Vec::new();
    walk(source, PathBuf::from(&name), &mut entries).map_err(io_err("couldn't read it"))?;
    let total: u64 = entries
        .iter()
        .filter(|(_, _, meta)| meta.is_file())
        .map(|(_, _, meta)| meta.len())
        .sum();
    job.total.store(total, Ordering::Relaxed);

    let file = File::create(out).map_err(io_err("couldn't create the archive"))?;
    let mut tar = tar::Builder::new(GzEncoder::new(file, Compression::default()));
    for (real, inside, meta) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_metadata_in_mode(&meta, tar::HeaderMode::Complete);
        let fail = io_err("couldn't add it to the archive");
        if meta.is_symlink() {
            let target = fs::read_link(&real).map_err(&fail)?;
            header.set_size(0);
            tar.append_link(&mut header, &inside, target)
                .map_err(&fail)?;
        } else if meta.is_dir() {
            header.set_size(0);
            tar.append_data(&mut header, &inside, io::empty())
                .map_err(&fail)?;
        } else {
            let file = File::open(&real).map_err(&fail)?;
            let reader = Counting {
                inner: file.take(meta.len()),
                done: &job.done,
            };
            tar.append_data(&mut header, &inside, reader)
                .map_err(&fail)?;
        }
    }
    tar.into_inner()
        .and_then(|gz| gz.finish())
        .and_then(|f| f.sync_all())
        .map_err(io_err("couldn't finish the archive"))
}

/// `real` and everything under it, without following links, `.access` files left out: each
/// as `(real path, path inside the archive, its own metadata)`, folders before what they hold.
fn walk(
    real: &Path,
    inside: PathBuf,
    out: &mut Vec<(PathBuf, PathBuf, fs::Metadata)>,
) -> io::Result<()> {
    let meta = fs::symlink_metadata(real)?;
    let is_dir = meta.is_dir();
    out.push((real.to_path_buf(), inside.clone(), meta));
    if is_dir {
        let mut children: Vec<_> = fs::read_dir(real)?.collect::<Result<_, _>>()?;
        children.sort_by_key(|e| e.file_name());
        for child in children {
            if child.file_name() == ACCESS_FILE {
                continue;
            }
            walk(&child.path(), inside.join(child.file_name()), out)?;
        }
    }
    Ok(())
}

/// One file into a gzip file at `out`.
fn gzip(source: &Path, out: &Path, job: &Job) -> Result<(), String> {
    let input = File::open(source).map_err(io_err("couldn't read it"))?;
    let len = input.metadata().map(|m| m.len()).unwrap_or(0);
    job.total.store(len, Ordering::Relaxed);
    let output = File::create(out).map_err(io_err("couldn't create the .gz"))?;
    let mut gz = GzEncoder::new(output, Compression::default());
    let mut reader = Counting {
        inner: input,
        done: &job.done,
    };
    io::copy(&mut reader, &mut gz).map_err(io_err("couldn't compress it"))?;
    gz.finish()
        .and_then(|f| f.sync_all())
        .map_err(io_err("couldn't finish the .gz"))
}

fn too_big(limits: Limits) -> String {
    format!(
        "it would unpack to more than {} MB, the most this server unpacks",
        limits.max_bytes >> 20
    )
}

/// Unpack the archive `source` into `out`: a new folder for a tar, a new file for a `.gz`.
fn extract(source: &Path, out: &Path, job: &Job, limits: Limits) -> Result<(), String> {
    let file = File::open(source).map_err(io_err("couldn't read it"))?;
    job.total.store(
        file.metadata().map(|m| m.len()).unwrap_or(0),
        Ordering::Relaxed,
    );
    let reader = Counting {
        inner: file,
        done: &job.done,
    };
    let name = source.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match archive_kind(name) {
        Some(ArchiveKind::Tar) => unpack_tar(reader, out, limits),
        Some(ArchiveKind::TarGz) => unpack_tar(GzDecoder::new(reader), out, limits),
        Some(ArchiveKind::Gz) => {
            let mut output = File::create(out).map_err(io_err("couldn't create the file"))?;
            let mut decoded = MultiGzDecoder::new(reader).take(limits.max_bytes + 1);
            let n =
                io::copy(&mut decoded, &mut output).map_err(io_err("it isn't a valid .gz file"))?;
            if n > limits.max_bytes {
                return Err(too_big(limits));
            }
            Ok(())
        }
        None => Err("it isn't an archive this can unpack".to_owned()),
    }
}

/// An entry's path as plain names, refusing anything that could climb out, re-root, or
/// name an `.access` file.
fn plain_names(path: &Path) -> Result<Vec<OsString>, String> {
    let shown = path.display();
    let mut names = Vec::new();
    for part in path.components() {
        match part {
            Component::Normal(name) if name == ACCESS_FILE => {
                return Err(format!("it holds a permissions file ({shown})"));
            }
            Component::Normal(name) => names.push(name.to_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!(
                    "it holds a path that climbs out with `..` ({shown})"
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("it holds an absolute path ({shown})"));
            }
        }
    }
    Ok(names)
}

/// `root` joined with `names`, every folder on the way made if it is missing. Something on
/// the way that is already there must be a real folder, not a link: writing through a link
/// is how an archive would reach outside `root`.
fn folders(root: &Path, names: &[OsString]) -> Result<PathBuf, String> {
    let mut at = root.to_path_buf();
    for name in names {
        at.push(name);
        match fs::symlink_metadata(&at) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "it writes inside {}, which isn't a folder",
                    shown(root, &at)
                ));
            }
            Err(_) => fs::create_dir(&at).map_err(io_err("couldn't make a folder"))?,
        }
    }
    Ok(at)
}

fn shown(root: &Path, at: &Path) -> String {
    at.strip_prefix(root).unwrap_or(at).display().to_string()
}

/// Where a file entry at `names` goes: its folders made, and nothing but an ordinary file
/// already there (an earlier entry of the same name, which this one replaces).
fn file_target(root: &Path, names: &[OsString]) -> Result<PathBuf, String> {
    let (last, parents) = names.split_last().ok_or("it holds an entry with no name")?;
    let target = folders(root, parents)?.join(last);
    match fs::symlink_metadata(&target) {
        Ok(meta) if meta.is_file() => {
            fs::remove_file(&target).map_err(io_err("couldn't replace a file"))?;
        }
        Ok(_) => {
            return Err(format!(
                "it writes over {}, which isn't a file",
                shown(root, &target)
            ));
        }
        Err(_) => {}
    }
    Ok(target)
}

/// Whether a symlink at `names` (inside the folder being unpacked into) pointing at `target`
/// stays inside that folder, judged by its path alone.
fn link_stays_inside(names: &[OsString], target: &Path) -> bool {
    let mut depth = names.len().saturating_sub(1);
    for part in target.components() {
        match part {
            Component::Normal(name) if name == ACCESS_FILE => return false,
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return false,
            },
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

fn unpack_tar(reader: impl Read, root: &Path, limits: Limits) -> Result<(), String> {
    fs::create_dir(root).map_err(io_err("couldn't make the folder"))?;
    let mut archive = tar::Archive::new(reader);
    let bad = |e: io::Error| format!("it isn't a valid tar archive: {e}");
    let mut written = 0u64;
    let mut count = 0u64;
    for entry in archive.entries().map_err(bad)? {
        let mut entry = entry.map_err(bad)?;
        count += 1;
        if count > limits.max_entries {
            return Err(format!(
                "it holds more than {} entries, the most this server unpacks",
                limits.max_entries
            ));
        }
        let path = entry.path().map_err(bad)?.into_owned();
        let names = plain_names(&path)?;
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            folders(root, &names)?;
        } else if kind.is_file() {
            let target = file_target(root, &names)?;
            let mut out = File::create(&target).map_err(io_err("couldn't write a file"))?;
            let room = limits.max_bytes - written;
            let n = io::copy(&mut (&mut entry).take(room + 1), &mut out).map_err(bad)?;
            written += n;
            if written > limits.max_bytes {
                return Err(too_big(limits));
            }
            out.flush().map_err(io_err("couldn't write a file"))?;
            set_executable(&target, entry.header().mode().unwrap_or(0) & 0o111 != 0);
        } else if kind.is_symlink() {
            let to = entry
                .link_name()
                .map_err(bad)?
                .ok_or("it holds a link that leads nowhere")?
                .into_owned();
            if !link_stays_inside(&names, &to) {
                return Err(format!(
                    "it holds a link that leads outside the folder ({} → {})",
                    path.display(),
                    to.display()
                ));
            }
            let target = file_target(root, &names)?;
            make_symlink(&to, &target)?;
        } else if kind.is_hard_link() {
            let to = entry
                .link_name()
                .map_err(bad)?
                .ok_or("it holds a link that leads nowhere")?
                .into_owned();
            // A hard link names another entry of the archive, by its path from the top.
            let from = root.join(
                plain_names(&to)
                    .map_err(|_| {
                        format!(
                            "it holds a link that leads outside the folder ({} → {})",
                            path.display(),
                            to.display()
                        )
                    })?
                    .iter()
                    .collect::<PathBuf>(),
            );
            match fs::symlink_metadata(&from) {
                Ok(meta) if meta.is_file() => {}
                _ => {
                    return Err(format!(
                        "it holds a link to something it doesn't contain ({} → {})",
                        path.display(),
                        to.display()
                    ));
                }
            }
            let target = file_target(root, &names)?;
            fs::hard_link(&from, &target).map_err(io_err("couldn't make a link"))?;
        }
        // Devices, pipes and the like are left out: there is nothing to do with one here.
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_executable(_: &Path, _: bool) {}

#[cfg(unix)]
fn make_symlink(to: &Path, at: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(to, at).map_err(io_err("couldn't make a link"))
}

/// Symlinks need privileges on Windows; an archive's are left out there.
#[cfg(not(unix))]
fn make_symlink(_: &Path, _: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests;
