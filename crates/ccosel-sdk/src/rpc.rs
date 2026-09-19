//! Calling the server from inside an app.
//!
//! Apps are synchronous and batch-only: `App::update` may not block or wait. RPC is
//! asynchronous. The bridge is a **request cache keyed by the request itself**, which fits
//! immediate mode exactly — an app already re-declares its whole UI every frame, so it
//! re-declares its data dependencies the same way:
//!
//! ```ignore
//! match ui.rpc().get::<ListDir>(&ListDirReq { path: &self.path }) {
//!     Poll::Pending      => ui.label("Loading…"),
//!     Poll::Failed(e)    => ui.label(e.message()),
//!     Poll::Ready(list)  => { /* draw it */ }
//! }
//! ```
//!
//! Notice what an app never writes: no call ids, no "have I asked yet" flag, no `on_event`
//! handler, no cancellation, and no stale-reply check. Changing `self.path` *is* the
//! re-request, because it changes the cache key.
//!
//! That last point is the reason for this design rather than an app-held future. Navigate
//! `/a → /b` while `list_dir(/a)` is in flight and it resolves second: with hand-rolled
//! state the app shows the wrong directory unless the author remembered to compare call ids.
//! Here the key already changed, the old entry is swept and its reply discarded — **the app
//! cannot express the bug.**

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::RefCell;

use ccosel_abi::event::{Event, decode_error, event_kind, rpc_error};
use ccosel_proto::{Method, Query, Rpc};

/// Entries untouched for this many frames are dropped, cancelling them if still in flight.
/// Long enough to survive a frame where a window is occluded, short enough that abandoned
/// calls do not pin the app against eviction.
const SWEEP_AFTER_FRAMES: u64 = 240;

/// The state of a request, as the app sees it.
#[derive(Clone, Debug)]
pub enum Poll<T> {
    /// In flight. The app is *not* re-run while waiting — the reply wakes it. Waiting is not
    /// animating, so do not raise `wants_repaint_after_ms` for this.
    Pending,
    Ready(T),
    Failed(RpcError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcError {
    pub code: u32,
}

impl RpcError {
    /// A short, renderable description. Errors never carry a type that needs decoding, so
    /// showing a failure can never itself fail.
    pub fn message(&self) -> &'static str {
        match self.code {
            rpc_error::REJECTED => "request refused",
            rpc_error::DENIED => "permission denied",
            rpc_error::TIMEOUT => "timed out",
            rpc_error::TRANSPORT => "connection lost",
            rpc_error::SERVER => "server error",
            rpc_error::DECODE => "bad response",
            rpc_error::CANCELLED => "cancelled",
            rpc_error::SHED => "superseded",
            _ => "request failed",
        }
    }
}

/// A queued outbound call, drained by the runtime after `update()` returns.
pub struct OutCall {
    pub call_id: u32,
    pub method: u32,
    pub args: Vec<u8>,
}

/// Turns reply bytes into a concrete type without the event path knowing which type that is.
/// Captured at request time, when `M` is still in scope.
type Decoder = fn(&[u8]) -> Option<Rc<dyn Any>>;

fn decode_reply<M: Rpc>(bytes: &[u8]) -> Option<Rc<dyn Any>> {
    postcard::from_bytes::<M::Reply>(bytes)
        .ok()
        .map(|v| Rc::new(v) as Rc<dyn Any>)
}

enum Slot {
    InFlight { call_id: u32, decode: Decoder },
    Ready(Rc<dyn Any>),
    Failed(RpcError),
}

struct Entry {
    slot: Slot,
    last_used_frame: u64,
}

/// Cache key. Holds the encoded argument bytes, not a hash of them: a hash collision here
/// would silently serve one directory's contents for another, which is a genuinely awful bug
/// to debug, and the bytes are usually a short path.
type Key = (u16, Vec<u8>);

#[derive(Default)]
struct Inner {
    entries: BTreeMap<Key, Entry>,
    /// Reverse index so an arriving reply can find its entry.
    by_call: BTreeMap<u32, Key>,
    outbox: Vec<OutCall>,
    cancels: Vec<u32>,
    next_call_id: u32,
    frame: u64,
}

/// Handle an app uses to talk to the server. Reached via [`crate::Ui::rpc`].
///
/// Interior mutability with a shared reference, so `ui.rpc().get(..)` does not borrow `ui`
/// and the result can be used while still drawing into `ui`.
#[derive(Default)]
pub struct RpcCtx {
    inner: RefCell<Inner>,
}

impl RpcCtx {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request a query, issuing it on a cache miss.
    ///
    /// Call this every frame with whatever you want to display. Only `Query` methods are
    /// accepted: caching or auto-retrying an effectful call would be a correctness bug, so the
    /// type system keeps them apart.
    pub fn get<M: Query>(&self, req: &M::Req<'_>) -> Poll<Rc<M::Reply>> {
        let Ok(args) = postcard::to_allocvec(req) else {
            return Poll::Failed(RpcError {
                code: rpc_error::DECODE,
            });
        };
        let key: Key = (M::METHOD as u16, args);

        let mut inner = self.inner.borrow_mut();
        let frame = inner.frame;

        if let Some(entry) = inner.entries.get_mut(&key) {
            entry.last_used_frame = frame;
            return match &entry.slot {
                Slot::InFlight { .. } => Poll::Pending,
                Slot::Failed(e) => Poll::Failed(*e),
                Slot::Ready(any) => match any.clone().downcast::<M::Reply>() {
                    Ok(v) => Poll::Ready(v),
                    // Only reachable if two methods shared a key, which they cannot.
                    Err(_) => Poll::Failed(RpcError {
                        code: rpc_error::DECODE,
                    }),
                },
            };
        }

        let call_id = {
            inner.next_call_id = inner.next_call_id.wrapping_add(1).max(1);
            inner.next_call_id
        };
        inner.outbox.push(OutCall {
            call_id,
            method: M::METHOD as u32,
            args: key.1.clone(),
        });
        inner.by_call.insert(call_id, key.clone());
        inner.entries.insert(
            key,
            Entry {
                slot: Slot::InFlight {
                    call_id,
                    decode: decode_reply::<M>,
                },
                last_used_frame: frame,
            },
        );
        Poll::Pending
    }

