//! The Music Score app: write LilyPond, see it engraved.
//!
//! You type LilyPond notation in the editor and press Engrave. The server runs LilyPond (GNU
//! LilyPond, `ccosel-server`'s `score_api`) and the pages come back as pictures, shown one at a
//! time beside a PDF and, if the score has `\midi`, a MIDI file to download.
//!
//! Engraving is a job, polled like the Compiler's build: `Engrave` with the source starts it,
//! and asking again with the same source reports how far it has got. The server keys the job by
//! a hash of the source, so engraving text that hasn't changed costs nothing. Once a result is
//! in, the app keeps it and stops asking, so the source isn't sent again every frame.
//!
//! `.ly` files are opened and saved with the ordinary file RPCs, under the same permissions as
//! anywhere else.

use ccosel_proto::fs::{PathReq, ReadFile, WriteFile, WriteFileReq};
use ccosel_proto::score::{
    Engrave, EngraveReq, EngraveResult, ScoreMessage, midi_url, page_url, pdf_url,
};
use ccosel_sdk::{App, CallId, Poll, Text, Ui, Vec2};

/// Twice a second while LilyPond runs: a score takes seconds, so this is plenty.
const POLL_MS: u32 = 500;

/// What a new score starts as: enough to engrave straight away, and to show the shape of a file.
pub const STARTER: &str = "\\version \"2.24.0\"
\\header { title = \"Untitled\" }

\\score {
  \\relative c' {
    \\clef treble \\time 4/4
    c4 d e f | g2 g | a4 a a a | g1 \\bar \"|.\"
  }
  \\layout { }
  \\midi { }
}
";

/// Log lines shown when an engraving failed without any message pointing at the score.
const LOG_LINES: usize = 12;

pub struct MusicScore {
    /// Where Open reads from and Save writes to.
    path: Text,
    editor: Text,
    /// The text as last opened or saved, to tell whether there are changes to lose.
    saved: String,
    /// A file being read into the editor.
    opening: Option<String>,
    /// Open was pressed with unsaved changes; pressing it again discards them.
    confirm_open: bool,
    /// A save in flight, and the text it is saving.
    saving: Option<(CallId, String)>,
    /// The source an engraving was asked for, until its result is in.
    engraving: Option<String>,
    elapsed_ms: u64,
    result: Option<EngraveResult>,
    /// The page shown, from 0.
    page: u32,
    /// A one-line note on the last action: "Saved", or why something failed.
    status: Option<String>,
    /// A click was acted on after this frame was drawn, so the next frame looks different and
    /// should come now rather than on the next input.
    changed: bool,
}

impl Default for MusicScore {
    fn default() -> Self {
        Self {
            path: Text::new(""),
            editor: Text::new(STARTER),
            saved: STARTER.to_owned(),
            opening: None,
            confirm_open: false,
            saving: None,
            engraving: None,
            elapsed_ms: 0,
            result: None,
            page: 0,
            status: None,
            changed: false,
        }
    }
}

/// What was clicked this frame, acted on once drawing is done.
#[derive(Default, PartialEq)]
struct Actions {
    open: bool,
    save: bool,
    engrave: bool,
    prev: bool,
    next: bool,
}

/// Where a message is in the score, for reading: `Line 4, column 9: `, or nothing for a message
/// about the whole score.
pub fn place(m: &ScoreMessage) -> String {
    match (m.line, m.column) {
        (0, _) => String::new(),
        (line, 0) => format!("Line {line}: "),
        (line, column) => format!("Line {line}, column {column}: "),
    }
}

impl MusicScore {
    fn dirty(&self) -> bool {
        self.editor.as_str() != self.saved
    }

