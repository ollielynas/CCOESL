//! The CCOSEL server.
//!
//! Serves the shell, the app modules and the RPC endpoint from **one origin**, which is why
//! there is no CORS configuration anywhere in this project.

pub mod access;
pub mod auth;
pub mod build_api;
pub mod desktop;
pub mod fs_api;
pub mod idp;
pub mod image_info;
pub mod keycloak;
pub mod rpc;
pub mod scratch;
pub mod stats;
pub mod upload_api;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path as AxPath, Query, State};
use axum::http::{StatusCode, header};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use tower_http::services::{ServeDir, ServeFile};

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

    // `/app/{id}` is the boot page again: the shell reads the id from the address and shows that
    // one app on its own. Any id gets the page, so a new app has one without touching this, and
    // the shell is what says an id names no app. Public, like the page at `/`.
    let boot_page = ServeFile::new(web_dir.join("index.html"));

    Router::new()
        .merge(protected)
        .merge(auth::router())
        .route_service("/app/{id}", boot_page.clone())
        .route_service("/app/{id}/", boot_page)
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
///
/// A download by default. With `?inline=1`, a picture, recording or PDF comes with its own type
/// instead, for the page to show it (the Viewer app). Either way it supports `Range`, which a
/// browser needs to seek in audio and video, and is streamed rather than read into memory.
async fn download(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    AxPath(path): AxPath<String>,
    Query(params): Query<DownloadParams>,
    req: axum::extract::Request,
) -> Response {
    state.scratch.touch(&path);
    let user = caller(user.as_ref().map(|u| &u.0));
    let Ok(real) = state.jail.authorize(&path, user, Need::Read) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !real.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let filename = real
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download")
        .replace(['"', '\\', '\r', '\n'], "_");
    let shown = params
        .inline
        .is_some()
        .then(|| inline_type(&real))
        .flatten();

    let mut resp = match ServeFile::new(&real).try_call(req).await {
        Ok(resp) => resp.map(axum::body::Body::new),
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let (kind, disposition) = match shown {
        Some(kind) => (kind, "inline"),
        None => ("application/octet-stream", "attachment"),
    };
    let headers = resp.headers_mut();
    // Only ever the type chosen here. A browser that sniffed an uploaded file into HTML would run
    // its scripts as this site, with the signed-in person's session.
    headers.insert(header::CONTENT_TYPE, header::HeaderValue::from_static(kind));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    if let Ok(v) = format!("{disposition}; filename=\"{filename}\"").parse() {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    resp
}

#[derive(serde::Deserialize)]
struct DownloadParams {
    /// Present (any value) to show the file in the page rather than download it.
    inline: Option<String>,
}

/// The type a file is shown in the page with, for the kinds a browser can show and that can't
/// run script: pictures, audio, video and PDF. Anything else (HTML and SVG included, which can)
/// is only ever a download.
fn inline_type(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "tif" | "tiff" => "image/tiff",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "ogv" => "video/ogg",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "caf" => "audio/x-caf",
        "aif" | "aiff" => "audio/aiff",
        "pdf" => "application/pdf",
        _ => return None,
    })
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
