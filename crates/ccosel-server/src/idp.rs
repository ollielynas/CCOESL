//! `/idp/...`: Keycloak's sign-in pages, forwarded to a Keycloak that only listens on this
//! machine.
//!
//! This is what lets one tunnel (or one open port) carry both CCOSEL and its login: browsers
//! only ever talk to this server, and Keycloak itself is never reachable from outside. It is
//! an allow-list, not a pass-through. Only the configured realm's pages (`/idp/realms/<realm>/`)
//! and the static assets they load (`/idp/resources/`) are forwarded; the admin console
//! (`/idp/admin/`), the `master` realm and anything else answer `404` here without Keycloak
//! ever seeing the request. The admin console stays reachable on the machine itself, straight
//! from Keycloak's own port.
//!
//! Keycloak is started with `--http-relative-path=/idp`, so paths pass through unchanged, and
//! with `--proxy-headers=xforwarded`, so the `X-Forwarded-*` headers set here make it build
//! its links for the address the browser actually used. Only this machine can reach
//! Keycloak's port, so only this forwarding can set those headers.

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::auth::SESSION_COOKIE;

/// The path prefix Keycloak is mounted under, on both sides of the forwarding.
pub const PREFIX: &str = "/idp";

/// Largest request body forwarded. Sign-in forms are a few hundred bytes.
const MAX_BODY: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct IdpProxy {
    /// Keycloak's own origin, e.g. `http://127.0.0.1:8080`. Requests go to
    /// `{upstream}/idp/...`.
    upstream: String,
    /// The one realm whose pages are forwarded.
    realm: String,
    http: reqwest::Client,
}

impl IdpProxy {
    pub fn new(upstream: String, realm: String) -> Self {
        Self {
            upstream: upstream.trim_end_matches('/').to_owned(),
            realm,
            http: reqwest::Client::builder()
                // Keycloak's redirects are for the browser to follow, not us.
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("a client with no TLS configuration builds"),
        }
    }

    /// Whether `rest`, the raw path after `/idp/`, may be forwarded.
    pub fn allows(&self, rest: &str) -> bool {
        // A `..` segment, even percent-encoded, would be resolved by the URL parser or by
        // Keycloak itself, turning `realms/<realm>/../../admin` into the admin console. So is a
        // percent-encoded `/` or `\`. None of them appears in a real Keycloak page path.
        let suspicious = rest.split('/').any(|seg| {
            let decoded = percent_decode(seg).to_ascii_lowercase();
            decoded == "." || decoded == ".." || decoded.contains(['/', '\\'])
        });
        if suspicious {
            return false;
        }
        if rest.starts_with("resources/") {
            return true;
        }
        rest.strip_prefix("realms/")
            .and_then(|r| r.strip_prefix(self.realm.as_str()))
            .is_some_and(|r| r.is_empty() || r.starts_with('/'))
    }
}

/// Decodes `%XX` escapes, leaving anything malformed as it was. Only used to spot disguised
/// dot segments and slashes, so it never needs to be exact about invalid UTF-8.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Headers that describe one hop of a connection rather than the request itself, so they are
/// never copied across the proxy (RFC 9110 §7.6.1), plus the ones this module sets itself.
fn is_hop_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
            | "content-length"
            | "forwarded"
            | "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
            | "x-forwarded-port"
            | "x-forwarded-prefix"
    )
}

/// The `Cookie` header minus this server's own session cookie: Keycloak has no use for it, so
/// it does not get to see it.
fn without_session_cookie(cookie: &str) -> Option<String> {
    let kept: Vec<&str> = cookie
        .split(';')
        .map(str::trim)
        .filter(|part| {
            part.split_once('=')
                .is_none_or(|(name, _)| name != SESSION_COOKIE)
        })
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

pub async fn forward(State(state): State<AppState>, req: Request) -> Response {
    let Some(idp) = &state.auth.idp else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // The raw path, not axum's decoded one, so `allows` sees any disguised segments as sent.
    let Some(rest) = req
        .uri()
        .path()
        .strip_prefix(PREFIX)
        .and_then(|p| p.strip_prefix('/'))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !idp.allows(rest) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let mut url = format!("{}{PREFIX}/{rest}", idp.upstream);
    if let Some(query) = req.uri().query() {
        url.push('?');
        url.push_str(query);
    }

    // Keycloak builds every link and redirect from these, and stamps tokens with them, so they
    // describe the address the browser used. The server's own calls to Keycloak send the same
    // (see `AuthState::backchannel_headers`), or it would refuse the tokens it issued here.
    let forwarded = state.auth.forwarded(req.headers());

    let method = req.method().clone();
    let mut headers = HeaderMap::new();
    for (name, value) in req.headers() {
        if is_hop_header(name) || name == header::COOKIE {
            continue;
        }
        headers.append(name, value.clone());
    }
    if let Some(cookie) = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(without_session_cookie)
        && let Ok(value) = HeaderValue::from_str(&cookie)
    {
        headers.insert(header::COOKIE, value);
    }
    for (name, value) in [
        ("x-forwarded-proto", forwarded.proto),
        ("x-forwarded-host", forwarded.host),
    ] {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(name, value);
        }
    }

    let Ok(body) = axum::body::to_bytes(req.into_body(), MAX_BODY).await else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };

    let upstream = match idp
        .http
        .request(method, &url)
        .headers(headers)
        .body(body)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("the sign-in service is not reachable: {e}"),
            )
                .into_response();
        }
    };

    let status = upstream.status();
    let mut out_headers = HeaderMap::new();
    for (name, value) in upstream.headers() {
        if !is_hop_header(name) {
            out_headers.append(name, value.clone());
        }
    }
    let body: Bytes = match upstream.bytes().await {
        Ok(body) => body,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    (status, out_headers, body).into_response()
}

#[cfg(test)]
mod tests;
