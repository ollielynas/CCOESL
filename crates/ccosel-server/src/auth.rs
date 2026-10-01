//! OAuth login against a Keycloak realm. Who may sign in is decided by who has an account in
//! that realm; this server keeps no list of its own.
//!
//! Scope, deliberately: Keycloak is the only provider wired up, and the server keeps sessions
//! in-memory (a restart signs everyone out). It works over plain `http://localhost`, and behind
//! an HTTPS tunnel once [`AuthState::with_public_url`] says where that tunnel ends.
//!
//! The flow is the standard OAuth "authorization code" dance, run entirely as ordinary browser
//! navigations (redirects), not as an app RPC method — a top-level redirect to Keycloak and
//! back cannot go through the guest command-buffer protocol, so this lives beside it instead:
//!
//! 1. `GET /auth/login` redirects to the provider with a random `state` token the server
//!    remembers.
//! 2. The provider redirects back to `GET /auth/callback?code=..&state=..`.
//! 3. The server checks `state`, exchanges `code` for an access token, and asks the provider
//!    who that token belongs to.
//! 4. A session cookie is set and the browser is sent to `/`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use rand::Rng;
use rand::distributions::Alphanumeric;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::app_passwords::{AppPasswords, Throttle};

/// A login has this long to complete (land on `/auth/callback` with a valid `state`) before the
/// `state` token is forgotten and the attempt has to restart.
const LOGIN_TTL: Duration = Duration::from_secs(10 * 60);

/// How long an established session lasts before its cookie stops working. Sessions live only
/// in memory, so a server restart signs everyone out well before this anyway.
const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Marked `Secure` only when the public URL is `https://`: over plain `http://localhost` a
/// `Secure` cookie is silently dropped by the browser, which would make login look broken
/// rather than insecure. `HttpOnly` always keeps it out of reach of scripts in the page.
pub(crate) const SESSION_COOKIE: &str = "ccosel_session";

fn random_token(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

/// Where the provider's endpoints are. A real deployment gets [`OAuthConfig::keycloak`]; tests
/// point `token_url`/`user_url` at a local mock server instead of a real Keycloak instance, the
/// same way `compile_http.rs` drives a real socket rather than mocking axum itself.
#[derive(Clone, Debug)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub authorize_url: String,
    pub token_url: String,
    pub user_url: String,
    /// Where the provider ends its own session. Without this, signing out here would leave
    /// Keycloak's session alive, and the next "Sign in" would go straight back in with no
    /// password asked.
    pub logout_url: String,
    /// The provider's own page for a signed-in user to manage their profile and password.
    pub account_url: String,
}

impl OAuthConfig {
    pub fn keycloak(base_url: String, client_id: String, client_secret: Option<String>) -> Self {
        Self {
            client_id,
            client_secret,
            authorize_url: format!("{base_url}/protocol/openid-connect/auth"),
            token_url: format!("{base_url}/protocol/openid-connect/token"),
            user_url: format!("{base_url}/protocol/openid-connect/userinfo"),
            logout_url: format!("{base_url}/protocol/openid-connect/logout"),
            account_url: format!("{base_url}/account"),
        }
    }

    /// A Keycloak this server talks to at `base_url` (say `http://127.0.0.1:8080/idp/realms/x`)
    /// but that browsers only reach through this server's `/idp` forwarding. The two pages a
    /// browser visits become same-origin paths; the calls this server makes itself stay on
    /// `base_url`, which never leaves the machine.
    pub fn proxied_keycloak(base_url: String, client_id: String) -> Self {
        // Everything after `scheme://authority`.
        let path = base_url
            .split_once("://")
            .and_then(|(_, rest)| rest.find('/').map(|i| rest[i..].to_owned()))
            .unwrap_or_default();
        Self {
            authorize_url: format!("{path}/protocol/openid-connect/auth"),
            account_url: format!("{path}/account"),
            ..Self::keycloak(base_url, client_id, None)
        }
    }
}

