//! The provider itself is not exercised here — these run in CI with no network access — but
//! everything this module controls is: the CSRF `state` token, session
//! cookies, and the two calls a provider's callback triggers, against a small mock HTTP server
//! standing in for Keycloak. Same approach `rpc_http.rs` and `compile_http.rs` take: a real
//! socket, not a mocked-out `Router`.

use std::sync::atomic::{AtomicU32, Ordering};

use axum::Router as AxRouter;
use axum::extract::State as AxState;
use axum::routing::{get as ax_get, post as ax_post};
use ccosel_proto::account::Account;
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
    /// The refresh tokens `/logout` has been called with, in order.
    logouts: Arc<Mutex<Vec<String>>>,
}

async fn mock_token(AxState(p): AxState<MockProvider>) -> axum::Json<Value> {
    if p.fail_token {
        axum::Json(json!({ "error": "bad_verification_code" }))
    } else {
        axum::Json(json!({
            "access_token": "test-access-token",
            "refresh_token": "test-refresh-token",
        }))
    }
}

async fn mock_user(AxState(p): AxState<MockProvider>) -> axum::Json<Value> {
    axum::Json(json!({ "preferred_username": p.login }))
}

async fn mock_logout(
    AxState(p): AxState<MockProvider>,
    axum::Form(form): axum::Form<HashMap<String, String>>,
) {
    p.logouts
        .lock()
        .unwrap()
        .push(form.get("refresh_token").cloned().unwrap_or_default());
}

/// A stand-in for Keycloak's OAuth endpoints. Returns the base URL to point `token_url` /
/// `user_url` / `logout_url` at (`{base}/token`, `{base}/user`, `{base}/logout`).
async fn spawn_mock_provider(login: &str, fail_token: bool) -> String {
    spawn_recording_provider(login, fail_token).await.0
}

/// [`spawn_mock_provider`], also handing back the provider's record of `/logout` calls.
async fn spawn_recording_provider(
    login: &str,
    fail_token: bool,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let logouts = Arc::new(Mutex::new(Vec::new()));
    let state = MockProvider {
        login: login.to_string(),
        fail_token,
        logouts: logouts.clone(),
    };
    let router = AxRouter::new()
        .route("/token", ax_post(mock_token))
        .route("/user", ax_get(mock_user))
        .route("/logout", ax_post(mock_logout))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{addr}"), logouts)
}

