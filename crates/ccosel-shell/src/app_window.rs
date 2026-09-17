//! One running app, and the per-frame dance around it.

use ccosel_abi::{RespRecord, REPAINT_ON_INPUT_ONLY};
use ccosel_host::{AppInstance, FrameArgs, Replayer};

pub struct AppWindow<I: AppInstance> {
    pub title: String,
    pub icon: &'static str,
    pub app_id: &'static str,
    /// Distinct per *instance*, not per app: two Files windows must not share egui state, or
    /// they would fight over scroll position and focus.
    pub instance_id: u64,
    pub open: bool,
    pub default_size: [f32; 2],
    instance: I,
    replayer: Replayer,
    /// Responses from the previous frame, handed back to the guest on the next one.
    responses: Vec<RespRecord>,
    frame_index: u64,
    /// Retained so a frame the guest failed to produce leaves the last good UI on screen
    /// instead of a blank window.
    last_commands: Vec<u8>,
    error: Option<String>,
}

impl<I: AppInstance> AppWindow<I> {
    pub fn new(
        instance: I,
        instance_id: u64,
        app_id: &'static str,
        title: String,
        icon: &'static str,
        default_size: [f32; 2],
    ) -> Self {
        Self {
            title,
            icon,
            app_id,
            instance_id,
            open: true,
            default_size,
            instance,
            replayer: Replayer::new(),
            responses: Vec::new(),
            frame_index: 0,
            last_commands: Vec::new(),
            error: None,
        }
    }

    /// Bytes the guest emitted last frame — the number that would cross the network if this
    /// app were ever hosted remotely, and a useful health signal either way.
    pub fn command_bytes(&self) -> usize {
        self.last_commands.len()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let screen = ui.ctx().viewport_rect();
        let args = FrameArgs {
            frame_index: self.frame_index,
            time_ms: ui.ctx().input(|i| i.time) * 1000.0,
            dt_ms: ui.ctx().input(|i| i.stable_dt) * 1000.0,
            pixels_per_point: ui.ctx().pixels_per_point(),
            screen_size: [screen.width(), screen.height()],
            flags: if ui.visuals().dark_mode {
                ccosel_abi::frame::input_flags::DARK_MODE
            } else {
                0
            },
            responses: &self.responses,
            events: &[],
        };
        self.frame_index += 1;

        match self.instance.frame(&args) {
            Ok(result) => {
                self.error = None;
                self.last_commands = result.commands;
                if result.wants_repaint_after_ms != REPAINT_ON_INPUT_ONLY {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(
                        result.wants_repaint_after_ms as u64,
                    ));
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err.as_str());
        }

        if !self.last_commands.is_empty() {
            match self.replayer.replay(ui, self.instance_id, &self.last_commands) {
                Ok(responses) => self.responses = responses,
                Err(e) => {
                    // A malformed frame is dropped whole; the previous one stays up.
                    self.error = Some(e.to_string());
                }
            }
        }
    }
}
