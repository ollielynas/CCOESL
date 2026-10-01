//! A native harness for testing apps, behind the `testing` feature.
//!
//! In production the shell supplies an app's recorder, frame context and RPC cache, and reaches
//! it through raw `u32` wasm pointers — which cannot be dereferenced on a 64-bit native target.
//! This wires those pieces together directly, so a test can drive an app the way a user and the
//! server would: run a frame, click a button by its label, answer an RPC, run the next frame,
//! and assert on what was drawn.
//!
//! ```ignore
//! let mut h = Harness::new(MyApp::default());
//! h.frame();                                   // the app asks the server for something
//! h.reply::<ListDir>(&DirListing { .. });      // the server answers
//! h.frame();
//! assert!(h.has_label("notes.md"));
//! h.click("Refresh");                          // observed by the app on the next frame
//! h.frame();
//! ```
//!
//! Off by default: this is test scaffolding and the SDK's budget is measured in kilobytes. Apps
//! enable it as a dev-dependency only, so it never reaches a shipped module.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use ccosel_abi::event::{Event, TextDelta, encode_error, encode_text_delta, event_kind};
use ccosel_abi::{Cmd, Decoder, RespRecord, ResponseFlags, ScopeKind, TextStyle};

/// The codes [`Harness::fail`] takes, re-exported so an app's tests need no `ccosel-abi`
/// dependency of their own.
pub use ccosel_abi::event::rpc_error;
use ccosel_proto::Rpc;
use serde::Serialize;

use crate::rpc::OutCall;
use crate::{App, FrameCtx, Recorder, RpcCtx, Ui};

pub struct Harness<A: App> {
    /// The app under test. Public so a test can set up or inspect its state directly.
    pub app: A,
    /// Inputs (time, screen size, flags) the next frame will see.
    pub ctx: FrameCtx,
    rec: Recorder,
    rpc: RpcCtx,
    clicks: Vec<RespRecord>,
    /// Per upload button id, the `aux` the "shell" reports in its response every frame, as the
    /// real shell does: a finished count for `UploadFolder`, a folder id for `UploadProject`.
    uploads_finished: Vec<(u64, u32)>,
    calls: Vec<OutCall>,
    last: Vec<u8>,
    /// The "shell's" copy of each text field, `(text, version)`, kept by the same rules as the
    /// real one so [`Harness::type_text`] produces the deltas a real shell would.
    texts: BTreeMap<u64, (String, u32)>,
}

impl<A: App> Harness<A> {
    pub fn new(app: A) -> Self {
        Self {
            app,
            ctx: FrameCtx::default(),
            rec: Recorder::new(),
            rpc: RpcCtx::new(),
            clicks: Vec::new(),
            uploads_finished: Vec::new(),
            calls: Vec::new(),
            last: Vec::new(),
            texts: BTreeMap::new(),
        }
    }

    /// Runs one frame, as the runtime would: clicks queued since the last frame are reported to
    /// the app now (a response describes the *previous* frame), and any RPC the app issues is
    /// held until the test answers it.
    ///
    /// Panics if the app emits a command buffer the host would reject, so every frame a test
    /// runs is also checked for well-formedness.
    pub fn frame(&mut self) {
        let mut responses = core::mem::take(&mut self.clicks);
        for &(id, n) in &self.uploads_finished {
            match responses.iter_mut().find(|r| r.local_id == id) {
                Some(r) => r.aux = n,
                None => responses.push(RespRecord {
                    local_id: id,
                    aux: n,
                    flags: ResponseFlags::ENABLED,
                    ..Default::default()
                }),
            }
        }
        self.rec.set_responses(responses);
        self.rpc.begin_frame();
        {
            let mut ui = Ui::root(&mut self.rec, self.ctx, &self.rpc);
            self.app.update(&mut ui);
        }
        self.calls.extend(self.rpc.take_outbox());
        // The runtime forwards cancellations to the host; there is no host here to tell.
        let _ = self.rpc.take_cancels();
        self.last = self.rec.commands().to_vec();
        ccosel_abi::validate(&self.last).expect("the app emitted a malformed command buffer");
        self.sync_texts();
    }

