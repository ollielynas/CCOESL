use super::*;

#[test]
fn loadavg_is_read_in_thousandths() {
    assert_eq!(parse_loadavg("0.52 0.58 0.59 1/467 12345\n"), Some(520));
    assert_eq!(parse_loadavg("12.05 1 1 1/1 1"), Some(12_050));
    assert_eq!(parse_loadavg("3 0 0"), Some(3000));
    assert_eq!(parse_loadavg("0.5"), Some(500));
    assert_eq!(parse_loadavg("1.23456"), Some(1234));
}

#[test]
fn malformed_loadavg_is_none_not_a_panic() {
    assert_eq!(parse_loadavg(""), None);
    assert_eq!(parse_loadavg("abc"), None);
    assert_eq!(parse_loadavg("1.x2"), None);
    assert_eq!(parse_loadavg("-1.0"), None);
}

#[test]
fn meminfo_used_is_total_minus_available() {
    let s =
        "MemTotal:       16000000 kB\nMemFree:          100000 kB\nMemAvailable:    4000000 kB\n";
    assert_eq!(parse_meminfo(s), Some((12_000_000, 16_000_000)));
}

#[test]
fn meminfo_missing_a_field_is_none() {
    assert_eq!(parse_meminfo("MemTotal: 100 kB\n"), None);
    // A field that merely starts with the name must not match.
    assert_eq!(
        parse_meminfo("MemTotalX: 5 kB\nMemTotal: 100 kB\nMemAvailable: 40 kB\n"),
        Some((60, 100))
    );
}

#[test]
fn rpc_calls_count_up() {
    let s = Stats::new();
    assert_eq!(s.rpc_calls(), 0);
    s.count_rpc();
    s.count_rpc();
    assert_eq!(s.rpc_calls(), 2);
}

/// The parsers are tested on fixed text above; this checks they are pointed at the real files.
/// Linux only, because the figures come from `/proc`; elsewhere they read as unavailable.
#[cfg(target_os = "linux")]
#[test]
fn host_sample_reads_load_and_memory_from_proc() {
    let host = sample_host();
    assert!(host.load_milli.is_some(), "no load from /proc/loadavg");
    let (used, total) = (host.mem_used_kib.unwrap(), host.mem_total_kib.unwrap());
    assert!(total > 0 && used <= total, "{used} of {total} KiB");
}