    /// Drop a cached result so the next `get` re-issues it. This is what a Refresh button does.
    pub fn invalidate<M: Query>(&self, req: &M::Req<'_>) {
        let Ok(args) = postcard::to_allocvec(req) else {
            return;
        };
        let key: Key = (M::METHOD as u16, args);
        let mut inner = self.inner.borrow_mut();
        if let Some(entry) = inner.entries.remove(&key) {
            Self::retire(&mut inner, entry);
        }
    }

    /// Cancel a still-in-flight slot and forget its reverse index entry.
    fn retire(inner: &mut Inner, entry: Entry) {
        if let Slot::InFlight { call_id, .. } = entry.slot {
            inner.by_call.remove(&call_id);
            inner.cancels.push(call_id);
        }
    }

    /// Apply one delivered event. Called by the runtime, never by app code.
    pub(crate) fn deliver(&self, event: &Event<'_>) {
        let mut inner = self.inner.borrow_mut();
        let Some(key) = inner.by_call.remove(&event.call_id) else {
            // A reply for something we cancelled or already resolved. Dropping it is the
            // contract: after a cancel, no event for that id is ever observable.
            return;
        };
        let Some(entry) = inner.entries.get_mut(&key) else {
            return;
        };
        let Slot::InFlight { decode, .. } = entry.slot else {
            return;
        };

        entry.slot = match event.kind {
            event_kind::RPC_OK => match decode(event.payload) {
                Some(value) => Slot::Ready(value),
                None => Slot::Failed(RpcError {
                    code: rpc_error::DECODE,
                }),
            },
            event_kind::RPC_ERR => {
                let (code, _detail) = decode_error(event.payload);
                Slot::Failed(RpcError { code })
            }
            // Unknown event kinds are ignored rather than treated as failures, so the shell can
            // add kinds without breaking older cached modules.
            _ => return,
        };
    }

