//! The shell's pending-call table.
//!
//! Deliberately contains **no browser types and no networking**. The actual bytes move through
//! a [`Wire`], which the shell implements with `fetch` and tests implement with a queue — so
//! coalescing, deadlines and delivery are all exercised natively, and the socket-versus-fetch
//! question stays a detail below this layer.
//!
//! Two properties this exists to guarantee:
//!
//! - **The shell owns the table, not the guest.** An app can be suspended or evicted with
//!   calls outstanding and the replies still arrive; a guest that is gone simply stops being a
//!   delivery target. Guests only *name* their calls.
//! - **Exactly one terminal event per call.** Every call ends in `RPC_OK` or one `RPC_ERR`,
//!   including calls that time out or get superseded — unless it was cancelled, which
//!   delivers nothing. So no app ever has to handle "this might never resolve", which is the
//!   branch app authors forget to write.

use std::collections::HashMap;
use std::rc::Rc;

use ccosel_abi::event::{encode_batch, encode_error, event_kind, rpc_error};
use ccosel_proto::{Coalesce, Method};

/// Identifies one call: the instance that made it, and the id that instance chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PendingKey {
    pub instance: u64,
    pub call: u32,
}

/// Where delivered events go. Implemented by the shell's per-window event queue.
pub trait EventSink {
    fn deliver(&self, batch: Vec<u8>);
    /// False once the window has been closed, so the table can drop the entry.
    fn alive(&self) -> bool;
}

/// One call, as it goes out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub seq: u32,
    pub method: u16,
    pub args: Vec<u8>,
}

/// One reply, as it comes back.
#[derive(Clone, Debug)]
pub struct Incoming {
    pub seq: u32,
    pub result: Result<Vec<u8>, (u32, String)>,
}

/// Moves batches. The shell's implementation posts to `/rpc`.
pub trait Wire {
    fn send(&self, batch: Vec<Outgoing>);
}

struct Entry {
    method: u16,
    sink: Rc<dyn EventSink>,
    deadline_ms: f64,
}

/// A set of calls sharing one wire request.
struct Group {
    /// Every caller waiting on this one request.
    members: Vec<PendingKey>,
}

/// Policy for a method, looked up rather than sent by the guest — a guest must not be able to
/// declare an effectful call idempotent and have it auto-retried.
fn policy(method: u16) -> (Coalesce, u32) {
    match Method::from_u16(method) {
        Some(Method::ListDir) => (Coalesce::ByArgs, 8_000),
        Some(Method::Stat) => (Coalesce::ByArgs, 4_000),
        Some(Method::ServerInfo) => (Coalesce::ByArgs, 4_000),
        None => (Coalesce::None, 4_000),
    }
}

pub struct Transport {
    wire: Box<dyn Wire>,
    entries: HashMap<PendingKey, Entry>,
    /// In-flight and staged requests, keyed by wire sequence number.
    groups: HashMap<u32, Group>,
    /// `(method, args)` -> wire sequence, so an identical call joins the existing request.
    by_args: HashMap<(u16, Vec<u8>), u32>,
    /// Reverse of `by_args`, so resolving a group can clear it.
    args_of: HashMap<u32, (u16, Vec<u8>)>,
    staged: Vec<Outgoing>,
    next_seq: u32,
}

impl Transport {
    pub fn new(wire: Box<dyn Wire>) -> Self {
        Self {
            wire,
            entries: HashMap::new(),
            groups: HashMap::new(),
            by_args: HashMap::new(),
            args_of: HashMap::new(),
            staged: Vec::new(),
            next_seq: 0,
        }
    }

    pub fn in_flight(&self) -> usize {
        self.entries.len()
    }

    /// Queue a call.
    ///
    /// An identical `(method, args)` already in flight is *joined* rather than re-sent: two
    /// Files windows showing the same directory cost one request and both get the reply.
    pub fn enqueue(
        &mut self,
        key: PendingKey,
        method: u16,
        args: Vec<u8>,
        sink: Rc<dyn EventSink>,
        now_ms: f64,
    ) {
        let (coalesce, deadline) = policy(method);

        // A repeat of a live id means the guest reset its counter (a restore) — retire the old
        // one deterministically rather than letting two calls alias.
        if self.entries.contains_key(&key) {
            self.fail(key, rpc_error::SHED, "");
        }

        self.entries.insert(
            key,
            Entry {
                method,
                sink,
                deadline_ms: now_ms + f64::from(deadline),
            },
        );

        let args_key = (method, args.clone());
        if coalesce != Coalesce::None
            && let Some(&seq) = self.by_args.get(&args_key)
            && let Some(group) = self.groups.get_mut(&seq)
        {
            group.members.push(key);
            return;
        }

        self.next_seq = self.next_seq.wrapping_add(1);
        let seq = self.next_seq;
        self.groups.insert(seq, Group { members: vec![key] });
        if coalesce != Coalesce::None {
            self.by_args.insert(args_key.clone(), seq);
            self.args_of.insert(seq, args_key);
        }
        self.staged.push(Outgoing { seq, method, args });
    }

