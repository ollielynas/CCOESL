//! The forwarding runs over real sockets against a mock Keycloak that records what reached
//! it, so the tests can check both what is passed through and what never arrives.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Request, State as AxState};
use axum::http::HeaderMap;
use axum::response::IntoResponse;

use super::*;
use crate::auth::AuthState;
use crate::fs_api::Jail;

fn proxy() -> IdpProxy {
    IdpProxy::new("http://127.0.0.1:1".to_owned(), "ccosel".to_owned())
}

#[test]
fn allows_only_the_realm_pages_and_assets() {
    let p = proxy();
    assert!(p.allows("realms/ccosel"));
    assert!(p.allows("realms/ccosel/protocol/openid-connect/auth"));
    assert!(p.allows("realms/ccosel/login-actions/authenticate"));
    assert!(p.allows("realms/ccosel/account"));
    assert!(p.allows("resources/abc12/login/keycloak.v2/css/styles.css"));

    assert!(!p.allows("admin/"));
    assert!(!p.allows("admin/master/console/"));
    assert!(!p.allows("realms/master/protocol/openid-connect/token"));
    assert!(!p.allows("realms/ccosel-other/account"));
    assert!(!p.allows("health"));
    assert!(!p.allows(""));
}

#[test]
fn refuses_paths_that_climb_out_of_the_realm() {
    let p = proxy();
    for path in [
        "realms/ccosel/../../admin/master/console/",
        "realms/ccosel/%2e%2e/%2e%2e/admin/",
        "realms/ccosel/%2E%2E/%2E%2E/admin/",
        "realms/ccosel/.%2e/.%2e/admin/",
        "resources/../admin/",
        "realms/ccosel/./x",
        "realms/ccosel%2f..%2f..%2fadmin",
        "realms/ccosel/%5c..%5cadmin",
    ] {
        assert!(!p.allows(path), "{path} was allowed");
    }
}

#[test]
fn percent_decode_leaves_malformed_escapes_alone() {
    assert_eq!(percent_decode("%2e%2E"), "..");
    assert_eq!(percent_decode("100%"), "100%");
    assert_eq!(percent_decode("%zz"), "%zz");
}

#[test]
fn drops_only_the_session_cookie() {
    assert_eq!(
        without_session_cookie("a=1; ccosel_session=secret; KEYCLOAK_IDENTITY=x"),
        Some("a=1; KEYCLOAK_IDENTITY=x".to_owned())
    );
    assert_eq!(without_session_cookie("ccosel_session=secret"), None);
}

/// One request as the mock Keycloak saw it.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path_and_query: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

async fn record(AxState(seen): AxState<Arc<Mutex<Vec<Seen>>>>, req: Request) -> impl IntoResponse {
    let method = req.method().to_string();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.to_string())
        .unwrap_or_default();
    let headers = req.headers().clone();
    let body = axum::body::to_bytes(req.into_body(), 1 << 20)
        .await
        .unwrap()
        .to_vec();
    seen.lock().unwrap().push(Seen {
        method,
        path_and_query,
        headers,
        body,
    });
    // Built with `append`: an array of header pairs would `insert`, keeping only the last
    // `Set-Cookie`, and Keycloak really does send several.
    let mut headers = HeaderMap::new();
    headers.append(
        header::LOCATION,
        HeaderValue::from_static("http://localhost:8777/auth/callback?code=c"),
    );
    for cookie in [
        "KC_A=1; Path=/idp/realms/ccosel/",
        "KC_B=2; Path=/idp/realms/ccosel/",
    ] {
        headers.append(header::SET_COOKIE, HeaderValue::from_static(cookie));
    }
    (StatusCode::FOUND, headers, "moved")
}

async fn spawn_mock_keycloak() -> (String, Arc<Mutex<Vec<Seen>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let router = Router::new().fallback(record).with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{addr}"), seen)
}

