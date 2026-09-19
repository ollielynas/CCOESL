//! Round-trip and hostile-input tests for the command stream.
//!
//! The decoder consumes guest output, which is untrusted. The bar these tests hold is not
//! "correct buffers decode correctly" but "no buffer, however malformed, panics".

use ccosel_abi::{
    Align, Cmd, DecodeError, Decoder, Encoder, Layout, MAX_SCOPE_DEPTH, ScopeKind, Vec2, id,
    validate,
};

fn encode(cmds: &[Cmd<'_>]) -> Vec<u8> {
    let mut e = Encoder::new();
    for c in cmds {
        e.push(c);
    }
    e.as_slice().to_vec()
}

fn decode(buf: &[u8]) -> Result<Vec<Cmd<'_>>, DecodeError> {
    Decoder::new(buf).collect()
}

fn sample() -> Vec<Cmd<'static>> {
    let root = id::ROOT;
    let win = id::hash_str(root, "main");
    let row = id::hash_str(win, "row");
    vec![
        Cmd::BeginWindow {
            id: win,
            title: "Files",
            flags: 0,
        },
        Cmd::Label {
            id: id::hash_str(win, "path"),
            text: "/home/shared",
        },
        Cmd::Separator,
        Cmd::BeginScope {
            id: row,
            layout: Layout::new(ScopeKind::Horizontal, Align::Center),
        },
        Cmd::Button {
            id: id::hash_str(row, "up"),
            text: "Up",
        },
        Cmd::Tooltip {
            id: id::hash_str(row, "up"),
            text: "Go to parent directory",
        },
        Cmd::TextEditSingle {
            id: id::hash_str(row, "filter"),
            version: 7,
            set: None,
        },
        Cmd::EndScope { id: row },
        Cmd::Image {
            id: id::hash_str(win, "thumb"),
            src: "/cas/abc123",
            size: Vec2::new(64.0, 64.0),
        },
        Cmd::EndWindow { id: win },
    ]
}

#[test]
fn round_trips() {
    let cmds = sample();
    let buf = encode(&cmds);
    assert_eq!(decode(&buf).unwrap(), cmds);
    validate(&buf).unwrap();
}

#[test]
fn text_edit_omits_the_buffer_by_default() {
    // The whole point of the text protocol: a text field costs a fixed handful of bytes per
    // frame, not the length of the document. Guard against a regression that starts sending
    // the buffer every frame.
    let id = 1234;
    let without = encode(&[Cmd::TextEditSingle {
        id,
        version: 1,
        set: None,
    }]);
    assert_eq!(without.len(), 1 + 8 + 4 + 1);
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    let buf = encode(&sample());
    for n in 0..buf.len() {
        let partial = &buf[..n];
        // Must not panic. Either it decodes a clean prefix or it reports an error.
        let _ = decode(partial);
        let _ = validate(partial);
    }
}

#[test]
fn arbitrary_bytes_never_panic() {
    // Cheap deterministic fuzz. A real guest can emit anything at all.
    let mut state = 0x12345678u64;
    for _ in 0..2000 {
        let mut buf = Vec::new();
        let len = (state % 64) as usize;
        for _ in 0..len {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            buf.push((state >> 33) as u8);
        }
        let _ = decode(&buf);
        let _ = validate(&buf);
        state = state.wrapping_add(1);
    }
}

#[test]
fn rejects_unknown_opcode() {
    assert_eq!(decode(&[0xEE]), Err(DecodeError::UnknownOpcode(0xEE)));
}

#[test]
fn rejects_invalid_utf8() {
    let mut buf = vec![0x01]; // Label
    buf.extend_from_slice(&7u64.to_le_bytes());
    buf.push(2); // varint len = 2
    buf.extend_from_slice(&[0xff, 0xfe]);
    assert_eq!(decode(&buf), Err(DecodeError::InvalidUtf8));
}

#[test]
fn rejects_unbalanced_scopes() {
    let open = encode(&[Cmd::BeginScope {
        id: 1,
        layout: Layout::default(),
    }]);
    assert_eq!(validate(&open), Err(DecodeError::UnclosedScope));

    let close = encode(&[Cmd::EndScope { id: 1 }]);
    assert_eq!(validate(&close), Err(DecodeError::UnbalancedScope));
}

#[test]
fn rejects_mismatched_scope_id() {
    // A guest whose own scope bookkeeping has drifted. Caught before it reaches the host's
    // `Ui` stack.
    let buf = encode(&[
        Cmd::BeginScope {
            id: 1,
            layout: Layout::default(),
        },
        Cmd::EndScope { id: 2 },
    ]);
    assert_eq!(validate(&buf), Err(DecodeError::ScopeIdMismatch));
}

#[test]
fn rejects_crossed_scope_kinds() {
    let buf = encode(&[
        Cmd::BeginScope {
            id: 1,
            layout: Layout::default(),
        },
        Cmd::EndWindow { id: 1 },
    ]);
    assert_eq!(validate(&buf), Err(DecodeError::UnbalancedScope));
}

#[test]
fn rejects_excessive_depth() {
    let mut cmds = Vec::new();
    let n = MAX_SCOPE_DEPTH as u64 + 1;
    for i in 0..n {
        cmds.push(Cmd::BeginScope {
            id: i,
            layout: Layout::default(),
        });
    }
    for i in (0..n).rev() {
        cmds.push(Cmd::EndScope { id: i });
    }
    assert_eq!(validate(&encode(&cmds)), Err(DecodeError::TooDeep));

    // One below the limit is fine.
    let mut ok = Vec::new();
    for i in 0..MAX_SCOPE_DEPTH as u64 {
        ok.push(Cmd::BeginScope {
            id: i,
            layout: Layout::default(),
        });
    }
    for i in (0..MAX_SCOPE_DEPTH as u64).rev() {
        ok.push(Cmd::EndScope { id: i });
    }
    validate(&encode(&ok)).unwrap();
}

#[test]
fn ids_are_stable_and_path_dependent() {
    let a = id::hash_str(id::ROOT, "window");
    assert_eq!(a, id::hash_str(id::ROOT, "window"));
    assert_ne!(a, id::hash_str(id::ROOT, "windos"));
    // Same salt under a different parent must not collide.
    assert_ne!(id::hash_str(a, "ok"), id::hash_str(id::ROOT, "ok"));
}
