//! The Server Dashboard app.
//!
//! Live graphs of how the server machine is doing: CPU load, memory in use and RPC traffic.
//! It polls `ServerInfo` and keeps the history itself, so the server stays stateless about who
//! is watching.
//!
//! Nothing on screen changes shape while a poll is in flight: the last reply stays drawn until
//! the next one lands, and a failed poll only changes the one status line. Swapping the stats
//! for "Refreshing…" on every poll is what made the first version flicker.

use ccosel_proto::info::{ServerInfo, ServerInfoReply, ServerInfoReq};
use ccosel_sdk::{App, Poll, Ui, Vec2};

/// How often to ask the server, in milliseconds.
pub const POLL_INTERVAL_MS: f64 = 2000.0;
/// Points kept per graph: two minutes at the poll interval.
pub const HISTORY: usize = 60;
/// Side of each square graph, in points. Two across fit the window's default width.
pub const TILE: f32 = 170.0;

/// The last [`HISTORY`] readings of one statistic, oldest first.
#[derive(Default)]
pub struct Series {
    pub values: Vec<u64>,
}

impl Series {
    pub fn push(&mut self, v: u64) {
        if self.values.len() == HISTORY {
            self.values.remove(0);
        }
        self.values.push(v);
    }

    /// The samples scaled to `0..=255` against `max`, as `Ui::plot` takes them. Values above
    /// `max` are pinned to the top rather than wrapping.
    pub fn scaled(&self, max: u64) -> Vec<u8> {
        let max = max.max(1);
        self.values
            .iter()
            .map(|&v| (v.min(max) * 255 / max) as u8)
            .collect()
    }

    pub fn peak(&self) -> u64 {
        self.values.iter().copied().max().unwrap_or(0)
    }

    pub fn latest(&self) -> u64 {
        self.values.last().copied().unwrap_or(0)
    }
}

#[derive(Default)]
pub struct ServerDashboard {
    /// The most recent reply. Drawn every frame, including while the next poll is in flight.
    pub last: Option<ServerInfoReply>,
    /// Whether the latest poll failed. Only changes the status line, never the graphs.
    pub unreachable: bool,
    last_poll_ms: f64,
    /// 1-minute load as a percentage of all CPUs.
    pub cpu: Series,
    /// Memory in use, as a percentage of total.
    pub mem: Series,
    /// RPC calls per minute between consecutive polls.
    pub rpc_rate: Series,
}

impl ServerDashboard {
    /// Fold a reply into the history. A reply for the same moment as the last one (the cached
    /// value, seen again on a later frame) is not a new reading.
    fn record(&mut self, info: &ServerInfoReply) {
        if let Some(prev) = &self.last {
            if info.uptime_ms == prev.uptime_ms {
                self.unreachable = false;
                return;
            }
            // A restarted server's counters start again from zero, so there's no rate to take.
            if info.uptime_ms > prev.uptime_ms {
                let calls = info.rpc_calls.saturating_sub(prev.rpc_calls);
                let ms = info.uptime_ms - prev.uptime_ms;
                self.rpc_rate.push(calls * 60_000 / ms);
            }
        }
        if let Some(load) = info.load_milli {
            self.cpu
                .push(u64::from(load) / 10 / u64::from(info.cpus.max(1)));
        }
        if let (Some(used), Some(total)) = (info.mem_used_kib, info.mem_total_kib) {
            self.mem.push(used * 100 / total.max(1));
        }
        self.last = Some(info.clone());
        self.unreachable = false;
    }
}

