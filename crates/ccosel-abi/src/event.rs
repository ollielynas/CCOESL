//! Asynchronous deliveries from the shell to a guest.
//!
//! Delivered through `ccosel_on_event`, which the shell calls **never during `ccosel_frame`**,
//! and always drains before the next one. Guests therefore see all outstanding events before
//! they next render, and after `ccosel_restore_state` before their first frame.
//!
//! The payload is **opaque here**. `ccosel-proto` decodes it. That is deliberate: it means
//! adding an RPC method never touches `ABI_VERSION`, so it never invalidates a module a client
//! has already cached — and a guest that makes no RPC calls links no decoder at all.
//!
//! Events arrive in **batches**. Each `ccosel_on_event` call costs an allocation, a write, a
//! guest call and a free on both backends, and coalesced replies naturally arrive in groups.
//!
//! Layout: `EventBatch` | `EventHeader[count]` | payload bytes.

use bytemuck::{Pod, Zeroable};

#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct EventBatch {
    pub count: u32,
    /// Offset from the start of the buffer to the payload region.
    pub payload_off: u32,
    pub _pad: [u32; 2],
}

#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct EventHeader {
    /// See [`event_kind`].
    pub kind: u32,
    /// The id the *guest* chose when it issued the call. Zero for events that are not replies.
    pub call_id: u32,
    /// Offset relative to `EventBatch::payload_off`.
    pub payload_off: u32,
    pub payload_len: u32,
}

pub mod event_kind {
    /// Payload is the postcard-encoded reply type for the method that was called.
    pub const RPC_OK: u32 = 1;
    /// Payload is a `u32` code (see [`super::rpc_error`]) followed by UTF-8 detail, possibly
    /// empty. Deliberately *not* postcard: an app must be able to render a failure without
    /// linking a decoder for it.
    pub const RPC_ERR: u32 = 2;
    /// Reserved: shell→guest text buffer deltas.
    pub const TEXT_DELTA: u32 = 3;
    /// Reserved: sent after `ccosel_restore_state`, before the first frame.
    pub const RESTORED: u32 = 4;
}

/// Why a call failed.
///
/// The shell guarantees **exactly one terminal event per `call_id`** — an `RPC_OK` or one of
/// these — unless the guest cancelled, in which case none. So an app's state machine has no
/// "might wait forever" branch, which is the branch app authors forget to write.
pub mod rpc_error {
    /// The shell refused to queue the call: backpressure, or an unknown method.
    pub const REJECTED: u32 = 1;
    /// Capability check failed.
    pub const DENIED: u32 = 2;
    /// The shell's deadline for this method expired.
    pub const TIMEOUT: u32 = 3;
    /// The link dropped and the call is not safe to retry.
    pub const TRANSPORT: u32 = 4;
    /// The server reported a typed failure; the detail carries it.
    pub const SERVER: u32 = 5;
    /// The reply did not match the expected schema.
    pub const DECODE: u32 = 6;
    pub const CANCELLED: u32 = 7;
    /// Superseded by a newer call under `Coalesce::Latest`.
    pub const SHED: u32 = 8;
}

/// One decoded event, borrowing its payload from the batch buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event<'a> {
    pub kind: u32,
    pub call_id: u32,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventError {
    Truncated,
    /// An offset or length pointed outside the buffer.
    OutOfBounds,
}

const BATCH_SIZE: usize = core::mem::size_of::<EventBatch>();
const HEADER_SIZE: usize = core::mem::size_of::<EventHeader>();

/// Decode a batch.
///
/// The shell is more trusted than a guest, but this still validates everything: a bug here
/// would be a panic inside a guest, which surfaces as an unexplained app crash. Every offset
/// is bounds-checked against the actual buffer.
pub fn decode_batch(buf: &[u8]) -> Result<alloc::vec::Vec<Event<'_>>, EventError> {
    use alloc::vec::Vec;

    if buf.len() < BATCH_SIZE {
        return Err(EventError::Truncated);
    }
    let batch: EventBatch = *bytemuck::from_bytes(&buf[..BATCH_SIZE]);

    let count = batch.count as usize;
    let headers_end = BATCH_SIZE
        .checked_add(
            count
                .checked_mul(HEADER_SIZE)
                .ok_or(EventError::OutOfBounds)?,
        )
        .ok_or(EventError::OutOfBounds)?;
    if headers_end > buf.len() {
        return Err(EventError::Truncated);
    }

    let payload_base = batch.payload_off as usize;
    if payload_base > buf.len() || payload_base < headers_end {
        return Err(EventError::OutOfBounds);
    }
    let payloads = &buf[payload_base..];

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let at = BATCH_SIZE + i * HEADER_SIZE;
        let header: EventHeader = *bytemuck::from_bytes(&buf[at..at + HEADER_SIZE]);

        let start = header.payload_off as usize;
        let end = start
            .checked_add(header.payload_len as usize)
            .ok_or(EventError::OutOfBounds)?;
        if end > payloads.len() {
            return Err(EventError::OutOfBounds);
        }

        out.push(Event {
            kind: header.kind,
            call_id: header.call_id,
            payload: &payloads[start..end],
        });
    }
    Ok(out)
}

/// Build a batch. Used by the shell; lives here so both sides share one definition.
pub fn encode_batch(events: &[(u32, u32, &[u8])]) -> alloc::vec::Vec<u8> {
    use alloc::vec::Vec;

    let headers_end = BATCH_SIZE + events.len() * HEADER_SIZE;
    let mut headers: Vec<EventHeader> = Vec::with_capacity(events.len());
    let mut payloads: Vec<u8> = Vec::new();

    for (kind, call_id, payload) in events {
        headers.push(EventHeader {
            kind: *kind,
            call_id: *call_id,
            payload_off: payloads.len() as u32,
            payload_len: payload.len() as u32,
        });
        payloads.extend_from_slice(payload);
    }

    let batch = EventBatch {
        count: events.len() as u32,
        payload_off: headers_end as u32,
        _pad: [0; 2],
    };

    let mut out = Vec::with_capacity(headers_end + payloads.len());
    out.extend_from_slice(bytemuck::bytes_of(&batch));
    for h in &headers {
        out.extend_from_slice(bytemuck::bytes_of(h));
    }
    out.extend_from_slice(&payloads);
    out
}

/// Encode an `RPC_ERR` payload: code, then UTF-8 detail.
pub fn encode_error(code: u32, detail: &str) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::with_capacity(4 + detail.len());
    out.extend_from_slice(&code.to_le_bytes());
    out.extend_from_slice(detail.as_bytes());
    out
}

/// Decode an `RPC_ERR` payload. Returns the code and the detail, lossily if it is not UTF-8 —
/// an error path must never itself fail.
pub fn decode_error(payload: &[u8]) -> (u32, &str) {
    if payload.len() < 4 {
        return (rpc_error::DECODE, "");
    }
    let code = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let detail = core::str::from_utf8(&payload[4..]).unwrap_or("");
    (code, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    #[test]
    fn layout_is_pinned() {
        assert_eq!(size_of::<EventBatch>(), 16);
        assert_eq!(size_of::<EventHeader>(), 16);
    }
}
