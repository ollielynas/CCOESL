//! `POST /rpc`: a batch of calls in, a batch of replies out.
//!
//! Batching is what makes the shell's coalescing worth anything over HTTP — N logical calls
//! share one set of headers and one round trip. It also means the envelope is unchanged if
//! this ever moves onto a WebSocket, so that swap stays a transport detail.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ccosel_proto::account::{Account, CreateAppPasswordReq, RevokeAppPasswordReq};
use ccosel_proto::archive::ArchiveReq;
use ccosel_proto::build::CompileReq;
use ccosel_proto::desktop::DesktopLayout;
use ccosel_proto::fs::{ListDirReq, PathReq, SearchReq, WriteFileReq};
use ccosel_proto::info::ServerInfoReply;
use ccosel_proto::{Method, PROTO_VERSION, WireReply, WireRequest, WireResult, server_error};

use serde::Serialize;

use crate::access::{self, User};
use crate::{AppState, stats};

/// What one call produced, before it is borrowed into a `WireReply`.
enum Outcome {
    Ok(Vec<u8>),
    Err(u32, String),
}

pub async fn handle(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(requests) = postcard::from_bytes::<Vec<WireRequest>>(&body) else {
        return (StatusCode::BAD_REQUEST, "malformed rpc batch").into_response();
    };
    let user = crate::caller(user.as_ref().map(|u| &u.0));
    if let Some(name) = user {
        // A user's private folder exists from their first request, so an app can offer it
        // without first having to create it.
        if let Err(e) = access::ensure_home(state.jail.root(), name) {
            eprintln!("rpc: creating the home folder for {name}: {e}");
        }
    }

    // Two passes: run every call into owned buffers, then borrow those into the reply batch.
    // `WireResult::Ok` borrows its payload, so the buffers have to outlive the encoding.
    // In order, one at a time: a batch that signs out and then asks who is signed in should
    // see the sign-out.
    let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok());
    let mut outcomes: Vec<(u32, Outcome)> = Vec::with_capacity(requests.len());
    for req in &requests {
        outcomes.push((req.seq, dispatch(&state, cookie, user, req).await));
    }

    let replies: Vec<WireReply> = outcomes
        .iter()
        .map(|(seq, outcome)| WireReply {
            seq: *seq,
            result: match outcome {
                Outcome::Ok(bytes) => WireResult::Ok(bytes),
                Outcome::Err(code, detail) => WireResult::Err {
                    code: *code,
                    detail,
                },
            },
        })
        .collect();

    match postcard::to_allocvec(&replies) {
        Ok(encoded) => (
            [(header::CONTENT_TYPE, "application/octet-stream")],
            encoded,
        )
            .into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "encode failed").into_response(),
    }
}

