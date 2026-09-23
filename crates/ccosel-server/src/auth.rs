//! OAuth login, gated by a list of approved accounts.
//!
//! Scope, deliberately: GitHub is the only provider wired up, the server keeps sessions
//! in-memory (a restart signs everyone out), and everything below assumes `http://` on
//! `localhost` — exactly what the ticket asked for. See the doc comments on [`SESSION_COOKIE`]
//! and [`OAuthConfig`] for what that means and what a later HTTPS pass would change.
//!
//! The flow is the standard OAuth "authorization code" dance, run entirely as ordinary browser
//! navigations (redirects), not as an app RPC method — a top-level redirect to github.com and
//! back cannot go through the guest command-buffer protocol, so this lives beside it instead:
//!
//! 1. `GET /auth/login` redirects to the provider with a random `state` token the server
//!    remembers.
//! 2. The provider redirects back to `GET /auth/callback?code=..&state=..`.
//! 3. The server checks `state`, exchanges `code` for an access token, and asks the provider
//!    who that token belongs to.
//! 4. That login is checked against [`ApprovedUsers`]. Approved: a session cookie is set and
//!    the browser is sent to `/`. Not approved: `403`, no cookie.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use rand::Rng;
use rand::distributions::Alphanumeric;
use serde::{Deserialize, Serialize};

use crate::AppState;

/// A login has this long to complete (land on `/auth/callback` with a valid `state`) before the
/// `state` token is forgotten and the attempt has to restart.
const LOGIN_TTL: Duration = Duration::from_secs(10 * 60);

/// How long an established session lasts before its cookie stops working. Sessions live only
/// in memory, so a server restart signs everyone out well before this anyway.
const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Not marked `Secure`, because the ticket is explicit that this has to work over plain
/// `http://localhost` for now — a `Secure` cookie is silently dropped by the browser on a
/// non-HTTPS origin, which would make login look broken rather than insecure. `HttpOnly` still
/// keeps it out of reach of any script running in the page. Revisit when HTTPS lands.
const SESSION_COOKIE: &str = "ccosel_session";

fn random_token(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

/// Where the provider's endpoints are. A real deployment gets [`OAuthConfig::github`]; tests
/// point `token_url`/`user_url` at a local mock server instead of the real GitHub API, the same
/// way `compile_http.rs` drives a real socket rather than mocking axum itself.
#[derive(Clone, Debug)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub authorize_url: String,
    pub token_url: String,
    pub user_url: String,
}

impl OAuthConfig {
    pub fn github(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
            authorize_url: "https://github.com/login/oauth/authorize".to_string(),
            token_url: "https://github.com/login/oauth/access_token".to_string(),
            user_url: "https://api.github.com/user".to_string(),
        }
    }
}

/// The allow-list a signed-in account is checked against. Logins are compared
/// case-insensitively, since GitHub usernames are case-insensitive.
#[derive(Clone, Default, Debug)]
pub struct ApprovedUsers(Arc<HashSet<String>>);

impl ApprovedUsers {
    pub fn new(names: impl IntoIterator<Item = String>) -> Self {
        Self(Arc::new(
            names.into_iter().map(|n| n.to_lowercase()).collect(),
        ))
    }

    /// One username per line. Blank lines and `#`-comments are ignored, so the file can explain
    /// itself. Not finding the file is the caller's decision (main.rs treats it as an empty
    /// list plus a warning, rather than refusing to start).
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self::new(text.lines().filter_map(|line| {
            let line = line.trim();
            (!line.is_empty() && !line.starts_with('#')).then(|| line.to_string())
        })))
    }

    pub fn is_approved(&self, login: &str) -> bool {
        self.0.contains(&login.to_lowercase())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

struct Session {
    login: String,
    expires_at: Instant,
}

/// Everything the auth routes need, cloned into [`AppState`] like `jail` is. `oauth` is `None`
/// when the server was started without provider credentials — the routes then answer `503`
/// instead of the server refusing to start, so a checkout with no OAuth app configured (the
/// common case while developing anything unrelated) still runs.
#[derive(Clone)]
pub struct AuthState {
    oauth: Option<OAuthConfig>,
    approved: ApprovedUsers,
    http: reqwest::Client,
    pending_logins: Arc<Mutex<HashMap<String, Instant>>>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new(None, ApprovedUsers::default())
    }
}

impl AuthState {
    pub fn new(oauth: Option<OAuthConfig>, approved: ApprovedUsers) -> Self {
        Self {
            oauth,
            approved,
            http: reqwest::Client::new(),
            pending_logins: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn session_login(&self, cookie_header: Option<&str>) -> Option<String> {
        let token = read_cookie(cookie_header?, SESSION_COOKIE)?;
        let mut sessions = self.sessions.lock().unwrap();
        // Swept lazily on lookup rather than on a timer: this process has no background tasks
        // today, and a login list short enough to hand-maintain never grows sessions enough for
        // that to matter.
        sessions.retain(|_, s| s.expires_at > Instant::now());
        sessions.get(&token).map(|s| s.login.clone())
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/logout", get(logout))
        .route("/auth/me", get(me))
}

/// The `Host` header of the incoming request, so the redirect URI matches however the server
/// was actually reached (`localhost:8777`, `127.0.0.1:8777`, a LAN hostname, ...) rather than a
/// value baked in at startup. `http://`, not `https://` — see the module doc.
fn origin(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:8777");
    format!("http://{host}")
}

async fn login(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(oauth) = &state.auth.oauth else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "OAuth is not configured on this server",
        )
            .into_response();
    };

    let csrf = random_token(32);
    state
        .auth
        .pending_logins
        .lock()
        .unwrap()
        .insert(csrf.clone(), Instant::now() + LOGIN_TTL);

    let redirect_uri = format!("{}/auth/callback", origin(&headers));
    let url = format!(
        "{}?client_id={}&redirect_uri={}&scope=read:user&state={}",
        oauth.authorize_url,
        urlencode(&oauth.client_id),
        urlencode(&redirect_uri),
        urlencode(&csrf),
    );
    Redirect::to(&url).into_response()
}

#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    /// Set by the provider instead of `code` when the user declines on its consent screen.
    error: Option<String>,
}

