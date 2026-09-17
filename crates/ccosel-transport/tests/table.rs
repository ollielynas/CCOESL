//! Pending-table behaviour, driven through a fake wire.
//!
//! No network and no wasm: coalescing, deadlines and the one-terminal-event rule are pure
//! logic, and this is where they get proven.

use std::cell::RefCell;
use std::rc::Rc;

use ccosel_abi::event::{decode_batch, decode_error, event_kind, rpc_error};
use ccosel_proto::{server_error, Method};
use ccosel_transport::{EventSink, Incoming, Outgoing, PendingKey, Transport, Wire};

#[derive(Default)]
struct FakeWire {
    sent: Rc<RefCell<Vec<Vec<Outgoing>>>>,
}

impl Wire for FakeWire {
    fn send(&self, batch: Vec<Outgoing>) {
        self.sent.borrow_mut().push(batch);
    }
}

#[derive(Default)]
struct Queue {
    batches: RefCell<Vec<Vec<u8>>>,
    alive: RefCell<bool>,
}

impl EventSink for Queue {
    fn deliver(&self, batch: Vec<u8>) {
        self.batches.borrow_mut().push(batch);
    }
    fn alive(&self) -> bool {
        *self.alive.borrow()
    }
}

fn queue() -> Rc<Queue> {
    let q = Rc::new(Queue::default());
    *q.alive.borrow_mut() = true;
    q
}

