//! `POST /upload`: receive a file from the browser and write it inside the jail.
//!
//! The client sends the file as a raw POST body; `path` and `filename` (query parameters)
//! say where it goes. Missing intermediate directories are created. An upload **overwrites**
//! an existing file at the same path — there is no separate create-vs-replace verb, which
//! matches what a folder upload or re-upload should do, and keeps the endpoint idempotent.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::AppState;

/// Caps a single upload well above what a source folder should need, but not unbounded: the
/// body is buffered in full before the handler runs, so leaving it uncapped would let one
/// upload exhaust server memory. axum's own default (2 MiB, see `DefaultBodyLimit`) is too
/// small for a folder carrying binaries or images, so the `/upload` route raises it to this
/// value explicitly rather than inheriting the default.
pub const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct UploadParams {
    /// Jail-relative directory to write into, e.g. `/my-project/src`. Created if missing.
    pub path: String,
    /// Filename within that directory, e.g. `main.rs`. May not contain a path separator.
    pub filename: String,
}

pub async fn upload(
    State(state): State<AppState>,
    Query(params): Query<UploadParams>,
    body: Bytes,
) -> Response {
    if !valid_filename(&params.filename) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let dir = match resolve_dir_create(state.jail.root(), &params.path).await {
        Ok(dir) => dir,
        Err(status) => return status.into_response(),
    };

    let file_path = dir.join(&params.filename);

    if let Err(e) = tokio::fs::write(&file_path, &body).await {
        eprintln!("upload: write {file_path:?}: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::OK.into_response()
}

/// No path separator (so the client cannot smuggle a subdirectory or an escape into what is
/// meant to be a bare name) and no `..` component, empty or otherwise.
fn valid_filename(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains('\\') && !name.contains("..")
}

/// Resolve `requested` to a real directory inside `root`, creating any missing components.
///
/// `Jail::resolve` (in `fs_api.rs`) can't be reused as-is: it insists the path already exists,
/// which is exactly what an upload into a new folder needs to not be true. This applies the
/// same two layers of defence in an order that stays safe while creating directories:
///
/// 1. **Lexical**: a `..` component, a rooted component, or an OS-specific prefix is refused
///    before anything touches disk — this also blocks the classic `PathBuf::join` footgun where
///    joining an absolute path silently discards the jail root, by never joining the raw
///    request string at all.
/// 2. **Canonical**: only the *canonical* form of the longest already-existing ancestor is ever
///    extended with new directories. A symlink among the existing components — pointing out of
///    the jail — is caught by resolving that ancestor before any `mkdir` runs, not after, so
///    nothing is ever created through it.
async fn resolve_dir_create(root: &Path, requested: &str) -> Result<PathBuf, StatusCode> {
    let rel = requested.trim_start_matches('/');
    let mut components: Vec<OsString> = Vec::new();
    for part in Path::new(rel).components() {
        match part {
            Component::Normal(p) => components.push(p.to_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(StatusCode::FORBIDDEN);
            }
        }
    }

    // Walk down from the root, stopping at the first component that doesn't exist yet.
    let mut existing = root.to_path_buf();
    let mut remaining: &[OsString] = &components;
    while let Some((head, rest)) = remaining.split_first() {
        let candidate = existing.join(head);
        if !matches!(tokio::fs::try_exists(&candidate).await, Ok(true)) {
            break;
        }
        existing = candidate;
        remaining = rest;
    }

    // Canonicalise the longest existing prefix *before* creating anything past it, so a
    // symlink anywhere in it that resolves outside the jail is refused rather than followed.
    let real_existing = tokio::fs::canonicalize(&existing)
        .await
        .map_err(|_| StatusCode::FORBIDDEN)?;
    if !real_existing.starts_with(root) {
        return Err(StatusCode::FORBIDDEN);
    }

    // Every remaining component is new, so joining it onto the verified canonical path can
    // never traverse a symlink: nothing under it exists yet.
    let target = remaining
        .iter()
        .fold(real_existing, |acc, part| acc.join(part));

    if let Err(e) = tokio::fs::create_dir_all(&target).await {
        eprintln!("upload: create_dir_all {target:?}: {e}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    // Defence in depth against a race between the checks above and the `mkdir`.
    let real_target = tokio::fs::canonicalize(&target)
        .await
        .map_err(|_| StatusCode::FORBIDDEN)?;
    if !real_target.starts_with(root) {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(real_target)
}
