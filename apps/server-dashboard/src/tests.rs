use ccosel_proto::fs::{DirEntry, DirListing, EntryKind, ListDir};
use ccosel_proto::info::{ServerInfo, ServerInfoReply};
use ccosel_sdk::testing::{Harness, rpc_error};

use super::*;

fn entry(name: &str, kind: EntryKind, size: u64) -> DirEntry {
    DirEntry {
        name: name.to_owned(),
        kind,
        size,
        mtime_s: 0,
    }
}

fn listing(entries: Vec<DirEntry>) -> DirListing {
    DirListing {
        entries,
        truncated: false,
    }
}

fn server_info(root: &str) -> ServerInfoReply {
    ServerInfoReply {
        proto_version: 7,
        root: root.to_owned(),
    }
}

/// A dashboard that has asked for server info and `/`'s listing, been answered with both, and
/// drawn the result.
fn connected(listing: DirListing) -> Harness<ServerDashboard> {
    let mut h = Harness::new(ServerDashboard::default());
    h.frame();
    h.reply::<ServerInfo>(&server_info("/srv/shared"));
    h.reply::<ListDir>(&listing);
    h.frame();
    h
}

#[test]
fn asks_server_for_info_and_listing_and_shows_loading() {
    let mut h = Harness::new(ServerDashboard::default());
    h.frame();
    assert!(h.has_label("Connecting…"));
    assert_eq!(h.outstanding::<ServerInfo>(), 1);
    assert_eq!(h.outstanding::<ListDir>(), 1);

    // Asking every frame must not duplicate calls.
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ServerInfo>(), 1);
    assert_eq!(h.outstanding::<ListDir>(), 1);
}

#[test]
fn shows_server_info_once_connected() {
    let h = connected(listing(vec![]));
    assert!(h.has_label("Protocol version"));
    assert!(h.has_label("7"));
    assert!(h.has_label("Shared root"));
    assert!(h.has_label("/srv/shared"));
}

#[test]
fn shows_uptime_after_first_response() {
    let h = connected(listing(vec![]));
    // Uptime should be shown in MM:SS format.
    let labels = h.labels();
    assert!(
        labels.iter().any(|l| l.starts_with("0:") || l == "Uptime"),
        "uptime should be visible"
    );
}

#[test]
fn shows_storage_stats() {
    let h = connected(listing(vec![
        entry("docs", EntryKind::Dir, 0),
        entry("notes.md", EntryKind::File, 2048),
        entry("code.rs", EntryKind::File, 4096),
    ]));
    assert!(h.has_label("1 folders, 2 files, 6K total"));
}

#[test]
fn empty_storage_says_so() {
    let h = connected(listing(vec![]));
    assert!(h.has_label("0 folders, 0 files, 0B total"));
}

#[test]
fn truncated_listing_says_so() {
    let mut l = listing(vec![entry("big.bin", EntryKind::File, 1024)]);
    l.truncated = true;
    let h = connected(l);
    assert!(h.has_label("(listing truncated — more files than the server returned)"));
}

#[test]
fn failed_server_info_can_be_retried() {
    let mut h = Harness::new(ServerDashboard::default());
    h.frame();
    h.fail::<ServerInfo>(rpc_error::SERVER);
    h.reply::<ListDir>(&listing(vec![]));
    h.frame();

    assert!(h.has_label("server error"));
    assert!(h.has_button("Retry"));

    h.click("Retry");
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ServerInfo>(), 1, "retrying asks again");
}

#[test]
fn failed_storage_can_be_retried() {
    let mut h = Harness::new(ServerDashboard::default());
    h.frame();
    h.reply::<ServerInfo>(&server_info("/srv"));
    h.fail::<ListDir>(rpc_error::DENIED);
    h.frame();

    assert!(h.has_label("permission denied"));
    assert!(h.has_button("Retry storage"));

    h.click("Retry storage");
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ListDir>(), 1, "retrying asks again");
}

#[test]
fn manual_refresh_asks_both_queries_again() {
    let mut h = connected(listing(vec![]));
    assert_eq!(h.outstanding::<ServerInfo>(), 0);
    assert_eq!(h.outstanding::<ListDir>(), 0);

    h.click("⟳ Refresh now");
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ServerInfo>(), 1);
    assert_eq!(h.outstanding::<ListDir>(), 1);
}

#[test]
fn human_size_uses_largest_whole_unit() {
    assert_eq!(human_size(0), "0B");
    assert_eq!(human_size(1023), "1023B");
    assert_eq!(human_size(1024), "1K");
    assert_eq!(human_size(1024 * 1024 - 1), "1023K");
    assert_eq!(human_size(5 * 1024 * 1024), "5M");
}

#[test]
fn itoa_handles_zero_and_wide_values() {
    assert_eq!(itoa(0), "0");
    assert_eq!(itoa(u64::MAX), "18446744073709551615");
}

#[test]
fn format_uptime_shows_mm_colon_ss() {
    assert_eq!(format_uptime(0.0), "0:00");
    assert_eq!(format_uptime(60_000.0), "1:00");
    assert_eq!(format_uptime(90_000.0), "1:30");
    assert_eq!(format_uptime(5_000.0), "0:05");
}
