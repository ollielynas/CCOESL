//! Where each person's desktop layout is kept: `<jail root>/.desktops/<user>.layout`.
//!
//! In the jail so it travels with the rest of the server's data (the Docker volume), but never
//! reachable through it: `access::perms` refuses every path under [`DIR`] to everyone, so it is
//! left out of listings and `/files`. Only [`load`] and [`save`] touch it, each for the caller's
//! own file.

use std::path::{Path, PathBuf};

use ccosel_proto::desktop::{DesktopLayout, MAX_WINDOWS};
use ccosel_proto::server_error;

/// The top-level folder in the jail that holds every saved layout.
pub const DIR: &str = ".desktops";

/// The most bytes one saved layout may take. Generous for [`MAX_WINDOWS`] windows with long
/// paths, and a bound on what one person can store.
pub const MAX_BYTES: usize = 64 * 1024;

fn file(root: &Path, user: &str) -> PathBuf {
    root.join(DIR).join(format!("{user}.layout"))
}

/// `user`'s saved desktop. Empty for someone not signed in, someone with nothing saved yet, or a
/// file this server can no longer read (from an older version, say): starting with an empty
/// desktop is better than not starting.
pub fn load(root: &Path, user: Option<&str>) -> DesktopLayout {
    user.and_then(|u| std::fs::read(file(root, u)).ok())
        .and_then(|bytes| postcard::from_bytes(&bytes).ok())
        .unwrap_or_default()
}

/// Replace `user`'s saved desktop with `layout`. Nothing is kept for someone not signed in.
pub fn save(root: &Path, user: Option<&str>, layout: &DesktopLayout) -> Result<(), u32> {
    let Some(user) = user else {
        return Ok(());
    };
    if layout.windows.len() > MAX_WINDOWS {
        return Err(server_error::TOO_LARGE);
    }
    let bytes = postcard::to_allocvec(layout).map_err(|_| server_error::MALFORMED)?;
    if bytes.len() > MAX_BYTES {
        return Err(server_error::TOO_LARGE);
    }
    let path = file(root, user);
    let dir = root.join(DIR);
    std::fs::create_dir_all(&dir).map_err(|_| server_error::IO)?;
    // Written beside it and renamed over it, so a crash mid-write leaves the old layout, never
    // half of a new one.
    let tmp = dir.join(format!("{user}.layout.tmp"));
    std::fs::write(&tmp, &bytes).map_err(|_| server_error::IO)?;
    std::fs::rename(&tmp, &path).map_err(|_| server_error::IO)
}
