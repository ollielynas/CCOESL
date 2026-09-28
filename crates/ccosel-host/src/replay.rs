//! The decode-index-render pipeline.

use std::collections::HashMap;
use std::ops::Range;

use ccosel_abi::event::{TextDelta, encode_batch, encode_text_delta, event_kind};
use ccosel_abi::{
    Align, Cmd, DecodeError, Decoder, RespRecord, ResponseFlags, ScopeKind, TextStyle, validate,
};

use crate::convert;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayError {
    Decode(DecodeError),
}

impl From<DecodeError> for ReplayError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "malformed command stream: {e:?}"),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Authoritative text state. The shell owns the buffer, the cursor, the selection, the undo
/// stack and IME — all of which egui already implements and none of which survives a trip
/// through a command stream.
///
/// It also remembers what the *guest* holds (`synced`), so that after the user types, the shell
/// can send the guest the smallest edit that brings its copy up to date — see
/// [`Replayer::take_text_events`].
#[derive(Default)]
struct TextState {
    buf: String,
    version: u32,
    /// The guest's copy of the buffer, and its version, as of the last delta or `set`.
    synced: String,
    synced_version: u32,
    /// The version of the last `set` accepted from the guest, so a frame replayed twice (the
    /// shell re-renders the previous buffer when a guest fails) cannot apply it twice and
    /// clobber what the user typed in between.
    set_version: Option<u32>,
}

impl TextState {
    /// Reconcile with what the guest emitted this frame.
    fn sync_from_guest(&mut self, version: u32, set: Option<&str>, is_new: bool) {
        match set {
            // An explicit `Text::set` is the app saying "this is the text now", e.g. loading a
            // file into an editor. It wins over anything typed since, and the version resets
            // to the guest's, so later deltas carry versions the guest will accept.
            Some(new_text) if self.set_version != Some(version) => {
                self.buf.clear();
                self.buf.push_str(new_text);
                self.synced.clear();
                self.synced.push_str(new_text);
                self.version = version;
                self.synced_version = version;
                self.set_version = Some(version);
            }
            Some(_) => {}
            // A field seen for the first time adopts the guest's version. Both sides start
            // empty, so the texts already agree; the versions have to as well, or the guest
            // would reject the first delta as stale.
            None if is_new => {
                self.version = version;
                self.synced_version = version;
            }
            None => {}
        }
    }

    /// The edit that turns the guest's copy into the shell's buffer, if they differ.
    fn delta(&mut self, id: u64) -> Option<Vec<u8>> {
        if self.version == self.synced_version {
            return None;
        }
        let (start, old_end, new_end) = diff_range(&self.synced, &self.buf);
        let payload = encode_text_delta(&TextDelta {
            id,
            version: self.version,
            start: start as u32,
            end: old_end as u32,
            inserted: &self.buf[start..new_end],
        });
        self.synced.clear();
        self.synced.push_str(&self.buf);
        self.synced_version = self.version;
        Some(payload)
    }
}

/// The byte range that changed between `old` and `new`: `(start, end in old, end in new)`,
/// after trimming the common prefix and suffix. Both ends land on char boundaries.
fn diff_range(old: &str, new: &str) -> (usize, usize, usize) {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .scan(0usize, |acc, n| {
            *acc += n;
            (*acc <= max_suffix).then_some(n)
        })
        .sum();
    (prefix, old.len() - suffix, new.len() - suffix)
}

/// Per-app replay state that must persist across frames.
#[derive(Default)]
pub struct Replayer {
    /// Keyed by the guest's local id.
    text: HashMap<u64, TextState>,
    /// Widget ids and destination folders of this frame's `UploadFolder` buttons. The shell
    /// checks these against the frame's clicks to know when to open the folder picker, and
    /// where the chosen folder goes.
    uploads: Vec<(u64, String)>,
    /// Widget ids of this frame's `UploadProject` buttons.
    project_uploads: Vec<u64>,
    /// Widget ids and target URLs from `OpenUrl` commands this frame. The shell checks these to
    /// know when, and where, to open a new browser tab (see #19).
    open_url_ids: Vec<(u64, String)>,
}