    /// Send everything staged since the last flush.
    ///
    /// Called once per shell frame. That *is* the coalescing window: calls made during one
    /// frame share a request, with no timer to arm and nothing to poll.
    pub fn flush(&mut self) {
        if self.staged.is_empty() {
            return;
        }
        self.wire.send(std::mem::take(&mut self.staged));
    }

    /// Drop a call. After this, no event for `key` is ever delivered.
    pub fn cancel(&mut self, key: PendingKey) {
        if self.entries.remove(&key).is_some() {
            self.detach(key);
        }
    }

    /// The window closed. Everything it was waiting on is abandoned.
    pub fn forget_instance(&mut self, instance: u64) {
        let keys: Vec<PendingKey> = self
            .entries
            .keys()
            .filter(|k| k.instance == instance)
            .copied()
            .collect();
        for key in keys {
            self.cancel(key);
        }
    }

    /// Expire deadlines, and drop entries whose window has gone.
    ///
    /// Synthesising a timeout is what lets an app's state machine have no "waiting forever"
    /// branch.
    pub fn tick(&mut self, now_ms: f64) {
        let expired: Vec<PendingKey> = self
            .entries
            .iter()
            .filter(|(_, e)| now_ms >= e.deadline_ms || !e.sink.alive())
            .map(|(k, _)| *k)
            .collect();

        for key in expired {
            let dead = self.entries.get(&key).is_some_and(|e| !e.sink.alive());
            if dead {
                self.cancel(key);
            } else {
                self.fail(key, rpc_error::TIMEOUT, "");
            }
        }
    }

    /// Apply replies from the wire.
    pub fn on_replies(&mut self, replies: Vec<Incoming>) {
        for reply in replies {
            let Some(group) = self.groups.remove(&reply.seq) else {
                continue;
            };
            if let Some(args_key) = self.args_of.remove(&reply.seq) {
                self.by_args.remove(&args_key);
            }

            for key in group.members {
                match &reply.result {
                    Ok(payload) => self.complete(key, event_kind::RPC_OK, payload.clone()),
                    Err((code, detail)) => {
                        let payload = encode_error(map_server_error(*code), detail);
                        self.complete(key, event_kind::RPC_ERR, payload);
                    }
                }
            }
        }
    }

    /// The whole batch failed — a dropped connection, a 500. Idempotent calls could be retried
    /// here; for now every member is failed, which keeps the one-terminal-event rule intact.
    pub fn fail_all_in_flight(&mut self, code: u32, detail: &str) {
        let keys: Vec<PendingKey> = self.entries.keys().copied().collect();
        for key in keys {
            self.fail(key, code, detail);
        }
    }

    fn fail(&mut self, key: PendingKey, code: u32, detail: &str) {
        self.complete(key, event_kind::RPC_ERR, encode_error(code, detail));
    }

    fn complete(&mut self, key: PendingKey, kind: u32, payload: Vec<u8>) {
        let Some(entry) = self.entries.remove(&key) else {
            return;
        };
        self.detach(key);
        if entry.sink.alive() {
            let batch = encode_batch(&[(kind, key.call, &payload)]);
            entry.sink.deliver(batch);
        }
        let _ = entry.method;
    }

    /// Remove a key from whatever group it belongs to, cleaning up an emptied group.
    fn detach(&mut self, key: PendingKey) {
        let mut empty = Vec::new();
        for (seq, group) in self.groups.iter_mut() {
            group.members.retain(|k| *k != key);
            if group.members.is_empty() {
                empty.push(*seq);
            }
        }
        for seq in empty {
            self.groups.remove(&seq);
            if let Some(args_key) = self.args_of.remove(&seq) {
                self.by_args.remove(&args_key);
            }
            self.staged.retain(|o| o.seq != seq);
        }
    }
}

/// Server-side codes are a different space from the guest-visible ones: the guest space also
/// covers failures that never reached the server at all.
fn map_server_error(code: u32) -> u32 {
    use ccosel_proto::server_error as se;
    match code {
        se::DENIED => rpc_error::DENIED,
        se::NOT_FOUND | se::NOT_A_DIRECTORY | se::IO => rpc_error::SERVER,
        se::UNKNOWN_METHOD | se::MALFORMED => rpc_error::DECODE,
        _ => rpc_error::SERVER,
    }
}