    /// Advance the frame counter and drop stale entries. Called by the runtime each frame,
    /// *before* `update()`, so entries touched this frame survive.
    pub(crate) fn begin_frame(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.frame = inner.frame.wrapping_add(1);
        let frame = inner.frame;

        let stale: Vec<Key> = inner
            .entries
            .iter()
            .filter(|(_, e)| frame.saturating_sub(e.last_used_frame) > SWEEP_AFTER_FRAMES)
            .map(|(k, _)| k.clone())
            .collect();
        for key in stale {
            if let Some(entry) = inner.entries.remove(&key) {
                Self::retire(&mut inner, entry);
            }
        }
    }

    /// Drain queued calls. Called by the runtime after `update()`.
    pub(crate) fn take_outbox(&self) -> Vec<OutCall> {
        core::mem::take(&mut self.inner.borrow_mut().outbox)
    }

    /// Drain queued cancellations.
    pub(crate) fn take_cancels(&self) -> Vec<u32> {
        core::mem::take(&mut self.inner.borrow_mut().cancels)
    }

    /// Number of cached entries, for tests and diagnostics.
    pub fn cached_len(&self) -> usize {
        self.inner.borrow().entries.len()
    }
}

/// Human-readable method name, for logs.
pub fn method_name(method: u32) -> String {
    let name = match Method::from_u16(method as u16) {
        Some(Method::ListDir) => "list_dir",
        Some(Method::Stat) => "stat",
        None => "unknown",
    };
    String::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use ccosel_abi::event::{encode_error, event_kind};
    use ccosel_proto::fs::{DirEntry, DirListing, EntryKind, ListDir, ListDirReq};

    fn listing(names: &[&str]) -> Vec<u8> {
        let reply = DirListing {
            entries: names
                .iter()
                .map(|n| DirEntry {
                    name: String::from(*n),
                    kind: EntryKind::File,
                    size: 1,
                    mtime_s: 0,
                })
                .collect(),
            truncated: false,
        };
        postcard::to_allocvec(&reply).unwrap()
    }

    fn ok_event(call_id: u32, payload: &[u8]) -> Event<'_> {
        Event {
            kind: event_kind::RPC_OK,
            call_id,
            payload,
        }
    }