impl Replayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode `buf` and render it into `ui`, returning the response table for the guest's
    /// *next* frame, sorted by local id.
    ///
    /// On error nothing is rendered and no state is mutated — the caller should re-render the
    /// previous frame's buffer instead of showing the user a half-drawn app.
    pub fn replay(
        &mut self,
        ui: &mut egui::Ui,
        app_instance: u64,
        buf: &[u8],
    ) -> Result<Vec<RespRecord>, ReplayError> {
        // Validate the whole buffer before touching the Ui. Replay mutates layout state, so a
        // buffer that turns out to be unbalanced partway through would leave the shell in a
        // broken state that outlives the bad frame.
        validate(buf)?;

        let cmds: Vec<Cmd<'_>> = Decoder::new(buf).collect::<Result<_, _>>()?;
        let closes = match_scopes(&cmds);

        // Tooltips are emitted *alongside* their widget rather than inside a hover branch, so
        // resolve them up front and attach on the way past. This is what makes tooltips
        // zero-latency despite responses being a frame stale.
        let mut tooltips: HashMap<u64, &str> = HashMap::new();
        self.uploads.clear();
        self.project_uploads.clear();
        self.open_url_ids.clear();
        for cmd in &cmds {
            match *cmd {
                Cmd::Tooltip { id, text } => {
                    tooltips.insert(id, text);
                }
                Cmd::UploadFolder { id, dest } => self.uploads.push((id, dest.to_owned())),
                Cmd::UploadProject { id } => self.project_uploads.push(id),
                Cmd::OpenUrl { id, url, .. } => self.open_url_ids.push((id, url.to_owned())),
                _ => {}
            }
        }

        let mut out = Vec::new();
        let mut cx = Cx {
            app_instance,
            tooltips: &tooltips,
            out: &mut out,
            text: &mut self.text,
        };
        cx.render(ui, &cmds, &closes, 0..cmds.len());

        out.sort_unstable_by_key(|r| r.local_id);
        Ok(out)
    }

    /// The authoritative contents of a text field, for the app's delta stream.
    pub fn text(&self, local_id: u64) -> Option<(&str, u32)> {
        self.text
            .get(&local_id)
            .map(|t| (t.buf.as_str(), t.version))
    }

    /// The edits the user made to this app's text fields since the last call, as one
    /// `TEXT_DELTA` event batch ready for the guest, or `None` if nothing changed. The shell
    /// delivers it before the guest's next frame, which is how an app learns what was typed.
    pub fn take_text_events(&mut self) -> Option<Vec<u8>> {
        let mut ids: Vec<u64> = self.text.keys().copied().collect();
        // Deterministic order, so a batch is reproducible in tests.
        ids.sort_unstable();
        let payloads: Vec<Vec<u8>> = ids
            .into_iter()
            .filter_map(|id| self.text.get_mut(&id).and_then(|t| t.delta(id)))
            .collect();
        if payloads.is_empty() {
            return None;
        }
        let events: Vec<(u32, u32, &[u8])> = payloads
            .iter()
            .map(|p| (event_kind::TEXT_DELTA, 0, p.as_slice()))
            .collect();
        Some(encode_batch(&events))
    }

    /// Widget ids and destination folders of the `UploadFolder` buttons in the last
    /// successfully replayed frame. Acting on a click (the picker, the upload) is the shell's job.
    pub fn uploads(&self) -> &[(u64, String)] {
        &self.uploads
    }

    /// Widget ids of the `UploadProject` buttons in the last successfully replayed frame.
    pub fn project_uploads(&self) -> &[u64] {
        &self.project_uploads
    }

    /// Widget ids and target URLs from `OpenUrl` commands in the last successfully replayed
    /// frame. Acting on a click (opening the tab) is the shell's job (#19).
    pub fn open_url_ids(&self) -> &[(u64, String)] {
        &self.open_url_ids
    }
}