struct Session {
    login: String,
    /// What the provider needs to end its side of the session, if it issued one.
    refresh_token: Option<String>,
    /// The address the browser signed in through, for a proxied Keycloak. See
    /// [`AuthState::backchannel_headers`].
    forwarded: Option<Forwarded>,
    expires_at: Instant,
}

/// The address a browser reached this server at, as `X-Forwarded-*` headers describe it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Forwarded {
    pub proto: String,
    /// `host[:port]`.
    pub host: String,
}

impl Forwarded {
    pub(crate) fn apply(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header("x-forwarded-proto", &self.proto)
            .header("x-forwarded-host", &self.host)
    }
}

/// Everything the auth routes need, cloned into [`AppState`] like `jail` is. `oauth` is `None`
/// when the server was started without provider credentials — the routes then answer `503`
/// instead of the server refusing to start, so a checkout with no OAuth app configured (the
/// common case while developing anything unrelated) still runs.
#[derive(Clone)]
pub struct AuthState {
    oauth: Option<OAuthConfig>,
    /// Where browsers reach this server, e.g. `https://ccosel.example.com` behind a tunnel.
    /// `None` means "however this request arrived, over `http://`".
    public_url: Option<String>,
    /// Set when Keycloak sits behind this server's `/idp` path. See [`crate::idp`].
    pub(crate) idp: Option<crate::idp::IdpProxy>,
    http: reqwest::Client,
    pending_logins: Arc<Mutex<HashMap<String, Instant>>>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    /// What WebDAV clients sign in with. See [`crate::app_passwords`].
    pub(crate) app_passwords: Arc<AppPasswords>,
    /// Failed WebDAV sign-ins, per login and per client address.
    pub(crate) throttle: Arc<Throttle>,
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new(None)
    }
}

impl AuthState {
    pub fn new(oauth: Option<OAuthConfig>) -> Self {
        Self {
            oauth,
            public_url: None,
            idp: None,
            http: reqwest::Client::new(),
            pending_logins: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            app_passwords: Arc::new(AppPasswords::in_memory()),
            throttle: Arc::new(Throttle::default()),
        }
    }

    /// Keep app passwords in `store` rather than in memory, where a restart forgets them.
    pub fn with_app_passwords(mut self, store: AppPasswords) -> Self {
        self.app_passwords = Arc::new(store);
        self
    }

    /// The app password store, for making one outside an RPC (tests, embedding).
    pub fn app_passwords(&self) -> &AppPasswords {
        &self.app_passwords
    }

    /// Where browsers reach this server, when that isn't simply `http://{Host}`: behind an
    /// HTTPS tunnel the sign-in callback must be the tunnel's `https://` address (Keycloak
    /// matches it exactly), and the session cookie must be `Secure`.
    pub fn with_public_url(mut self, url: Option<String>) -> Self {
        self.public_url = url.map(|u| u.trim_end_matches('/').to_owned());
        self
    }

    /// Forward browsers' `/idp/...` requests to a Keycloak only this machine can reach.
    pub fn with_idp_proxy(mut self, idp: crate::idp::IdpProxy) -> Self {
        self.idp = Some(idp);
        self
    }

    /// The address this request's browser used: the public URL behind a tunnel, else the
    /// request's own `Host` over `http://`.
    pub(crate) fn forwarded(&self, headers: &HeaderMap) -> Forwarded {
        match &self.public_url {
            Some(public) => {
                let (proto, rest) = public.split_once("://").unwrap_or(("http", public));
                // Just `host[:port]`: the server is always mounted at the root of its URL.
                let host = rest.split('/').next().unwrap_or(rest);
                Forwarded {
                    proto: proto.to_owned(),
                    host: host.to_owned(),
                }
            }
            None => Forwarded {
                proto: "http".to_owned(),
                host: headers
                    .get(header::HOST)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("localhost")
                    .to_owned(),
            },
        }
    }

    /// Headers for this server's own calls to a proxied Keycloak (token exchange, user info,
    /// sign-out). Keycloak stamps a token with the address the browser signed in through, and
    /// refuses it at any other address, so these calls must claim that same address even
    /// though they go straight to its loopback port. `None` for a Keycloak of the user's own,
    /// which browsers and this server reach at the same address anyway.
    fn backchannel_headers(&self, headers: &HeaderMap) -> Option<Forwarded> {
        self.idp.as_ref().map(|_| self.forwarded(headers))
    }

