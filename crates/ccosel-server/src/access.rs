//! Who may read and write what.
//!
//! Permissions are set on **folders** and inherited: a file has its folder's permissions, and
//! a folder without its own rules has its parent's. So the rules for any path are found by
//! walking up from it to the jail root and taking the first `.access` file on the way. With
//! none at all, everything is open, which is how the server behaved before this existed.
//!
//! An `.access` file is a few lines of text:
//!
//! ```text
//! # Everyone may read these docs; only the maintainers may change them.
//! read: *
//! write: alice bob
//! ```
//!
//! `*` is anyone, signed in or not; `@users` is anyone signed in; anything else is a user
//! name. A permission with no line is granted to nobody, so a folder whose `.access` says only
//! `read: *` is read-only for everyone who uses the server. It can still be changed by editing
//! the files on disk, which is the point: that is how the app documentation under `/Docs` is
//! maintained.
//!
//! `/home` is built in rather than configured. Each signed-in user has `/home/{name}`, which
//! only they can read or write, whatever any `.access` file says; `/home` itself lists, for
//! each user, only their own folder.
//!
//! `.access` files are invisible through the API: not listed, not readable, not writable. They
//! are configuration for whoever runs the server, not content.
//!
//! Who the caller is comes from a [`User`] request extension, which `auth::require_session`
//! sets from the sign-in session. With login turned off, every caller is anonymous.

use std::path::{Component, Path, PathBuf};

use ccosel_proto::server_error;

/// The file that holds a folder's rules.
pub const ACCESS_FILE: &str = ".access";

/// The folder holding every user's private folder.
pub const HOME: &str = "home";

/// The signed-in user making a request, as a request extension. Absent means anonymous.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User(pub String);

impl User {
    /// Whether `name` can safely be a folder name under `/home`. A name that could climb out
    /// of it (or hide as a dotfile) never gets a folder.
    pub fn valid_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 64
            && !name.starts_with('.')
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Perms {
    pub read: bool,
    pub write: bool,
}

impl Perms {
    const ALL: Self = Self {
        read: true,
        write: true,
    };
    const NONE: Self = Self {
        read: false,
        write: false,
    };
}

/// Split a client path into its components, refusing anything that could climb or re-root,
/// and any mention of an `.access` file.
pub fn components(requested: &str) -> Result<Vec<String>, u32> {
    let mut out = Vec::new();
    for part in Path::new(requested.trim_start_matches('/')).components() {
        match part {
            Component::Normal(p) => {
                let p = p.to_str().ok_or(server_error::DENIED)?;
                if p == ACCESS_FILE {
                    return Err(server_error::DENIED);
                }
                out.push(p.to_owned());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(server_error::DENIED);
            }
        }
    }
    Ok(out)
}

/// What `user` may do with the path `parts` (from [`components`]) under `root`.
pub fn perms(root: &Path, parts: &[String], user: Option<&str>) -> Perms {
    // Everyone's saved desktops: only the server's own code, for the caller's own file, reads
    // these (see `crate::desktop`).
    if parts.first().is_some_and(|p| p == crate::desktop::DIR) {
        return Perms::NONE;
    }
    if parts.first().is_some_and(|p| p == HOME) {
        return match parts.get(1) {
            // `/home` itself: listable by anyone signed in (the listing is filtered down to
            // their own folder), and nobody may add to it directly.
            None => Perms {
                read: user.is_some(),
                write: false,
            },
            Some(owner) if user == Some(owner.as_str()) => Perms::ALL,
            Some(_) => Perms::NONE,
        };
    }

    let mut dir = root.to_path_buf();
    let mut chain = vec![dir.join(ACCESS_FILE)];
    for part in parts {
        dir.push(part);
        chain.push(dir.join(ACCESS_FILE));
    }
    // Nearest first. For a file, `file/.access` cannot exist, so its folder's rules apply.
    for candidate in chain.iter().rev() {
        match std::fs::read_to_string(candidate) {
            Ok(text) => return Rules::parse(&text).perms(user),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // An `.access` file that exists but cannot be read (a directory by that name, bad
            // permissions, not UTF-8) must not silently fall through to a laxer parent.
            Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => {}
            Err(_) => return Perms::NONE,
        }
    }
    Perms::ALL
}

/// [`perms`] for a real path under `root`, e.g. after following symlinks. `None` if it is not
/// under `root` at all.
pub fn perms_of_real(root: &Path, real: &Path, user: Option<&str>) -> Option<Perms> {
    let rel = real.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    if parts.iter().any(|p| p == ACCESS_FILE) {
        return Some(Perms::NONE);
    }
    Some(perms(root, &parts, user))
}

/// Make `/home/{user}` if it is missing, so a user's private folder exists the first time they
/// look for it.
pub fn ensure_home(root: &Path, user: &str) -> std::io::Result<PathBuf> {
    let home = root.join(HOME).join(user);
    std::fs::create_dir_all(&home)?;
    Ok(home)
}

/// One `.access` file, parsed.
#[derive(Debug, Default, PartialEq, Eq)]
struct Rules {
    read: Vec<String>,
    write: Vec<String>,
}

impl Rules {
    fn parse(text: &str) -> Self {
        let mut rules = Self::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, who)) = line.split_once(':') else {
                continue;
            };
            let list = match key.trim() {
                "read" => &mut rules.read,
                "write" => &mut rules.write,
                // Unknown keys are ignored, so a newer server's rules do not lock an older one
                // out entirely.
                _ => continue,
            };
            list.extend(who.split_whitespace().map(str::to_owned));
        }
        rules
    }

    fn perms(&self, user: Option<&str>) -> Perms {
        let allows = |list: &[String]| {
            list.iter().any(|who| match who.as_str() {
                "*" => true,
                "@users" => user.is_some(),
                name => user == Some(name),
            })
        };
        let write = allows(&self.write);
        Perms {
            // Being able to change a file without being able to see it would be strange;
            // write implies read.
            read: write || allows(&self.read),
            write,
        }
    }
}

#[cfg(test)]
mod tests;
