//! The CCOSEL server.
//!
//! Serves the shell, the app modules and the RPC endpoint from **one origin**, which is why
//! there is no CORS configuration anywhere in this project.

pub mod access;
pub mod auth;
pub mod build_api;
pub mod fs_api;
pub mod idp;
pub mod keycloak;
pub mod rpc;
pub mod scratch;
pub mod stats;
pub mod upload_api;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path as AxPath, State};
use axum::http::{StatusCode, header};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use tower_http::services::ServeDir;

use access::User;
use auth::AuthState;
use fs_api::{Jail, Need};

/// The caller's user name, if they are signed in under a name that can safely own a folder.
/// Anything else is treated as anonymous, which can only ever see less.
pub(crate) fn caller(user: Option<&User>) -> Option<&str> {
    user.map(|u| u.0.as_str()).filter(|n| User::valid_name(n))
}

/// Treat every request as coming from `name`, whatever its session says. For tests, and for
/// embedding the server behind something else that has already identified the user; never on
/// a router real people reach directly.
pub fn as_user(router: Router, name: String) -> Router {
    router.layer(Extension(User(name)))
}

#[derive(Clone)]
pub struct AppState {
    pub jail: Arc<Jail>,
    pub auth: AuthState,
    /// Builds outlive the request that started them, so they live on the server rather than in
    /// any one call. See `build_api`.
    pub jobs: Arc<build_api::Jobs>,
    pub stats: Arc<stats::Stats>,
    /// Temporary project folders. See `scratch`.
    pub scratch: Arc<scratch::Scratch>,
}

/// Build the router. Separated from `serve` so tests can drive it on an ephemeral port.
///
/// Panics if it can't create `.scratch` in the jail: a server that can't write to its own jail
/// can't accept uploads either, and failing at start is clearer than failing on first use.
pub fn app(jail: Jail, web_dir: PathBuf, auth: AuthState) -> Router {
    let scratch = Arc::new(
        scratch::Scratch::new(jail.root()).expect("create the .scratch directory in the jail"),
    );
    // Deletes unused scratch folders even when nobody is using the server, so an uploaded
    // project doesn't wait for the next request to be cleaned up. Only when there is a runtime
    // to run on, which is always true of `serve` and of the async tests.
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        let scratch = Arc::downgrade(&scratch);
        rt.spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tick.tick().await;
                // The router, and with it the state, is gone: stop.
                let Some(scratch) = scratch.upgrade() else {
                    return;
                };
                scratch.sweep(std::time::Instant::now());
            }
        });
    }
    let state = AppState {
        scratch,
        jail: Arc::new(jail),
        auth,
        jobs: Arc::new(build_api::Jobs::new()),
        stats: Arc::new(stats::Stats::new()),
    };

    // Everything that reads or writes the jail needs a session once OAuth is configured. The
    // shell and app modules (the fallback below) stay public: they hold no user data, and the
    // boot page needs them reachable to show its sign-in button in the first place.
    let protected = Router::new()
        .route("/rpc", post(rpc::handle))
        .route(
            "/upload",
            post(upload_api::upload).layer(DefaultBodyLimit::max(upload_api::MAX_UPLOAD_BYTES)),
        )
        .route("/files/{*path}", get(download))
        .route("/scratch", post(new_scratch))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_session,
        ));

    Router::new()
        .merge(protected)
        .merge(auth::router())
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
    state.scratch.touch(&path);
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

/// `POST /scratch`: a new temporary project folder. Answers with its id as decimal text; the
/// folder is at `ccosel_proto::scratch::path(id)`.
async fn new_scratch(State(state): State<AppState>) -> Response {
    match state.scratch.create() {
        Ok(id) => id.to_string().into_response(),
        Err(e) => {
            eprintln!("scratch: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn serve(
    addr: SocketAddr,
    jail: Jail,
    web_dir: PathBuf,
    auth: AuthState,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("CCOSEL serving http://{addr}/");
    axum::serve(listener, app(jail, web_dir, auth)).await?;
    Ok(())
}
