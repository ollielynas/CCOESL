//! The File Browser app.
//!
//! Compiles to a standalone `.wasm` module with no `wasm-bindgen` and no egui — it records
//! commands, the shell draws them. The same binary runs under `wasmtime` on the server and
//! under `WebAssembly.instantiate` in the browser.
//!
//! Directory contents are stubbed until `ccosel-transport` lands; everything else is real.

use ccosel_sdk::{App, Text, Ui, Vec2};

struct Entry {
    name: &'static str,
    is_dir: bool,
}

pub struct FileBrowser {
    path: String,
    filter: Text,
    entries: Vec<Entry>,
    selected: Option<usize>,
}

impl Default for FileBrowser {
    fn default() -> Self {
        Self {
            path: "/shared".to_owned(),
            filter: Text::new(""),
            entries: vec![
                Entry { name: "Documents", is_dir: true },
                Entry { name: "Projects", is_dir: true },
                Entry { name: "notes.md", is_dir: false },
                Entry { name: "build.log", is_dir: false },
                Entry { name: "photo.jpg", is_dir: false },
            ],
            selected: None,
        }
    }
}

impl App for FileBrowser {
    fn update(&mut self, ui: &mut Ui<'_>) {
        ui.horizontal(|ui| {
            if ui.button("Up").clicked()
                && let Some(i) = self.path.rfind('/')
                && i > 0
            {
                self.path.truncate(i);
            }
            // Emitted unconditionally: the shell decides hover and draws it, so there is no
            // one-frame lag. Gating this on `hovered()` would visibly stutter.
            ui.tooltip("Go to parent directory");
            ui.label(&self.path);
        });

        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.text_edit(&mut self.filter);
        });

        ui.separator();

        let needle = self.filter.as_str().to_ascii_lowercase();
        for i in 0..self.entries.len() {
            let name = self.entries[i].name;
            let is_dir = self.entries[i].is_dir;
            if !needle.is_empty() && !name.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            // Salt by name, not position: filtering changes which rows are emitted, and
            // position-derived ids would make a click land on whichever row moved into that
            // slot. This is the same failure mode egui has, with the same fix.
            ui.push_id(name, |ui| {
                ui.horizontal(|ui| {
                    // The shell fetches icons by URL, so they hit the HTTP cache and never
                    // occupy this module's linear memory.
                    ui.image("/cas/icon", Vec2::new(16.0, 16.0));
                    ui.label(if is_dir { "[dir]" } else { "     " });
                    if ui.button(name).clicked() {
                        self.selected = Some(i);
                    }
                });
            });
        }

        ui.separator();
        match self.selected {
            Some(i) => ui.label(self.entries[i].name),
            None => ui.label("nothing selected"),
        }
    }
}

ccosel_sdk::ccosel_app!(FileBrowser);
