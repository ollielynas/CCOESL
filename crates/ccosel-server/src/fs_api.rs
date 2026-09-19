//! Filesystem methods, and the jail that keeps them honest.

use std::path::{Component, Path, PathBuf};

use ccosel_proto::fs::{DirEntry, DirListing, EntryKind, ListDirReq};
use ccosel_proto::server_error;

/// A listing is capped so a guest is never handed an unbounded response over a bad link.
/// `DirListing::truncated` tells the app it happened, so the cap is part of the contract
/// rather than a silent surprise.
const MAX_ENTRIES: usize = 2_000;

pub struct Jail {
    root: PathBuf,
}

impl Jail {
    /// Canonicalises the root once, so every later comparison is against a real path.
    pub fn new(root: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            root: root.as_ref().canonicalize()?,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a client-supplied path to a real one inside the jail.
    ///
    /// Two layers, because either alone is insufficient:
    ///
    /// 1. Lexical: reject `..` and absolute components up front, so a path cannot *aim*
    ///    outside the root.
    /// 2. Canonical: resolve symlinks and require the result still sits under the root. This
    ///    is what catches a symlink inside the jail pointing out of it, which no amount of
    ///    string handling would.
    pub fn resolve(&self, requested: &str) -> Result<PathBuf, u32> {
        let rel = requested.trim_start_matches('/');
        let mut candidate = self.root.clone();

        for part in Path::new(rel).components() {
            match part {
                Component::Normal(p) => candidate.push(p),
                // A no-op, harmless.
                Component::CurDir => {}
                // Anything that could climb or re-root is refused outright.
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(server_error::DENIED);
                }
            }
        }

        let real = candidate.canonicalize().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => server_error::NOT_FOUND,
            std::io::ErrorKind::PermissionDenied => server_error::DENIED,
            _ => server_error::IO,
        })?;

        if !real.starts_with(&self.root) {
            return Err(server_error::DENIED);
        }
        Ok(real)
    }

    pub fn list_dir(&self, req: &ListDirReq<'_>) -> Result<DirListing, u32> {
        let dir = self.resolve(req.path)?;

        let meta = std::fs::metadata(&dir).map_err(|_| server_error::IO)?;
        if !meta.is_dir() {
            return Err(server_error::NOT_A_DIRECTORY);
        }

        let read = std::fs::read_dir(&dir).map_err(|_| server_error::IO)?;
        let mut entries = Vec::new();
        let mut truncated = false;

        for item in read {
            if entries.len() >= MAX_ENTRIES {
                truncated = true;
                break;
            }
            let Ok(item) = item else { continue };
            let Ok(meta) = item.metadata() else { continue };
            let Ok(name) = item.file_name().into_string() else {
                // Non-UTF-8 names cannot cross a postcard `String`. Skipping is better than
                // failing the whole listing over one odd file.
                continue;
            };

            entries.push(DirEntry {
                name,
                kind: if meta.is_dir() {
                    EntryKind::Dir
                } else if meta.is_file() {
                    EntryKind::File
                } else if meta.is_symlink() {
                    EntryKind::Symlink
                } else {
                    EntryKind::Other
                },
                size: meta.len(),
                mtime_s: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
            });
        }

        // Directories first, then name. Sorting here rather than in the app means every client
        // agrees, and a guest does not pay to sort a large listing.
        entries.sort_by(|a, b| {
            b.is_dir()
                .cmp(&a.is_dir())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(DirListing { entries, truncated })
    }
}