/// For each scope-opening command, the index of the command that closes it.
///
/// `validate` has already proven the stream is balanced and within the depth limit, so this
/// cannot underflow and the recursion it drives is bounded by `MAX_SCOPE_DEPTH`.
fn match_scopes(cmds: &[Cmd<'_>]) -> Vec<usize> {
    let mut closes = vec![usize::MAX; cmds.len()];
    let mut stack = Vec::new();
    for (i, cmd) in cmds.iter().enumerate() {
        match cmd {
            Cmd::BeginScope { .. } | Cmd::BeginWindow { .. } => stack.push(i),
            Cmd::EndScope { .. } | Cmd::EndWindow { .. } => {
                if let Some(open) = stack.pop() {
                    closes[open] = i;
                }
            }
            _ => {}
        }
    }
    closes
}

/// Borrowed working set for one replay pass.
///
/// Bundling these lets the renderer recurse through `egui`'s closure-based containers
/// (`ui.horizontal(|ui| ..)`) rather than hand-managing a `Vec<Ui>` stack. Indexing the tree
/// up front is what buys that: because the scopes are known to be balanced before rendering
/// starts, the natural egui API is available and there is no stack to corrupt.
struct Cx<'a> {
    app_instance: u64,
    tooltips: &'a HashMap<u64, &'a str>,
    out: &'a mut Vec<RespRecord>,
    text: &'a mut HashMap<u64, TextState>,
}

