//! The CCOSEL server.
//!
//! Serves the shell, the app modules and the RPC endpoint from **one origin**, which is why
//! there is no CORS configuration anywhere in this project.

pub mod access;
pub mod fs_api;
pub mod rpc;
pub mod stats;
pub mod upload_api;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path as AxPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use tower_http::services::ServeDir;

use access::User;
use fs_api::{Jail, Need};

/// The caller's user name, if they are signed in under a name that can safely own a folder.
/// Anything else is treated as anonymous, which can only ever see less.
pub(crate) fn caller(user: Option<&User>) -> Option<&str> {
    user.map(|u| u.0.as_str()).filter(|n| User::valid_name(n))
}

/// Treat every request as coming from `name`. For trying per-user folders on a machine of
/// your own before sign-in exists; on a shared network it hands everyone that user's files.
pub fn as_user(router: Router, name: String) -> Router {
    router.layer(Extension(User(name)))
}

#[derive(Clone)]
pub struct AppState {
    pub jail: Arc<Jail>,
    pub stats: Arc<stats::Stats>,
}

/// Build the router. Separated from `serve` so tests can drive it on an ephemeral port.
pub fn app(jail: Jail, web_dir: PathBuf) -> Router {
    let state = AppState {
        jail: Arc::new(jail),
        stats: Arc::new(stats::Stats::new()),
    };

    Router::new()
        .route("/rpc", post(rpc::handle))
        .route(
            "/upload",
            post(upload_api::upload).layer(DefaultBodyLimit::max(upload_api::MAX_UPLOAD_BYTES)),
        )
        .route("/files/{*path}", get(download))
        .fallback_service(
            // Precompressed assets are served as-is when the client accepts them: compressing
            // a 5 MB shell on every request would be absurd, and `xtask` can do it once at
            // build time.
            ServeDir::new(web_dir)
                .precompressed_br()
                .precompressed_gzip(),
        )
        .with_state(state)
}

/// Streams a file out of the jail, if the caller may read it. This is the plain jailed-path counterpart
/// to `ListDir` rather than the content-addressed `/cas/{blake3}` scheme `ARCHITECTURE.md`
/// describes for other assets, which needs `ccosel-cas` to exist first. It discloses no more
/// than `ListDir` already does: anything under the jail is already fair game to enumerate, this
/// just answers "and can I have the bytes." — under the same permissions (see `access`).
async fn download(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    AxPath(path): AxPath<String>,
) -> Response {
    let user = caller(user.as_ref().map(|u| &u.0));
    let Ok(real) = state.jail.authorize(&path, user, Need::Read) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Ok(bytes) = tokio::fs::read(&real).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let filename = real
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download")
        .replace('"', "_");

    (
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        bytes,
    )
        .into_response()
}

pub async fn serve(
    addr: SocketAddr,
    jail: Jail,
    web_dir: PathBuf,
    user: Option<String>,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("CCOSEL serving http://{addr}/");
    let mut router = app(jail, web_dir);
    if let Some(name) = user {
        if !User::valid_name(&name) {
            anyhow::bail!("--user {name:?} is not a usable user name");
        }
        println!("every request is signed in as {name} (--user)");
        router = as_user(router, name);
    }
    axum::serve(listener, router).await?;
    Ok(())
}
