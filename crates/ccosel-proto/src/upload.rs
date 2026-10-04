//! `POST /upload`: one file per request, into a jail directory.
//!
//! Files are sent whole, not chunked. The largest one is capped at [`MAX_FILE_BYTES`], which
//! the server enforces as the request's body limit (it buffers the body before writing it)
//! and the shell checks first, so a file that would be refused is never read into the
//! browser's memory at all. Chunking would lift the cap but needs resumable server-side state;
//! for source folders and the odd binary or image, the cap is the simpler answer.

/// The largest file `POST /upload` accepts, in bytes.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