impl Cx<'_> {
    fn egui_id(&self, local_id: u64) -> egui::Id {
        egui::Id::new((self.app_instance, local_id))
    }

    /// Attach a tooltip if one was declared, then record the response for the guest.
    fn finish(&mut self, local_id: u64, response: egui::Response) {
        let response = match self.tooltips.get(&local_id) {
            Some(tip) => response.on_hover_text(*tip),
            None => response,
        };
        self.out.push(to_record(local_id, &response));
    }

    fn text_edit(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        version: u32,
        set: Option<&str>,
        multiline: bool,
    ) {
        let is_new = !self.text.contains_key(&id);
        let state = self.text.entry(id).or_default();
        state.sync_from_guest(version, set, is_new);
        let edit = if multiline {
            // Monospace, full width and tab-to-indent: this is for writing documents and
            // code, where columns line up and a tab should not move focus away.
            egui::TextEdit::multiline(&mut state.buf)
                .code_editor()
                .desired_width(f32::INFINITY)
                .desired_rows(16)
        } else {
            egui::TextEdit::singleline(&mut state.buf)
        };
        let edit = edit.id(egui::Id::new((self.app_instance, id)));
        // Only the single-line field is a control that sits in a row of buttons; a document
        // editor with a shadow would read as one giant button.
        let r = if multiline {
            ui.add(edit)
        } else {
            add_with_shadow(ui, edit)
        };
        if r.changed() {
            state.version = state.version.wrapping_add(1);
        }
        // `aux` carries the committed version so the guest can tell whether the shell holds
        // newer text than it does.
        let mut rec = to_record(id, &r);
        rec.aux = state.version;
        if let Some(tip) = self.tooltips.get(&id) {
            r.on_hover_text(*tip);
        }
        self.out.push(rec);
    }

    fn render(
        &mut self,
        ui: &mut egui::Ui,
        cmds: &[Cmd<'_>],
        closes: &[usize],
        range: Range<usize>,
    ) {
        let mut i = range.start;
        // Counts `Frame`-kind children seen so far at *this* nesting level, so consecutive rows
        // (e.g. a file list) alternate background without the guest ever naming a color. Local
        // to one call, so it naturally resets per sibling list instead of per whole frame.
        let mut frame_row: u32 = 0;
        while i < range.end {
            match cmds[i] {
                Cmd::Nop | Cmd::Tooltip { .. } => {}

                Cmd::Label { id, text } => {
                    let r = ui.label(text);
                    self.finish(id, r);
                }

                Cmd::Button { id, text } => {
                    let r = add_with_shadow(ui, egui::Button::new(text));
                    self.finish(id, r);
                }

                Cmd::Separator => {
                    ui.separator();
                }

                Cmd::UploadProject { id } => {
                    let r = add_with_shadow(ui, egui::Button::new("⬆ Upload project"));
                    self.finish(id, r);
                }

                Cmd::UploadFolder { id, .. } => {
                    let r = add_with_shadow(ui, egui::Button::new("⬆ Upload folder"));
                    self.finish(id, r);
                }

                Cmd::OpenUrl { id, label, .. } => {
                    // Draw `label`, never `url` — a file listing that put the URL itself on
                    // every row would be unreadable (`/files/notes.md` repeated next to each
                    // file). The URL is only for the shell's click handler (#19).
                    let r = add_with_shadow(ui, egui::Button::new(label));
                    self.finish(id, r);
                }

                Cmd::Image { id, src, size } => {
                    // Images are fetched by the *shell*, by URL, so they hit the browser's HTTP
                    // cache and never occupy guest memory. Until the shell's loader lands, draw
                    // a placeholder of the right size so layout is already correct.
                    let (rect, r) =
                        ui.allocate_exact_size(convert::vec2(size), egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        let visuals = ui.visuals();
                        ui.painter()
                            .rect_filled(rect, 4.0, visuals.extreme_bg_color);
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            src,
                            egui::FontId::proportional(9.0),
                            visuals.weak_text_color(),
                        );
                    }
                    self.finish(id, r);
                }

                Cmd::Plot { id, size, samples } => {
                    // A width of 0 (or less) means "fill the row", so a guest can size a chart
                    // to its window without knowing the window's width.
                    let mut size = convert::vec2(size);
                    if size.x <= 0.0 {
                        size.x = ui.available_width();
                    }
                    let (rect, r) = ui.allocate_exact_size(size, egui::Sense::hover());
                    if ui.is_rect_visible(rect) {
                        paint_plot(ui, rect, samples);
                    }
                    self.finish(id, r);
                }

                Cmd::TextEditSingle { id, version, set } => {
                    self.text_edit(ui, id, version, set, false);
                }

                Cmd::TextEditMulti { id, version, set } => {
                    self.text_edit(ui, id, version, set, true);
                }

                Cmd::Selectable { id, text, selected } => {
                    // egui frames an unselected selectable only on hover, and a frame's stroke
                    // adds to a widget's size, so hovering grew it by the stroke width and
                    // shoved everything after it along. Always frame it instead, with the
                    // resting outline and fill invisible: the size never changes, and hovering
                    // only changes colour.
                    let r = ui
                        .scope(|ui| {
                            let rest = &mut ui.visuals_mut().widgets.inactive;
                            rest.bg_stroke.color = egui::Color32::TRANSPARENT;
                            rest.bg_fill = egui::Color32::TRANSPARENT;
                            rest.weak_bg_fill = egui::Color32::TRANSPARENT;
                            ui.add(
                                egui::Button::selectable(selected, text).frame_when_inactive(true),
                            )
                        })
                        .inner;
                    self.finish(id, r);
                }

                Cmd::Styled { id, text, style } => {
                    let rich = rich_text(ui, text, style);
                    let r = if style.contains(TextStyle::LINK) {
                        ui.add(egui::Link::new(rich))
                    } else {
                        ui.label(rich)
                    };
                    self.finish(id, r);
                }

                Cmd::BeginScope { id, layout } => {
                    let end = closes[i];
                    let inner = i + 1..end;

                    // Dispatch to egui's own container helpers rather than building a layout by
                    // hand. They size the child region to its row/column, whereas
                    // `with_layout(left_to_right(Center))` hands the child the *full remaining
                    // height* and centres within it — which silently pushes every subsequent
                    // row off the bottom of the viewport.
                    ui.push_id(self.egui_id(id), |ui| {
                        match (layout.kind, layout.cross_align) {
                            (ScopeKind::Horizontal, Align::Min) => {
                                ui.horizontal_top(|ui| {
                                    self.render(ui, cmds, closes, inner.clone())
                                });
                            }
                            (ScopeKind::Horizontal, _) => {
                                ui.horizontal(|ui| self.render(ui, cmds, closes, inner.clone()));
                            }
                            (ScopeKind::Vertical, Align::Center) => {
                                ui.vertical_centered(|ui| {
                                    self.render(ui, cmds, closes, inner.clone())
                                });
                            }
                            (ScopeKind::Vertical, _) => {
                                ui.vertical(|ui| self.render(ui, cmds, closes, inner.clone()));
                            }
                            (ScopeKind::Frame, _) => {
                                // Flat alternating fill rather than a bordered box: this is what
                                // gives a list of `Frame`-wrapped rows (e.g. file entries) a
                                // zebra-striped look, the same purpose `visuals.faint_bg_color`
                                // serves for `Grid::striped`.
                                let fill = if frame_row % 2 == 1 {
                                    ui.visuals().faint_bg_color
                                } else {
                                    egui::Color32::TRANSPARENT
                                };
                                frame_row += 1;
                                egui::Frame::new()
                                    .fill(fill)
                                    .corner_radius(4)
                                    .inner_margin(egui::Margin::symmetric(6, 3))
                                    .show(ui, |ui| {
                                        // A row's stripe should span the whole list, not just hug
                                        // its own content's width.
                                        ui.set_min_width(ui.available_width());
                                        self.render(ui, cmds, closes, inner.clone())
                                    });
                            }
                            (ScopeKind::Group, _) => {
                                ui.scope(|ui| self.render(ui, cmds, closes, inner.clone()));
                            }
                            (ScopeKind::Sidebar, _) => {
                                // A fixed-width column with a rule after it. Its width does not
                                // follow its content, so the main column beside it stays put as
                                // the guest opens folders with longer names.
                                let width = (ui.available_width() * 0.3).clamp(160.0, 240.0);
                                ui.vertical(|ui| {
                                    ui.set_width(width);
                                    self.render(ui, cmds, closes, inner.clone())
                                });
                                ui.separator();
                            }
                            (ScopeKind::Scroll, _) => {
                                // The shell already scrolls the whole window, which gives its
                                // content unbounded height; this bounds it to what is actually
                                // visible below here, so the region scrolls instead of the
                                // window, and two of them side by side scroll separately.
                                let visible = ui.clip_rect().bottom() - ui.cursor().top();
                                let height = (visible - ui.spacing().item_spacing.y).max(80.0);
                                egui::ScrollArea::vertical()
                                    .id_salt(self.egui_id(id))
                                    .auto_shrink([false, false])
                                    .max_height(height)
                                    .min_scrolled_height(height)
                                    .show(ui, |ui| {
                                        // Top to bottom, whatever row it sits in: a scroll area
                                        // otherwise inherits its parent's direction.
                                        ui.vertical(|ui| {
                                            self.render(ui, cmds, closes, inner.clone())
                                        })
                                    });
                            }
                            (ScopeKind::Indent, _) => {
                                ui.indent(self.egui_id(id), |ui| {
                                    self.render(ui, cmds, closes, inner.clone())
                                });
                            }
                            (ScopeKind::Wrapped, _) => {
                                ui.horizontal_wrapped(|ui| {
                                    // Runs of styled text carry their own spaces, so the
                                    // default gap between widgets would double them up.
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    self.render(ui, cmds, closes, inner.clone())
                                });
                            }
                        }
                    });

                    i = end + 1;
                    continue;
                }

                Cmd::BeginWindow { id, title, .. } => {
                    let end = closes[i];
                    let inner = i + 1..end;
                    // Windows are context-level areas, so a guest can open one from anywhere in
                    // its tree; the shell hoists it to the desktop.
                    let ctx = ui.ctx().clone();
                    egui::Window::new(title)
                        .id(self.egui_id(id))
                        .show(&ctx, |ui| {
                            self.render(ui, cmds, closes, inner.clone());
                        });

                    i = end + 1;
                    continue;
                }

                // Unreachable for a validated buffer: every close is consumed by its opener.
                Cmd::EndScope { .. } | Cmd::EndWindow { .. } => {}
            }
            i += 1;
        }
    }
}

