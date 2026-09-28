//! Per-user desktop settings, kept as `/home/{user}/.settings.json`.
//!
//! JSON rather than postcard because this outlives any one version of [`Settings`]: with
//! `#[serde(default)]` on the struct, a file written before a field existed still loads, and
//! a field that has since been removed is ignored. postcard, which is positional, would do
//! neither.
//!
//! The file is reserved (see [`access::is_reserved`]), so the file RPCs, `/files` and
//! `/upload` can neither see nor overwrite it. The settings methods are the only way in.

use std::path::{Path, PathBuf};

use ccosel_proto::server_error;
use ccosel_proto::settings::Settings;

use crate::access;

fn file(root: &Path, user: Option<&str>) -> Result<PathBuf, u32> {
    let user = user.ok_or(server_error::NO_USER)?;
    let home = access::ensure_home(root, user).map_err(|_| server_error::IO)?;
    Ok(home.join(access::SETTINGS_FILE))
}

/// The user's settings, or the defaults if they have never saved any. A file that no longer
/// parses also reads as the defaults, so a bad write can never lock someone out of the
/// desktop's appearance; the next save replaces it.
pub fn get(root: &Path, user: Option<&str>) -> Result<Settings, u32> {
    let path = file(root, user)?;
    match std::fs::read(&path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(_) => Err(server_error::IO),
    }
}

pub fn set(root: &Path, user: Option<&str>, settings: &Settings) -> Result<(), u32> {
    let path = file(root, user)?;
    let json = serde_json::to_vec_pretty(settings).map_err(|_| server_error::IO)?;
    std::fs::write(path, json).map_err(|_| server_error::IO)
}

#[cfg(test)]
mod tests;