async fn spawn_app(auth: AuthState) -> std::net::SocketAddr {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-idp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let app = crate::app(Jail::new(&dir).unwrap(), dir, auth);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

#[tokio::test]
async fn forwards_a_sign_in_post_and_passes_the_reply_back_untouched() {
    let (upstream, seen) = spawn_mock_keycloak().await;
    let addr =
        spawn_app(AuthState::default().with_idp_proxy(IdpProxy::new(upstream, "ccosel".into())))
            .await;

    let resp = client()
        .post(format!(
            "http://{addr}/idp/realms/ccosel/login-actions/authenticate?session_code=s&tab_id=t"
        ))
        .header(header::HOST, "ccosel.lan:8777")
        .header(header::COOKIE, "ccosel_session=secret; AUTH_SESSION_ID=abc")
        .header("x-forwarded-host", "spoofed.example")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body("username=alice&password=pw")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::FOUND);
    assert_eq!(
        resp.headers()[header::LOCATION],
        "http://localhost:8777/auth/callback?code=c"
    );
    let cookies: Vec<_> = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    assert_eq!(cookies.len(), 2, "every Set-Cookie survives: {cookies:?}");
    assert_eq!(resp.text().await.unwrap(), "moved");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let req = &seen[0];
    assert_eq!(req.method, "POST");
    assert_eq!(
        req.path_and_query,
        "/idp/realms/ccosel/login-actions/authenticate?session_code=s&tab_id=t"
    );
    assert_eq!(req.body, b"username=alice&password=pw");
    assert_eq!(
        req.headers[header::CONTENT_TYPE],
        "application/x-www-form-urlencoded"
    );
    // Keycloak builds its links for the address the browser used, never a client's claim.
    assert_eq!(req.headers["x-forwarded-host"], "ccosel.lan:8777");
    assert_eq!(req.headers["x-forwarded-proto"], "http");
    // Our own session cookie is none of Keycloak's business; its own cookies pass through.
    assert_eq!(req.headers[header::COOKIE], "AUTH_SESSION_ID=abc");
}

#[tokio::test]
async fn describes_the_public_url_to_keycloak_behind_a_tunnel() {
    let (upstream, seen) = spawn_mock_keycloak().await;
    let addr = spawn_app(
        AuthState::default()
            .with_idp_proxy(IdpProxy::new(upstream, "ccosel".into()))
            .with_public_url(Some("https://ccosel.example.com/".into())),
    )
    .await;

    client()
        .get(format!("http://{addr}/idp/realms/ccosel/account"))
        .send()
        .await
        .unwrap();

    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].headers["x-forwarded-proto"], "https");
    assert_eq!(seen[0].headers["x-forwarded-host"], "ccosel.example.com");
}

#[tokio::test]
async fn never_forwards_the_admin_console_or_other_realms() {
    let (upstream, seen) = spawn_mock_keycloak().await;
    let addr =
        spawn_app(AuthState::default().with_idp_proxy(IdpProxy::new(upstream, "ccosel".into())))
            .await;

    for path in [
        "/idp/admin/master/console/",
        "/idp/admin/realms/ccosel/users",
        "/idp/realms/master/protocol/openid-connect/token",
        "/idp/realms/ccosel/%2e%2e/%2e%2e/admin/master/console/",
    ] {
        let resp = client()
            .get(format!("http://{addr}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND, "{path}");
    }
    assert!(
        seen.lock().unwrap().is_empty(),
        "Keycloak saw a blocked path"
    );
}

#[tokio::test]
async fn is_absent_when_keycloak_is_not_managed_here() {
    let addr = spawn_app(AuthState::default()).await;
    let resp = client()
        .get(format!("http://{addr}/idp/realms/ccosel/account"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reports_an_unreachable_keycloak() {
    // Port 1 on loopback: nothing listens there.
    let addr = spawn_app(AuthState::default().with_idp_proxy(proxy())).await;
    let resp = client()
        .get(format!("http://{addr}/idp/realms/ccosel/account"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
}