    fn cookie_attrs(&self) -> &'static str {
        match &self.public_url {
            Some(url) if url.starts_with("https://") => "Path=/; HttpOnly; SameSite=Lax; Secure",
            _ => "Path=/; HttpOnly; SameSite=Lax",
        }
    }

    /// Where this request's browser should come back to after signing in: the public URL if
    /// there is one, else however the request reached us (`localhost:8777`, a LAN address, ...).
    fn origin(&self, headers: &HeaderMap) -> String {
        if let Some(url) = &self.public_url {
            return url.clone();
        }
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("localhost:8777");
        format!("http://{host}")
    }

    /// Whether login is turned on. Off, every request is let in.
    pub fn enabled(&self) -> bool {
        self.oauth.is_some()
    }

    /// Where a signed-in user manages their account at the provider, if login is on.
    pub fn account_url(&self) -> Option<&str> {
        self.oauth.as_ref().map(|o| o.account_url.as_str())
    }

    /// The account signed in with this `Cookie` header, if any.
    pub fn session_login(&self, cookie_header: Option<&str>) -> Option<String> {
        let token = read_cookie(cookie_header?, SESSION_COOKIE)?;
        let mut sessions = self.sessions.lock().unwrap();
        // Swept lazily on lookup rather than on a timer: this process has no background tasks
        // today, and a login list short enough to hand-maintain never grows sessions enough for
        // that to matter.
        sessions.retain(|_, s| s.expires_at > Instant::now());
        sessions.get(&token).map(|s| s.login.clone())
    }
}

