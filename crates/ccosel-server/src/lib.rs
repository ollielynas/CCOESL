//! The CCOSEL server.
//!
//! Serves the shell, the app modules and the RPC endpoint from **one origin**, which is why
//! there is no CORS configuration anywhere in this project.

pub mod auth;
pub mod fs_api;
pub mod rpc;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::routing::post;
use tower_http::services::ServeDir;

use auth::AuthState;
use fs_api::Jail;

#[derive(Clone)]
pub struct AppState {
    pub jail: Arc<Jail>,
    pub auth: AuthState,
}

/// Build the router. Separated from `serve` so tests can drive it on an ephemeral port.
pub fn app(jail: Jail, web_dir: PathBuf, auth: AuthState) -> Router {
    let state = AppState {
        jail: Arc::new(jail),
        auth,
    };

    Router::new()
        .route("/rpc", post(rpc::handle))
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
