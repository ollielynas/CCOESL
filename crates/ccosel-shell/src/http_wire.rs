//! `Wire` over `fetch`, posting batches to `/rpc`.
//!
//! No WebSocket yet, deliberately. `list_dir` is strictly request/response, so a socket would
//! carry nothing a POST does not — while costing reconnect, resume-from-cursor, an app-level
//! heartbeat (browsers cannot observe ping/pong), and a queue for calls made before the socket
//! opens. On a bad LAN the simplest reliable thing is a request you can just retry. The socket
//! earns its place when there is genuine push: compile progress, or filesystem change events.
//!
//! Replies land in a shared inbox that the desktop drains next frame, rather than calling back
//! into the transport directly — same pattern as module loading, and it avoids the circular
//! ownership a direct callback would need.

use std::cell::RefCell;
use std::rc::Rc;

use ccosel_proto::{WireReply, WireRequest, WireResult};
use ccosel_transport::{Incoming, Outgoing, Wire};

use crate::fetch;

pub type Inbox = Rc<RefCell<Vec<Incoming>>>;

pub struct HttpWire {
    url: String,
    inbox: Inbox,
    /// Replies resolve outside the frame loop, so nothing would redraw on its own.
    wake: egui::Context,
}

impl HttpWire {
    pub fn new(url: impl Into<String>, inbox: Inbox, wake: egui::Context) -> Self {
        Self {
            url: url.into(),
            inbox,
            wake,
        }
    }
}

impl Wire for HttpWire {
    fn send(&self, batch: Vec<Outgoing>) {
        let requests: Vec<WireRequest> = batch
            .iter()
            .map(|o| WireRequest {
                seq: o.seq,
                method: o.method,
                args: &o.args,
            })
            .collect();

        let Ok(body) = postcard::to_allocvec(&requests) else {
            return;
        };
        let seqs: Vec<u32> = batch.iter().map(|o| o.seq).collect();
        let url = self.url.clone();
        let inbox = self.inbox.clone();
        let wake = self.wake.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let incoming = match fetch::post_bytes(&url, &body).await {
                Ok(bytes) => decode_replies(&bytes, &seqs),
                // A failed batch fails every call in it. The transport turns that into one
                // terminal event per call, so no app is left waiting on a request that died
                // in transit.
                Err(e) => transport_failures(&seqs, &e),
            };
            inbox.borrow_mut().extend(incoming);
            wake.request_repaint();
        });
    }
}

fn decode_replies(bytes: &[u8], seqs: &[u32]) -> Vec<Incoming> {
    let Ok(replies) = postcard::from_bytes::<Vec<WireReply>>(bytes) else {
        return transport_failures(seqs, "malformed reply batch");
    };
    replies
        .into_iter()
        .map(|r| Incoming {
            seq: r.seq,
            result: match r.result {
                WireResult::Ok(payload) => Ok(payload.to_vec()),
                WireResult::Err { code, detail } => Err((code, detail.to_owned())),
            },
        })
        .collect()
}

fn transport_failures(seqs: &[u32], detail: &str) -> Vec<Incoming> {
    seqs.iter()
        .map(|seq| Incoming {
            seq: *seq,
            result: Err((u32::MAX, detail.to_owned())),
        })
        .collect()
}