    fn toolbar(&mut self, ui: &mut Ui<'_>, act: &mut Actions) {
        ui.horizontal(|ui| {
            ui.label("File");
            ui.text_edit(&mut self.path);
            ui.tooltip("A .ly file on the server, such as /home/you/song.ly");
            act.open = ui.button("📂 Open").clicked();
            let saving = self.saving.is_some();
            act.save = ui
                .button(if saving { "Saving…" } else { "💾 Save" })
                .clicked();
            if self.dirty() {
                ui.label("(unsaved)");
            }
        });
        ui.push_id("engrave", |ui| {
            ui.horizontal(|ui| {
                if self.engraving.is_some() {
                    ui.label(&format!("Engraving… {} s", self.elapsed_ms / 1000));
                } else {
                    act.engrave = ui.button("🎼 Engrave").clicked();
                    ui.tooltip("Turn the score into sheet music on the server");
                }
                if let Some(result) = &self.result {
                    if result.pdf {
                        ui.open_url("Download PDF", &pdf_url(&result.key));
                    }
                    if result.midi {
                        ui.open_url("Download MIDI", &midi_url(&result.key));
                    }
                }
            })
        });
        if let Some(status) = &self.status {
            ui.label(status);
        }
    }

    fn messages(&self, ui: &mut Ui<'_>) {
        let Some(result) = &self.result else {
            return;
        };
        if result.success && result.messages.is_empty() {
            return;
        }
        ui.separator();
        if !result.success {
            ui.label("❌ LilyPond could not engrave this score");
        }
        for m in &result.messages {
            let icon = if m.error { "❌ " } else { "⚠ " };
            ui.label(&format!("{icon}{}{}", place(m), m.text));
        }
        // A failure with nothing pointing at the score, such as LilyPond being missing: its log
        // is all there is to go on.
        if !result.success && result.messages.is_empty() {
            if result.output_truncated {
                ui.label("(earlier output trimmed)");
            }
            let lines: Vec<&str> = result.output.lines().collect();
            for line in &lines[lines.len().saturating_sub(LOG_LINES)..] {
                ui.label(line);
            }
        }
    }

    fn preview(&self, ui: &mut Ui<'_>, act: &mut Actions) {
        let Some(result) = self.result.as_ref().filter(|r| r.pages > 0) else {
            return;
        };
        ui.separator();
        if result.pages > 1 {
            ui.horizontal(|ui| {
                act.prev = ui.button("◀ Previous").clicked();
                ui.label(&format!("Page {} of {}", self.page + 1, result.pages));
                act.next = ui.button("Next ▶").clicked();
            });
        }
        // As wide as the window, and as tall as the page's own shape makes it.
        ui.image(&page_url(&result.key, self.page + 1), Vec2::new(0.0, 0.0));
    }

    /// Check on the file being opened and the save in flight, if any.
    fn poll_files(&mut self, ui: &mut Ui<'_>) {
        if let Some(path) = self.opening.clone() {
            let rpc = ui.rpc();
            match rpc.get::<ReadFile>(&PathReq { path: &path }) {
                Poll::Pending => {}
                Poll::Ready(file) => {
                    // Read afresh next time: the file may have changed by then.
                    rpc.invalidate::<ReadFile>(&PathReq { path: &path });
                    self.editor.set(&file.text);
                    self.saved = file.text.clone();
                    self.result = None;
                    self.page = 0;
                    self.opening = None;
                    self.status = Some(format!("Opened {path}"));
                }
                Poll::Failed(e) => {
                    rpc.invalidate::<ReadFile>(&PathReq { path: &path });
                    self.opening = None;
                    self.status = Some(format!("Could not open {path}: {}", e.message()));
                }
            }
        }
        if let Some((id, text)) = &self.saving {
            match ui.rpc().outcome::<WriteFile>(*id) {
                Poll::Pending => {}
                Poll::Ready(_) => {
                    self.saved = text.clone();
                    self.status = Some("Saved".to_owned());
                    self.saving = None;
                }
                Poll::Failed(e) => {
                    self.status = Some(format!("Could not save: {}", e.message()));
                    self.saving = None;
                }
            }
        }
    }

