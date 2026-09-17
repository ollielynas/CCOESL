//! Milestone-1 checkpoint: pixels on screen, with no wasm anywhere.
//!
//! An app written against `ccosel-sdk` records a command buffer; `ccosel-host` decodes it and
//! replays it into a real `egui::Ui`. This is the whole rendering thesis end to end, running
//! natively so it can be stepped in a debugger.
//!
//! The side panel shows the per-frame byte cost, because that number is the reason the design
//! exists: what crosses the boundary here is what would cross a slow LAN if the app were
//! server-hosted.
//!
//! Run with: `cargo run -p ccosel-host --example replay_demo`

use ccosel_host::Replayer;
use ccosel_sdk::{App, Recorder, Text, Ui, Vec2};

const APP_ID: u64 = 1;

/// Stand-in for the real File Browser, exercising every opcode in the v0 ABI.
struct MockFileBrowser {
    path: String,
    filter: Text,
    entries: Vec<(&'static str, bool)>,
    selected: Option<usize>,
    clicks: u32,
}

impl Default for MockFileBrowser {
    fn default() -> Self {
        Self {
            path: "/shared".to_owned(),
            filter: Text::new(""),
            entries: vec![
                ("Documents", true),
                ("Projects", true),
                ("notes.md", false),
                ("build.log", false),
                ("photo.jpg", false),
            ],
            selected: None,
            clicks: 0,
        }
    }
}

impl App for MockFileBrowser {
    fn update(&mut self, ui: &mut Ui<'_>) {
        ui.horizontal(|ui| {
            if ui.button("Up").clicked() {
                self.clicks += 1;
                if let Some(i) = self.path.rfind('/')
                    && i > 0
                {
                    self.path.truncate(i);
                }
            }
            ui.tooltip("Go to parent directory");
            ui.label(&self.path);
        });

        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.text_edit(&mut self.filter);
        });

        ui.separator();

        // Explicit id salts, so a row keeps its identity when the filter changes the list.
        // Without this, auto-ids shift with position and clicks land on the wrong row.
        let needle = self.filter.as_str().to_ascii_lowercase();
        for (i, (name, is_dir)) in self.entries.clone().iter().enumerate() {
            if !needle.is_empty() && !name.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            ui.push_id(name, |ui| {
                ui.horizontal(|ui| {
                    ui.image("/cas/icon", Vec2::new(16.0, 16.0));
                    let label = if *is_dir { "[dir]" } else { "     " };
                    ui.label(label);
                    if ui.button(name).clicked() {
                        self.selected = Some(i);
                    }
                });
            });
        }

        ui.separator();
        match self.selected {
            Some(i) => ui.label(self.entries[i].0),
            None => ui.label("nothing selected"),
        }
    }
}

struct DemoShell {
    app: MockFileBrowser,
    recorder: Recorder,
    replayer: Replayer,
    last_bytes: usize,
    last_error: Option<String>,
}

impl Default for DemoShell {
    fn default() -> Self {
        Self {
            app: MockFileBrowser::default(),
            recorder: Recorder::new(),
            replayer: Replayer::new(),
            last_bytes: 0,
            last_error: None,
        }
    }
}

impl eframe::App for DemoShell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 1. The guest records a frame. In the real system this happens inside a wasm module
        //    with its own linear memory; here it is a plain function call.
        {
            let mut ui = Ui::root(&mut self.recorder);
            self.app.update(&mut ui);
        }
        self.last_bytes = self.recorder.commands().len();

        egui::Panel::right("stats").show(ui, |ui| {
            ui.heading("Command stream");
            ui.label(format!("{} bytes/frame", self.last_bytes));
            ui.label(format!("clicks: {}", self.app.clicks));
            ui.separator();
            ui.small(
                "This is what crosses the memory boundary each frame. \
                 The app links no egui at all — it records, the shell draws.",
            );
            if let Some(err) = &self.last_error {
                ui.separator();
                ui.colored_label(egui::Color32::RED, err);
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            // 2. The shell decodes and replays into a Ui it owns.
            match self.replayer.replay(ui, APP_ID, self.recorder.commands()) {
                Ok(responses) => {
                    self.last_error = None;
                    // 3. Responses go back to the guest for its *next* frame.
                    self.recorder.set_responses(responses);
                }
                Err(e) => {
                    // A malformed frame is discarded whole; the app stays on screen.
                    self.last_error = Some(e.to_string());
                }
            }
        });
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "CCOSEL — replay demo",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::<DemoShell>::default())),
    )
}
