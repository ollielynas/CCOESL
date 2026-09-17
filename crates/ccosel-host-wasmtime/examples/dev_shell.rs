//! The native dev shell: a real `.wasm` app, loaded across a real memory boundary, rendered
//! with real egui.
//!
//! This is the day-to-day development loop for the whole project. The module it loads is
//! byte-identical to the one the browser will fetch — no `wasm-bindgen`, no JS glue — so a bug
//! reproduced here is the bug that would happen in the browser, but with a debugger, real
//! backtraces and a one-second iteration time.
//!
//! Run with: `cargo run -p ccosel-host-wasmtime --example dev_shell`

use std::path::PathBuf;
use std::time::Instant;

use ccosel_abi::{RespRecord, REPAINT_ON_INPUT_ONLY};
use ccosel_host::{AppHost, AppInstance, FrameArgs, Replayer};
use ccosel_host_wasmtime::{WasmtimeHost, WasmtimeInstance};

const APP_ID: u64 = 1;

fn guest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("apps/target/wasm32-unknown-unknown/release/file_browser.wasm")
}

struct DevShell {
    app: WasmtimeInstance,
    replayer: Replayer,
    responses: Vec<RespRecord>,
    started: Instant,
    frame_index: u64,
    module_bytes: usize,
    last_cmd_bytes: usize,
    wants_repaint: u32,
    error: Option<String>,
    log: Vec<String>,
}

impl DevShell {
    fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let path = guest_path();
        let wasm = std::fs::read(&path).map_err(|e| {
            format!(
                "{}: {e}\nBuild the guest first:\n  cd apps && cargo build --release --target wasm32-unknown-unknown",
                path.display()
            )
        })?;

        let host = WasmtimeHost::new();
        let module = pollster::block_on(host.compile(&wasm))?;
        let app = host.instantiate(&module)?;

        Ok(Self {
            app,
            replayer: Replayer::new(),
            responses: Vec::new(),
            started: Instant::now(),
            frame_index: 0,
            module_bytes: wasm.len(),
            last_cmd_bytes: 0,
            wants_repaint: REPAINT_ON_INPUT_ONLY,
            error: None,
            log: Vec::new(),
        })
    }
}

impl eframe::App for DevShell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let screen = ui.ctx().viewport_rect();

        // 1. Run the guest, handing back the responses egui produced last frame.
        let args = FrameArgs {
            frame_index: self.frame_index,
            time_ms: self.started.elapsed().as_secs_f64() * 1000.0,
            dt_ms: ui.ctx().input(|i| i.stable_dt) * 1000.0,
            pixels_per_point: ui.ctx().pixels_per_point(),
            screen_size: [screen.width(), screen.height()],
            flags: 0,
            responses: &self.responses,
            events: &[],
        };
        self.frame_index += 1;

        let commands = match self.app.frame(&args) {
            Ok(result) => {
                self.last_cmd_bytes = result.commands.len();
                self.wants_repaint = result.wants_repaint_after_ms;
                self.error = None;
                Some(result.commands)
            }
            Err(e) => {
                self.error = Some(e.to_string());
                None
            }
        };

        for (_level, line) in self.app.take_log() {
            self.log.push(line);
        }

        egui::Panel::right("stats").show(ui, |ui| {
            ui.heading("Boundary");
            ui.label(format!("module: {} bytes", self.module_bytes));
            ui.label(format!("commands: {} bytes/frame", self.last_cmd_bytes));
            ui.label(format!(
                "repaint: {}",
                if self.wants_repaint == REPAINT_ON_INPUT_ONLY {
                    "on input only".to_owned()
                } else {
                    format!("{} ms", self.wants_repaint)
                }
            ));
            ui.separator();
            ui.small(
                "The app is a separate wasm module with its own linear memory. It links no \
                 egui at all — it records commands, the shell draws them.",
            );
            if let Some(err) = &self.error {
                ui.separator();
                ui.colored_label(egui::Color32::RED, err);
            }
            if !self.log.is_empty() {
                ui.separator();
                ui.label("guest log");
                for line in self.log.iter().rev().take(8) {
                    ui.small(line);
                }
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let Some(commands) = commands else { return };
            // 2. Decode and replay into a Ui the shell owns.
            match self.replayer.replay(ui, APP_ID, &commands) {
                // 3. Responses go back across the boundary on the next frame.
                Ok(responses) => self.responses = responses,
                Err(e) => self.error = Some(e.to_string()),
            }
        });
    }
}

fn main() -> eframe::Result {
    let shell = match DevShell::load() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    eframe::run_native(
        "CCOSEL — native dev shell",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(shell))),
    )
}
