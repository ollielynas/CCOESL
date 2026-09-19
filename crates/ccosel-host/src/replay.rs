//! The decode-index-render pipeline.

use std::collections::HashMap;
use std::ops::Range;

use ccosel_abi::{
    Align, Cmd, DecodeError, Decoder, RespRecord, ResponseFlags, ScopeKind, validate,
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
#[derive(Default)]
struct TextState {
    buf: String,
    version: u32,
}

/// Per-app replay state that must persist across frames.
#[derive(Default)]
pub struct Replayer {
    /// Keyed by the guest's local id.
    text: HashMap<u64, TextState>,
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
        for cmd in &cmds {
            if let Cmd::Tooltip { id, text } = *cmd {
                tooltips.insert(id, text);
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
                    // Flat at rest, framed on hover/press: modern toolbars and list rows read
                    // as buttons without every one of them drawing a permanent box outline.
                    let r = ui.add(egui::Button::new(text).frame_when_inactive(false));
                    self.finish(id, r);
                }

                Cmd::Separator => {
                    ui.separator();
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

                Cmd::TextEditSingle { id, version, set } => {
                    let state = self.text.entry(id).or_default();
                    // The guest only wins if it carries a newer version; otherwise the shell's
                    // buffer is authoritative and the guest's `set` is a stale echo.
                    if let Some(new_text) = set
                        && version > state.version
                    {
                        state.buf.clear();
                        state.buf.push_str(new_text);
                        state.version = version;
                    }
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut state.buf)
                            .id(egui::Id::new((self.app_instance, id))),
                    );
                    if r.changed() {
                        state.version = state.version.wrapping_add(1);
                    }
                    // `aux` carries the committed version so the guest can tell whether the
                    // shell holds newer text than it does.
                    let mut rec = to_record(id, &r);
                    rec.aux = state.version;
                    if let Some(tip) = self.tooltips.get(&id) {
                        r.on_hover_text(*tip);
                    }
                    self.out.push(rec);
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
