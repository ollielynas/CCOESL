//! `POST /upload`: receive files from the browser, one at a time.
//!
//! The client sends each file as a raw POST body with query parameters describing where it
//! goes. The server creates intermediate directories as needed and writes the file, staying
//! within the jail.

use std::path::PathBuf;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct UploadParams {
    /// Jail-relative directory to write into, e.g. `/my-project/src`.
    pub path: String,
    /// Filename within that directory, e.g. `main.rs`.
    pub filename: String,
}

pub async fn upload(
    State(state): State<AppState>,
    Query(params): Query<UploadParams>,
    body: Bytes,
) -> Response {
    // Resolve the parent directory (must exist or be creatable).
    let dir = match state.jail.resolve(&params.path) {
        Ok(p) => p,
        Err(_) => return StatusCode::FORBIDDEN.into_response(),
    };

    // Create intermediate directories.
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        log::error!("upload: create_dir_all {dir:?}: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Validate the filename has no path separators (defence in depth).
    if params.filename.contains('/')
        || params.filename.contains('\\')
        || params.filename.contains("..")
    {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let file_path = dir.join(&params.filename);

    // Double-check the resolved path is still inside the jail.
    if let Ok(real) = file_path.canonicalize() {
        if !real.starts_with(state.jail.root()) {
            return StatusCode::FORBIDDEN.into_response();
        }
    }

    if let Err(e) = tokio::fs::write(&file_path, &body).await {
        log::error!("upload: write {file_path:?}: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::OK.into_response()
}
