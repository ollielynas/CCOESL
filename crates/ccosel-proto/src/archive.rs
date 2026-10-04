//! The File Browser's archive method: bundle a file or folder into a `.tar.gz`, gzip one file,
//! or unpack a `.tar`, `.tar.gz`, `.tgz` or `.gz`, all on the server.
//!
//! A big folder takes a while, so this is a **job**, polled the way `build::Compile` is: the
//! first call for a `(path, action, generation)` starts it, every later one reports how far it
//! has got. The archive's bytes never pass through the browser.

use alloc::string::String;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Effect, Method, Query, Rpc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArchiveAction {
    /// A file or folder into `<name>.tar.gz` beside it.
    Compress,
    /// One file into `<name>.gz` beside it.
    Gzip,
    /// A `.tar`, `.tar.gz` or `.tgz` into a new folder beside it, or a `.gz` into the file it
    /// holds.
    Extract,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ArchiveReq<'a> {
    /// The file or folder to compress, or the archive to extract. Jail-relative.
    #[serde(borrow)]
    pub path: &'a str,
    pub action: ArchiveAction,
    /// Bumped by the app once per press: the job's idempotency key, as in `CompileReq`.
    pub generation: u32,
}

/// Where a job has got to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveStatus {
    pub finished: bool,
    /// Bytes read so far, out of `total_bytes`: of the files going in when compressing, of the
    /// archive when extracting.
    pub done_bytes: u64,
    pub total_bytes: u64,
    /// Present exactly when `finished`.
    pub result: Option<ArchiveResult>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchiveResult {
    /// The jail path of what it made.
    Made(String),
    /// Why nothing was made. A refused or failed job leaves nothing behind.
    Failed(String),
}

impl ArchiveStatus {
    /// Progress in 0..=1, or negative while there is nothing to divide by.
    pub fn fraction(&self) -> f32 {
        if self.finished {
            1.0
        } else if self.total_bytes == 0 {
            -1.0
        } else {
            (self.done_bytes as f64 / self.total_bytes as f64).min(0.99) as f32
        }
    }
}

/// The kinds of archive [`ArchiveAction::Extract`] unpacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveKind {
    Tar,
    TarGz,
    Gz,
}

/// What kind of archive `name` is, by its extension, or `None` if it isn't one this unpacks.
pub fn archive_kind(name: &str) -> Option<ArchiveKind> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some(ArchiveKind::TarGz)
    } else if lower.ends_with(".tar") {
        Some(ArchiveKind::Tar)
    } else if lower.ends_with(".gz") {
        Some(ArchiveKind::Gz)
    } else {
        None
    }
}

/// `name` without its archive extension: what extracting it is called before any suffix.
pub fn extracted_name(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    let cut = [".tar.gz", ".tgz", ".tar", ".gz"]
        .iter()
        .find(|ext| lower.ends_with(*ext))
        .map_or(0, |ext| ext.len());
    let stem = &name[..name.len() - cut];
    if stem.is_empty() { "archive" } else { stem }
}

pub struct Archive;

impl Rpc for Archive {
    const METHOD: Method = Method::Archive;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    // As for `Compile`: repeating a `(path, action, generation)` reports on the same job and
    // starts nothing, so it is safe to retry and to cache.
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = ArchiveReq<'a>;
    type Reply = ArchiveStatus;
}

impl Query for Archive {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension_any_case() {
        assert_eq!(archive_kind("a.tar.gz"), Some(ArchiveKind::TarGz));
        assert_eq!(archive_kind("A.TGZ"), Some(ArchiveKind::TarGz));
        assert_eq!(archive_kind("a.tar"), Some(ArchiveKind::Tar));
        assert_eq!(archive_kind("log.gz"), Some(ArchiveKind::Gz));
        assert_eq!(archive_kind("a.zip"), None);
        assert_eq!(archive_kind("gz"), None);
    }

    #[test]
    fn extracted_names() {
        assert_eq!(extracted_name("photos.tar.gz"), "photos");
        assert_eq!(extracted_name("photos.TGZ"), "photos");
        assert_eq!(extracted_name("x.tar"), "x");
        assert_eq!(extracted_name("notes.txt.gz"), "notes.txt");
        assert_eq!(extracted_name(".tar.gz"), "archive");
    }

    #[test]
    fn fraction() {
        let mut s = ArchiveStatus {
            finished: false,
            done_bytes: 0,
            total_bytes: 0,
            result: None,
        };
        assert!(s.fraction() < 0.0);
        s.total_bytes = 10;
        s.done_bytes = 10;
        assert!(s.fraction() < 1.0, "full only once finished");
        s.finished = true;
        assert_eq!(s.fraction(), 1.0);
    }
}
