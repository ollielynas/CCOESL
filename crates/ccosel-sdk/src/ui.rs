//! The egui-shaped recording facade.

use alloc::string::String;
use ccosel_abi::{
    Align, Cmd, CodeLang, FrameInput, Layout, MAX_SCOPE_DEPTH, MediaKind, ScopeKind, TextStyle,
    Vec2, id as ids,
};

use crate::recorder::Recorder;
use crate::response::Response;
use crate::rpc::RpcCtx;

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

/// Facts about the page the app runs in, which only the shell knows: they arrive once, before
/// the first frame (see `event_kind::PAGE_INFO`). Unlike [`FrameCtx`] they do not change from
/// frame to frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageInfo {
    origin: Option<String>,
}

impl PageInfo {
    /// The page's origin as the browser has it, such as `https://example.com` or
    /// `http://192.168.1.20:8777`: what to put in front of a path for a link that works from
    /// another machine. Behind a tunnel this is its public address.
    ///
    /// `None` until the shell has said, which an older shell never does, so an app must still
    /// work without it.
    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    /// Take what a `PAGE_INFO` event says. Keys it does not mention keep their values.
    pub(crate) fn apply(&mut self, payload: &[u8]) {
        if let Some(origin) = ccosel_abi::event::page_info(payload, "origin") {
            self.origin = Some(String::from(origin));
        }
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
    page: &'a PageInfo,
}

impl<'a> Ui<'a> {
    /// Begin a frame. The shell calls this; apps receive the `Ui` already built.
    pub fn root(rec: &'a mut Recorder, ctx: FrameCtx, rpc: &'a RpcCtx, page: &'a PageInfo) -> Self {
        rec.begin_frame();
        Self {
            rec,
            id: ids::ROOT,
            next_salt: 0,
            depth: 0,
            last_id: ids::ROOT,
            ctx,
            rpc,
            page,
        }
    }

    /// The page the app runs in, such as its [origin](PageInfo::origin).
    pub fn page(&self) -> &'a PageInfo {
        self.page
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
            page: self.page,
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
                page: self.page,
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

