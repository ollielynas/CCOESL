//! Widget identity.
//!
//! The guest must **not** reimplement egui's hasher — it would drift silently on every egui
//! upgrade and every app would break in a way that looks like a layout bug. Instead the guest
//! computes a *local id* with the pinned hasher below, and the host derives the real
//! `egui::Id` as `Id::new((app_instance_id, local_id))`.
//!
//! Consequences: response records are keyed by local id so a guest can find its own widgets
//! knowing nothing about egui; identity survives egui upgrades; and id collisions are the app
//! author's problem in exactly the way they already are in egui.
//!
//! This hasher is part of the ABI. Changing it changes `ABI_VERSION`.

/// FxHash's multiplier. Pinned here forever.
const SEED: u64 = 0x517c_c1b7_2722_0a95;
const ROTATE: u32 = 5;

/// Mix one 64-bit word into a hash state.
#[inline]
pub const fn hash_u64(state: u64, word: u64) -> u64 {
    (state.rotate_left(ROTATE) ^ word).wrapping_mul(SEED)
}

/// Derive a child id from a parent id and a numeric salt.
#[inline]
pub const fn hash_id(parent: u64, salt: u64) -> u64 {
    hash_u64(parent, salt)
}

#[inline]
pub const fn hash_bytes(mut state: u64, bytes: &[u8]) -> u64 {
    // const-compatible: no iterators.
    let mut i = 0;
    while i < bytes.len() {
        state = hash_u64(state, bytes[i] as u64);
        i += 1;
    }
    state
}

/// Derive a child id from a parent id and a string salt — the common case
/// (`ui.button("Save")` salts with `"Save"`).
#[inline]
pub const fn hash_str(parent: u64, s: &str) -> u64 {
    hash_bytes(parent, s.as_bytes())
}

/// Root id for an app's widget tree. Every guest starts here.
///
/// It must not be `0`. The hasher maps `(0, 0)` to `0`, so with a zero root an app's first
/// auto-id (usually a top-level `horizontal`) *was* the root, and the `n`th widget inside that
/// scope got the same id as the `n`th top-level widget: one click reached both. Any root that
/// is not a fixed point of `hash_id(_, 0)` avoids that; this one is checked by a test.
pub const ROOT: u64 = hash_str(0, "ccosel-root");
