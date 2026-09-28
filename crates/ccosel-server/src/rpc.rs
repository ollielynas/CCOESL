//! `POST /rpc`: a batch of calls in, a batch of replies out.
//!
//! Batching is what makes the shell's coalescing worth anything over HTTP — N logical calls
//! share one set of headers and one round trip. It also means the envelope is unchanged if
//! this ever moves onto a WebSocket, so that swap stays a transport detail.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use ccosel_proto::build::CompileReq;
use ccosel_proto::fs::ListDirReq;
use ccosel_proto::info::ServerInfoReply;
use ccosel_proto::{Method, PROTO_VERSION, WireReply, WireRequest, WireResult, server_error};

use crate::{AppState, stats};

/// What one call produced, before it is borrowed into a `WireReply`.
enum Outcome {
    Ok(Vec<u8>),
    Err(u32, String),
}

pub async fn handle(State(state): State<AppState>, body: Bytes) -> Response {
    let Ok(requests) = postcard::from_bytes::<Vec<WireRequest>>(&body) else {
        return (StatusCode::BAD_REQUEST, "malformed rpc batch").into_response();
    };

    // Two passes: run every call into owned buffers, then borrow those into the reply batch.
    // `WireResult::Ok` borrows its payload, so the buffers have to outlive the encoding.
    let outcomes: Vec<(u32, Outcome)> = requests
        .iter()
        .map(|req| (req.seq, dispatch(&state, req)))
        .collect();

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
fn dispatch(state: &AppState, req: &WireRequest<'_>) -> Outcome {
    state.stats.count_rpc();
    let Some(method) = Method::from_u16(req.method) else {
        return Outcome::Err(server_error::UNKNOWN_METHOD, String::new());
    };

    match method {
        Method::ListDir => {
            let Ok(args) = postcard::from_bytes::<ListDirReq>(req.args) else {
                return Outcome::Err(server_error::MALFORMED, String::new());
            };
            state.scratch.touch(args.path);
            match state.jail.list_dir(&args) {
                Ok(mut listing) => {
                    // Temporary project folders are the Compiler's business, not a folder
                    // anyone browses to.
                    if args.path.trim_matches('/').is_empty() {
                        listing
                            .entries
                            .retain(|e| e.name != ccosel_proto::scratch::DIR);
                    }
                    match postcard::to_allocvec(&listing) {
                        Ok(bytes) => Outcome::Ok(bytes),
                        Err(_) => Outcome::Err(server_error::IO, String::new()),
                    }
                }
                Err(code) => Outcome::Err(code, args.path.to_owned()),
            }
        }
        Method::Stat => Outcome::Err(server_error::UNKNOWN_METHOD, String::new()),
        Method::Compile => {
            let Ok(args) = postcard::from_bytes::<CompileReq>(req.args) else {
                return Outcome::Err(server_error::MALFORMED, String::new());
            };
            state.scratch.touch(args.path);
            match crate::build_api::compile(&state.jail, &state.jobs, &args) {
                Ok(status) => match postcard::to_allocvec(&status) {
                    Ok(bytes) => Outcome::Ok(bytes),
                    Err(_) => Outcome::Err(server_error::IO, String::new()),
                },
                Err(code) => Outcome::Err(code, args.path.to_owned()),
            }
        }
        Method::ServerInfo => {
            let host = stats::sample_host();
            let info = ServerInfoReply {
                proto_version: PROTO_VERSION,
                root: state.jail.root().display().to_string(),
                uptime_ms: state.stats.uptime_ms(),
                rpc_calls: state.stats.rpc_calls(),
                cpus: host.cpus,
                load_milli: host.load_milli,
                mem_used_kib: host.mem_used_kib,
                mem_total_kib: host.mem_total_kib,
            };
            match postcard::to_allocvec(&info) {
                Ok(bytes) => Outcome::Ok(bytes),
                Err(_) => Outcome::Err(server_error::IO, String::new()),
            }
        }
    }
}