    /// A row that wraps like a paragraph, with no gap between children: put [`Ui::styled`]
    /// runs in it (each carrying its own spaces) to lay out formatted text.
    pub fn wrapped<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::Wrapped, Align::Min, add)
    }

    /// A row whose children line up along the top rather than the middle. With a
    /// [`Ui::side_column`] first and a [`Ui::vertical`] second, it is a sidebar layout:
    ///
    /// ```ignore
    /// ui.horizontal_top(|ui| {
    ///     ui.side_column(|ui| { /* navigation */ });
    ///     ui.vertical(|ui| { /* the page */ });
    /// });
    /// ```
    pub fn horizontal_top<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::Horizontal, Align::Min, add)
    }

    /// A fixed-width column with a rule after it, for a sidebar. Its width does not follow its
    /// content, so the page beside it stays put. See [`Ui::horizontal_top`].
    pub fn side_column<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::Sidebar, Align::Min, add)
    }

    /// A region that scrolls by itself, taking the rest of the window's height. Use one for each
    /// column that should scroll on its own, such as a sidebar and the page beside it.
    pub fn scroll<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::Scroll, Align::Min, add)
    }

    /// A terminal: `body` scrolls, and `footer` is a row pinned below it, such as a prompt.
    /// Together they take the rest of the window's height, so the footer stays in view however
    /// long the body grows. The body keeps its newest (bottom) line in view as lines arrive,
    /// unless the user has scrolled up to read back.
    ///
    /// A single-line field in the footer keeps the keyboard focus when Enter is pressed in it,
    /// so commands can be typed one after another. See [`Response::submitted`].
    pub fn scroll_with_footer<R>(
        &mut self,
        body: impl FnOnce(&mut Ui<'_>),
        footer: impl FnOnce(&mut Ui<'_>) -> R,
    ) -> R {
        self.scope(ScopeKind::ScrollFooter, Align::Min, |ui| {
            ui.vertical(body);
            ui.horizontal(footer)
        })
    }

    /// Children that can be used only when `enabled`: otherwise the shell greys them out and
    /// they ignore clicks and typing, so a button that would do nothing says so. The layout
    /// and the widgets' ids are the same either way.
    pub fn enabled<R>(&mut self, enabled: bool, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        let kind = if enabled {
            ScopeKind::Group
        } else {
            ScopeKind::Disabled
        };
        self.scope(kind, Align::Min, add)
    }

    /// Children indented one step, for nested lists and trees.
    pub fn indent<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::Indent, Align::Min, add)
    }

    /// A visually framed group.
    pub fn group<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::Frame, Align::Min, add)
    }

    /// A table. Add each row with [`Ui::row`], and the cells of a row inside it: cells line up
    /// in columns, and a table wider than the window scrolls sideways.
    ///
    /// ```ignore
    /// ui.table(|ui| {
    ///     for record in &rows {
    ///         ui.row(|ui| for cell in record { ui.label(cell); });
    ///     }
    /// });
    /// ```
    pub fn table<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::Table, Align::Min, add)
    }

    /// One row of a [`Ui::table`]: each widget added inside it is a cell.
    pub fn row<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::Group, Align::Min, add)
    }

    /// A right-click menu on the widget added just before this call. What `add` draws are the
    /// menu's entries: they appear only while it is open, and a click on one closes it.
    ///
    /// Call it unconditionally, like [`Ui::tooltip`]: the shell decides when the menu opens.
    pub fn context_menu<R>(&mut self, add: impl FnOnce(&mut Ui<'_>) -> R) -> R {
        self.scope(ScopeKind::ContextMenu, Align::Min, add)
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
                page: self.page,
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

    /// A line graph of `samples`, oldest first, each already scaled to `0..=255` (bottom to
    /// top). Scaling is the app's job so the wire carries one byte per point; label the range
    /// with ordinary [`Ui::label`]s. An `x` of `0.0` fills the row's width.
    pub fn plot(&mut self, samples: &[u8], size: Vec2) -> Response {
        let id = self.auto_id();
        self.rec.push(&Cmd::Plot { id, size, samples });
        self.response(id)
    }

    /// Audio, video or a PDF from `src`, played or shown by the browser itself, with its own
    /// controls. Which formats work is up to the browser.
    ///
    /// A size component of `0.0` is the shell's choice: the row's width, and a height that suits
    /// `kind` (16:9 for video, a control bar for audio, the rest of the window for a document).
    pub fn media(&mut self, src: &str, kind: MediaKind, size: Vec2) -> Response {
        let id = self.auto_id();
        self.rec.push(&Cmd::Media {
            id,
            src,
            kind,
            size,
        });
        self.response(id)
    }

    /// A single-line text field. See [`Text`] for why the buffer usually isn't sent.
    pub fn text_edit(&mut self, text: &mut Text) -> Response {
        self.text_field(text, Field::Single)
    }

    /// A multi-line, monospace text area for documents and code. Same protocol as
    /// [`Ui::text_edit`]: typing costs the guest a small delta, not the document.
    pub fn text_edit_multiline(&mut self, text: &mut Text) -> Response {
        self.text_field(text, Field::Multi)
    }

    /// [`Ui::text_edit_multiline`] for source code in `lang`, which the shell colours as it
    /// is typed: keywords, strings, comments, numbers. The colouring costs the guest nothing;
    /// the shell does it from the text it already holds.
    pub fn code_editor(&mut self, text: &mut Text, lang: CodeLang) -> Response {
        self.text_field(text, Field::Code(lang, None))
    }

    /// [`Ui::code_editor`] with an error marked at `line` and `column` (both from 1; column 0
    /// for the whole line): the shell shades the line and underlines it from the column. Say
    /// what the error was beside it; the mark only shows where.
    pub fn code_editor_marked(
        &mut self,
        text: &mut Text,
        lang: CodeLang,
        line: u32,
        column: u32,
    ) -> Response {
        self.text_field(text, Field::Code(lang, Some((line, column))))
    }

    /// Read-only, multi-line, monospace text that the user can select and copy, for showing a
    /// file. Like a text field, it costs the whole text only in the frame after [`Text::set`].
    pub fn text_view(&mut self, text: &mut Text) -> Response {
        let id = self.auto_id();
        let set = text.push_pending.then_some(text.buf.as_str());
        self.rec.push(&Cmd::TextView {
            id,
            version: text.version,
            set,
        });
        text.push_pending = false;
        self.response(id)
    }

    fn text_field(&mut self, text: &mut Text, field: Field) -> Response {
        let id = self.auto_id();
        // Apply what the user typed since this field was last drawn. Skipped when the app has
        // just called `Text::set`: the set replaces the whole buffer, and the shell accepts it
        // over anything typed in the meantime, so the two sides still agree.
        for d in self.rec.take_text_deltas(id) {
            if !text.push_pending {
                text.apply_delta(d.version, d.start, d.end, &d.inserted);
            }
        }
        let set = if text.push_pending {
            Some(text.buf.as_str())
        } else {
            None
        };
        let version = text.version;
        self.rec.push(&match field {
            Field::Single => Cmd::TextEditSingle { id, version, set },
            Field::Multi => Cmd::TextEditMulti { id, version, set },
            Field::Code(lang, mark) => Cmd::TextEditCode {
                id,
                version,
                set,
                lang,
                mark,
            },
        });
        text.push_pending = false;
        self.response(id)
    }

    /// A clickable row that shows whether it is the selected one: an entry in a file tree or a
    /// list you pick from.
    pub fn selectable(&mut self, selected: bool, text: &str) -> Response {
        let id = self.auto_id();
        self.rec.push(&Cmd::Selectable { id, text, selected });
        self.response(id)
    }

    /// A run of formatted text: a heading, bold, a code span, a link. Links respond to
    /// [`Response::clicked`]; what a click does is up to the app.
    pub fn styled(&mut self, text: &str, style: TextStyle) -> Response {
        let id = self.auto_id();
        self.rec.push(&Cmd::Styled { id, text, style });
        self.response(id)
    }

    /// A button that opens the browser's folder picker and uploads the chosen folder into
    /// `dest`, a server path such as `/Documents`: the folder `photos` lands at
    /// `/Documents/photos`.
    ///
    /// The shell does the picking and uploading. The app learns an upload into this button's
    /// folder finished through [`Response::uploads_finished`], a count that goes up by one each
    /// time; that is its cue to re-list `dest`.
    pub fn upload_folder(&mut self, dest: &str) -> Response {
        let id = self.auto_id();
        self.rec.push(&Cmd::UploadFolder { id, dest });
        self.response(id)
    }

    /// A button that uploads a project folder from this computer into a new temporary folder
    /// on the server, leaving out everything its `.gitignore` files exclude (and `.git/`).
    ///
    /// For building, not keeping: the server deletes the folder after an hour unused, or on
    /// restart, and it never appears in the Files app. When an upload finishes,
    /// [`Response::uploaded_project`] gives the folder's id; its path is
    /// `ccosel_proto::scratch::path(id)`, and the picked folder is the one directory inside.
    pub fn upload_project(&mut self) -> Response {
        let id = self.auto_id();
        self.rec.push(&Cmd::UploadProject { id });
        self.response(id)
    }

    /// Open `url` in a new browser tab. Useful for downloads: point `url` at `/files/{path}` to
    /// stream a file out of the jail.
    ///
    /// `label` is what the button shows. Keep it separate from `url` on purpose — a file
    /// listing that put the URL itself on every row would be unreadable (`/files/notes.md`
    /// repeated next to each file); pass something like `"Download"` instead.
    pub fn open_url(&mut self, label: &str, url: &str) -> Response {
        let id = self.auto_id();
        self.rec.push(&Cmd::OpenUrl { id, label, url });
        self.response(id)
    }

    /// A button that opens another app on something: `app` is its id (such as `"viewer"`) and
    /// `arg` what it should open, which it receives through [`App::open`](crate::App::open).
    /// On the desktop the app opens in a new window; on an app's own page, in a new tab.
    pub fn open_app(&mut self, label: &str, app: &str, arg: &str) -> Response {
        let id = self.auto_id();
        self.rec.push(&Cmd::OpenApp {
            id,
            label,
            app,
            arg,
        });
        self.response(id)
    }

    /// A button that copies a link to this server to the clipboard. `path` is everything after
    /// the server's address, starting with `/` (see [`crate::url::app_link`]); the shell adds the
    /// address, which an app can't know, and tells the user the link was copied.
    pub fn copy_link(&mut self, label: &str, path: &str) -> Response {
        let id = self.auto_id();
        self.rec.push(&Cmd::CopyLink { id, label, path });
        self.response(id)
    }
}

/// Which kind of text field to draw.
#[derive(Clone, Copy)]
enum Field {
    Single,
    Multi,
    Code(CodeLang, Option<(u32, u32)>),
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