/// Flatten every event a sink received into `(kind, call_id, error_code)`.
fn events(q: &Rc<Queue>) -> Vec<(u32, u32, u32)> {
    q.batches
        .borrow()
        .iter()
        .flat_map(|b| {
            decode_batch(b)
                .unwrap()
                .into_iter()
                .map(|e| {
                    let code = if e.kind == event_kind::RPC_ERR {
                        decode_error(e.payload).0
                    } else {
                        0
                    };
                    (e.kind, e.call_id, code)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn setup() -> (Transport, Rc<RefCell<Vec<Vec<Outgoing>>>>) {
    let sent = Rc::new(RefCell::new(Vec::new()));
    let wire = FakeWire { sent: sent.clone() };
    (Transport::new(Box::new(wire)), sent)
}

const LIST: u16 = Method::ListDir as u16;

#[test]
fn identical_calls_share_one_request_and_both_get_the_reply() {
    // Two Files windows on the same directory. One request, two deliveries.
    let (mut t, sent) = setup();
    let a = queue();
    let b = queue();

    t.enqueue(PendingKey { instance: 1, call: 10 }, LIST, b"/shared".to_vec(), a.clone(), 0.0);
    t.enqueue(PendingKey { instance: 2, call: 99 }, LIST, b"/shared".to_vec(), b.clone(), 0.0);
    t.flush();

    let batches = sent.borrow();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 1, "coalesced into a single wire call");
    let seq = batches[0][0].seq;
    drop(batches);

    t.on_replies(vec![Incoming { seq, result: Ok(b"payload".to_vec()) }]);

    // Each window is told using *its own* call id, which is the only id it knows.
    assert_eq!(events(&a), vec![(event_kind::RPC_OK, 10, 0)]);
    assert_eq!(events(&b), vec![(event_kind::RPC_OK, 99, 0)]);
    assert_eq!(t.in_flight(), 0);
}

#[test]
fn different_arguments_are_not_coalesced() {
    let (mut t, sent) = setup();
    let q = queue();
    t.enqueue(PendingKey { instance: 1, call: 1 }, LIST, b"/a".to_vec(), q.clone(), 0.0);
    t.enqueue(PendingKey { instance: 1, call: 2 }, LIST, b"/b".to_vec(), q.clone(), 0.0);
    t.flush();
    assert_eq!(sent.borrow()[0].len(), 2);
}

#[test]
fn one_flush_per_frame_batches_everything_made_that_frame() {
    let (mut t, sent) = setup();
    let q = queue();
    for i in 0..5u32 {
        t.enqueue(
            PendingKey { instance: 1, call: i },
            LIST,
            format!("/dir{i}").into_bytes(),
            q.clone(),
            0.0,
        );
    }
    t.flush();
    t.flush(); // nothing staged; must not send an empty batch
    assert_eq!(sent.borrow().len(), 1);
    assert_eq!(sent.borrow()[0].len(), 5);
}

#[test]
fn server_errors_reach_the_guest_as_guest_error_codes() {
    let (mut t, sent) = setup();
    let q = queue();
    t.enqueue(PendingKey { instance: 1, call: 4 }, LIST, b"/nope".to_vec(), q.clone(), 0.0);
    t.flush();
    let seq = sent.borrow()[0][0].seq;

    t.on_replies(vec![Incoming {
        seq,
        result: Err((server_error::DENIED, "outside the jail".into())),
    }]);
    assert_eq!(events(&q), vec![(event_kind::RPC_ERR, 4, rpc_error::DENIED)]);
}

#[test]
fn a_deadline_produces_exactly_one_terminal_event() {
    // The property that lets an app's state machine omit a "waiting forever" branch.
    let (mut t, _sent) = setup();
    let q = queue();
    t.enqueue(PendingKey { instance: 1, call: 5 }, LIST, b"/slow".to_vec(), q.clone(), 0.0);
    t.flush();

    t.tick(1_000.0);
    assert!(events(&q).is_empty(), "not due yet");

    t.tick(9_000.0);
    assert_eq!(events(&q), vec![(event_kind::RPC_ERR, 5, rpc_error::TIMEOUT)]);

    // And no second event, even if the reply turns up afterwards.
    t.tick(20_000.0);
    assert_eq!(events(&q).len(), 1);
    assert_eq!(t.in_flight(), 0);
}

#[test]
fn a_cancelled_call_delivers_nothing_ever() {
    let (mut t, sent) = setup();
    let q = queue();
    let key = PendingKey { instance: 1, call: 6 };
    t.enqueue(key, LIST, b"/a".to_vec(), q.clone(), 0.0);
    t.flush();
    let seq = sent.borrow()[0][0].seq;

    t.cancel(key);
    t.on_replies(vec![Incoming { seq, result: Ok(b"late".to_vec()) }]);
    t.tick(60_000.0);

    assert!(events(&q).is_empty(), "cancel must be absolute");
    assert_eq!(t.in_flight(), 0);
}

#[test]
fn closing_a_window_drops_its_calls_without_delivering() {
    let (mut t, _sent) = setup();
    let q = queue();
    t.enqueue(PendingKey { instance: 7, call: 1 }, LIST, b"/a".to_vec(), q.clone(), 0.0);
    t.enqueue(PendingKey { instance: 8, call: 1 }, LIST, b"/b".to_vec(), q.clone(), 0.0);

    t.forget_instance(7);
    assert_eq!(t.in_flight(), 1, "only the closed window's call went away");
}

#[test]
fn a_dead_sink_is_reaped_on_tick() {
    // A window closed without the desktop telling the transport. `alive()` is the backstop, so
    // the table cannot accumulate entries pointing at windows that are gone.
    let (mut t, _sent) = setup();
    let q = queue();
    t.enqueue(PendingKey { instance: 1, call: 1 }, LIST, b"/a".to_vec(), q.clone(), 0.0);
    *q.alive.borrow_mut() = false;

    t.tick(1.0);
    assert_eq!(t.in_flight(), 0);
    assert!(events(&q).is_empty());
}

#[test]
fn a_reused_call_id_retires_the_old_entry() {
    // A restored guest that reset its counter must not end up with two calls aliasing one id.
    let (mut t, _sent) = setup();
    let q = queue();
    let key = PendingKey { instance: 1, call: 1 };
    t.enqueue(key, LIST, b"/a".to_vec(), q.clone(), 0.0);
    t.enqueue(key, LIST, b"/b".to_vec(), q.clone(), 0.0);

    assert_eq!(events(&q), vec![(event_kind::RPC_ERR, 1, rpc_error::SHED)]);
    assert_eq!(t.in_flight(), 1);
}

#[test]
fn a_dropped_connection_fails_every_outstanding_call() {
    let (mut t, _sent) = setup();
    let q = queue();
    t.enqueue(PendingKey { instance: 1, call: 1 }, LIST, b"/a".to_vec(), q.clone(), 0.0);
    t.enqueue(PendingKey { instance: 1, call: 2 }, LIST, b"/b".to_vec(), q.clone(), 0.0);
    t.flush();

    t.fail_all_in_flight(rpc_error::TRANSPORT, "link lost");
    let got = events(&q);
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|(k, _, c)| *k == event_kind::RPC_ERR && *c == rpc_error::TRANSPORT));
    assert_eq!(t.in_flight(), 0);
}
