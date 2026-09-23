//! The provider itself is not exercised here — these run in CI with no network access — but
//! everything this module controls is: the approved-list check, the CSRF `state` token, session
//! cookies, and the two calls a provider's callback triggers, against a small mock HTTP server
//! standing in for GitHub. Same approach `rpc_http.rs` and `compile_http.rs` take: a real
//! socket, not a mocked-out `Router`.

use std::sync::atomic::{AtomicU32, Ordering};

use axum::Router as AxRouter;
use axum::extract::State as AxState;
use axum::routing::{get as ax_get, post as ax_post};
use serde_json::{Value, json};

use super::*;
use crate::fs_api::Jail;

fn temp_dir(name: &str) -> std::path::PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-auth-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Starts our real router (the same `app()` the binary serves) on an ephemeral port.
async fn spawn_app(auth: AuthState) -> std::net::SocketAddr {
    let dir = temp_dir("app");
    let jail = Jail::new(&dir).unwrap();
    let app = crate::app(jail, dir, auth);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

#[derive(Clone)]
struct MockProvider {
    login: String,
    fail_token: bool,
}

async fn mock_token(AxState(p): AxState<MockProvider>) -> axum::Json<Value> {
    if p.fail_token {
        axum::Json(json!({ "error": "bad_verification_code" }))
    } else {
        axum::Json(json!({ "access_token": "test-access-token" }))
    }
}

async fn mock_user(AxState(p): AxState<MockProvider>) -> axum::Json<Value> {
    axum::Json(json!({ "login": p.login }))
}

/// A stand-in for GitHub's two OAuth endpoints. Returns the base URL to point `token_url` /
/// `user_url` at (`{base}/token`, `{base}/user`).
async fn spawn_mock_provider(login: &str, fail_token: bool) -> String {
    let state = MockProvider {
        login: login.to_string(),
        fail_token,
    };
    let router = AxRouter::new()
        .route("/token", ax_post(mock_token))
        .route("/user", ax_get(mock_user))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

fn oauth_config_for(provider_base: &str) -> OAuthConfig {
    OAuthConfig {
        client_id: "test-client-id".to_string(),
        client_secret: "test-client-secret".to_string(),
        authorize_url: format!("{provider_base}/authorize"),
        token_url: format!("{provider_base}/token"),
        user_url: format!("{provider_base}/user"),
    }
}

fn no_redirect_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn query_param<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    let (_, query) = url.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == name).then_some(v)
    })
}

// ---- ApprovedUsers ----

#[test]
fn approved_users_is_case_insensitive() {
    let approved = ApprovedUsers::new(["Octocat".to_string()]);
    assert!(approved.is_approved("octocat"));
    assert!(approved.is_approved("OCTOCAT"));
    assert!(!approved.is_approved("someone-else"));
}

#[test]
fn approved_users_load_skips_blank_lines_and_comments() {
    let dir = temp_dir("approved-users-file");
    let path = dir.join("approved_users.txt");
    std::fs::write(
        &path,
        "# approved accounts\noctocat\n\n  torvalds  \n# not-a-user\n",
    )
    .unwrap();

    let approved = ApprovedUsers::load(&path).unwrap();
    assert_eq!(approved.len(), 2);
    assert!(approved.is_approved("octocat"));
    assert!(approved.is_approved("torvalds"));
    assert!(!approved.is_approved("not-a-user"));
}