    fn req(path: &str) -> ListDirReq<'_> {
        ListDirReq { path }
    }

    #[test]
    fn first_get_issues_exactly_one_call_and_repeats_do_not() {
        let rpc = RpcCtx::new();
        rpc.begin_frame();
        assert!(matches!(rpc.get::<ListDir>(&req("/a")), Poll::Pending));

        let out = rpc.take_outbox();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].method, ccosel_proto::Method::ListDir as u32);

        // Nine more frames of asking for the same thing must put nothing on the wire. This is
        // the property that makes "ask every frame" a safe idiom for app authors.
        for _ in 0..9 {
            rpc.begin_frame();
            assert!(matches!(rpc.get::<ListDir>(&req("/a")), Poll::Pending));
            assert!(rpc.take_outbox().is_empty());
        }
    }

    #[test]
    fn a_reply_resolves_the_slot() {
        let rpc = RpcCtx::new();
        rpc.begin_frame();
        rpc.get::<ListDir>(&req("/a"));
        let call_id = rpc.take_outbox()[0].call_id;

        let payload = listing(&["notes.md", "build.log"]);
        rpc.deliver(&ok_event(call_id, &payload));

        rpc.begin_frame();
        match rpc.get::<ListDir>(&req("/a")) {
            Poll::Ready(list) => {
                assert_eq!(list.entries.len(), 2);
                assert_eq!(list.entries[0].name, "notes.md");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        assert!(rpc.take_outbox().is_empty(), "must not re-request");
    }

    #[test]
    fn errors_surface_without_a_decoder() {
        let rpc = RpcCtx::new();
        rpc.begin_frame();
        rpc.get::<ListDir>(&req("/nope"));
        let call_id = rpc.take_outbox()[0].call_id;

        let payload = encode_error(rpc_error::DENIED, "outside the jail");
        rpc.deliver(&Event {
            kind: event_kind::RPC_ERR,
            call_id,
            payload: &payload,
        });

        rpc.begin_frame();
        match rpc.get::<ListDir>(&req("/nope")) {
            Poll::Failed(e) => {
                assert_eq!(e.code, rpc_error::DENIED);
                assert_eq!(e.message(), "permission denied");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn a_garbled_reply_fails_the_slot_rather_than_hanging() {
        let rpc = RpcCtx::new();
        rpc.begin_frame();
        rpc.get::<ListDir>(&req("/a"));
        let call_id = rpc.take_outbox()[0].call_id;

        rpc.deliver(&ok_event(call_id, b"\xff\xff\xff not postcard"));

        rpc.begin_frame();
        match rpc.get::<ListDir>(&req("/a")) {
            Poll::Failed(e) => assert_eq!(e.code, rpc_error::DECODE),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn navigating_while_in_flight_cannot_show_the_wrong_directory() {
        // The classic async-UI bug: /a is requested, the user navigates to /b, and /a resolves
        // second. A hand-rolled state machine shows /a's contents under /b's heading unless the
        // author remembered to compare call ids. Here the key already moved on.
        let rpc = RpcCtx::new();

        rpc.begin_frame();
        rpc.get::<ListDir>(&req("/a"));
        let a_call = rpc.take_outbox()[0].call_id;

        rpc.begin_frame();
        rpc.get::<ListDir>(&req("/b"));
        let b_call = rpc.take_outbox()[0].call_id;
        assert_ne!(a_call, b_call);

        // /b resolves first, then the stale /a arrives.
        rpc.deliver(&ok_event(b_call, &listing(&["b-file"])));
        rpc.deliver(&ok_event(a_call, &listing(&["a-file"])));

        rpc.begin_frame();
        match rpc.get::<ListDir>(&req("/b")) {
            Poll::Ready(list) => assert_eq!(list.entries[0].name, "b-file"),
            other => panic!("expected /b contents, got {other:?}"),
        }
    }

    #[test]
    fn invalidate_cancels_an_in_flight_call_and_re_requests() {
        let rpc = RpcCtx::new();
        rpc.begin_frame();
        rpc.get::<ListDir>(&req("/a"));
        let first = rpc.take_outbox()[0].call_id;

        rpc.invalidate::<ListDir>(&req("/a"));
        assert_eq!(rpc.take_cancels(), vec![first]);

        rpc.begin_frame();
        rpc.get::<ListDir>(&req("/a"));
        let second = rpc.take_outbox();
        assert_eq!(second.len(), 1);
        assert_ne!(second[0].call_id, first);

        // The cancelled call's reply must be unobservable, per the cancel contract.
        rpc.deliver(&ok_event(first, &listing(&["stale"])));
        rpc.begin_frame();
        assert!(matches!(rpc.get::<ListDir>(&req("/a")), Poll::Pending));
    }

    #[test]
    fn replies_for_unknown_calls_are_ignored() {
        let rpc = RpcCtx::new();
        rpc.begin_frame();
        rpc.deliver(&ok_event(9999, &listing(&["ghost"])));
        assert_eq!(rpc.cached_len(), 0);
    }

    #[test]
    fn abandoned_entries_are_swept_and_cancelled() {
        // An app that stops asking for something must not pin the call forever: the shell's
        // eviction policy refuses to evict an app with in-flight RPC, so a leaked entry would
        // make the app permanently unevictable.
        let rpc = RpcCtx::new();
        rpc.begin_frame();
        rpc.get::<ListDir>(&req("/a"));
        let call_id = rpc.take_outbox()[0].call_id;
        assert_eq!(rpc.cached_len(), 1);

        for _ in 0..(SWEEP_AFTER_FRAMES + 2) {
            rpc.begin_frame();
        }
        assert_eq!(rpc.cached_len(), 0);
        assert_eq!(rpc.take_cancels(), vec![call_id]);
    }
}
