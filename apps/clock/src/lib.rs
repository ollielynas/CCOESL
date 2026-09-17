//! A clock and stopwatch.
//!
//! Small on purpose: it exists to exercise the parts of the system the File Browser does not.
//! It *animates*, so it is the app that proves the repaint contract — an app that wants to be
//! re-run on a timer says so via `wants_repaint_after_ms`, and an app that does not costs
//! nothing between interactions. A desktop full of idle windows should burn no CPU.

use ccosel_sdk::{App, Ui};

#[derive(Default)]
pub struct Clock {
    running: bool,
    /// Accumulated stopwatch time, excluding any currently-running segment.
    elapsed_ms: f64,
    /// Frame clock reading when the current segment started.
    started_at_ms: f64,
    laps: Vec<f64>,
}

impl Clock {
    fn total(&self, now_ms: f64) -> f64 {
        if self.running {
            self.elapsed_ms + (now_ms - self.started_at_ms)
        } else {
            self.elapsed_ms
        }
    }
}

/// Format as `M:SS.t` without touching float `Display`.
///
/// `format!("{:.1}", x)` on an f64 pulls 20–40 KB of float-formatting machinery into the
/// module. Integer maths costs nothing, and on this project a per-app download is charged to
/// every user on a bad link.
fn format_ms(ms: f64) -> String {
    let total = ms.max(0.0) as u64;
    let tenths = (total / 100) % 10;
    let secs = (total / 1000) % 60;
    let mins = total / 60_000;
    let mut out = String::new();
    out.push_str(itoa(mins).as_str());
    out.push(':');
    if secs < 10 {
        out.push('0');
    }
    out.push_str(itoa(secs).as_str());
    out.push('.');
    out.push_str(itoa(tenths).as_str());
    out
}

fn itoa(mut n: u64) -> String {
    if n == 0 {
        return "0".to_owned();
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    String::from_utf8_lossy(&buf[i..]).into_owned()
}

impl App for Clock {
    fn update(&mut self, ui: &mut Ui<'_>) {
        let now = ui.ctx().time_ms;

        ui.label(format_ms(self.total(now)).as_str());
        ui.separator();

        ui.horizontal(|ui| {
            let label = if self.running { "Stop" } else { "Start" };
            if ui.button(label).clicked() {
                if self.running {
                    self.elapsed_ms += now - self.started_at_ms;
                } else {
                    self.started_at_ms = now;
                }
                self.running = !self.running;
            }

            if ui.button("Lap").clicked() && self.running {
                self.laps.push(self.total(now));
            }

            if ui.button("Reset").clicked() {
                self.running = false;
                self.elapsed_ms = 0.0;
                self.laps.clear();
            }
        });

        if !self.laps.is_empty() {
            ui.separator();
            for (i, lap) in self.laps.iter().enumerate().take(8) {
                ui.push_id(itoa(i as u64).as_str(), |ui| {
                    ui.label(format_ms(*lap).as_str());
                });
            }
        }
    }

    fn wants_repaint_after_ms(&self) -> u32 {
        if self.running {
            // Fast enough that tenths look smooth, slow enough that a windowful of clocks is
            // not a CPU problem.
            50
        } else {
            // Idle: do not re-run me at all until someone clicks.
            ccosel_sdk::REPAINT_ON_INPUT_ONLY
        }
    }
}

ccosel_sdk::ccosel_app!(Clock);