async fn callback(
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
    headers: HeaderMap,
) -> Response {
    let Some(oauth) = &state.auth.oauth else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "OAuth is not configured on this server",
        )
            .into_response();
    };

    if let Some(err) = params.error {
        return (
            StatusCode::BAD_REQUEST,
            format!("login was not completed: {err}"),
        )
            .into_response();
    }
    let Some(code) = params.code else {
        return (StatusCode::BAD_REQUEST, "missing code").into_response();
    };
    let Some(csrf) = params.state else {
        return (StatusCode::BAD_REQUEST, "missing state").into_response();
    };

    {
        let mut pending = state.auth.pending_logins.lock().unwrap();
        pending.retain(|_, exp| *exp > Instant::now());
        if pending.remove(&csrf).is_none() {
            return (
                StatusCode::BAD_REQUEST,
                "login expired or was never started, try again",
            )
                .into_response();
        }
    }

    let redirect_uri = format!("{}/auth/callback", origin(&headers));
    let login = match exchange_and_fetch_login(&state.auth.http, oauth, &code, &redirect_uri).await
    {
        Ok(login) => login,
        Err(msg) => return (StatusCode::BAD_GATEWAY, msg).into_response(),
    };

    if !state.auth.approved.is_approved(&login) {
        return (
            StatusCode::FORBIDDEN,
            format!(
                "signed in as {login}, but that account is not on this server's approved list. \
                 Ask the server's owner to add it."
            ),
        )
            .into_response();
    }

    let token = random_token(48);
    state.auth.sessions.lock().unwrap().insert(
        token.clone(),
        Session {
            login,
            expires_at: Instant::now() + SESSION_TTL,
        },
    );

    let mut resp = Redirect::to("/").into_response();
    resp.headers_mut().append(
        header::SET_COOKIE,
        format!(
            "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
            SESSION_TTL.as_secs()
        )
        .parse()
        .unwrap(),
    );
    resp
}

/// The two calls a provider's OAuth flow needs after the redirect: trade the one-time `code`
/// for an access token, then ask who it belongs to. Split out from [`callback`] so the network
/// half is one function with one error path, independent of routing and session bookkeeping.
async fn exchange_and_fetch_login(
    http: &reqwest::Client,
    oauth: &OAuthConfig,
    code: &str,
    redirect_uri: &str,
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct TokenResp {
        access_token: Option<String>,
        error_description: Option<String>,
        error: Option<String>,
    }

    let token_resp: TokenResp = http
        .post(&oauth.token_url)
        .header(header::ACCEPT, "application/json")
        .form(&[
            ("client_id", oauth.client_id.as_str()),
            ("client_secret", oauth.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await
        .map_err(|e| format!("could not reach the OAuth provider: {e}"))?
        .json()
        .await
        .map_err(|e| format!("the OAuth provider's token response was not understood: {e}"))?;

    let token = token_resp.access_token.ok_or_else(|| {
        token_resp
            .error_description
            .or(token_resp.error)
            .unwrap_or_else(|| "the OAuth provider did not return an access token".to_string())
    })?;

    #[derive(Deserialize)]
    struct UserResp {
        login: Option<String>,
    }

    let user: UserResp = http
        .get(&oauth.user_url)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::USER_AGENT, "ccosel-server")
        .send()
        .await
        .map_err(|e| format!("could not fetch the account from the OAuth provider: {e}"))?
        .json()
        .await
        .map_err(|e| format!("the OAuth provider's account response was not understood: {e}"))?;

    user.login
        .ok_or_else(|| "the OAuth provider did not return an account name".to_string())
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok())
        && let Some(token) = read_cookie(cookie, SESSION_COOKIE)
    {
        state.auth.sessions.lock().unwrap().remove(&token);
    }
    let mut resp = Redirect::to("/").into_response();
    // A negative Max-Age tells the browser to drop the cookie now, regardless of what it was.
    resp.headers_mut().append(
        header::SET_COOKIE,
        format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
            .parse()
            .unwrap(),
    );
    resp
}

#[derive(Serialize)]
struct MeResponse {
    /// Whether this server has an OAuth app configured at all. The boot page in `web/` uses
    /// this to decide whether to gate on login in the first place — an operator who has not
    /// set up an OAuth app yet still gets a working, ungated desktop.
    configured: bool,
    authenticated: bool,
    login: Option<String>,
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> Json<MeResponse> {
    let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok());
    let login = state.auth.session_login(cookie);
    Json(MeResponse {
        configured: state.auth.oauth.is_some(),
        authenticated: login.is_some(),
        login,
    })
}

/// A `Cookie` header holds `name=value` pairs separated by `; `. No library pulled in for this:
/// it is one pass over one header, the same size class as `rpc_http.rs`'s hand-rolled HTTP.
fn read_cookie(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|part| {
        let part = part.trim();
        let (k, v) = part.split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

/// `application/x-www-form-urlencoded`-shaped escaping, sufficient for the ids, URLs and random
/// tokens this module puts into a query string — none contain bytes outside what this covers.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests;