impl AuthState {
    /// Signs this `Cookie` header's session out, here and at the provider. A session that is
    /// already gone, or never existed, is not an error: the caller ends up signed out either
    /// way, which is all it asked for.
    pub async fn end_session(&self, cookie_header: Option<&str>) {
        let Some(token) = cookie_header.and_then(|c| read_cookie(c, SESSION_COOKIE)) else {
            return;
        };
        let Some(session) = self.sessions.lock().unwrap().remove(&token) else {
            return;
        };
        let (Some(oauth), Some(refresh_token)) = (&self.oauth, session.refresh_token) else {
            return;
        };

        let mut form = vec![
            ("client_id", oauth.client_id.as_str()),
            ("refresh_token", refresh_token.as_str()),
        ];
        if let Some(secret) = &oauth.client_secret {
            form.push(("client_secret", secret));
        }
        let mut req = self.http.post(&oauth.logout_url).form(&form);
        if let Some(forwarded) = &session.forwarded {
            req = forwarded.apply(req);
        }
        // Our session is already gone, so failing here only leaves the provider's alive: the
        // next sign-in skips the password prompt. Worth a line in the log, not an error page.
        if let Err(e) = req.send().await.and_then(|r| r.error_for_status()) {
            eprintln!(
                "warning: could not end the provider session for {}: {e}",
                session.login
            );
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/logout", get(logout))
        .route("/auth/me", get(me))
        .route("/idp/{*rest}", any(crate::idp::forward))
}

/// Guards the routes that touch the jail (`/rpc`, `/upload`, `/files`). Without OAuth configured
/// every request passes, exactly as before login existed; with it, a request needs a live
/// session cookie or gets `401`. `SameSite=Lax` on that cookie is also what keeps another site
/// from making a signed-in browser `POST` to `/rpc` on its behalf.
///
/// It is also where the caller becomes a [`User`](crate::access::User): the signed-in login
/// goes into the request's extensions, and every permission check downstream reads it from
/// there. A `User` already present (see `crate::as_user`) is left alone.
pub async fn require_session(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok());
    let login = state.auth.session_login(cookie);
    if state.auth.oauth.is_some() && login.is_none() {
        return (StatusCode::UNAUTHORIZED, "sign in first: /auth/login").into_response();
    }
    if let Some(login) = login
        && req.extensions().get::<crate::access::User>().is_none()
    {
        req.extensions_mut().insert(crate::access::User(login));
    }
    next.run(req).await
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

    let redirect_uri = format!("{}/auth/callback", state.auth.origin(&headers));
    let url = format!(
        "{}?client_id={}&redirect_uri={}&scope=openid&response_type=code&state={}",
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

    let redirect_uri = format!("{}/auth/callback", state.auth.origin(&headers));
    let forwarded = state.auth.backchannel_headers(&headers);
    let (login, refresh_token) = match exchange_and_fetch_login(
        &state.auth.http,
        oauth,
        &code,
        &redirect_uri,
        forwarded.as_ref(),
    )
    .await
    {
        Ok(found) => found,
        Err(msg) => return (StatusCode::BAD_GATEWAY, msg).into_response(),
    };

    let token = random_token(48);
    state.auth.sessions.lock().unwrap().insert(
        token.clone(),
        Session {
            login,
            refresh_token,
            forwarded,
            expires_at: Instant::now() + SESSION_TTL,
        },
    );

    let mut resp = Redirect::to("/").into_response();
    resp.headers_mut().append(
        header::SET_COOKIE,
        format!(
            "{SESSION_COOKIE}={token}; {}; Max-Age={}",
            state.auth.cookie_attrs(),
            SESSION_TTL.as_secs()
        )
        .parse()
        .unwrap(),
    );
    resp
}

/// The two calls a provider's OAuth flow needs after the redirect: trade the one-time `code`
/// for an access token, then ask who it belongs to. Returns the login and, if the provider
/// issued one, the refresh token that [`AuthState::end_session`] later signs out with. Split
/// out from [`callback`] so the network half is one function with one error path, independent
/// of routing and session bookkeeping.
async fn exchange_and_fetch_login(
    http: &reqwest::Client,
    oauth: &OAuthConfig,
    code: &str,
    redirect_uri: &str,
    forwarded: Option<&Forwarded>,
) -> Result<(String, Option<String>), String> {
    let with_forwarded = |req: reqwest::RequestBuilder| match forwarded {
        Some(f) => f.apply(req),
        None => req,
    };
    #[derive(Deserialize)]
    struct TokenResp {
        access_token: Option<String>,
        refresh_token: Option<String>,
        error_description: Option<String>,
        error: Option<String>,
    }

    let mut form = vec![
        ("client_id", oauth.client_id.as_str()),
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
    ];
    if let Some(secret) = &oauth.client_secret {
        form.push(("client_secret", secret));
    }

    let token_resp: TokenResp = with_forwarded(http.post(&oauth.token_url))
        .header(header::ACCEPT, "application/json")
        .form(&form)
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
        preferred_username: Option<String>,
    }

    let user_resp = with_forwarded(http.get(&oauth.user_url))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::USER_AGENT, "ccosel-server")
        .send()
        .await
        .map_err(|e| format!("could not fetch the account from the OAuth provider: {e}"))?;
    // A refusal comes back with an empty body and the reason in `WWW-Authenticate`; say that
    // rather than failing to parse nothing as JSON.
    if !user_resp.status().is_success() {
        let reason = user_resp
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        return Err(format!(
            "the OAuth provider refused to say who signed in ({}) {reason}",
            user_resp.status()
        ));
    }
    let user: UserResp = user_resp
        .json()
        .await
        .map_err(|e| format!("the OAuth provider's account response was not understood: {e}"))?;

    let login = user
        .preferred_username
        .ok_or_else(|| "the OAuth provider did not return an account name".to_string())?;
    Ok((login, token_resp.refresh_token))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok());
    state.auth.end_session(cookie).await;
    let mut resp = Redirect::to("/").into_response();
    // A negative Max-Age tells the browser to drop the cookie now, regardless of what it was.
    resp.headers_mut().append(
        header::SET_COOKIE,
        format!(
            "{SESSION_COOKIE}=; {}; Max-Age=0",
            state.auth.cookie_attrs()
        )
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