#[test]
fn approved_users_load_reports_a_missing_file() {
    let dir = temp_dir("approved-users-missing");
    let err = ApprovedUsers::load(&dir.join("does-not-exist.txt")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

// ---- /auth/me ----

#[tokio::test]
async fn me_reports_unconfigured_when_no_oauth_is_set_up() {
    let addr = spawn_app(AuthState::default()).await;
    let body: Value = reqwest::get(format!("http://{addr}/auth/me"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["configured"], false);
    assert_eq!(body["authenticated"], false);
    assert!(body["login"].is_null());
}

// ---- /auth/login ----

#[tokio::test]
async fn login_is_unavailable_when_oauth_is_not_configured() {
    let addr = spawn_app(AuthState::default()).await;
    let resp = reqwest::get(format!("http://{addr}/auth/login"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn login_redirects_to_the_provider_with_a_state_token() {
    let provider = spawn_mock_provider("alice", false).await;
    let oauth = oauth_config_for(&provider);
    let addr = spawn_app(AuthState::new(Some(oauth), ApprovedUsers::default())).await;

    let client = no_redirect_client();
    let resp = client
        .get(format!("http://{addr}/auth/login"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::SEE_OTHER);
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.starts_with(&format!("{provider}/authorize?")));
    assert_eq!(query_param(&location, "client_id"), Some("test-client-id"));
    assert!(query_param(&location, "state").is_some());
    assert!(query_param(&location, "redirect_uri").is_some());
}

// ---- /auth/callback ----

#[tokio::test]
async fn callback_rejects_an_unknown_or_expired_state() {
    let provider = spawn_mock_provider("alice", false).await;
    let oauth = oauth_config_for(&provider);
    let addr = spawn_app(AuthState::new(Some(oauth), ApprovedUsers::default())).await;

    let resp = reqwest::get(format!(
        "http://{addr}/auth/callback?code=whatever&state=never-issued"
    ))
    .await
    .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn callback_rejects_a_provider_declining_consent() {
    let provider = spawn_mock_provider("alice", false).await;
    let oauth = oauth_config_for(&provider);
    let addr = spawn_app(AuthState::new(Some(oauth), ApprovedUsers::default())).await;

    let resp = reqwest::get(format!(
        "http://{addr}/auth/callback?error=access_denied&state=whatever"
    ))
    .await
    .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
}

/// Runs `/auth/login` for real to get a live `state`, so the callback tests exercise the same
/// path a browser would rather than forging a state token directly.
async fn start_login(client: &reqwest::Client, addr: std::net::SocketAddr) -> String {
    let resp = client
        .get(format!("http://{addr}/auth/login"))
        .send()
        .await
        .unwrap();
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    query_param(&location, "state").unwrap().to_string()
}

#[tokio::test]
async fn full_login_flow_approves_and_sets_a_session_cookie() {
    let provider = spawn_mock_provider("alice", false).await;
    let oauth = oauth_config_for(&provider);
    let addr = spawn_app(AuthState::new(
        Some(oauth),
        ApprovedUsers::new(["alice".to_string()]),
    ))
    .await;

    let client = no_redirect_client();
    let state = start_login(&client, addr).await;

    let callback = client
        .get(format!(
            "http://{addr}/auth/callback?code=good-code&state={state}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(callback.status(), reqwest::StatusCode::SEE_OTHER);
    assert_eq!(
        callback.headers().get(reqwest::header::LOCATION).unwrap(),
        "/"
    );
    let set_cookie = callback
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(set_cookie.starts_with("ccosel_session="));
    assert!(set_cookie.contains("HttpOnly"));
    let cookie_pair = set_cookie.split(';').next().unwrap().to_string();

    // The state token is single-use: replaying the same callback must not work a second time.
    let replay = client
        .get(format!(
            "http://{addr}/auth/callback?code=good-code&state={state}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), reqwest::StatusCode::BAD_REQUEST);

    let me: Value = client
        .get(format!("http://{addr}/auth/me"))
        .header(reqwest::header::COOKIE, &cookie_pair)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["configured"], true);
    assert_eq!(me["authenticated"], true);
    assert_eq!(me["login"], "alice");

    // Logging out drops the session; the same cookie no longer authenticates.
    client
        .get(format!("http://{addr}/auth/logout"))
        .header(reqwest::header::COOKIE, &cookie_pair)
        .send()
        .await
        .unwrap();
    let me_after_logout: Value = client
        .get(format!("http://{addr}/auth/me"))
        .header(reqwest::header::COOKIE, &cookie_pair)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me_after_logout["authenticated"], false);
}

#[tokio::test]
async fn callback_refuses_a_login_not_on_the_approved_list() {
    let provider = spawn_mock_provider("mallory", false).await;
    let oauth = oauth_config_for(&provider);
    // Approved list has someone else on it — "mallory" authenticates with the provider fine,
    // but is not on it.
    let addr = spawn_app(AuthState::new(
        Some(oauth),
        ApprovedUsers::new(["alice".to_string()]),
    ))
    .await;

    let client = no_redirect_client();
    let state = start_login(&client, addr).await;

    let callback = client
        .get(format!(
            "http://{addr}/auth/callback?code=good-code&state={state}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(callback.status(), reqwest::StatusCode::FORBIDDEN);
    assert!(
        callback
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .is_none()
    );

    let me: Value = client
        .get(format!("http://{addr}/auth/me"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["authenticated"], false);
}

#[tokio::test]
async fn callback_reports_a_provider_side_token_failure() {
    let provider = spawn_mock_provider("alice", true).await;
    let oauth = oauth_config_for(&provider);
    let addr = spawn_app(AuthState::new(
        Some(oauth),
        ApprovedUsers::new(["alice".to_string()]),
    ))
    .await;

    let client = no_redirect_client();
    let state = start_login(&client, addr).await;

    let callback = client
        .get(format!(
            "http://{addr}/auth/callback?code=good-code&state={state}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(callback.status(), reqwest::StatusCode::BAD_GATEWAY);
}
