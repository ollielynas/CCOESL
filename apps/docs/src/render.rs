//! Drawing parsed Markdown with the SDK's widgets.
//!
//! Each block becomes a `wrapped` row of `styled` runs, so a paragraph reads as a paragraph
//! and each run carries its own formatting. The shell knows nothing of Markdown; it only draws
//! styled text.

use ccosel_sdk::{TextStyle, Ui};

use crate::markdown::{self, Block, Inline};
use crate::paths;

/// What following a link should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Follow {
    /// Open this document in the app.
    Doc(String),
}

/// Draw `blocks`, a document at `path` (links are relative to it). Returns the document link
/// clicked last frame, if any. Links to the web and to other files are handed to the shell,
/// which opens them in a new tab, so they never come back here.
pub fn document(ui: &mut Ui<'_>, path: &str, blocks: &[Block]) -> Option<Follow> {
    let mut follow = None;
    for (i, block) in blocks.iter().enumerate() {
        // Keyed by position so one block's links keep their ids while another is edited.
        ui.push_id(&i.to_string(), |ui| {
            if let Some(f) = self::block(ui, path, block) {
                follow = Some(f);
            }
        });
    }
    if blocks.is_empty() {
        ui.styled(
            "This document is empty.",
            TextStyle::WEAK | TextStyle::ITALIC,
        );
    }
    follow
}

fn block(ui: &mut Ui<'_>, path: &str, block: &Block) -> Option<Follow> {
    match block {
        Block::Heading { level, inlines } => {
            let style = TextStyle::heading((*level).min(3));
            ui.wrapped(|ui| runs(ui, path, inlines, style))
        }
        Block::Paragraph(inlines) => ui.wrapped(|ui| runs(ui, path, inlines, TextStyle::PLAIN)),
        Block::Item {
            depth,
            number,
            checked,
            inlines,
        } => ui.wrapped(|ui| {
            let marker = match (checked, number) {
                (Some(true), _) => "☑ ".to_owned(),
                (Some(false), _) => "☐ ".to_owned(),
                (None, Some(n)) => format!("{n}. "),
                (None, None) => "• ".to_owned(),
            };
            let indent = "    ".repeat(usize::from(*depth) + 1);
            ui.styled(&format!("{indent}{marker}"), TextStyle::PLAIN);
            let style = if *checked == Some(true) {
                TextStyle::WEAK
            } else {
                TextStyle::PLAIN
            };
            runs(ui, path, inlines, style)
        }),
        Block::Quote(inlines) => ui.wrapped(|ui| {
            ui.styled("▎ ", TextStyle::WEAK);
            runs(ui, path, inlines, TextStyle::WEAK | TextStyle::ITALIC)
        }),
        Block::Code { lang, text } => {
            ui.group(|ui| {
                if !lang.is_empty() {
                    ui.styled(lang, TextStyle::WEAK);
                }
                for line in text.lines() {
                    // An empty line would collapse to nothing and close the gap it stands for.
                    let line = if line.is_empty() { " " } else { line };
                    ui.styled(line, TextStyle::CODE);
                }
            });
            None
        }
        Block::Rule => {
            ui.separator();
            None
        }
        Block::Table(rows) => {
            let mut follow = None;
            ui.group(|ui| {
                for (r, row) in rows.iter().enumerate() {
                    let base = if r == 0 {
                        TextStyle::STRONG
                    } else {
                        TextStyle::PLAIN
                    };
                    let f = ui.wrapped(|ui| {
                        let mut follow = None;
                        for (c, cell) in row.iter().enumerate() {
                            if c > 0 {
                                ui.styled("  │  ", TextStyle::WEAK);
                            }
                            if let Some(f) = runs(ui, path, cell, base) {
                                follow = Some(f);
                            }
                        }
                        follow
                    });
                    if f.is_some() {
                        follow = f;
                    }
                }
            });
            follow
        }
    }
}

fn runs(ui: &mut Ui<'_>, path: &str, inlines: &[Inline], base: TextStyle) -> Option<Follow> {
    let mut follow = None;
    for run in inlines {
        let mut style = base;
        if run.style & markdown::STRONG != 0 {
            style = style | TextStyle::STRONG;
        }
        if run.style & markdown::ITALIC != 0 {
            style = style | TextStyle::ITALIC;
        }
        if run.style & markdown::CODE != 0 {
            style = style | TextStyle::CODE;
        }
        if run.style & markdown::STRIKE != 0 {
            style = style | TextStyle::STRIKE;
        }
        match &run.link {
            None => {
                ui.styled(&run.text, style);
            }
            // The web and email: only the shell can open a tab.
            Some(target) if markdown::is_external(target) => {
                ui.open_url(&format!("🔗 {}", run.text), target);
                ui.tooltip(target);
            }
            Some(target) => match paths::resolve(path, target) {
                // Another document: followed inside the app.
                Some(to) if paths::is_doc(&to) => {
                    if ui.styled(&run.text, style | TextStyle::LINK).clicked() {
                        follow = Some(Follow::Doc(to.clone()));
                    }
                    ui.tooltip(&to);
                }
                // Any other file on the server: the browser downloads it.
                Some(to) => {
                    ui.open_url(&format!("📎 {}", run.text), &paths::download_url(&to));
                    ui.tooltip(&to);
                }
                // `[top](#top)`: in-page anchors have nowhere to go yet.
                None => {
                    ui.styled(&run.text, style);
                }
            },
        }
    }
    follow
}