    /// Ask how the engraving is going, if there is one.
    fn poll_engraving(&mut self, ui: &mut Ui<'_>) {
        let Some(source) = self.engraving.as_deref() else {
            return;
        };
        let req = EngraveReq { source };
        let rpc = ui.rpc();
        match rpc.get::<Engrave>(&req) {
            Poll::Pending => {}
            Poll::Ready(status) => {
                // Asked again next time, for fresher progress or the result. Keyed by the
                // source, so this never starts a second engraving.
                rpc.invalidate::<Engrave>(&req);
                self.elapsed_ms = status.elapsed_ms;
                if let Some(result) = &status.result {
                    self.result = Some(result.clone());
                    self.page = 0;
                    self.engraving = None;
                    self.status = None;
                }
            }
            Poll::Failed(e) => {
                rpc.invalidate::<Engrave>(&req);
                self.engraving = None;
                self.status = Some(format!("Could not engrave: {}", e.message()));
            }
        }
    }

    fn apply(&mut self, ui: &mut Ui<'_>, act: Actions) {
        self.changed = act != Actions::default();
        if act.open {
            let path = self.path.as_str().trim().to_owned();
            if path.is_empty() {
                self.status = Some("Type the path of a .ly file to open.".to_owned());
            } else if self.dirty() && !self.confirm_open {
                self.confirm_open = true;
                self.status =
                    Some("You have unsaved changes. Press Open again to discard them.".to_owned());
            } else {
                self.confirm_open = false;
                self.status = Some(format!("Opening {path}…"));
                self.opening = Some(path);
            }
        }
        if act.save && self.saving.is_none() {
            let path = self.path.as_str().trim();
            if path.is_empty() {
                self.status =
                    Some("Type where to save it first, such as /home/you/song.ly.".to_owned());
            } else {
                let text = self.editor.as_str().to_owned();
                let id = ui.rpc().send::<WriteFile>(&WriteFileReq {
                    path,
                    text: &text,
                    create_only: false,
                });
                self.saving = Some((id, text));
                self.status = None;
            }
        }
        if act.engrave && self.engraving.is_none() {
            self.engraving = Some(self.editor.as_str().to_owned());
            self.elapsed_ms = 0;
            self.status = None;
        }
        let pages = self.result.as_ref().map_or(0, |r| r.pages);
        if act.prev {
            self.page = self.page.saturating_sub(1);
        }
        if act.next && self.page + 1 < pages {
            self.page += 1;
        }
    }
}

impl App for MusicScore {
    fn update(&mut self, ui: &mut Ui<'_>) {
        self.poll_files(ui);
        self.poll_engraving(ui);

        // Each part under its own id, so a status line or a message appearing above a widget
        // doesn't change the widget's id: the shell keeps a text field's contents by its id.
        let mut act = Actions::default();
        ui.push_id("toolbar", |ui| self.toolbar(ui, &mut act));
        ui.separator();
        let was_dirty = self.dirty();
        ui.push_id("editor", |ui| ui.text_edit_multiline(&mut self.editor));
        // What was typed reaches the app as the editor is drawn, after the toolbar that says
        // whether there are unsaved changes, so that has to be drawn again.
        let dirty_changed = self.dirty() != was_dirty;
        ui.push_id("messages", |ui| self.messages(ui));
        ui.push_id("preview", |ui| self.preview(ui, &mut act));
        self.apply(ui, act);
        self.changed |= dirty_changed;
    }

    /// Opened on a `.ly` file: read it into the editor.
    fn open(&mut self, arg: &str) {
        self.path.set(arg);
        self.opening = Some(arg.to_owned());
    }

    fn wants_repaint_after_ms(&self) -> u32 {
        if self.changed {
            0
        } else if self.engraving.is_some() {
            POLL_MS
        } else {
            ccosel_sdk::REPAINT_ON_INPUT_ONLY
        }
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(MusicScore);
