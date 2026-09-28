//! Filesystem methods, and the jail that keeps them honest.

use std::path::{Component, Path, PathBuf};

use ccosel_proto::fs::{
    AccessReply, DirEntry, DirListing, EntryKind, FileText, ListDirReq, MAX_TEXT_BYTES, SearchHit,
    SearchReply, SearchReq, WriteFileReq,
};
use ccosel_proto::server_error;

use crate::access::{self, Perms};

/// A listing is capped so a guest is never handed an unbounded response over a bad link.
/// `DirListing::truncated` tells the app it happened, so the cap is part of the contract
/// rather than a silent surprise.
const MAX_ENTRIES: usize = 2_000;

/// A search stops after this many hits, and after looking at this many files, so one query
/// over a huge tree cannot tie the server up.
const MAX_HITS: usize = 100;
const MAX_SEARCHED: usize = 5_000;

/// What a caller needs to be allowed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    Read,
    Write,
}

impl Need {
    fn granted(self, p: Perms) -> bool {
        match self {
            Self::Read => p.read,
            Self::Write => p.write,
        }
    }
}

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

    /// [`Jail::resolve`], but only if `user` may do `need` to the path — checked both on the
    /// path as asked for and on where it really leads, so a symlink cannot lend a private
    /// folder the permissions of a public one.
    pub fn authorize(
        &self,
        requested: &str,
        user: Option<&str>,
        need: Need,
    ) -> Result<PathBuf, u32> {
        let parts = access::components(requested)?;
        if !need.granted(access::perms(&self.root, &parts, user)) {
            return Err(server_error::DENIED);
        }
        let real = self.resolve(requested)?;
        match access::perms_of_real(&self.root, &real, user) {
            Some(p) if need.granted(p) => Ok(real),
            _ => Err(server_error::DENIED),
        }
    }

    /// Where a new entry `requested` would go, if `user` may create it: its parent must exist
    /// and be a folder, and the caller needs write permission for the new path. Returns the
    /// real parent joined with the new name.
    fn authorize_new(&self, requested: &str, user: Option<&str>) -> Result<PathBuf, u32> {
        let mut parts = access::components(requested)?;
        let Some(name) = parts.pop() else {
            // The root itself is never "new".
            return Err(server_error::EXISTS);
        };
        let mut lexical = parts.clone();
        lexical.push(name.clone());
        if !access::perms(&self.root, &lexical, user).write {
            return Err(server_error::DENIED);
        }
        let parent = self.resolve(&parts.join("/"))?;
        if !parent.is_dir() {
            return Err(server_error::NOT_A_DIRECTORY);
        }
        let target = parent.join(&name);
        match access::perms_of_real(&self.root, &target, user) {
            Some(p) if p.write => Ok(target),
            _ => Err(server_error::DENIED),
        }
    }

    /// What `user` may do with `requested`, which need not exist.
    pub fn access(&self, requested: &str, user: Option<&str>) -> Result<AccessReply, u32> {
        let parts = access::components(requested)?;
        let p = access::perms(&self.root, &parts, user);
        Ok(AccessReply {
            read: p.read,
            write: p.write,
            user: user.map(str::to_owned),
        })
    }

    pub fn read_file(&self, requested: &str, user: Option<&str>) -> Result<FileText, u32> {
        let real = self.authorize(requested, user, Need::Read)?;
        let meta = std::fs::metadata(&real).map_err(|_| server_error::IO)?;
        if meta.is_dir() {
            return Err(server_error::NOT_A_DIRECTORY);
        }
        if meta.len() > MAX_TEXT_BYTES as u64 {
            return Err(server_error::TOO_LARGE);
        }
        let bytes = std::fs::read(&real).map_err(|_| server_error::IO)?;
        let text = String::from_utf8(bytes).map_err(|_| server_error::NOT_TEXT)?;
        let writable = self.authorize(requested, user, Need::Write).is_ok();
        Ok(FileText { text, writable })
    }

    pub fn write_file(&self, req: &WriteFileReq<'_>, user: Option<&str>) -> Result<(), u32> {
        if req.text.len() > MAX_TEXT_BYTES {
            return Err(server_error::TOO_LARGE);
        }
        let target = self.authorize_new(req.path, user)?;
        match std::fs::symlink_metadata(&target) {
            Ok(_) if req.create_only => return Err(server_error::EXISTS),
            Ok(meta) if meta.is_dir() => return Err(server_error::EXISTS),
            // Replacing an existing file: it must be one this caller may write where it really
            // lives, so a symlink to somewhere private cannot be written through.
            Ok(_) => {
                self.authorize(req.path, user, Need::Write)?;
            }
            Err(_) => {}
        }
        std::fs::write(&target, req.text).map_err(|_| server_error::IO)
    }

    pub fn create_dir(&self, requested: &str, user: Option<&str>) -> Result<(), u32> {
        let target = self.authorize_new(requested, user)?;
        std::fs::create_dir(&target).map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => server_error::EXISTS,
            _ => server_error::IO,
        })
    }

    /// Walk `req.path` for files whose name or text contains `req.query`, skipping anything
    /// the caller may not read.
    pub fn search(&self, req: &SearchReq<'_>, user: Option<&str>) -> Result<SearchReply, u32> {
        let start = self.authorize(req.path, user, Need::Read)?;
        let base = access::components(req.path)?.join("/");
        let needle = req.query.trim().to_lowercase();
        let mut reply = SearchReply {
            hits: Vec::new(),
            truncated: false,
        };
        if needle.is_empty() {
            return Ok(reply);
        }

        // Breadth-first over (real dir, jail path), so shallow matches come first.
        let mut queue = std::collections::VecDeque::from([(start, base)]);
        let mut searched = 0usize;
        while let Some((dir, rel)) = queue.pop_front() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut items: Vec<_> = read.flatten().collect();
            items.sort_by_key(|e| e.file_name());
            for item in items {
                let Ok(name) = item.file_name().into_string() else {
                    continue;
                };
                if access::is_reserved(&name) {
                    continue;
                }
                let path = if rel.is_empty() {
                    name.clone()
                } else {
                    format!("{rel}/{name}")
                };
                // Everything looked at counts, not just candidate files, so a big tree of
                // other files cannot keep a search going either.
                searched += 1;
                if searched > MAX_SEARCHED || reply.hits.len() >= MAX_HITS {
                    reply.truncated = true;
                    return Ok(reply);
                }
                // `DirEntry::metadata` does not follow symlinks, so a symlinked folder is never
                // entered: a link back to its own ancestor cannot make the walk loop.
                let Ok(meta) = item.metadata() else { continue };
                if meta.is_dir() {
                    if self.authorize(&path, user, Need::Read).is_ok() {
                        queue.push_back((item.path(), path));
                    }
                    continue;
                }
                if !meta.is_file() || !name.ends_with(req.suffix) {
                    continue;
                }
                let line = if meta.len() <= MAX_TEXT_BYTES as u64 {
                    std::fs::read_to_string(item.path()).ok().and_then(|text| {
                        text.lines()
                            .find(|l| l.to_lowercase().contains(&needle))
                            .map(|l| l.trim().chars().take(160).collect::<String>())
                    })
                } else {
                    None
                };
                let name_hit = name.to_lowercase().contains(&needle);
                if name_hit || line.is_some() {
                    reply.hits.push(SearchHit {
                        path: format!("/{path}"),
                        line: line.unwrap_or_default(),
                    });
                }
            }
        }
        Ok(reply)
    }

    pub fn list_dir(&self, req: &ListDirReq<'_>, user: Option<&str>) -> Result<DirListing, u32> {
        let dir = self.authorize(req.path, user, Need::Read)?;
        let parts = access::components(req.path)?;

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
            if access::is_reserved(&name) {
                continue;
            }
            // A folder the caller may not open is not shown at all: listing `/home` shows each
            // user their own folder and nobody else's.
            if meta.is_dir() {
                let mut child = parts.clone();
                child.push(name.clone());
                if !access::perms(&self.root, &child, user).read {
                    continue;
                }
            }

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
