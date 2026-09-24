//! The Server Dashboard app.
//!
//! A live stats panel: polls the server periodically and shows protocol info, storage usage,
//! and uptime. No file browser — that's the File Browser app's job.

use ccosel_proto::fs::{ListDir, ListDirReq};
use ccosel_proto::info::{ServerInfo, ServerInfoReq};
use ccosel_sdk::{App, Poll, Ui};

/// How often to re-ask the server, in milliseconds.
const REFRESH_INTERVAL_MS: f64 = 3000.0;

pub struct ServerDashboard {
    last_refresh_ms: f64,
    /// Monotonic timestamp of the first successful `ServerInfo` response, for uptime.
    started_at_ms: Option<f64>,
    /// Cached storage stats from the last successful `ListDir("/")` response.
    total_dirs: usize,
    total_files: usize,
    total_bytes: u64,
}

impl Default for ServerDashboard {
    fn default() -> Self {
        Self {
            last_refresh_ms: 0.0,
            started_at_ms: None,
            total_dirs: 0,
            total_files: 0,
            total_bytes: 0,
        }
    }
}

/// Human-readable size without touching float `Display`.
fn human_size(bytes: u64) -> String {
    let (n, unit) = if bytes >= 1 << 20 {
        (bytes / (1 << 20), "M")
    } else if bytes >= 1 << 10 {
        (bytes / (1 << 10), "K")
    } else {
        (bytes, "B")
    };
    let mut s = String::new();
    s.push_str(itoa(n).as_str());
    s.push_str(unit);
    s
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

/// Format milliseconds as `MM:SS`.
fn format_uptime(ms: f64) -> String {
    let secs = (ms / 1000.0) as u64;
    let m = secs / 60;
    let s = secs % 60;
    let mut out = String::new();
    out.push_str(itoa(m).as_str());
    out.push(':');
    if s < 10 {
        out.push('0');
    }
    out.push_str(itoa(s).as_str());
    out
}

impl App for ServerDashboard {
    fn update(&mut self, ui: &mut Ui<'_>) {
        let now = ui.ctx().time_ms;

        // Auto-refresh: re-ask the server periodically.
        if now - self.last_refresh_ms >= REFRESH_INTERVAL_MS {
            self.last_refresh_ms = now;
            ui.rpc().invalidate::<ServerInfo>(&ServerInfoReq);
            ui.rpc().invalidate::<ListDir>(&ListDirReq { path: "/" });
        }

        ui.label("Server Dashboard");
        ui.separator();

        // ── Server info ──
        ui.label("Server");
        match ui.rpc().get::<ServerInfo>(&ServerInfoReq) {
            Poll::Pending => {
                if self.started_at_ms.is_some() {
                    ui.label("Refreshing…");
                } else {
                    ui.label("Connecting…");
                }
            }
            Poll::Failed(err) => {
                ui.label(err.message());
                if ui.button("Retry").clicked() {
                    ui.rpc().invalidate::<ServerInfo>(&ServerInfoReq);
                }
            }
            Poll::Ready(info) => {
                if self.started_at_ms.is_none() {
                    self.started_at_ms = Some(now);
                }
                ui.horizontal(|ui| {
                    ui.label("Protocol version");
                    ui.label(itoa(u64::from(info.proto_version)).as_str());
                });
                ui.horizontal(|ui| {
                    ui.label("Shared root");
                    ui.label(info.root.as_str());
                });
            }
        }

        // ── Uptime ──
        if let Some(start) = self.started_at_ms {
            ui.horizontal(|ui| {
                ui.label("Uptime");
                ui.label(format_uptime(now - start).as_str());
            });
        }

        // ── Storage stats ──
        ui.separator();
        ui.label("Storage");
        match ui.rpc().get::<ListDir>(&ListDirReq { path: "/" }) {
            Poll::Pending => {
                if self.total_dirs + self.total_files > 0 {
                    // Show cached stats while refreshing.
                    ui.label(
                        format_storage(self.total_dirs, self.total_files, self.total_bytes)
                            .as_str(),
                    );
                } else {
                    ui.label("Loading…");
                }
            }
            Poll::Failed(err) => {
                ui.label(err.message());
                if ui.button("Retry storage").clicked() {
                    ui.rpc().invalidate::<ListDir>(&ListDirReq { path: "/" });
                }
            }
            Poll::Ready(listing) => {
                self.total_dirs = listing.entries.iter().filter(|e| e.is_dir()).count();
                self.total_files = listing.entries.len() - self.total_dirs;
                self.total_bytes = listing
                    .entries
                    .iter()
                    .filter(|e| !e.is_dir())
                    .map(|e| e.size)
                    .sum();
                ui.label(
                    format_storage(self.total_dirs, self.total_files, self.total_bytes).as_str(),
                );
                if listing.truncated {
                    ui.label("(listing truncated — more files than the server returned)");
                }
            }
        }

        // ── Refresh control ──
        ui.separator();
        if ui.button("⟳ Refresh now").clicked() {
            self.last_refresh_ms = now;
            ui.rpc().invalidate::<ServerInfo>(&ServerInfoReq);
            ui.rpc().invalidate::<ListDir>(&ListDirReq { path: "/" });
        }
        ui.label("Auto-refreshes every 3 seconds.");
    }
}

fn format_storage(dirs: usize, files: usize, bytes: u64) -> String {
    let mut out = String::new();
    out.push_str(itoa(dirs as u64).as_str());
    out.push_str(" folders, ");
    out.push_str(itoa(files as u64).as_str());
    out.push_str(" files, ");
    out.push_str(human_size(bytes).as_str());
    out.push_str(" total");
    out
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(ServerDashboard);
