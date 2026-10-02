//! The shell's own files: the boot page, the shell, and the app modules, from `--web`.
//!
//! These live at fixed URLs that every build reuses (`/dist/ccosel-shell_bg.wasm`,
//! `/dist/octave.wasm`), so a browser must never use one it has without asking whether it is
//! still current. Without that, a browser that had loaded an earlier build kept its shell for
//! hours after the server changed (browsers guess at freshness from `Last-Modified` when told
//! nothing), and ran it against the new build's apps: apps missing from the old shell's list,
//! and apps built for a newer ABI refused.
//!
//! So every response says `Cache-Control: no-cache` (keep it, but check first) and carries an
//! `ETag`, which a check (`If-None-Match`) is answered against. The tag comes from the file's
//! size, modification time and inode, so a file replaced by a new build always gets a new one,
//! even one older by the clock than the copy the browser holds. That is why `If-Modified-Since`
//! is not used for these: a build restored from a cache can have an older date than the build
//! it replaces, and a date comparison would call the old copy current.
//!
//! Content-addressed URLs, as `ARCHITECTURE.md` plans for app modules, would make the check
//! unnecessary; until then, it costs one small request per file per page load.

use std::path::{Component, Path, PathBuf};

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower_http::services::ServeDir;

/// Serve `req` from `web`, telling the browser to revalidate, and answering revalidation.
pub async fn serve(web: &Path, mut req: Request) -> Response {
    let tag = file_for(web, req.uri().path()).and_then(|f| etag(&f));
    let no_cache = HeaderValue::from_static("no-cache");
    if let Some(tag) = &tag
        && matches(req.headers().get(header::IF_NONE_MATCH), tag)
    {
        let mut res = StatusCode::NOT_MODIFIED.into_response();
        res.headers_mut().insert(header::ETAG, tag.clone());
        res.headers_mut().insert(header::CACHE_CONTROL, no_cache);
        return res;
    }
    // See the module docs: a date can't tell this build from the last.
    req.headers_mut().remove(header::IF_MODIFIED_SINCE);
    // Precompressed assets are served as-is when the client accepts them: compressing a 5 MB
    // shell on every request would be absurd, and `xtask` can do it once at build time.
    let files = ServeDir::new(web).precompressed_br().precompressed_gzip();
    let mut res = match files.oneshot(req).await {
        Ok(res) => res.into_response(),
        Err(never) => match never {},
    };
    if res.status().is_success() {
        res.headers_mut().insert(header::CACHE_CONTROL, no_cache);
        if let Some(tag) = tag {
            res.headers_mut().insert(header::ETAG, tag);
        }
    }
    res
}

/// The file `path` names under `web`, as `ServeDir` would pick it: `index.html` for a folder.
/// `None` for anything that could step outside `web`, or that isn't plainly spelled (escaped
/// characters): those are left to `ServeDir`, without a tag.
fn file_for(web: &Path, path: &str) -> Option<PathBuf> {
    let rel = Path::new(path.trim_start_matches('/'));
    if path.contains('%') || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    let file = web.join(rel);
    Some(if file.is_dir() {
        file.join("index.html")
    } else {
        file
    })
}

/// A weak tag for `file` as it is now on disk. Weak, because the same tag is given to a
/// precompressed copy of it, whose bytes differ but whose content does not.
fn etag(file: &Path) -> Option<HeaderValue> {
    let meta = std::fs::metadata(file).ok().filter(|m| m.is_file())?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&meta);
    #[cfg(not(unix))]
    let inode = 0u64;
    HeaderValue::from_str(&format!("W/\"{:x}-{mtime:x}-{inode:x}\"", meta.len())).ok()
}

/// Whether an `If-None-Match` header names `tag` (or is `*`).
fn matches(header: Option<&HeaderValue>, tag: &HeaderValue) -> bool {
    let Some(Ok(header)) = header.map(HeaderValue::to_str) else {
        return false;
    };
    let tag = tag.to_str().unwrap_or_default();
    header
        .split(',')
        .map(str::trim)
        .any(|t| t == "*" || t == tag || t.trim_start_matches("W/") == tag.trim_start_matches("W/"))
}
