//! What a widget did — *last* frame.

use ccosel_abi::{Rect, RespRecord, Vec2};

/// The result of a widget call.
///
/// Always describes the previous frame. See the crate docs for why that is invisible to the
/// user, and for the one rule that makes it so: never branch on a `Response` in order to draw.
#[derive(Clone, Copy, Debug, Default)]
pub struct Response {
    pub(crate) rec: RespRecord,
}

impl Response {
    pub(crate) fn from_record(rec: RespRecord) -> Self {
        Self { rec }
    }

    /// A widget the shell has never seen — first frame, or an id that changed underneath us.
    pub fn is_new(&self) -> bool {
        self.rec.local_id == 0 && self.rec.flags == 0
    }

    pub fn clicked(&self) -> bool {
        self.rec.clicked()
    }

    /// True while the pointer is over the widget.
    ///
    /// Use this for logic, never for drawing — a hover-driven visual built on this lags a
    /// frame. For tooltips and highlights, emit them unconditionally instead.
    pub fn hovered(&self) -> bool {
        self.rec.hovered()
    }

    /// The underlying value was edited. For text and sliders this means *committed*: the shell
    /// owns the live value mid-drag and paints it, so there is no visual lag.
    pub fn changed(&self) -> bool {
        self.rec.changed()
    }

    pub fn dragged(&self) -> bool {
        self.rec.dragged()
    }

    /// A single-line text field was left by pressing Enter: the user means "run this".
    pub fn submitted(&self) -> bool {
        self.rec.flags & ccosel_abi::ResponseFlags::SUBMITTED != 0
    }

    pub fn has_focus(&self) -> bool {
        self.rec.flags & ccosel_abi::ResponseFlags::HAS_FOCUS != 0
    }

    /// Where the widget was laid out. Useful for virtualized lists, which size themselves from
    /// last frame's viewport.
    pub fn rect(&self) -> Rect {
        Rect::from_array(self.rec.rect)
    }

    pub fn drag_delta(&self) -> Vec2 {
        Vec2::new(self.rec.drag_delta[0], self.rec.drag_delta[1])
    }

    /// Committed value for sliders and drag-values.
    pub fn value(&self) -> f32 {
        self.rec.value
    }

    /// For an [`upload_folder`](crate::Ui::upload_folder) button: how many uploads started from
    /// it have finished, successfully or not. Compare with the last value seen to know when to
    /// re-list the folder.
    pub fn uploads_finished(&self) -> u32 {
        self.rec.aux
    }

    /// For an [`upload_project`](crate::Ui::upload_project) button: the temporary folder id of
    /// the last upload from it that finished, or `None` before any has. A new value means a new
    /// upload landed.
    pub fn uploaded_project(&self) -> Option<u32> {
        (self.rec.aux != 0).then_some(self.rec.aux)
    }

    /// Text version for a text edit — compare against your own to decide whether the shell has
    /// newer content than you do.
    pub fn text_version(&self) -> u32 {
        self.rec.aux
    }
}
