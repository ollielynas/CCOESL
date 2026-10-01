use super::*;

#[test]
fn resample_interpolates_rising_x() {
    let y = resample(&[0.0, 1.0, 3.0], &[0.0, 10.0, 30.0]);
    assert_eq!(y.len(), SAMPLES);
    assert_eq!(y[0], 0.0);
    assert_eq!(y[SAMPLES - 1], 30.0);
    // Linear throughout, since y = 10x.
    let mid = 3.0 * 60.0 / (SAMPLES - 1) as f64;
    assert!((y[60] - 10.0 * mid).abs() < 1e-9);
}

#[test]
fn resample_keeps_order_when_x_goes_back() {
    let y = resample(&[0.0, 2.0, 1.0], &[5.0, 6.0, 7.0]);
    assert_eq!((y[0], y[SAMPLES - 1]), (5.0, 7.0));
}

#[test]
fn resample_handles_tiny_inputs() {
    assert!(resample(&[], &[]).is_empty());
    assert_eq!(resample(&[1.0], &[4.0]), vec![4.0; SAMPLES]);
}

#[test]
fn bytes_span_the_range() {
    assert_eq!(to_byte(-1.0, -1.0, 1.0), 0);
    assert_eq!(to_byte(1.0, -1.0, 1.0), 255);
    assert_eq!(
        to_byte(5.0, 2.0, 2.0),
        128,
        "a flat line sits in the middle"
    );
    assert_eq!(to_byte(f64::NAN, 0.0, 1.0), 0);
}

#[test]
fn numbers_read_like_percent_g() {
    assert_eq!(fmt_num(0.0), "0");
    assert_eq!(fmt_num(3.456789), "3.4568");
    assert_eq!(fmt_num(-2.0), "-2");
    assert_eq!(fmt_num(1.5e7), "1.500e7");
    assert_eq!(fmt_num(0.0001), "1.000e-4");
    let n = numbers("1 NaN Inf -2.5e-3");
    assert_eq!((n[0], n[2], n[3]), (1.0, f64::INFINITY, -0.0025));
    assert!(n[1].is_nan());
    assert!(numbers("x")[0].is_nan());
}

#[test]
fn working_folders_become_jail_paths() {
    let root = Path::new("/srv/jail");
    assert_eq!(jail_path(Path::new("/srv/jail"), root), "/");
    assert_eq!(jail_path(Path::new("/srv/jail/home/al"), root), "/home/al");
    assert_eq!(jail_path(Path::new("/etc"), root), "");
}

#[test]
fn strings_are_escaped_for_octave() {
    assert_eq!(octave_string(r#"a"b\c"#), r#"a\"b\\c"#);
}

#[test]
fn report_caps_what_it_carries() {
    let mut lines: Vec<String> = (0..MAX_VARIABLES + 5)
        .map(|i| format!("VAR\tv{i}\tdouble\t1x1\t1\t"))
        .collect();
    for i in 0..MAX_FIGURES + 2 {
        lines.push(format!("FIG\t{i}\t"));
    }
    lines.push("LINE\torphan\t1 2\t3 4".to_owned());
    let r = parse_report(&lines, Path::new("/"));
    assert_eq!(r.variables.len(), MAX_VARIABLES);
    assert!(r.variables_truncated);
    assert_eq!(r.figures.len(), MAX_FIGURES);
    assert!(r.figures.iter().all(|f| f.axes.is_empty()));
}

#[test]
fn output_is_sent_from_where_the_app_is_up_to() {
    let job = Job::new(JOB_TIMEOUT);
    job.push_line("héllo");
    let s = job.snapshot(0);
    assert_eq!(s.output, "héllo\n");
    assert!(!s.finished);
    // Mid-character: back up to the start of it rather than split it.
    let s = job.snapshot(2);
    assert_eq!(s.output, "éllo\n");
    let s = job.snapshot(1000);
    assert_eq!((s.output.as_str(), s.next), ("", 7));
}

#[test]
fn output_stops_at_the_cap() {
    let job = Job::new(JOB_TIMEOUT);
    let line = "x".repeat(1000);
    for _ in 0..(MAX_OUTPUT_BYTES / 1000 + 5) {
        job.push_line(&line);
    }
    let s = job.snapshot(0);
    assert!(s.output_truncated);
    assert!(s.output.len() <= MAX_OUTPUT_BYTES);
}
