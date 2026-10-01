//! Filesystem methods, and the jail that keeps them honest.

use std::path::{Component, Path, PathBuf};

use ccosel_proto::fs::{
    AccessReply, DirEntry, DirListing, EntryKind, FileText, ImageInfoReply, ListDirReq,
    MAX_TEXT_BYTES, SearchHit, SearchReply, SearchReq, WriteFileReq,
};
use ccosel_proto::server_error;

use crate::access::{self, ACCESS_FILE, Perms};

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

/// Whether `p` allows `need`, and if not, the error that says so. Something the caller may not
/// read answers `NOT_FOUND`, exactly as if it were not there, so no answer can tell a hidden
/// file or folder apart from a missing one. `DENIED` is only for something the caller can see
/// but not change.
fn check(p: Perms, need: Need) -> Result<(), u32> {
    if !p.read {
        Err(server_error::NOT_FOUND)
    } else if !need.granted(p) {
        Err(server_error::DENIED)
    } else {
        Ok(())
    }
}

/// What is allowed by both of two sets of rules: those for a path as asked for, and those for
/// where it really leads.
fn both(a: Perms, b: Perms) -> Perms {
    Perms {
        read: a.read && b.read,
        write: a.write && b.write,
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
    /// folder the permissions of a public one. See [`check`] for which error a refusal is.
    pub fn authorize(
        &self,
        requested: &str,
        user: Option<&str>,
        need: Need,
    ) -> Result<PathBuf, u32> {
        let parts = access::components(requested)?;
        check(access::perms(&self.root, &parts, user), need)?;
        let real = self.find(requested)?;
        check(self.perms_of_real(&real, user), need)?;
        Ok(real)
    }

    /// [`Jail::resolve`] for a caller: a symlink leading out of the jail is as absent as
    /// anything else they may not read.
    fn find(&self, requested: &str) -> Result<PathBuf, u32> {
        self.resolve(requested).map_err(|e| match e {
            server_error::DENIED => server_error::NOT_FOUND,
            e => e,
        })
    }

    /// The rules for a real path, or none at all if it is not in the jail.
    fn perms_of_real(&self, real: &Path, user: Option<&str>) -> Perms {
        access::perms_of_real(&self.root, real, user).unwrap_or_default()
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
        check(access::perms(&self.root, &lexical, user), Need::Write)?;
        let parent = self.find(&parts.join("/"))?;
        if !parent.is_dir() {
            return Err(server_error::NOT_A_DIRECTORY);
        }
        let target = parent.join(&name);
        check(self.perms_of_real(&target, user), Need::Write)?;
        Ok(target)
    }

    /// What `user` may do with `requested`. Something missing and something hidden from them
    /// both answer no access at all, so this cannot find a hidden folder by guessing its name.
    pub fn access(&self, requested: &str, user: Option<&str>) -> Result<AccessReply, u32> {
        let parts = access::components(requested)?;
        let p = match self.authorize(requested, user, Need::Read) {
            Ok(real) => both(
                access::perms(&self.root, &parts, user),
                self.perms_of_real(&real, user),
            ),
            Err(_) => Perms::default(),
        };
        Ok(AccessReply {
            read: p.read,
            write: p.write,
            user: user.map(str::to_owned),
        })
    }

    /// A picture's size and EXIF details (see [`crate::image_info`]), if the caller may read it.
    pub fn image_info(&self, requested: &str, user: Option<&str>) -> Result<ImageInfoReply, u32> {
        let real = self.authorize(requested, user, Need::Read)?;
        if !real.is_file() {
            return Err(server_error::NOT_FOUND);
        }
        Ok(crate::image_info::read(&real))
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
                if name == ACCESS_FILE {
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

    /// The entries of a folder the caller may read, leaving out everything in it they may not.
    /// Each entry is checked the way opening it would be, both as named and where it really
    /// leads, so a symlink to somewhere private is left out like the private folder itself.
    pub fn list_dir(&self, req: &ListDirReq<'_>, user: Option<&str>) -> Result<DirListing, u32> {
        let dir = self.authorize(req.path, user, Need::Read)?;
        let parts = access::components(req.path)?;
        // A plain file has its folder's rules, so they are worked out once, not per file.
        let here = both(
            access::perms(&self.root, &parts, user),
            self.perms_of_real(&dir, user),
        );

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
            if name == ACCESS_FILE {
                continue;
            }
            // `DirEntry::metadata` does not follow symlinks, so `meta` is about the entry
            // itself: a real folder, a real file, or a link to be followed and checked.
            let perms = if meta.is_file() {
                here
            } else if meta.is_dir() {
                // A folder can have rules of its own. One the caller may not open is not shown
                // at all: listing `/home` shows each user their own folder and nobody else's.
                let mut child = parts.clone();
                child.push(name.clone());
                both(
                    access::perms(&self.root, &child, user),
                    self.perms_of_real(&item.path(), user),
                )
            } else {
                let child = format!("{}/{name}", parts.join("/"));
                match self.authorize(&child, user, Need::Read) {
                    Ok(_) => Perms {
                        read: true,
                        write: self.authorize(&child, user, Need::Write).is_ok(),
                    },
                    // Dangling, out of the jail, or somewhere the caller may not read.
                    Err(_) => continue,
                }
            };
            if !perms.read {
                continue;
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
                writable: perms.write,
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
