//! The egui-shaped recording facade.

use alloc::string::String;
use ccosel_abi::{id as ids, Align, Cmd, FrameInput, Layout, ScopeKind, Vec2, MAX_SCOPE_DEPTH};

use crate::recorder::Recorder;
use crate::rpc::RpcCtx;
use crate::response::Response;

/// Per-frame facts from the shell.
///
/// Carried by value through every nested [`Ui`], so an app can read the clock or the theme
/// without a host call. `low_bandwidth` is the one apps are expected to *act* on: when the
/// shell sets it, degrade (icons instead of thumbnails, summaries instead of full output)
/// rather than queueing more requests onto a link that is already struggling.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameCtx {
    pub time_ms: f64,
    pub dt_ms: f32,
    pub screen_size: Vec2,
    pub flags: u32,
}

impl FrameCtx {
    pub fn from_input(input: &FrameInput) -> Self {
        Self {
            time_ms: input.time_ms,
            dt_ms: input.dt_ms,
            screen_size: Vec2::new(input.screen_size[0], input.screen_size[1]),
            flags: input.flags,
        }
    }

    pub fn dark_mode(&self) -> bool {
        self.flags & ccosel_abi::frame::input_flags::DARK_MODE != 0
    }

    pub fn low_bandwidth(&self) -> bool {
        self.flags & ccosel_abi::frame::input_flags::LOW_BANDWIDTH != 0
    }
}

/// A handle for emitting widgets into a scope.
///
/// Shaped like `egui::Ui`, but it records rather than draws. Nesting works by reborrowing the
/// shared [`Recorder`], so `ui.horizontal(|ui| ..)` costs nothing beyond two opcodes.
pub struct Ui<'a> {
    rec: &'a mut Recorder,
    id: u64,
    /// Auto-id counter, exactly like egui's `next_auto_id_salt`. Widget ids are derived from
    /// call *order* within a scope, so a conditionally-emitted widget shifts the ids of its
    /// siblings. That is the same tradeoff egui makes, and has the same fix: [`Ui::push_id`].
    next_salt: u64,
    depth: u32,
    /// The most recently emitted widget, so [`Ui::tooltip`] can attach to it.
    last_id: u64,
    ctx: FrameCtx,
    rpc: &'a RpcCtx,
}

impl<'a> Ui<'a> {
    /// Begin a frame. The shell calls this; apps receive the `Ui` already built.
    pub fn root(rec: &'a mut Recorder, ctx: FrameCtx, rpc: &'a RpcCtx) -> Self {
        rec.begin_frame();
        Self {
            rec,
            id: ids::ROOT,
            next_salt: 0,
            depth: 0,
            last_id: ids::ROOT,
            ctx,
            rpc,
        }
    }

