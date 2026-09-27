//! Live numbers about the server and the machine it runs on, for `ServerInfo`.
//!
//! Host figures come from `/proc`, since the server ships in a Linux container. Elsewhere they
//! are `None` rather than a guess, and the dashboard says "unavailable".

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub struct Stats {
    started: Instant,
    rpc_calls: AtomicU64,
}

impl Default for Stats {
    fn default() -> Self {
        Self::new()
    }
}

impl Stats {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            rpc_calls: AtomicU64::new(0),
        }
    }

    pub fn count_rpc(&self) {
        self.rpc_calls.fetch_add(1, Ordering::Relaxed);
    }

    pub fn rpc_calls(&self) -> u64 {
        self.rpc_calls.load(Ordering::Relaxed)
    }

    pub fn uptime_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// One reading of the host.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Host {
    pub cpus: u32,
    pub load_milli: Option<u32>,
    pub mem_used_kib: Option<u64>,
    pub mem_total_kib: Option<u64>,
}

pub fn sample_host() -> Host {
    let cpus = std::thread::available_parallelism()
        .map(|n| u32::try_from(n.get()).unwrap_or(u32::MAX))
        .unwrap_or(1);
    let load_milli = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| parse_loadavg(&s));
    let (mem_used_kib, mem_total_kib) = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| parse_meminfo(&s))
        .map_or((None, None), |(used, total)| (Some(used), Some(total)));
    Host {
        cpus,
        load_milli,
        mem_used_kib,
        mem_total_kib,
    }
}

/// The 1-minute figure of `/proc/loadavg` (`"0.52 0.58 0.59 1/467 12345"`), ×1000.
pub fn parse_loadavg(s: &str) -> Option<u32> {
    let first = s.split_whitespace().next()?;
    let (int, frac) = first.split_once('.').unwrap_or((first, "0"));
    let int: u32 = int.parse().ok()?;
    // Pad or cut the fraction to exactly three digits, so "0.5" is 500, not 5.
    let mut milli = 0u32;
    let mut digits = frac.bytes();
    for scale in [100, 10, 1] {
        let d = match digits.next() {
            Some(b @ b'0'..=b'9') => u32::from(b - b'0'),
            Some(_) => return None,
            None => 0,
        };
        milli += d * scale;
    }
    int.checked_mul(1000)?.checked_add(milli)
}

/// `(used, total)` in KiB from `/proc/meminfo`. "Used" is total minus `MemAvailable`, which
/// counts reclaimable cache as free — what `free` and `htop` show.
pub fn parse_meminfo(s: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        s.lines().find_map(|l| {
            let rest = l.strip_prefix(name)?.strip_prefix(':')?;
            rest.split_whitespace().next()?.parse::<u64>().ok()
        })
    };
    let total = field("MemTotal")?;
    let available = field("MemAvailable")?;
    Some((total.saturating_sub(available), total))
}

#[cfg(test)]
mod tests;