/// A failed call is a failed *entry*, never a failed batch: one bad path must not take down
/// the other calls that were coalesced into the same request.
async fn dispatch(
    state: &AppState,
    cookie: Option<&str>,
    user: Option<&str>,
    req: &WireRequest<'_>,
) -> Outcome {
    state.stats.count_rpc();
    let Some(method) = Method::from_u16(req.method) else {
        return Outcome::Err(server_error::UNKNOWN_METHOD, String::new());
    };
    let jail = &state.jail;

    match method {
        Method::ListDir => run::<ListDirReq, _>(req, |a| {
            state.scratch.touch(a.path);
            let listing = jail.list_dir(&a, user).map(|mut listing| {
                // Temporary project folders are the Compiler's business, not a folder anyone
                // browses to.
                if a.path.trim_matches('/').is_empty() {
                    listing
                        .entries
                        .retain(|e| e.name != ccosel_proto::scratch::DIR);
                }
                listing
            });
            (listing, a.path)
        }),
        Method::Stat => Outcome::Err(server_error::UNKNOWN_METHOD, String::new()),
        Method::Compile => run::<CompileReq, _>(req, |a| {
            state.scratch.touch(a.path);
            // Building reads the project, so it needs the same permission as reading it.
            let result = jail
                .authorize(a.path, user, crate::fs_api::Need::Read)
                .and_then(|_| crate::build_api::compile(jail, &state.jobs, &a));
            (result, a.path)
        }),
        Method::Archive => run::<ArchiveReq, _>(req, |a| {
            let result = crate::archive_api::archive(jail, &state.archive_jobs, &a, user);
            (result, a.path)
        }),
        Method::ServerInfo => {
            let host = stats::sample_host();
            let info = ServerInfoReply {
                proto_version: PROTO_VERSION,
                root: jail.root().display().to_string(),
                uptime_ms: state.stats.uptime_ms(),
                rpc_calls: state.stats.rpc_calls(),
                cpus: host.cpus,
                load_milli: host.load_milli,
                mem_used_kib: host.mem_used_kib,
                mem_total_kib: host.mem_total_kib,
            };
            encode(&info)
        }
        Method::WhoAmI => encode(&Account {
            login_enabled: state.auth.enabled(),
            name: state.auth.session_login(cookie),
            account_url: state.auth.account_url().map(str::to_owned),
        }),
        Method::SignOut => {
            state.auth.end_session(cookie).await;
            encode(&())
        }
        Method::ReadFile => run::<PathReq, _>(req, |a| (jail.read_file(a.path, user), a.path)),
        Method::WriteFile => run::<WriteFileReq, _>(req, |a| (jail.write_file(&a, user), a.path)),
        Method::CreateDir => run::<PathReq, _>(req, |a| (jail.create_dir(a.path, user), a.path)),
        Method::Remove => run::<PathReq, _>(req, |a| (jail.remove(a.path, user), a.path)),
        Method::Access => run::<PathReq, _>(req, |a| (jail.access(a.path, user), a.path)),
        Method::Search => run::<SearchReq, _>(req, |a| (jail.search(&a, user), a.path)),
        Method::ImageInfo => run::<PathReq, _>(req, |a| (jail.image_info(a.path, user), a.path)),
        // App passwords belong to whoever is signed in; anonymous callers have none to manage.
        Method::ListAppPasswords => match user {
            Some(name) => encode(&state.auth.app_passwords.list(name)),
            None => Outcome::Err(server_error::DENIED, String::new()),
        },
        Method::CreateAppPassword => run::<CreateAppPasswordReq, _>(req, |a| {
            let made = user
                .ok_or(server_error::DENIED)
                .and_then(|name| state.auth.app_passwords.create(name, a.name));
            (made, "")
        }),
        Method::RevokeAppPassword => run::<RevokeAppPasswordReq, _>(req, |a| {
            let revoked = user
                .ok_or(server_error::DENIED)
                .and_then(|name| state.auth.app_passwords.revoke(name, a.id));
            (revoked, "")
        }),
        Method::LoadDesktop => encode(&crate::desktop::load(jail.root(), user)),
        Method::SaveDesktop => run::<DesktopLayout, _>(req, |layout| {
            (crate::desktop::save(jail.root(), user, &layout), "")
        }),
    }
}

/// Decode a call's arguments, run it, and encode what it returned. A failure's detail is the
/// path it was about, which is what an error message most needs to show.
fn run<'a, A, T>(
    req: &WireRequest<'a>,
    call: impl FnOnce(A) -> (Result<T, u32>, &'a str),
) -> Outcome
where
    A: serde::Deserialize<'a>,
    T: Serialize,
{
    let Ok(args) = postcard::from_bytes::<A>(req.args) else {
        return Outcome::Err(server_error::MALFORMED, String::new());
    };
    match call(args) {
        (Ok(reply), _) => encode(&reply),
        (Err(code), path) => Outcome::Err(code, path.to_owned()),
    }
}

fn encode<T: serde::Serialize>(reply: &T) -> Outcome {
    match postcard::to_allocvec(reply) {
        Ok(bytes) => Outcome::Ok(bytes),
        Err(_) => Outcome::Err(server_error::IO, String::new()),
    }
}