impl App for ServerDashboard {
    fn update(&mut self, ui: &mut Ui<'_>) {
        let now = ui.ctx().time_ms;

        let settled = match ui.rpc().get::<ServerInfo>(&ServerInfoReq) {
            Poll::Pending => false,
            Poll::Failed(_) => {
                self.unreachable = true;
                true
            }
            Poll::Ready(info) => {
                self.record(&info);
                true
            }
        };
        // Ask again only once the previous call has settled: invalidating an in-flight call
        // cancels it, so a server slower than the interval would otherwise never answer.
        if settled && now - self.last_poll_ms >= POLL_INTERVAL_MS {
            self.last_poll_ms = now;
            ui.rpc().invalidate::<ServerInfo>(&ServerInfoReq);
        }

        let Some(info) = self.last.clone() else {
            ui.label(if self.unreachable {
                "Can't reach the server. Retrying…"
            } else {
                "Connecting…"
            });
            return;
        };

        ui.label(if self.unreachable {
            "Can't reach the server. Showing the last reading."
        } else {
            "Live · updates every 2 seconds"
        });
        ui.separator();

        // A 2×2 grid of square tiles: three graphs, and the server's vital facts in the fourth.
        let cpu = match info.load_milli {
            Some(_) => percent(self.cpu.latest()),
            None => "unavailable".to_owned(),
        };
        let mem = match (info.mem_used_kib, info.mem_total_kib) {
            (Some(used), Some(total)) => {
                let mut s = kib(used);
                s.push_str(" of ");
                s.push_str(&kib(total));
                s
            }
            _ => "unavailable".to_owned(),
        };
        let mut rate = itoa(self.rpc_rate.latest());
        rate.push_str("/min");
        // Scaled to the busiest moment on screen, so a quiet server still shows its shape.
        let rate_samples = self.rpc_rate.scaled(self.rpc_rate.peak());

        ui.horizontal(|ui| {
            tile(
                ui,
                "CPU load",
                &cpu,
                &self.cpu.scaled(100),
                "1-minute load average across all CPUs, over the last 2 minutes",
            );
            tile(
                ui,
                "Memory",
                &mem,
                &self.mem.scaled(100),
                "Memory in use, as a share of the total, over the last 2 minutes",
            );
        });
        ui.horizontal(|ui| {
            tile(
                ui,
                "Requests",
                &rate,
                &rate_samples,
                "RPC calls the server answered per minute, from every client",
            );
            ui.vertical(|ui| {
                ui.label("Server");
                ui.label(&count(u64::from(info.cpus), "CPU", "CPUs"));
                let mut up = "Up ".to_owned();
                up.push_str(&format_duration(info.uptime_ms));
                ui.label(&up);
                let mut proto = "Protocol v".to_owned();
                proto.push_str(&itoa(u64::from(info.proto_version)));
                ui.label(&proto);
            });
        });
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("Serving");
            ui.label(&info.root);
        });
    }

    fn wants_repaint_after_ms(&self) -> u32 {
        // Often enough to notice a settled poll promptly; the poll itself is still 2 s apart.
        250
    }
}

/// One square graph with its name and current value above it.
fn tile(ui: &mut Ui<'_>, title: &str, value: &str, samples: &[u8], tip: &str) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(title);
            ui.label(value);
        });
        ui.plot(samples, Vec2::new(TILE, TILE));
        ui.tooltip(tip);
    });
}

/// Decimal digits, without float `Display` (which alone would blow the size budget).
pub fn itoa(mut n: u64) -> String {
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

pub fn percent(n: u64) -> String {
    let mut s = itoa(n);
    s.push('%');
    s
}

pub fn count(n: u64, one: &str, many: &str) -> String {
    let mut s = itoa(n);
    s.push(' ');
    s.push_str(if n == 1 { one } else { many });
    s
}

/// KiB as whole MB or GB, whichever reads better.
pub fn kib(n: u64) -> String {
    let (v, unit) = if n >= 1 << 20 {
        (n >> 20, " GB")
    } else {
        (n >> 10, " MB")
    };
    let mut s = itoa(v);
    s.push_str(unit);
    s
}

/// `3d 4h`, `4h 12m` or `12m 5s`: the two largest units, which is all an uptime needs.
pub fn format_duration(ms: u64) -> String {
    let secs = ms / 1000;
    let (d, h, m, s) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let (a, au, b, bu) = if d > 0 {
        (d, "d ", h, "h")
    } else if h > 0 {
        (h, "h ", m, "m")
    } else {
        (m, "m ", s, "s")
    };
    let mut out = itoa(a);
    out.push_str(au);
    out.push_str(&itoa(b));
    out.push_str(bu);
    out
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(ServerDashboard);
