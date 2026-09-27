//! `POST /rpc`: a batch of calls in, a batch of replies out.
//!
//! Batching is what makes the shell's coalescing worth anything over HTTP — N logical calls
//! share one set of headers and one round trip. It also means the envelope is unchanged if
//! this ever moves onto a WebSocket, so that swap stays a transport detail.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ccosel_proto::account::Account;
use ccosel_proto::fs::ListDirReq;
use ccosel_proto::{Method, WireReply, WireRequest, WireResult, server_error};

use crate::AppState;

/// What one call produced, before it is borrowed into a `WireReply`.
enum Outcome {
    Ok(Vec<u8>),
    Err(u32, String),
}

pub async fn handle(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(requests) = postcard::from_bytes::<Vec<WireRequest>>(&body) else {
        return (StatusCode::BAD_REQUEST, "malformed rpc batch").into_response();
    };

    // Two passes: run every call into owned buffers, then borrow those into the reply batch.
    // `WireResult::Ok` borrows its payload, so the buffers have to outlive the encoding.
    // In order, one at a time: a batch that signs out and then asks who is signed in should
    // see the sign-out.
    let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok());
    let mut outcomes: Vec<(u32, Outcome)> = Vec::with_capacity(requests.len());
    for req in &requests {
        outcomes.push((req.seq, dispatch(&state, cookie, req).await));
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
async fn dispatch(state: &AppState, cookie: Option<&str>, req: &WireRequest<'_>) -> Outcome {
    let Some(method) = Method::from_u16(req.method) else {
        return Outcome::Err(server_error::UNKNOWN_METHOD, String::new());
    };

    match method {
        Method::ListDir => {
            let Ok(args) = postcard::from_bytes::<ListDirReq>(req.args) else {
                return Outcome::Err(server_error::MALFORMED, String::new());
            };
            match state.jail.list_dir(&args) {
                Ok(listing) => match postcard::to_allocvec(&listing) {
                    Ok(bytes) => Outcome::Ok(bytes),
                    Err(_) => Outcome::Err(server_error::IO, String::new()),
                },
                Err(code) => Outcome::Err(code, args.path.to_owned()),
            }
        }
        Method::Stat => Outcome::Err(server_error::UNKNOWN_METHOD, String::new()),
        Method::WhoAmI => encode(&Account {
            login_enabled: state.auth.enabled(),
            name: state.auth.session_login(cookie),
        }),
        Method::SignOut => {
            state.auth.end_session(cookie).await;
            encode(&())
        }
    }
}

fn encode<T: serde::Serialize>(reply: &T) -> Outcome {
    match postcard::to_allocvec(reply) {
        Ok(bytes) => Outcome::Ok(bytes),
        Err(_) => Outcome::Err(server_error::IO, String::new()),
    }
}
