use ccosel_proto::info::{ServerInfo, ServerInfoReply};
use ccosel_sdk::testing::{Harness, rpc_error};

use super::*;

fn info(uptime_ms: u64, rpc_calls: u64) -> ServerInfoReply {
    ServerInfoReply {
        proto_version: 7,
        root: "/srv/shared".to_owned(),
        uptime_ms,
        rpc_calls,
        cpus: 4,
        // 2.0 load over 4 CPUs: 50%.
        load_milli: Some(2000),
        mem_used_kib: Some(3 << 20),
        mem_total_kib: Some(4 << 20),
    }
}

/// A dashboard that has asked once, been answered with `first`, and drawn it.
fn connected(first: &ServerInfoReply) -> Harness<ServerDashboard> {
    let mut h = Harness::new(ServerDashboard::default());
    h.frame();
    h.reply::<ServerInfo>(first);
    h.frame();
    h
}

/// Let the poll interval elapse and answer the re-ask with `next`.
fn poll(h: &mut Harness<ServerDashboard>, next: &ServerInfoReply) {
    h.ctx.time_ms += POLL_INTERVAL_MS;
    h.frame(); // sees the interval has passed and invalidates
    h.frame(); // re-asks
    assert_eq!(h.outstanding::<ServerInfo>(), 1);
    h.reply::<ServerInfo>(next);
    h.frame();
}

#[test]
fn says_connecting_until_the_first_reply() {
    let mut h = Harness::new(ServerDashboard::default());
    h.frame();
    assert_eq!(h.labels(), vec!["Connecting…"]);
    assert!(h.plots().is_empty());
}

#[test]
fn draws_three_graphs_and_the_current_values() {
    let h = connected(&info(65_000, 10));
    assert!(h.has_label("Live · updates every 2 seconds"));
    assert!(h.has_label("Up 1m 5s"));
    assert!(h.has_label("4 CPUs"));
    assert!(h.has_label("50%"));
    assert!(h.has_label("3 GB of 4 GB"));
    assert!(h.has_label("/srv/shared"));
    assert!(h.has_label("Protocol v7"));
    let plots = h.plots();
    assert_eq!(plots.len(), 3);
    assert_eq!(plots[0], vec![127], "50% load is half height");
    assert_eq!(plots[1], vec![191], "75% memory");
    assert!(plots[2].is_empty(), "no rate until there are two readings");
}

#[test]
fn each_poll_adds_a_point_to_every_graph() {
    let mut h = connected(&info(10_000, 0));
    poll(&mut h, &info(12_000, 20));
    poll(&mut h, &info(14_000, 30));
    assert_eq!(h.app.cpu.values.len(), 3);
    assert_eq!(h.app.mem.values.len(), 3);
    // 20 calls in 2 s, then 10 in 2 s.
    assert_eq!(h.app.rpc_rate.values, vec![600, 300]);
    assert!(h.has_label("300/min"));
    // The rate graph is scaled to its own peak.
    assert_eq!(h.plots()[2], vec![255, 127]);
}

#[test]
fn the_same_reply_seen_again_is_not_a_new_point() {
    let mut h = connected(&info(10_000, 0));
    for _ in 0..5 {
        h.frame();
    }
    assert_eq!(h.app.cpu.values.len(), 1);
}

#[test]
fn nothing_changes_while_a_poll_is_in_flight() {
    // The flicker regression: the in-flight frames must draw exactly what the settled ones did.
    let mut h = connected(&info(10_000, 0));
    let before = (h.labels(), h.plots());
    h.ctx.time_ms += POLL_INTERVAL_MS;
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ServerInfo>(), 1, "a poll is in flight");
    assert_eq!((h.labels(), h.plots()), before);
}

#[test]
fn does_not_re_ask_before_the_interval() {
    let mut h = connected(&info(10_000, 0));
    h.ctx.time_ms += POLL_INTERVAL_MS - 1.0;
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ServerInfo>(), 0);
}

#[test]
fn a_failed_poll_keeps_the_graphs_and_says_so() {
    let mut h = connected(&info(10_000, 0));
    let plots = h.plots();
    h.ctx.time_ms += POLL_INTERVAL_MS;
    h.frame();
    h.frame();
    h.fail::<ServerInfo>(rpc_error::TIMEOUT);
    h.frame();
    assert!(h.has_label("Can't reach the server. Showing the last reading."));
    assert_eq!(h.plots(), plots);

    // And recovers on the next good reply.
    h.ctx.time_ms += POLL_INTERVAL_MS;
    h.frame();
    h.frame();
    h.reply::<ServerInfo>(&info(14_000, 5));
    h.frame();
    assert!(h.has_label("Live · updates every 2 seconds"));
}

#[test]
fn failing_before_any_reply_says_retrying() {
    let mut h = Harness::new(ServerDashboard::default());
    h.frame();
    h.fail::<ServerInfo>(rpc_error::TIMEOUT);
    h.frame();
    assert_eq!(h.labels(), vec!["Can't reach the server. Retrying…"]);
}

#[test]
fn a_restarted_server_adds_no_bogus_rate() {
    let mut h = connected(&info(100_000, 500));
    poll(&mut h, &info(1_000, 1));
    assert!(h.app.rpc_rate.values.is_empty());
    assert_eq!(h.app.cpu.values.len(), 2);
}

#[test]
fn missing_host_figures_read_unavailable() {
    let mut r = info(10_000, 0);
    r.load_milli = None;
    r.mem_used_kib = None;
    let h = connected(&r);
    assert_eq!(h.labels().iter().filter(|l| *l == "unavailable").count(), 2);
    assert!(h.app.cpu.values.is_empty());
}

#[test]
fn history_is_capped() {
    let mut s = Series::default();
    for v in 0..(HISTORY as u64 + 5) {
        s.push(v);
    }
    assert_eq!(s.values.len(), HISTORY);
    assert_eq!(s.values[0], 5);
}

#[test]
fn scaling_pins_overflow_and_survives_a_zero_max() {
    let mut s = Series::default();
    s.push(0);
    s.push(50);
    s.push(250);
    assert_eq!(s.scaled(100), vec![0, 127, 255]);
    assert_eq!(Series::default().scaled(0), Vec::<u8>::new());
    let mut z = Series::default();
    z.push(0);
    assert_eq!(z.scaled(0), vec![0]);
}

#[test]
fn formatting() {
    assert_eq!(itoa(0), "0");
    assert_eq!(itoa(1234), "1234");
    assert_eq!(format_duration(5_000), "0m 5s");
    assert_eq!(format_duration(3 * 3_600_000 + 12 * 60_000), "3h 12m");
    assert_eq!(format_duration(2 * 86_400_000 + 5 * 3_600_000), "2d 5h");
    assert_eq!(kib(512 << 10), "512 MB");
    assert_eq!(kib(16 << 20), "16 GB");
    assert_eq!(count(1, "CPU", "CPUs"), "1 CPU");
    assert_eq!(percent(7), "7%");
}

#[test]
fn polls_on_a_timer() {
    assert_eq!(ServerDashboard::default().wants_repaint_after_ms(), 250);
}

#[test]
fn graphs_are_square_tiles_two_to_a_row() {
    let h = connected(&info(10_000, 0));
    let sizes = h.plot_sizes();
    assert_eq!(sizes.len(), 3);
    for size in sizes {
        assert_eq!((size.x, size.y), (TILE, TILE));
    }
}
