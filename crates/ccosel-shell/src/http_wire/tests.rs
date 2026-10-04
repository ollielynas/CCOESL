//! Turning a reply batch from `/rpc` back into replies. Run under node by `cargo xtask test-wasm`.

use ccosel_proto::{WireReply, WireResult};
use wasm_bindgen_test::wasm_bindgen_test;

use super::*;

fn batch(replies: &[WireReply<'_>]) -> Vec<u8> {
    postcard::to_allocvec(replies).unwrap()
}

/// One reply, as `(seq, result)`.
type Reply = (u32, Result<Vec<u8>, (u32, String)>);

/// Each reply, sorted by seq so the order they came back in doesn't matter.
fn by_seq(incoming: Vec<Incoming>) -> Vec<Reply> {
    let mut out: Vec<_> = incoming.into_iter().map(|i| (i.seq, i.result)).collect();
    out.sort_by_key(|(seq, _)| *seq);
    out
}

#[wasm_bindgen_test]
fn replies_are_matched_to_their_calls_by_seq_not_position() {
    let bytes = batch(&[
        WireReply {
            seq: 2,
            result: WireResult::Err {
                code: 4,
                detail: "/x",
            },
        },
        WireReply {
            seq: 1,
            result: WireResult::Ok(b"ok"),
        },
    ]);
    assert_eq!(
        by_seq(decode_replies(&bytes, &[1, 2])),
        [(1, Ok(b"ok".to_vec())), (2, Err((4, "/x".to_owned())))]
    );
}

#[wasm_bindgen_test]
fn a_call_the_batch_has_no_reply_for_fails_now() {
    let bytes = batch(&[WireReply {
        seq: 1,
        result: WireResult::Ok(b""),
    }]);
    assert_eq!(
        by_seq(decode_replies(&bytes, &[1, 2, 3])),
        [
            (1, Ok(vec![])),
            (2, Err((u32::MAX, "no reply in the batch".to_owned()))),
            (3, Err((u32::MAX, "no reply in the batch".to_owned()))),
        ]
    );
}

#[wasm_bindgen_test]
fn a_reply_to_a_call_that_was_not_asked_is_dropped() {
    let bytes = batch(&[
        WireReply {
            seq: 9,
            result: WireResult::Ok(b"stray"),
        },
        WireReply {
            seq: 1,
            result: WireResult::Ok(b""),
        },
    ]);
    assert_eq!(by_seq(decode_replies(&bytes, &[1])), [(1, Ok(vec![]))]);
}

#[wasm_bindgen_test]
fn garbage_fails_every_call_in_the_batch_rather_than_panicking() {
    for bytes in [&b""[..], b"\xff\xff\xff\xff\xff", b"not postcard at all"] {
        let got = by_seq(decode_replies(bytes, &[4, 5]));
        assert_eq!(got.len(), 2, "{bytes:?}");
        assert!(
            got.iter()
                .all(|(_, r)| r.as_ref().is_err_and(|(code, _)| *code == u32::MAX)),
            "{bytes:?}: {got:?}"
        );
    }
    // An empty batch is a valid one with no replies in it: every call is missing from it.
    let empty = batch(&[]);
    let got = by_seq(decode_replies(&empty, &[4]));
    assert_eq!(
        got,
        [(4, Err((u32::MAX, "no reply in the batch".to_owned())))]
    );
}

#[wasm_bindgen_test]
fn a_failed_request_fails_every_call_in_it_once() {
    assert_eq!(
        by_seq(transport_failures(&[3, 1], "signed out")),
        [
            (1, Err((u32::MAX, "signed out".to_owned()))),
            (3, Err((u32::MAX, "signed out".to_owned()))),
        ]
    );
    assert!(transport_failures(&[], "x").is_empty());
}