    /// Talk to the server.
    ///
    /// Returns a shared reference deliberately: `ui.rpc().get(..)` must not borrow `ui`, or
    /// the result could not be drawn while the borrow is live.
    pub fn rpc(&self) -> &'a RpcCtx {
        self.rpc
    }

    /// This frame's context: clock, theme, link quality.
    pub fn ctx(&self) -> FrameCtx {
        self.ctx
    }

    fn auto_id(&mut self) -> u64 {
        let salt = self.next_salt;
        self.next_salt += 1;
        let id = ids::hash_id(self.id, salt);
        self.last_id = id;
        id
    }

    fn response(&mut self, id: u64) -> Response {
        Response::from_record(self.rec.lookup(id))
    }

    /// Run `add` under a stable id salt, so the widgets inside keep their identity even when
    /// siblings appear and disappear. Use this around anything conditional or list-driven.
    pub fn push_id<R>(&mut self, salt: &str, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        let id = ids::hash_str(self.id, salt);
        self.last_id = id;
        let mut child = Ui {
            rec: &mut *self.rec,
            id,
            next_salt: 0,
            depth: self.depth,
            last_id: id,
            ctx: self.ctx,
            rpc: self.rpc,
        };
        add(&mut child)
    }

    fn scope<R>(
        &mut self,
        kind: ScopeKind,
        cross_align: Align,
        add: impl FnOnce(&mut Ui<'_>) -> R,
    ) -> R {
        let id = self.auto_id();

        // Past the depth limit the host would reject the whole frame. Flattening one pathological
        // subtree loses its layout but keeps the rest of the app on screen, which is the better
        // failure for something the user is looking at.
        let emit = self.depth < MAX_SCOPE_DEPTH;
        if emit {
            self.rec.push(&Cmd::BeginScope {
                id,
                layout: Layout::new(kind, cross_align),
            });
        }

        let out = {
            let mut child = Ui {
                rec: &mut *self.rec,
                id,
                next_salt: 0,
                depth: self.depth + 1,
                last_id: id,
                ctx: self.ctx,
                rpc: self.rpc,
            };
            add(&mut child)
        };

        if emit {
            self.rec.push(&Cmd::EndScope { id });
        }
        out
    }

    pub fn horizontal<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::Horizontal, Align::Center, add)
    }

    pub fn vertical<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::Vertical, Align::Min, add)
    }

    /// A visually framed group.
    pub fn group<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::Frame, Align::Min, add)
    }

    /// A floating window, hoisted by the shell to the desktop. Use for dialogs; the app's main
    /// content already lives in a shell-owned window.
    pub fn window<R>(&mut self, title: &str, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        let id = ids::hash_str(self.id, title);
        self.last_id = id;
        self.rec.push(&Cmd::BeginWindow {
            id,
            title,
            flags: 0,
        });
        let out = {
            let mut child = Ui {
                rec: &mut *self.rec,
                id,
                next_salt: 0,
                depth: self.depth + 1,
                last_id: id,
                ctx: self.ctx,
                rpc: self.rpc,
            };
            add(&mut child)
        };
        self.rec.push(&Cmd::EndWindow { id });
        out
    }

    pub fn label(&mut self, text: &str) {
        let id = self.auto_id();
        self.rec.push(&Cmd::Label { id, text });
    }

    pub fn button(&mut self, text: &str) -> Response {
        let id = self.auto_id();
        self.rec.push(&Cmd::Button { id, text });
        self.response(id)
    }

    pub fn separator(&mut self) {
        self.rec.push(&Cmd::Separator);
    }

    /// Attach a tooltip to the widget emitted immediately before this call.
    ///
    /// Emit it *unconditionally* — never inside `if response.hovered()`. The shell decides
    /// hover and draws the tooltip itself, so this has zero latency; a hover-gated version
    /// would lag a frame.
    pub fn tooltip(&mut self, text: &str) {
        let id = self.last_id;
        self.rec.push(&Cmd::Tooltip { id, text });
    }

    /// An image the *shell* fetches by URL.
    ///
    /// Guests must never push pixels through the ABI: routing images this way means they hit
    /// the browser's HTTP cache and never occupy guest linear memory, which is what keeps file
    /// thumbnails from blowing the app's memory budget.
    pub fn image(&mut self, src: &str, size: Vec2) -> Response {
        let id = self.auto_id();
        self.rec.push(&Cmd::Image { id, src, size });
        self.response(id)
    }

    /// A single-line text field. See [`Text`] for why the buffer usually isn't sent.
    pub fn text_edit(&mut self, text: &mut Text) -> Response {
        let id = self.auto_id();
        let set = if text.push_pending {
            Some(text.buf.as_str())
        } else {
            None
        };
        self.rec.push(&Cmd::TextEditSingle {
            id,
            version: text.version,
            set,
        });
        text.push_pending = false;
        self.response(id)
    }
}

/// A text field's contents.
///
/// The shell holds the authoritative buffer along with the cursor, selection, undo stack and
/// IME state — all of which egui already implements, and none of which survives a trip through
/// a command stream. So the common case sends **no text at all**: just an id and a version,
/// thirteen bytes, regardless of how long the document is. That is what makes a code editor in
/// the Compiler app viable.
///
/// The guest's copy is updated from deltas the shell sends back. Calling [`Text::set`] pushes
/// the other way, and wins by carrying a higher version.
#[derive(Clone, Debug, Default)]
pub struct Text {
    buf: String,
    version: u32,
    push_pending: bool,
}

impl Text {
    pub fn new(initial: &str) -> Self {
        Self {
            buf: String::from(initial),
            version: 1,
            // An empty initial buffer needs no push: the shell's own default is already
            // empty, so sending one would cost bytes to say nothing. It also means an app
            // whose fields start empty emits byte-identical frames from the very first one,
            // which is what subtree caching will later depend on.
            push_pending: !initial.is_empty(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.buf
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    /// Overwrite the field, authoritatively. Bumps the version so the shell accepts it.
    pub fn set(&mut self, s: &str) {
        self.buf.clear();
        self.buf.push_str(s);
        self.version += 1;
        self.push_pending = true;
    }

    /// Apply a delta from the shell. Called by the frame glue, not by app code.
    pub fn apply_delta(&mut self, version: u32, start: usize, end: usize, inserted: &str) {
        if version <= self.version || start > end || end > self.buf.len() {
            return;
        }
        if !self.buf.is_char_boundary(start) || !self.buf.is_char_boundary(end) {
            return;
        }
        self.buf.replace_range(start..end, inserted);
        self.version = version;
    }
}