/// How far a button's or text field's shadow sits below and to the right of it.
const WIDGET_SHADOW_OFFSET: f32 = 3.0;

/// Adds `widget` with a hard shadow painted *behind* it: a solid block in the widget outline's
/// colour, offset by [`WIDGET_SHADOW_OFFSET`]. The shape slot is reserved before the widget is
/// added, which is how egui paints underneath something whose rect isn't known yet.
///
/// A pressed widget has no shadow, so it reads as pushed in. A disabled one has none either,
/// since there is nothing to push.
fn add_with_shadow(ui: &mut egui::Ui, widget: impl egui::Widget) -> egui::Response {
    let slot = ui.painter().add(egui::Shape::Noop);
    let r = ui.add(widget);
    if r.enabled() && !r.is_pointer_button_down_on() {
        let style = &ui.visuals().widgets.inactive;
        ui.painter().set(
            slot,
            egui::Shape::rect_filled(
                r.rect.translate(egui::Vec2::splat(WIDGET_SHADOW_OFFSET)),
                style.corner_radius,
                style.bg_stroke.color,
            ),
        );
    }
    r
}

/// The egui form of a `Styled` run. Sizes are relative to the body font, so the app's
/// headings scale with the shell's theme rather than being fixed pixel sizes.
fn rich_text(ui: &egui::Ui, text: &str, style: TextStyle) -> egui::RichText {
    let mut rich = egui::RichText::new(text);
    let level = style.heading_level();
    if level > 0 {
        let scale = match level {
            1 => 1.6,
            2 => 1.35,
            _ => 1.15,
        };
        let body = ui
            .style()
            .text_styles
            .get(&egui::TextStyle::Body)
            .map_or(14.0, |f| f.size);
        rich = rich.size(body * scale).color(strong_color(ui.visuals()));
    }
    if style.contains(TextStyle::STRONG) {
        rich = rich.color(strong_color(ui.visuals()));
    }
    if style.contains(TextStyle::ITALIC) {
        rich = rich.italics();
    }
    if style.contains(TextStyle::CODE) {
        rich = rich.code();
    }
    if style.contains(TextStyle::STRIKE) {
        rich = rich.strikethrough();
    }
    if style.contains(TextStyle::WEAK) {
        rich = rich.weak();
    }
    rich
}

