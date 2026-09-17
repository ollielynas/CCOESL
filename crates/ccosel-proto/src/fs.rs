//! Filesystem methods.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Effect, Method, Query, Rpc};

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
