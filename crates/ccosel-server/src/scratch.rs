//! Temporary project folders (`ccosel_proto::scratch`): made on `POST /scratch`, deleted once
//! unused for `TTL_SECS`, and all wiped when the server starts, so an uploaded project never
//! outlives the session that built it.
//!
//! They live inside the jail, at `/.scratch/<id>`, so upload, build and download work on them
//! unchanged. Every handler that takes a jail path calls [`Scratch::touch`] with it, which is
//! what "unused" is measured from.

use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ccosel_proto::scratch;

pub struct Scratch {
    /// `<jail root>/.scratch`.
    dir: PathBuf,
    ttl: Duration,
    /// When each live folder was last used.
    used: Mutex<HashMap<u32, Instant>>,
}

impl Scratch {
    /// Takes over `<jail root>/.scratch`, deleting whatever a previous run left there.
    pub fn new(jail_root: &Path) -> std::io::Result<Self> {
        let dir = jail_root.join(scratch::DIR);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            ttl: Duration::from_secs(scratch::TTL_SECS),
            used: Mutex::new(HashMap::new()),
        })
    }

    /// A new, empty folder, returned as its id.
    pub fn create(&self) -> std::io::Result<u32> {
        let mut used = self.used.lock().unwrap();
        loop {
            // Random rather than counted, so ids aren't reused across restarts and aren't a
            // sequence to walk. Not a secret: nothing in the jail is private yet.
            let id = (RandomState::new().hash_one(Instant::now()) as u32).max(1);
            if used.contains_key(&id) {
                continue;
            }
            std::fs::create_dir_all(self.dir.join(id.to_string()))?;
            used.insert(id, Instant::now());
            return Ok(id);
        }
    }

    /// Note that the scratch folder `jail_path` is in, if any, was just used.
    pub fn touch(&self, jail_path: &str) {
        if let Some(id) = scratch::id_of(jail_path) {
            let mut used = self.used.lock().unwrap();
            if let Some(t) = used.get_mut(&id) {
                *t = Instant::now();
            }
        }
    }

    /// Delete every folder unused since `now - ttl`, and anything in `.scratch` this server
    /// didn't make. Returns how many it deleted.
    pub fn sweep(&self, now: Instant) -> usize {
        let mut used = self.used.lock().unwrap();
        used.retain(|_, t| now.saturating_duration_since(*t) < self.ttl);
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let live = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
                .is_some_and(|id| used.contains_key(&id));
            if !live {
                let path = entry.path();
                let gone = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                if gone.is_ok() {
                    removed += 1;
                }
            }
        }
        removed
    }

    /// Whether folder `id` is live.
    pub fn exists(&self, id: u32) -> bool {
        self.used.lock().unwrap().contains_key(&id)
    }
}

#[cfg(test)]
mod tests;
