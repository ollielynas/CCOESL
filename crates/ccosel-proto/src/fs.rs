//! Filesystem methods.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Command, Effect, Method, Query, Rpc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

/// Note there is no `path` field: it is redundant with the request, and bytes on the wire are
/// the scarce resource here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub kind: EntryKind,
    /// postcard varints this, so small files cost one byte.
    pub size: u64,
    pub mtime_s: i64,
    /// Whether the caller may change it. There is no `readable`: a listing only ever holds
    /// what the caller may read, so everything in one is.
    pub writable: bool,
}

impl DirEntry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirListing {
    pub entries: Vec<DirEntry>,
    /// The server caps how many entries it will return. A guest must never be handed an
    /// unbounded listing over a bad link, so the cap is part of the contract rather than a
    /// server-side surprise.
    pub truncated: bool,
}

/// Borrowed on the guest (which only serialises) and borrowed out of the request body on the
/// server, so a path costs no allocation on either side.
#[derive(Debug, Serialize, Deserialize)]
pub struct ListDirReq<'a> {
    #[serde(borrow)]
    pub path: &'a str,
}

pub struct ListDir;

impl Rpc for ListDir {
    const METHOD: Method = Method::ListDir;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = ListDirReq<'a>;
    type Reply = DirListing;
}

impl Query for ListDir {}

/// The most text one `ReadFile` or `WriteFile` carries. A document, not a dataset: anything
/// larger belongs on `/files` and `/upload`, which stream rather than buffering in an RPC batch.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// A path, and nothing else. Shared by the methods that only need one.
#[derive(Debug, Serialize, Deserialize)]
pub struct PathReq<'a> {
    #[serde(borrow)]
    pub path: &'a str,
}

/// A text file's contents.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileText {
    pub text: String,
    /// Whether the caller may save over it, so an app can offer "Edit" only when a save
    /// would succeed. The server checks again on write; this is a hint for the UI, not a lock.
    pub writable: bool,
}

/// Read a UTF-8 text file of at most [`MAX_TEXT_BYTES`].
pub struct ReadFile;

impl Rpc for ReadFile {
    const METHOD: Method = Method::ReadFile;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = PathReq<'a>;
    type Reply = FileText;
}

impl Query for ReadFile {}

#[derive(Debug, Serialize, Deserialize)]
pub struct WriteFileReq<'a> {
    #[serde(borrow)]
    pub path: &'a str,
    #[serde(borrow)]
    pub text: &'a str,
    /// Refuse with `EXISTS` rather than overwrite, so "New document" cannot clobber one that
    /// is already there.
    pub create_only: bool,
}

/// Write a text file, replacing it if it exists (unless `create_only`). The folder it goes in
/// must already exist.
pub struct WriteFile;

impl Rpc for WriteFile {
    const METHOD: Method = Method::WriteFile;
    const COALESCE: Coalesce = Coalesce::None;
    const EFFECT: Effect = Effect::Effectful;
    const DEADLINE_MS: u32 = 15_000;
    type Req<'a> = WriteFileReq<'a>;
    type Reply = ();
}

impl Command for WriteFile {}

/// Create one folder. Its parent must exist.
pub struct CreateDir;

impl Rpc for CreateDir {
    const METHOD: Method = Method::CreateDir;
    const COALESCE: Coalesce = Coalesce::None;
    const EFFECT: Effect = Effect::Effectful;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = PathReq<'a>;
    type Reply = ();
}

impl Command for CreateDir {}

/// Delete a file, link or whole folder. The caller needs write access to the folder it is in,
/// to it, and, for a folder, to every folder inside it, so a folder whose own rules grant less
/// can't be removed by removing its parent. The top of the server (`/`) is never removed.
pub struct Remove;

impl Rpc for Remove {
    const METHOD: Method = Method::Remove;
    const COALESCE: Coalesce = Coalesce::None;
    const EFFECT: Effect = Effect::Effectful;
    const DEADLINE_MS: u32 = 15_000;
    type Req<'a> = PathReq<'a>;
    type Reply = ();
}

impl Command for Remove {}

/// What the caller may do with a path, and who the server thinks the caller is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessReply {
    pub read: bool,
    pub write: bool,
    /// The signed-in user, or `None` for an anonymous caller. An app finds the user's private
    /// folder here: it is `/home/{user}`.
    pub user: Option<String>,
}

/// Ask what the caller may do with a path. A path that does not exist and one the caller may
/// not read both answer no access at all, so this cannot be used to find a hidden folder by
/// guessing its name. `user` is answered either way.
pub struct Access;

impl Rpc for Access {
    const METHOD: Method = Method::Access;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 4_000;
    type Req<'a> = PathReq<'a>;
    type Reply = AccessReply;
}

impl Query for Access {}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchReq<'a> {
    /// The folder to search under, recursively.
    #[serde(borrow)]
    pub path: &'a str,
    /// Matched case-insensitively against file names and, for text files, their contents.
    #[serde(borrow)]
    pub query: &'a str,
    /// Only files whose name ends with this, e.g. `".md"`. Empty means every file.
    #[serde(borrow)]
    pub suffix: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    /// Full jail path, e.g. `/Docs/Apps/files.md`.
    pub path: String,
    /// The first matching line, trimmed, or empty when only the name matched.
    pub line: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchReply {
    pub hits: Vec<SearchHit>,
    /// The server stops at a fixed number of hits, like `DirListing::truncated`.
    pub truncated: bool,
}

/// Find files under a folder by name or content. Only files the caller may read are searched,
/// and folders it may not read are not entered.
pub struct Search;

impl Rpc for Search {
    const METHOD: Method = Method::Search;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = SearchReq<'a>;
    type Reply = SearchReply;
}

impl Query for Search {}

/// What a picture says about itself: its size in pixels, and what the camera recorded (EXIF).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInfoReply {
    /// `None` when the server can't read the picture's header.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// What the camera recorded, as `(label, value)` in the order to show them, such as
    /// `("Camera", "Apple iPhone 15 Pro")` or `("Taken", "2024-05-01 12:34:56")`. Only what the
    /// picture actually has: empty for one with no EXIF, such as a screenshot.
    pub fields: Vec<(String, String)>,
}

/// Read a picture's size and EXIF details, for the Viewer. The server reads them because the
/// app never holds the picture's bytes: the browser decodes it.
pub struct ImageInfo;

impl Rpc for ImageInfo {
    const METHOD: Method = Method::ImageInfo;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = PathReq<'a>;
    type Reply = ImageInfoReply;
}

impl Query for ImageInfo {}

/// Where the server has got to making a copy of a file that every browser can show: JPEG for
/// an Apple or TIFF photo, H.264 MP4 for HEVC or ProRes video, AAC for Apple audio. The copy
/// itself is fetched from `/files/<path>?inline=1&as=web` once `finished` without an error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebCopyStatus {
    pub finished: bool,
    /// How far along, in thousandths, or `None` while that isn't known.
    pub permille: Option<u16>,
    /// Why there is no copy, once `finished`.
    pub error: Option<String>,
}

/// Start making the web copy of a file if it isn't made or being made, and say how far along it
/// is. A job: the first call starts it and every later one reports on it, so the app polls. The
/// copy is kept, so asking again for an unchanged file finds it ready at once.
pub struct WebCopy;

impl Rpc for WebCopy {
    const METHOD: Method = Method::WebCopy;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    // Polling an unchanged file reports on the one job, or finds the copy already made.
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = PathReq<'a>;
    type Reply = WebCopyStatus;
}

impl Query for WebCopy {}