/// The colour for headings and bold text.
///
/// Not `RichText::strong`: egui takes that from the *active* widget's text colour, which is
/// the shell theme's choice for pressed buttons, not for text. A theme that once set it to
/// white made every heading white on white. This takes the theme's body text colour and moves it
/// halfway to full contrast instead, so strong text stands out in either mode.
pub(crate) fn strong_color(visuals: &egui::Visuals) -> egui::Color32 {
    let body = visuals.text_color();
    let extreme = if visuals.dark_mode {
        egui::Color32::WHITE
    } else {
        egui::Color32::BLACK
    };
    body.lerp_to_gamma(extreme, 0.5)
}

fn to_record(local_id: u64, r: &egui::Response) -> RespRecord {
    let mut flags = 0u32;
    let mut set = |cond: bool, bit: u32| {
        if cond {
            flags |= bit;
        }
    };
    set(r.enabled(), ResponseFlags::ENABLED);
    set(r.contains_pointer(), ResponseFlags::CONTAINS_POINTER);
    set(r.hovered(), ResponseFlags::HOVERED);
    set(r.highlighted(), ResponseFlags::HIGHLIGHTED);
    set(r.clicked(), ResponseFlags::CLICKED);
    set(r.drag_started(), ResponseFlags::DRAG_STARTED);
    set(r.dragged(), ResponseFlags::DRAGGED);
    set(r.drag_stopped(), ResponseFlags::DRAG_STOPPED);
    set(
        r.is_pointer_button_down_on(),
        ResponseFlags::IS_POINTER_BUTTON_DOWN_ON,
    );
    set(r.changed(), ResponseFlags::CHANGED);
    set(r.has_focus(), ResponseFlags::HAS_FOCUS);
    set(r.gained_focus(), ResponseFlags::GAINED_FOCUS);
    set(r.lost_focus(), ResponseFlags::LOST_FOCUS);

    let delta = r.drag_delta();
    RespRecord {
        local_id,
        flags,
        aux: 0,
        rect: convert::rect_array(r.rect),
        drag_delta: [delta.x, delta.y],
        value: 0.0,
        _pad: 0,
    }
}

