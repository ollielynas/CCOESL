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
    pub fn authorize_new(&self, requested: &str, user: Option<&str>) -> Result<PathBuf, u32> {
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
        let listing = self.visible_entries(req.path, user, MAX_ENTRIES)?;
        let mut entries: Vec<DirEntry> = listing
            .entries
            .into_iter()
            .map(|e| DirEntry {
                kind: if e.meta.is_dir() {
                    EntryKind::Dir
                } else if e.meta.is_file() {
                    EntryKind::File
                } else if e.meta.is_symlink() {
                    EntryKind::Symlink
                } else {
                    EntryKind::Other
                },
                size: e.meta.len(),
                mtime_s: e
                    .meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
                writable: e.perms.write,
                name: e.name,
            })
            .collect();

        // Directories first, then name. Sorting here rather than in the app means every client
        // agrees, and a guest does not pay to sort a large listing.
        entries.sort_by(|a, b| {
            b.is_dir()
                .cmp(&a.is_dir())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(DirListing {
            entries,
            truncated: listing.truncated,
        })
    }

    /// What [`Jail::list_dir`] shows, unsorted and with each entry's own metadata (not
    /// following a symlink), at most `limit` of them. Also where the folder really is.
    pub fn visible_entries(
        &self,
        requested: &str,
        user: Option<&str>,
        limit: usize,
    ) -> Result<Visible, u32> {
        let dir = self.authorize(requested, user, Need::Read)?;
        let parts = access::components(requested)?;
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
            if entries.len() >= limit {
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
            entries.push(Entry { name, meta, perms });
        }

        Ok(Visible {
            dir,
            entries,
            truncated,
        })
    }

    /// Where the existing entry `requested` is, if `user` may remove it, without following it
    /// if it is a symlink. Removing something changes the folder it is in as well as the thing
    /// itself, so it needs:
    ///
    /// - write on the folder it is in, so nothing can be taken out of a read-only folder, and
    ///   `/home/{user}` itself (in `/home`, which nobody may change) stays put;
    /// - write on the entry, which for a folder with rules of its own is not the same thing;
    /// - for a folder, write on **every** folder inside it, hidden ones included. Otherwise a
    ///   folder whose `.access` grants less could be removed by removing its parent.
    ///
    /// A symlink is removed as a link, so only the first applies to it, but it has to be one
    /// the caller can see: one to somewhere they may not read is as absent as that place.
    pub fn check_removable(&self, requested: &str, user: Option<&str>) -> Result<PathBuf, u32> {
        let mut parts = access::components(requested)?;
        let Some(name) = parts.pop() else {
            // The root of the jail is never removed.
            return Err(server_error::DENIED);
        };
        let parent = parts.join("/");
        // The folder itself has to be visible before whether it is writable says anything.
        self.authorize(requested, user, Need::Read)?;
        let parent_real = self.authorize(&parent, user, Need::Write)?;
        let target = parent_real.join(&name);
        let meta = std::fs::symlink_metadata(&target).map_err(|_| server_error::NOT_FOUND)?;
        if meta.is_symlink() {
            return Ok(target);
        }
        parts.push(name);
        check(
            both(
                access::perms(&self.root, &parts, user),
                self.perms_of_real(&target, user),
            ),
            Need::Write,
        )?;
        if meta.is_dir() {
            self.check_tree(&target, user, Need::Write)?;
        }
        Ok(target)
    }

    /// Every folder under the real folder `top`, however deep, allows `need`. Symlinks are not
    /// followed: removing one removes the link, and copying one checks it as it is copied.
    fn check_tree(&self, top: &Path, user: Option<&str>, need: Need) -> Result<(), u32> {
        let mut stack = vec![top.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let read = std::fs::read_dir(&dir).map_err(|_| server_error::IO)?;
            for item in read {
                let item = item.map_err(|_| server_error::IO)?;
                let meta = item.metadata().map_err(|_| server_error::IO)?;
                if meta.is_dir() {
                    let p = self.perms_of_real(&item.path(), user);
                    // Something inside the caller cannot see stops them as surely as something
                    // they cannot change, but saying `NOT_FOUND` about a folder they can see
                    // would be wrong: it is there, they just may not do this to all of it.
                    if !need.granted(p) {
                        return Err(server_error::DENIED);
                    }
                    stack.push(item.path());
                }
            }
        }
        Ok(())
    }

    /// Remove `requested`, a file, link or whole folder, if [`Jail::check_removable`] allows.
    pub fn remove(&self, requested: &str, user: Option<&str>) -> Result<(), u32> {
        let target = self.check_removable(requested, user)?;
        let meta = std::fs::symlink_metadata(&target).map_err(|_| server_error::NOT_FOUND)?;
        if meta.is_dir() {
            std::fs::remove_dir_all(&target)
        } else {
            std::fs::remove_file(&target)
        }
        .map_err(|_| server_error::IO)
    }

    /// Where `requested` would be written, as a new entry or replacing a file that is already
    /// there: write on where it goes, and if a file is there, everything removing it needs.
    /// A folder already there is `EXISTS`; replacing one is a removal and a creation, which
    /// is for the caller to ask for separately.
    pub fn check_writable_target(
        &self,
        requested: &str,
        user: Option<&str>,
    ) -> Result<PathBuf, u32> {
        let target = self.authorize_new(requested, user)?;
        match std::fs::symlink_metadata(&target) {
            Ok(meta) if meta.is_dir() => Err(server_error::EXISTS),
            Ok(_) => self.check_removable(requested, user),
            Err(_) => Ok(target),
        }
    }

    /// Move `from` to `to`: removing it from where it is, and creating it where it goes.
    /// What it holds keeps its own rules (any `.access` in it moves with it) but otherwise
    /// takes on those of where it lands, like anything else created there.
    pub fn rename(&self, from: &str, to: &str, user: Option<&str>) -> Result<(), u32> {
        let source = self.check_removable(from, user)?;
        let dest = self.check_writable_target(to, user)?;
        if dest.starts_with(&source) {
            // Into itself.
            return Err(server_error::DENIED);
        }
        std::fs::rename(&source, &dest).map_err(|_| server_error::IO)
    }

    /// Where to write the contents of the file `requested`: [`Jail::check_writable_target`],
    /// except that a symlink already there is written through, so it also needs write where
    /// it really leads. Otherwise a link in a writable folder would let anyone change what it
    /// points at.
    pub fn write_target(&self, requested: &str, user: Option<&str>) -> Result<PathBuf, u32> {
        let target = self.check_writable_target(requested, user)?;
        if target.is_symlink() {
            return self.authorize(requested, user, Need::Write);
        }
        Ok(target)
    }

    /// Copy the file `from` to `to`. It needs read on `from` and what writing `to` needs.
    pub fn copy_file(&self, from: &str, to: &str, user: Option<&str>) -> Result<(), u32> {
        let source = self.authorize(from, user, Need::Read)?;
        if !source.is_file() {
            return Err(server_error::DENIED);
        }
        let dest = self.write_target(to, user)?;
        std::fs::copy(&source, &dest)
            .map(|_| ())
            .map_err(|_| server_error::IO)
    }

    /// Whether `user` may read all of `requested`: it, and for a folder every folder inside
    /// it. Copying a folder needs this, so a copy is never quietly missing what the caller
    /// could not see.
    pub fn check_readable_tree(&self, requested: &str, user: Option<&str>) -> Result<(), u32> {
        let real = self.authorize(requested, user, Need::Read)?;
        if real.is_dir() {
            self.check_tree(&real, user, Need::Read)?;
        }
        Ok(())
    }

    /// The real path of `requested` without following it if it is a symlink, if `user` may
    /// see it: what a WebDAV client is told about the entry itself.
    pub fn locate(&self, requested: &str, user: Option<&str>) -> Result<PathBuf, u32> {
        let mut parts = access::components(requested)?;
        let real = self.authorize(requested, user, Need::Read)?;
        let Some(name) = parts.pop() else {
            return Ok(real);
        };
        Ok(self.find(&parts.join("/"))?.join(name))
    }
}

/// One entry of [`Visible`].
pub struct Entry {
    pub name: String,
    /// About the entry itself: a symlink's own metadata, not its target's.
    pub meta: std::fs::Metadata,
    pub perms: Perms,
}

/// What [`Jail::visible_entries`] found.
pub struct Visible {
    /// Where the folder really is.
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub truncated: bool,
}
