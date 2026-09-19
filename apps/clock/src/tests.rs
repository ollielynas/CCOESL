//! The clock is driven through the SDK's native harness: time is an input (`ctx.time_ms`), a click
//! reaches the app one frame after it happens, and the assertions are on what was drawn.

use ccosel_sdk::REPAINT_ON_INPUT_ONLY;
use ccosel_sdk::testing::Harness;

use super::*;

fn clock() -> Harness<Clock> {
    Harness::new(Clock::default())
}

/// Clicks `button` at time `now`, then runs the frame that observes the click and the one that
/// redraws with its effect.
fn press(h: &mut Harness<Clock>, button: &str, now: f64) {
    h.ctx.time_ms = now;
    h.click(button);
    h.frame();
    h.frame();
}

fn at(h: &mut Harness<Clock>, now: f64) {
    h.ctx.time_ms = now;
    h.frame();
}

#[test]
fn formats_minutes_seconds_and_tenths() {
    assert_eq!(format_ms(0.0), "0:00.0");
    assert_eq!(format_ms(61_234.0), "1:01.2");
    assert_eq!(format_ms(3_599_999.0), "59:59.9");
    assert_eq!(format_ms(3_600_000.0), "60:00.0");
}

#[test]
fn a_negative_time_formats_as_zero() {
    // A frame clock that steps backwards must not underflow into a huge number.
    assert_eq!(format_ms(-5.0), "0:00.0");
}

#[test]
fn itoa_handles_zero_and_the_widest_value() {
    assert_eq!(itoa(0), "0");
    assert_eq!(itoa(1_234_567_890), "1234567890");
    // Exactly fills the 20-byte buffer.
    assert_eq!(itoa(u64::MAX), "18446744073709551615");
}

#[test]
fn an_idle_clock_shows_zero_and_costs_no_repaints() {
    let mut h = clock();
    at(&mut h, 1000.0);
    assert_eq!(h.labels(), ["0:00.0"]);
    assert_eq!(h.buttons(), ["Start", "Lap", "Reset"]);
    assert_eq!(h.app.wants_repaint_after_ms(), REPAINT_ON_INPUT_ONLY);
}

#[test]
fn running_counts_time_and_asks_to_be_repainted() {
    let mut h = clock();
    at(&mut h, 1000.0);
    press(&mut h, "Start", 1000.0);
    assert_eq!(h.app.wants_repaint_after_ms(), 50);

    at(&mut h, 3500.0);
    assert_eq!(h.labels(), ["0:02.5"]);
    assert!(h.has_button("Stop"), "Start becomes Stop while running");
}

#[test]
fn stopping_freezes_the_time() {
    let mut h = clock();
    at(&mut h, 1000.0);
    press(&mut h, "Start", 1000.0);
    at(&mut h, 3500.0);
    press(&mut h, "Stop", 3500.0);

    at(&mut h, 9000.0);
    assert_eq!(
        h.labels(),
        ["0:02.5"],
        "time must not advance while stopped"
    );
    assert!(h.has_button("Start"));
    assert_eq!(h.app.wants_repaint_after_ms(), REPAINT_ON_INPUT_ONLY);
}

#[test]
fn restarting_resumes_from_the_frozen_time() {
    let mut h = clock();
    at(&mut h, 0.0);
    press(&mut h, "Start", 0.0);
    press(&mut h, "Stop", 2000.0);
    press(&mut h, "Start", 5000.0);

    at(&mut h, 6000.0);
    assert_eq!(h.labels(), ["0:03.0"], "2s before the pause plus 1s after");
}

#[test]
fn laps_are_recorded_while_running() {
    let mut h = clock();
    at(&mut h, 0.0);
    press(&mut h, "Start", 0.0);
    press(&mut h, "Lap", 1500.0);
    press(&mut h, "Lap", 4000.0);

    assert_eq!(h.labels(), ["0:04.0", "0:01.5", "0:04.0"]);
}

#[test]
fn lap_does_nothing_while_stopped() {
    let mut h = clock();
    at(&mut h, 0.0);
    press(&mut h, "Lap", 500.0);
    assert_eq!(h.labels(), ["0:00.0"]);
}

#[test]
fn only_the_first_eight_laps_are_shown() {
    let mut h = clock();
    at(&mut h, 0.0);
    press(&mut h, "Start", 0.0);
    for i in 1..=10 {
        press(&mut h, "Lap", i as f64 * 1000.0);
    }
    assert_eq!(h.app.laps.len(), 10, "all laps are kept");
    assert_eq!(h.labels().len(), 1 + 8, "the display plus eight laps");
}

#[test]
fn reset_clears_time_laps_and_running() {
    let mut h = clock();
    at(&mut h, 0.0);
    press(&mut h, "Start", 0.0);
    press(&mut h, "Lap", 2000.0);
    press(&mut h, "Reset", 3000.0);

    at(&mut h, 4000.0);
    assert_eq!(h.labels(), ["0:00.0"]);
    assert!(h.has_button("Start"));
    assert_eq!(h.app.wants_repaint_after_ms(), REPAINT_ON_INPUT_ONLY);
}