    /// Mirror the shell's text bookkeeping: take the guest's `set`s, and adopt its version for
    /// a field seen for the first time.
    fn sync_texts(&mut self) {
        let fields: Vec<(u64, u32, Option<String>)> = self
            .commands()
            .filter_map(|c| match c {
                Cmd::TextEditSingle { id, version, set }
                | Cmd::TextEditMulti { id, version, set } => {
                    Some((id, version, set.map(ToString::to_string)))
                }
                _ => None,
            })
            .collect();
        for (id, version, set) in fields {
            match (set, self.texts.get_mut(&id)) {
                (Some(text), _) => {
                    self.texts.insert(id, (text, version));
                }
                (None, None) => {
                    self.texts.insert(id, (String::new(), version));
                }
                (None, Some(_)) => {}
            }
        }
    }

    fn text_field_ids(&self) -> Vec<u64> {
        self.commands()
            .filter_map(|c| match c {
                Cmd::TextEditSingle { id, .. } | Cmd::TextEditMulti { id, .. } => Some(id),
                _ => None,
            })
            .collect()
    }

    /// What each text field drawn in the last frame holds, as the shell sees it, in order.
    pub fn text_fields(&self) -> Vec<String> {
        self.text_field_ids()
            .iter()
            .map(|id| self.texts.get(id).map(|t| t.0.clone()).unwrap_or_default())
            .collect()
    }

    /// Plays the user replacing everything in the `index`th text field of the last frame
    /// (counting single- and multi-line fields together) with `text`. The app sees it the next
    /// time it draws that field, as with a real shell.
    pub fn type_text(&mut self, index: usize, text: &str) {
        let ids = self.text_field_ids();
        let id = *ids.get(index).unwrap_or_else(|| {
            panic!(
                "no text field {index} in the last frame; there were {}",
                ids.len()
            )
        });
        let (old, version) = self.texts.entry(id).or_default();
        *version += 1;
        let payload = encode_text_delta(&TextDelta {
            id,
            version: *version,
            start: 0,
            end: old.len() as u32,
            inserted: text,
        });
        *old = text.to_string();
        crate::runtime::deliver(
            &self.rpc,
            &mut self.rec,
            &Event {
                kind: event_kind::TEXT_DELTA,
                call_id: 0,
                payload: &payload,
            },
        );
    }

    /// Plays the user pressing Enter in the `index`th text field of the last frame, as
    /// [`type_text`](Self::type_text) counts them. The app sees [`Response::submitted`] on the
    /// next [`frame`](Self::frame).
    ///
    /// [`Response::submitted`]: crate::Response::submitted
    pub fn press_enter(&mut self, index: usize) {
        let ids = self.text_field_ids();
        let id = *ids.get(index).unwrap_or_else(|| {
            panic!(
                "no text field {index} in the last frame; there were {}",
                ids.len()
            )
        });
        self.clicks.push(RespRecord {
            local_id: id,
            flags: ResponseFlags::SUBMITTED | ResponseFlags::LOST_FOCUS | ResponseFlags::ENABLED,
            ..Default::default()
        });
    }

    /// Every styled run drawn in the last frame, in order.
    pub fn styled(&self) -> Vec<(String, TextStyle)> {
        self.commands()
            .filter_map(|c| match c {
                Cmd::Styled { text, style, .. } => Some((text.to_string(), style)),
                _ => None,
            })
            .collect()
    }

    /// Whether any label or styled run in the last frame reads exactly `text`.
    pub fn has_text(&self, text: &str) -> bool {
        self.has_label(text) || self.styled().iter().any(|(t, _)| t == text)
    }

