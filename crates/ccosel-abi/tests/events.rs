//! Event batch round-trip and malformed-input tests.
//!
//! The shell is more trusted than a guest, but a bug in batch construction would surface
//! inside the guest as an unexplained app crash. Same bar as the command decoder: no input
//! produces a panic.

use ccosel_abi::event::{
    decode_batch, decode_error, encode_batch, encode_error, event_kind, rpc_error, EventError,
};

#[test]
fn round_trips_a_batch() {
    let a = b"first payload".as_slice();
    let b = b"".as_slice();
    let c = b"third".as_slice();
    let buf = encode_batch(&[
        (event_kind::RPC_OK, 7, a),
        (event_kind::RPC_OK, 8, b),
        (event_kind::RPC_ERR, 9, c),
    ]);

    let events = decode_batch(&buf).unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!((events[0].call_id, events[0].payload), (7, a));
    assert_eq!((events[1].call_id, events[1].payload), (8, b));
    assert_eq!((events[2].kind, events[2].call_id), (event_kind::RPC_ERR, 9));
    assert_eq!(events[2].payload, c);
}

#[test]
fn empty_batch_is_valid() {
    let buf = encode_batch(&[]);
    assert!(decode_batch(&buf).unwrap().is_empty());
}

#[test]
fn errors_round_trip_without_a_decoder() {
    // The whole point of not using postcard here: rendering a failure must never itself be
    // able to fail.
    let payload = encode_error(rpc_error::TIMEOUT, "list_dir took too long");
    let (code, detail) = decode_error(&payload);
    assert_eq!(code, rpc_error::TIMEOUT);
    assert_eq!(detail, "list_dir took too long");

    // Degenerate inputs still return something renderable.
    assert_eq!(decode_error(&[]).0, rpc_error::DECODE);
    assert_eq!(decode_error(&[1, 0, 0, 0, 0xff, 0xfe]).1, "");
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    let buf = encode_batch(&[
        (event_kind::RPC_OK, 1, b"aaaa"),
        (event_kind::RPC_ERR, 2, b"bb"),
    ]);
    for n in 0..buf.len() {
        let _ = decode_batch(&buf[..n]);
    }
}

#[test]
fn rejects_a_count_that_overflows_the_buffer() {
    let mut buf = encode_batch(&[(event_kind::RPC_OK, 1, b"x")]);
    buf[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        decode_batch(&buf),
        Err(EventError::Truncated | EventError::OutOfBounds)
    ));
}

#[test]
fn rejects_a_payload_offset_past_the_end() {
    let mut buf = encode_batch(&[(event_kind::RPC_OK, 1, b"x")]);
    // EventHeader::payload_len sits at byte 12 of the header, which starts at 16.
    buf[16 + 12..16 + 16].copy_from_slice(&9999u32.to_le_bytes());
    assert_eq!(decode_batch(&buf), Err(EventError::OutOfBounds));
}

#[test]
fn rejects_a_payload_region_overlapping_the_headers() {
    let mut buf = encode_batch(&[(event_kind::RPC_OK, 1, b"x")]);
    buf[4..8].copy_from_slice(&0u32.to_le_bytes()); // payload_off = 0
    assert_eq!(decode_batch(&buf), Err(EventError::OutOfBounds));
}

#[test]
fn arbitrary_bytes_never_panic() {
    let mut state = 0xC0FFEEu64;
    for _ in 0..3000 {
        let mut buf = Vec::new();
        for _ in 0..(state % 80) {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            buf.push((state >> 33) as u8);
        }
        let _ = decode_batch(&buf);
        state = state.wrapping_add(1);
    }
}