/// Draw `samples` (each `0..=255`, oldest first) as a filled line graph filling `rect`.
fn paint_plot(ui: &egui::Ui, rect: egui::Rect, samples: &[u8]) {
    let visuals = ui.visuals();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
    // Quarter gridlines, so a reader can judge a level without an axis per chart.
    let grid = egui::Stroke::new(1.0, visuals.faint_bg_color);
    for q in 1..4 {
        let y = rect.top() + rect.height() * q as f32 / 4.0;
        painter.hline(rect.x_range(), y, grid);
    }
    if samples.len() < 2 {
        return;
    }
    let step = rect.width() / (samples.len() - 1) as f32;
    let points: Vec<egui::Pos2> = samples
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            egui::pos2(
                rect.left() + step * i as f32,
                rect.bottom() - rect.height() * f32::from(v) / 255.0,
            )
        })
        .collect();
    // The theme's link colour: the shell sets it to something that reads on a window's fill,
    // unlike the selection fill, which may be pale.
    let accent = visuals.hyperlink_color;
    // Fill under the line as one quad per segment: a single polygon would be concave, which
    // egui's convex-polygon fill cannot draw correctly.
    let fill = accent.gamma_multiply(0.2);
    for w in points.windows(2) {
        painter.add(egui::Shape::convex_polygon(
            vec![
                w[0],
                w[1],
                egui::pos2(w[1].x, rect.bottom()),
                egui::pos2(w[0].x, rect.bottom()),
            ],
            fill,
            egui::Stroke::NONE,
        ));
    }
    painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, accent)));
}

#[cfg(test)]
mod tests {
    use super::{diff_range, strong_color};

    #[test]
    fn strong_text_is_never_the_white_of_a_pressed_button() {
        // The shell's theme, reduced to the part that caused this: white text on active
        // widgets. Headings and bold text were drawn in it, white on a light window.
        let mut light = egui::Visuals::light();
        light.widgets.active.fg_stroke.color = egui::Color32::WHITE;
        let c = strong_color(&light);
        assert!(
            c.r() < 80 && c.g() < 80 && c.b() < 80,
            "dark on light: {c:?}"
        );

        let mut dark = egui::Visuals::dark();
        dark.widgets.active.fg_stroke.color = egui::Color32::BLACK;
        let c = strong_color(&dark);
        assert!(
            c.r() > 180 && c.g() > 180 && c.b() > 180,
            "light on dark: {c:?}"
        );
    }

    #[test]
    fn diff_range_trims_the_common_ends() {
        assert_eq!(diff_range("abc", "abc"), (3, 3, 3));
        assert_eq!(diff_range("", "new"), (0, 0, 3));
        assert_eq!(diff_range("hello world", "hello, world"), (5, 5, 6));
        assert_eq!(diff_range("abcdef", "abef"), (2, 4, 2));
        // A repeated character: prefix and suffix must not overlap.
        assert_eq!(diff_range("aa", "aaa"), (2, 2, 3));
        assert_eq!(diff_range("aaa", "aa"), (2, 3, 2));
    }

    #[test]
    fn diff_range_stays_on_char_boundaries() {
        let (old, new) = ("né", "nè");
        let (start, old_end, new_end) = diff_range(old, new);
        assert!(old.is_char_boundary(start) && old.is_char_boundary(old_end));
        assert!(new.is_char_boundary(new_end));
        assert_eq!(&new[start..new_end], "è");
    }
}