    /// Clicks the first styled *link* reading `text`. Panics, listing the links, if none does.
    pub fn click_link(&mut self, text: &str) {
        let id = self
            .commands()
            .find_map(|c| match c {
                Cmd::Styled { id, text: t, style }
                    if t == text && style.contains(TextStyle::LINK) =>
                {
                    Some(id)
                }
                _ => None,
            })
            .unwrap_or_else(|| {
                let links: Vec<String> = self
                    .styled()
                    .into_iter()
                    .filter(|(_, s)| s.contains(TextStyle::LINK))
                    .map(|(t, _)| t)
                    .collect();
                panic!("no link reading {text:?} in the last frame; links were {links:?}")
            });
        if !self.disabled_ids().contains(&id) {
            self.press(id);
        }
    }

    fn commands(&self) -> impl Iterator<Item = Cmd<'_>> {
        Decoder::new(&self.last).map(|c| c.expect("last frame decodes"))
    }

    /// Text of every label drawn in the last frame, in order.
    pub fn labels(&self) -> Vec<String> {
        self.commands()
            .filter_map(|c| match c {
                Cmd::Label { text, .. } => Some(text.to_string()),
                _ => None,
            })
            .collect()
    }

    /// Text of every button drawn in the last frame, in order.
    pub fn buttons(&self) -> Vec<String> {
        self.commands()
            .filter_map(|c| match c {
                Cmd::Button { text, .. } => Some(text.to_string()),
                _ => None,
            })
            .collect()
    }

    /// Text and selected state of every selectable row drawn in the last frame, in order.
    pub fn selectables(&self) -> Vec<(String, bool)> {
        self.commands()
            .filter_map(|c| match c {
                Cmd::Selectable { text, selected, .. } => Some((text.to_string(), selected)),
                _ => None,
            })
            .collect()
    }

    /// Samples of every plot drawn in the last frame, in order.
    pub fn plots(&self) -> Vec<Vec<u8>> {
        self.commands()
            .filter_map(|c| match c {
                Cmd::Plot { samples, .. } => Some(samples.to_vec()),
                _ => None,
            })
            .collect()
    }

    /// Requested size of every plot drawn in the last frame, in order.
    pub fn plot_sizes(&self) -> Vec<ccosel_abi::Vec2> {
        self.commands()
            .filter_map(|c| match c {
                Cmd::Plot { size, .. } => Some(size),
                _ => None,
            })
            .collect()
    }

    /// Destination folder of every `upload_folder` button drawn in the last frame, in order.
    pub fn upload_buttons(&self) -> Vec<String> {
        self.commands()
            .filter_map(|c| match c {
                Cmd::UploadFolder { dest, .. } => Some(dest.to_string()),
                _ => None,
            })
            .collect()
    }

    /// `(label, url)` of every `open_url` button drawn in the last frame, in order.
    pub fn open_urls(&self) -> Vec<(String, String)> {
        self.commands()
            .filter_map(|c| match c {
                Cmd::OpenUrl { label, url, .. } => Some((label.to_string(), url.to_string())),
                _ => None,
            })
            .collect()
    }

    /// Plays the shell finishing an upload started from the first `upload_folder` button in
    /// the last frame. The app sees it on the next [`frame`](Self::frame).
    pub fn finish_upload(&mut self) {
        let id = self
            .commands()
            .find_map(|c| match c {
                Cmd::UploadFolder { id, .. } => Some(id),
                _ => None,
            })
            .expect("no upload_folder button in the last frame");
        match self.uploads_finished.iter_mut().find(|(i, _)| *i == id) {
            Some((_, n)) => *n += 1,
            None => self.uploads_finished.push((id, 1)),
        }
    }

    /// Whether the last frame drew an `upload_project` button.
    pub fn has_project_upload(&self) -> bool {
        self.commands()
            .any(|c| matches!(c, Cmd::UploadProject { .. }))
    }

    /// Plays the shell finishing a project upload, into temporary folder `folder_id`, from the
    /// first `upload_project` button in the last frame. The app sees it on the next
    /// [`frame`](Self::frame).
    pub fn finish_project_upload(&mut self, folder_id: u32) {
        let id = self
            .commands()
            .find_map(|c| match c {
                Cmd::UploadProject { id } => Some(id),
                _ => None,
            })
            .expect("no upload_project button in the last frame");
        match self.uploads_finished.iter_mut().find(|(i, _)| *i == id) {
            Some((_, n)) => *n = folder_id,
            None => self.uploads_finished.push((id, folder_id)),
        }
    }

    pub fn has_label(&self, text: &str) -> bool {
        self.labels().iter().any(|l| l == text)
    }

    pub fn has_button(&self, text: &str) -> bool {
        self.buttons().iter().any(|b| b == text)
    }

    /// Ids of the widgets the last frame drew inside a disabled scope ([`Ui::enabled`]).
    fn disabled_ids(&self) -> BTreeSet<u64> {
        let mut out = BTreeSet::new();
        // Per open scope, whether it disables what is in it.
        let mut stack: Vec<bool> = Vec::new();
        for c in self.commands() {
            match c {
                Cmd::BeginScope { layout, .. } => stack.push(layout.kind == ScopeKind::Disabled),
                Cmd::EndScope { .. } => {
                    stack.pop();
                }
                Cmd::Button { id, .. } | Cmd::Selectable { id, .. } if stack.contains(&true) => {
                    out.insert(id);
                }
                _ => {}
            }
        }
        out
    }

    /// Whether the first button, or selectable row, with this label in the last frame is
    /// greyed out. Panics if there is no such widget.
    pub fn is_disabled(&self, text: &str) -> bool {
        let id = self
            .commands()
            .find_map(|c| match c {
                Cmd::Button { id, text: t } | Cmd::Selectable { id, text: t, .. } if t == text => {
                    Some(id)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no button labelled {text:?} in the last frame"));
        self.disabled_ids().contains(&id)
    }

    /// Clicks the first button, or selectable row, with this label in the last frame. The app
    /// observes it on the next [`frame`](Self::frame). Panics, listing what was drawn, if there
    /// is no such widget. A greyed-out one ignores the click, as in the shell.
    pub fn click(&mut self, text: &str) {
        let id = self
            .commands()
            .find_map(|c| match c {
                Cmd::Button { id, text: t } | Cmd::Selectable { id, text: t, .. } if t == text => {
                    Some(id)
                }
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "no button labelled {text:?} in the last frame; buttons were {:?}, \
                     selectable rows {:?}",
                    self.buttons(),
                    self.selectables()
                )
            });
        self.press(id);
    }

    fn press(&mut self, id: u64) {
        self.clicks.push(RespRecord {
            local_id: id,
            flags: ResponseFlags::CLICKED | ResponseFlags::HOVERED | ResponseFlags::ENABLED,
            ..Default::default()
        });
    }

    /// How many calls to `M` the app has issued that the test has not yet answered.
    pub fn outstanding<M: Rpc>(&self) -> usize {
        self.calls
            .iter()
            .filter(|c| c.method == M::METHOD as u32)
            .count()
    }

    /// Answers the oldest outstanding call to `M` with `reply`. Panics if there is none.
    pub fn reply<M: Rpc>(&mut self, reply: &M::Reply)
    where
        M::Reply: Serialize,
    {
        let payload = postcard::to_allocvec(reply).expect("the reply serializes");
        self.answer::<M>(event_kind::RPC_OK, &payload);
    }

    /// Fails the oldest outstanding call to `M` with an `rpc_error` code. Panics if there is none.
    pub fn fail<M: Rpc>(&mut self, code: u32) {
        let payload = encode_error(code, "injected by the test harness");
        self.answer::<M>(event_kind::RPC_ERR, &payload);
    }

    fn answer<M: Rpc>(&mut self, kind: u32, payload: &[u8]) {
        let at = self
            .calls
            .iter()
            .position(|c| c.method == M::METHOD as u32)
            .expect("the app has no outstanding call to answer");
        let call = self.calls.remove(at);
        crate::runtime::deliver(
            &self.rpc,
            &mut self.rec,
            &Event {
                kind,
                call_id: call.call_id,
                payload,
            },
        );
    }
}