fn oauth_config_for(provider_base: &str) -> OAuthConfig {
    OAuthConfig {
        client_id: "test-client-id".to_string(),
        client_secret: Some("test-client-secret".to_string()),
        authorize_url: format!("{provider_base}/authorize"),
        token_url: format!("{provider_base}/token"),
        user_url: format!("{provider_base}/user"),
        logout_url: format!("{provider_base}/logout"),
        account_url: format!("{provider_base}/account"),
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
    let addr = spawn_app(AuthState::new(Some(oauth))).await;

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
    let addr = spawn_app(AuthState::new(Some(oauth))).await;

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
    let addr = spawn_app(AuthState::new(Some(oauth))).await;

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
    let addr = spawn_app(AuthState::new(Some(oauth))).await;

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

    // The session opens the jail-backed routes that were closed before signing in.
    for status in protected_statuses(&client, addr, Some(&cookie_pair)).await {
        assert_ne!(status, reqwest::StatusCode::UNAUTHORIZED);
    }

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
    for status in protected_statuses(&client, addr, Some(&cookie_pair)).await {
        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
    }
}

/// Signs in through `/auth/login?return=<to>` and reports where the callback sends the browser.
async fn signed_in_lands_on(to: &str) -> String {
    let provider = spawn_mock_provider("alice", false).await;
    let addr = spawn_app(AuthState::new(Some(oauth_config_for(&provider)))).await;
    let client = no_redirect_client();
    let resp = client
        .get(format!("http://{addr}/auth/login"))
        .query(&[("return", to)])
        .send()
        .await
        .unwrap();
    let location = resp.headers()[reqwest::header::LOCATION].to_str().unwrap();
    let state = query_param(location, "state").unwrap().to_string();
    let callback = client
        .get(format!(
            "http://{addr}/auth/callback?code=good-code&state={state}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(callback.status(), reqwest::StatusCode::SEE_OTHER);
    callback.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned()
}

/// A shared link to one app's own page survives having to sign in first.
#[tokio::test]
async fn signing_in_comes_back_to_the_page_it_started_from() {
    assert_eq!(
        signed_in_lands_on("/app/viewer?open=%2FDocs%2Fa.png").await,
        "/app/viewer?open=%2FDocs%2Fa.png"
    );
}

/// Sign-in must not become a way to send someone to another site.
#[tokio::test]
async fn signing_in_never_returns_to_another_site() {
    for to in [
        "https://evil.example/",
        "//evil.example/",
        "/\\evil.example/",
        "evil.example",
        "/ok\nSet-Cookie: x=y",
    ] {
        assert_eq!(signed_in_lands_on(to).await, "/", "{to:?}");
    }
}

#[tokio::test]
async fn callback_reports_a_provider_side_token_failure() {
    let provider = spawn_mock_provider("alice", true).await;
    let oauth = oauth_config_for(&provider);
    let addr = spawn_app(AuthState::new(Some(oauth))).await;

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

// ---- require_session ----

/// The status of one request to each jail-backed route, sent with `cookie` if given. The bodies
/// are junk on purpose: only whether the auth gate let the request through matters here.
async fn protected_statuses(
    client: &reqwest::Client,
    addr: std::net::SocketAddr,
    cookie: Option<&str>,
) -> Vec<reqwest::StatusCode> {
    let requests = [
        client.post(format!("http://{addr}/rpc")).body("junk"),
        client
            .post(format!("http://{addr}/upload?path=/&filename=a.txt"))
            .body("hi"),
        client.get(format!("http://{addr}/files/missing.txt")),
    ];
    let mut statuses = Vec::new();
    for req in requests {
        let req = match cookie {
            Some(c) => req.header(reqwest::header::COOKIE, c),
            None => req,
        };
        statuses.push(req.send().await.unwrap().status());
    }
    statuses
}

#[tokio::test]
async fn jail_routes_are_open_when_oauth_is_not_configured() {
    let addr = spawn_app(AuthState::default()).await;
    for status in protected_statuses(&no_redirect_client(), addr, None).await {
        assert_ne!(status, reqwest::StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn jail_routes_need_a_session_when_oauth_is_configured() {
    let provider = spawn_mock_provider("alice", false).await;
    let addr = spawn_app(AuthState::new(Some(oauth_config_for(&provider)))).await;
    let client = no_redirect_client();

    for status in protected_statuses(&client, addr, None).await {
        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
    }
    for status in protected_statuses(&client, addr, Some("ccosel_session=forged")).await {
        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
    }

    // The public side stays reachable, or the boot page could never offer the sign-in button.
    let me = client
        .get(format!("http://{addr}/auth/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), reqwest::StatusCode::OK);
}

// ---- WhoAmI and SignOut over /rpc ----

/// Signs `login` in through the real callback and returns the `name=value` cookie pair.
async fn signed_in_cookie(client: &reqwest::Client, addr: std::net::SocketAddr) -> String {
    let state = start_login(client, addr).await;
    let callback = client
        .get(format!(
            "http://{addr}/auth/callback?code=good-code&state={state}"
        ))
        .send()
        .await
        .unwrap();
    let set_cookie = callback.headers().get(reqwest::header::SET_COOKIE).unwrap();
    set_cookie
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

/// Sends one argument-less call to `/rpc` and returns the HTTP status and, on success, the
/// decoded reply.
async fn rpc<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    addr: std::net::SocketAddr,
    cookie: Option<&str>,
    method: ccosel_proto::Method,
) -> (reqwest::StatusCode, Option<T>) {
    let args = postcard::to_allocvec(&()).unwrap();
    let batch = vec![ccosel_proto::WireRequest {
        seq: 1,
        method: method as u16,
        args: &args,
    }];
    let mut req = client
        .post(format!("http://{addr}/rpc"))
        .body(postcard::to_allocvec(&batch).unwrap());
    if let Some(c) = cookie {
        req = req.header(reqwest::header::COOKIE, c);
    }
    let resp = req.send().await.unwrap();
    let status = resp.status();
    if !status.is_success() {
        return (status, None);
    }
    let bytes = resp.bytes().await.unwrap();
    let replies: Vec<ccosel_proto::WireReply> = postcard::from_bytes(&bytes).unwrap();
    match &replies[0].result {
        ccosel_proto::WireResult::Ok(payload) => {
            (status, Some(postcard::from_bytes(payload).unwrap()))
        }
        ccosel_proto::WireResult::Err { code, .. } => panic!("rpc failed with {code}"),
    }
}

#[tokio::test]
async fn who_am_i_reports_login_off_when_oauth_is_not_configured() {
    let addr = spawn_app(AuthState::default()).await;
    let (status, account) = rpc::<Account>(
        &no_redirect_client(),
        addr,
        None,
        ccosel_proto::Method::WhoAmI,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(
        account,
        Some(Account {
            login_enabled: false,
            name: None,
            account_url: None,
        })
    );
}

#[tokio::test]
async fn sign_out_over_rpc_ends_the_session_here_and_at_the_provider() {
    let (provider, logouts) = spawn_recording_provider("alice", false).await;
    let addr = spawn_app(AuthState::new(Some(oauth_config_for(&provider)))).await;
    let client = no_redirect_client();
    let cookie = signed_in_cookie(&client, addr).await;

    let (_, account) =
        rpc::<Account>(&client, addr, Some(&cookie), ccosel_proto::Method::WhoAmI).await;
    assert_eq!(
        account,
        Some(Account {
            login_enabled: true,
            name: Some("alice".to_string()),
            account_url: Some(format!("{provider}/account")),
        })
    );

    let (status, _) = rpc::<()>(&client, addr, Some(&cookie), ccosel_proto::Method::SignOut).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(*logouts.lock().unwrap(), ["test-refresh-token"]);

    // The cookie no longer opens /rpc, which is what sends the shell back to sign-in.
    let (status, _) =
        rpc::<Account>(&client, addr, Some(&cookie), ccosel_proto::Method::WhoAmI).await;
    assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_route_also_ends_the_provider_session() {
    let (provider, logouts) = spawn_recording_provider("alice", false).await;
    let addr = spawn_app(AuthState::new(Some(oauth_config_for(&provider)))).await;
    let client = no_redirect_client();
    let cookie = signed_in_cookie(&client, addr).await;

    client
        .get(format!("http://{addr}/auth/logout"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(*logouts.lock().unwrap(), ["test-refresh-token"]);

    // Logging out again, with the dead cookie, has nothing left to end.
    client
        .get(format!("http://{addr}/auth/logout"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(logouts.lock().unwrap().len(), 1);
}

// ---- Behind a tunnel, and with Keycloak behind /idp ----

#[tokio::test]
async fn behind_https_the_callback_is_the_public_url_and_the_cookie_is_secure() {
    let provider = spawn_mock_provider("alice", false).await;
    let addr = spawn_app(
        AuthState::new(Some(oauth_config_for(&provider)))
            .with_public_url(Some("https://ccosel.example.com/".to_string())),
    )
    .await;
    let client = no_redirect_client();

    let login = client
        .get(format!("http://{addr}/auth/login"))
        .send()
        .await
        .unwrap();
    let location = login.headers()[reqwest::header::LOCATION].to_str().unwrap();
    assert_eq!(
        query_param(location, "redirect_uri"),
        Some("https%3A%2F%2Fccosel.example.com%2Fauth%2Fcallback")
    );

    let state = query_param(location, "state").unwrap().to_string();
    let callback = client
        .get(format!(
            "http://{addr}/auth/callback?code=good-code&state={state}"
        ))
        .send()
        .await
        .unwrap();
    let set_cookie = callback.headers()[reqwest::header::SET_COOKIE]
        .to_str()
        .unwrap();
    assert!(set_cookie.contains("; Secure"), "{set_cookie}");
}

#[tokio::test]
async fn over_plain_http_the_cookie_is_not_secure() {
    let provider = spawn_mock_provider("alice", false).await;
    let addr = spawn_app(AuthState::new(Some(oauth_config_for(&provider)))).await;
    let client = no_redirect_client();
    let state = start_login(&client, addr).await;
    let callback = client
        .get(format!(
            "http://{addr}/auth/callback?code=good-code&state={state}"
        ))
        .send()
        .await
        .unwrap();
    let set_cookie = callback.headers()[reqwest::header::SET_COOKIE]
        .to_str()
        .unwrap();
    assert!(!set_cookie.contains("Secure"), "{set_cookie}");
}

/// A stricter stand-in for Keycloak: like the real one, it stamps tokens with the address the
/// browser used and refuses them (`401`, empty body, reason in `WWW-Authenticate`) when this
/// server asks about them from any other address. Records the `X-Forwarded-Host` of every
/// call, keyed by endpoint.
#[derive(Clone, Default)]
struct StrictProvider {
    calls: Arc<Mutex<Vec<Call>>>,
}

/// `(endpoint, X-Forwarded-Host)` of one call to [`StrictProvider`].
type Call = (&'static str, Option<String>);

impl StrictProvider {
    fn record(&self, endpoint: &'static str, headers: &HeaderMap) -> Option<String> {
        let host = headers
            .get("x-forwarded-host")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        self.calls.lock().unwrap().push((endpoint, host.clone()));
        host
    }
}

const BROWSER_HOST: &str = "ccosel.lan:8777";

async fn strict_token(
    AxState(p): AxState<StrictProvider>,
    headers: HeaderMap,
) -> axum::Json<Value> {
    p.record("token", &headers);
    axum::Json(json!({ "access_token": "at", "refresh_token": "rt" }))
}

async fn strict_user(AxState(p): AxState<StrictProvider>, headers: HeaderMap) -> Response {
    if p.record("user", &headers).as_deref() != Some(BROWSER_HOST) {
        return (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                r#"Bearer error="invalid_token", error_description="Token verification failed""#,
            )],
        )
            .into_response();
    }
    axum::Json(json!({ "preferred_username": "alice" })).into_response()
}

async fn strict_logout(AxState(p): AxState<StrictProvider>, headers: HeaderMap) {
    p.record("logout", &headers);
}

async fn spawn_strict_provider() -> (String, StrictProvider) {
    let provider = StrictProvider::default();
    let router = AxRouter::new()
        .route("/token", ax_post(strict_token))
        .route("/user", ax_get(strict_user))
        .route("/logout", ax_post(strict_logout))
        .with_state(provider.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{addr}"), provider)
}

/// Runs `/auth/login` then `/auth/callback` as a browser that reached the server at
/// [`BROWSER_HOST`], and returns the callback's response.
async fn sign_in_as_browser_at_host(addr: std::net::SocketAddr) -> reqwest::Response {
    let client = no_redirect_client();
    let login = client
        .get(format!("http://{addr}/auth/login"))
        .header(reqwest::header::HOST, BROWSER_HOST)
        .send()
        .await
        .unwrap();
    let location = login.headers()[reqwest::header::LOCATION].to_str().unwrap();
    let state = query_param(location, "state").unwrap().to_string();
    client
        .get(format!(
            "http://{addr}/auth/callback?code=good-code&state={state}"
        ))
        .header(reqwest::header::HOST, BROWSER_HOST)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_proxied_keycloak_is_asked_about_tokens_from_the_browsers_address() {
    let (provider, seen) = spawn_strict_provider().await;
    let addr = spawn_app(
        AuthState::new(Some(oauth_config_for(&provider)))
            .with_idp_proxy(crate::idp::IdpProxy::new(provider.clone(), "ccosel".into())),
    )
    .await;

    let callback = sign_in_as_browser_at_host(addr).await;
    assert_eq!(callback.status(), reqwest::StatusCode::SEE_OTHER);
    let cookie = callback.headers()[reqwest::header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // Signing out later, from anywhere, still speaks for the address the session began at.
    no_redirect_client()
        .get(format!("http://{addr}/auth/logout"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();

    let host = Some(BROWSER_HOST.to_string());
    assert_eq!(
        *seen.calls.lock().unwrap(),
        [
            ("token", host.clone()),
            ("user", host.clone()),
            ("logout", host)
        ]
    );
}

#[tokio::test]
async fn a_refused_lookup_says_why_instead_of_failing_to_parse() {
    let (provider, seen) = spawn_strict_provider().await;
    // Not proxied: no forwarded headers go out, so the strict provider refuses the token, the
    // way Keycloak refused them before this server sent them.
    let addr = spawn_app(AuthState::new(Some(oauth_config_for(&provider)))).await;

    let callback = sign_in_as_browser_at_host(addr).await;
    assert_eq!(callback.status(), reqwest::StatusCode::BAD_GATEWAY);
    let body = callback.text().await.unwrap();
    assert!(body.contains("refused"), "{body}");
    assert!(body.contains("Token verification failed"), "{body}");
    // A Keycloak of the user's own is reached at one address by everyone: nothing to claim.
    assert!(seen.calls.lock().unwrap().iter().all(|(_, h)| h.is_none()));
}

#[test]
fn a_proxied_keycloak_sends_browsers_to_same_origin_paths() {
    let config = OAuthConfig::proxied_keycloak(
        "http://127.0.0.1:8080/idp/realms/ccosel".to_string(),
        "ccosel".to_string(),
    );
    assert_eq!(
        config.authorize_url,
        "/idp/realms/ccosel/protocol/openid-connect/auth"
    );
    assert_eq!(config.account_url, "/idp/realms/ccosel/account");
    // What this server calls itself stays on the loopback address.
    assert_eq!(
        config.token_url,
        "http://127.0.0.1:8080/idp/realms/ccosel/protocol/openid-connect/token"
    );
    assert_eq!(config.client_secret, None);
}

#[tokio::test]
async fn a_signed_in_session_is_the_user_permissions_are_checked_for() {
    let provider = spawn_mock_provider("alice", false).await;
    let addr = spawn_app(AuthState::new(Some(oauth_config_for(&provider)))).await;
    let client = no_redirect_client();
    let cookie = signed_in_cookie(&client, addr).await;

    let args = postcard::to_allocvec(&ccosel_proto::fs::PathReq {
        path: "/home/alice",
    })
    .unwrap();
    let batch = vec![ccosel_proto::WireRequest {
        seq: 1,
        method: ccosel_proto::Method::Access as u16,
        args: &args,
    }];
    let bytes = client
        .post(format!("http://{addr}/rpc"))
        .header(reqwest::header::COOKIE, &cookie)
        .body(postcard::to_allocvec(&batch).unwrap())
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let replies: Vec<ccosel_proto::WireReply> = postcard::from_bytes(&bytes).unwrap();
    let ccosel_proto::WireResult::Ok(payload) = replies[0].result else {
        panic!("Access failed");
    };
    let access: ccosel_proto::fs::AccessReply = postcard::from_bytes(payload).unwrap();
    assert_eq!(access.user.as_deref(), Some("alice"));
    assert!(access.read && access.write, "alice's own folder is hers");

    // And /files sees the same user: alice's folder was created for her on that call, and a
    // file in it downloads with her cookie.
    let files = client
        .get(format!("http://{addr}/files/home/alice"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    // A folder is not a file, but it is *hers*: not found, rather than forbidden.
    assert_eq!(files.status(), reqwest::StatusCode::NOT_FOUND);
}

/// Sends one call to `/rpc` with `cookie`, returning the decoded reply or the error code.
async fn rpc_call<M: ccosel_proto::Rpc>(
    client: &reqwest::Client,
    addr: std::net::SocketAddr,
    cookie: &str,
    req: &M::Req<'_>,
) -> Result<M::Reply, u32> {
    let args = postcard::to_allocvec(req).unwrap();
    let batch = vec![ccosel_proto::WireRequest {
        seq: 1,
        method: M::METHOD as u16,
        args: &args,
    }];
    let bytes = client
        .post(format!("http://{addr}/rpc"))
        .header(reqwest::header::COOKIE, cookie)
        .body(postcard::to_allocvec(&batch).unwrap())
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let replies: Vec<ccosel_proto::WireReply> = postcard::from_bytes(&bytes).unwrap();
    match replies[0].result {
        ccosel_proto::WireResult::Ok(payload) => Ok(postcard::from_bytes(payload).unwrap()),
        ccosel_proto::WireResult::Err { code, .. } => Err(code),
    }
}

#[tokio::test]
async fn webdav_takes_an_app_password_made_in_a_session_but_never_the_session_itself() {
    use ccosel_proto::account::{
        CreateAppPassword, CreateAppPasswordReq, ListAppPasswords, RevokeAppPassword,
        RevokeAppPasswordReq,
    };

    let provider = spawn_mock_provider("alice", false).await;
    let addr = spawn_app(AuthState::new(Some(oauth_config_for(&provider)))).await;
    let client = no_redirect_client();
    let cookie = signed_in_cookie(&client, addr).await;
    let propfind = || {
        client
            .request(
                reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
                format!("http://{addr}/dav/home/"),
            )
            .header("depth", "1")
    };

    // A live session is not a way in: another site could make a signed-in browser send it.
    let with_cookie = propfind()
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(with_cookie.status(), reqwest::StatusCode::UNAUTHORIZED);

    let made = rpc_call::<CreateAppPassword>(
        &client,
        addr,
        &cookie,
        &CreateAppPasswordReq { name: "Laptop" },
    )
    .await
    .unwrap();
    let listed = rpc_call::<ListAppPasswords>(&client, addr, &cookie, &())
        .await
        .unwrap();
    assert_eq!(listed, std::slice::from_ref(&made.info));
    assert_eq!(
        rpc_call::<CreateAppPassword>(&client, addr, &cookie, &CreateAppPasswordReq { name: "" })
            .await,
        Err(ccosel_proto::server_error::MALFORMED)
    );

    // The app password is alice, exactly as her session is: her home folder and nobody else's.
    let listing = propfind()
        .basic_auth("alice", Some(&made.password))
        .send()
        .await
        .unwrap();
    assert_eq!(listing.status(), reqwest::StatusCode::MULTI_STATUS);
    let body = listing.text().await.unwrap();
    assert!(body.contains("/dav/home/alice/"), "{body}");

    rpc_call::<RevokeAppPassword>(
        &client,
        addr,
        &cookie,
        &RevokeAppPasswordReq { id: &made.info.id },
    )
    .await
    .unwrap();
    let revoked = propfind()
        .basic_auth("alice", Some(&made.password))
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(
        rpc_call::<RevokeAppPassword>(
            &client,
            addr,
            &cookie,
            &RevokeAppPasswordReq { id: &made.info.id },
        )
        .await,
        Err(ccosel_proto::server_error::NOT_FOUND)
    );
}

#[tokio::test]
async fn anonymous_callers_have_no_app_passwords_to_manage() {
    use ccosel_proto::account::{
        CreateAppPassword, CreateAppPasswordReq, ListAppPasswords, RevokeAppPassword,
        RevokeAppPasswordReq,
    };

    // Login off: every caller is anonymous.
    let addr = spawn_app(AuthState::default()).await;
    let client = reqwest::Client::new();
    let denied = Err(ccosel_proto::server_error::DENIED);
    assert_eq!(
        rpc_call::<ListAppPasswords>(&client, addr, "", &())
            .await
            .map(|_| ()),
        denied
    );
    assert_eq!(
        rpc_call::<CreateAppPassword>(&client, addr, "", &CreateAppPasswordReq { name: "x" })
            .await
            .map(|_| ()),
        denied
    );
    assert_eq!(
        rpc_call::<RevokeAppPassword>(&client, addr, "", &RevokeAppPasswordReq { id: "x" }).await,
        denied
    );
}
