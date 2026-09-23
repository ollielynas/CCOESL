//! Pure decision logic for issue #3: given a coarse read of the user's connection, decide
//! whether the desktop background should be a photographic image or drawn abstract shapes.
//!
//! Deliberately has no `web-sys`/`js-sys` dependency, so it is unit-testable as plain Rust on
//! any target, with no browser and no `wasm32` build in the loop. The browser-side glue that
//! reads the actual `navigator.connection` values and does the fetch/paint lives in
//! `ccosel-shell` (wasm-only, so it cannot itself be exercised outside a browser) and calls
//! straight into this crate for the decision.

#![forbid(unsafe_code)]

/// A coarse read of the network, as reported by the browser's Network Information API
/// (`navigator.connection`). Every field is optional/best-effort: that API is experimental and
/// unimplemented outright in some browsers (notably Firefox and Safari), and even where it
/// exists the browser may decline to report a given field.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Signal {
    /// The browser's own bucketed judgment: `"slow-2g"`, `"2g"`, `"3g"` or `"4g"`. Preferred
    /// over `downlink_mbps` when both are present, since it already accounts for round-trip
    /// time and the browser's own smoothing, not just raw throughput.
    pub effective_type: Option<String>,
    /// Estimated downlink bandwidth in megabits/second. Consulted only when `effective_type`
    /// is unavailable.
    pub downlink_mbps: Option<f32>,
    /// `navigator.connection.saveData`: the user (or their browser/OS) has explicitly asked
    /// sites to use less data. This always wins over the other two signals.
    pub save_data: bool,
}

/// How good the connection looks, coarsely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Good,
    Poor,
    Unknown,
}

/// What the desktop should draw as its background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    /// Fetch and display a real image. Chosen only when the connection looks like it can
    /// afford the extra bytes.
    Image,
    /// Draw cheap, local, procedural shapes instead. The safe default: it costs nothing on the
    /// wire, which matters on a connection that is slow, unmeasurable, or has asked for less
    /// data.
    Shapes,
}

/// Classifies a [`Signal`] into a coarse [`Quality`].
///
/// Assumptions, documented here because the ticket only says "detect the strength of the
/// user's internet":
/// - `save_data` always wins: it is the user asking for less data, not a bandwidth measurement,
///   so it overrides whatever the other two fields say.
/// - Of `effective_type`'s four buckets, only `"4g"` counts as good; `"3g"` and below count as
///   poor. This project targets a LAN that can be slower than the label suggests (see
///   `ARCHITECTURE.md`: "the LAN is unreliable and slow"), so the bar is deliberately not
///   generous.
/// - Without `effective_type`, `downlink_mbps >= 1.5` counts as good — roughly the floor the
///   Network Information spec itself uses for its own `"4g"` bucket.
/// - With no usable signal at all (the common case today: Firefox and Safari do not implement
///   the API), the connection defaults to `Good` — this is a LAN app where most users have
///   fast, reliable connections, so assuming the worst would leave the wallpaper permanently
///   disabled for a large portion of the user base.
pub fn classify(signal: &Signal) -> Quality {
    if signal.save_data {
        return Quality::Poor;
    }
    if let Some(effective_type) = signal.effective_type.as_deref() {
        return match effective_type {
            "4g" => Quality::Good,
            "slow-2g" | "2g" | "3g" => Quality::Poor,
            _ => {
                // Unrecognised bucket — fall through to check downlink rather than giving up.
                if let Some(downlink) = signal.downlink_mbps {
                    return if downlink >= 1.5 {
                        Quality::Good
                    } else {
                        Quality::Poor
                    };
                }
                Quality::Good
            }
        };
    }
    if let Some(downlink) = signal.downlink_mbps {
        return if downlink >= 1.5 {
            Quality::Good
        } else {
            Quality::Poor
        };
    }
    Quality::Good
}

/// Turns a [`Quality`] into a background choice. `Unknown` draws shapes rather than risk an
/// image download on a connection that might not be able to afford it — consistent with this
/// project's stance that bytes on the wire are the scarce resource (see `ARCHITECTURE.md`).
pub fn choose_background(quality: Quality) -> Background {
    match quality {
        Quality::Good => Background::Image,
        Quality::Poor | Quality::Unknown => Background::Shapes,
    }
}

/// Convenience: goes straight from a [`Signal`] to a [`Background`], the way callers use it.
pub fn choose(signal: &Signal) -> Background {
    choose_background(classify(signal))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(effective_type: Option<&str>, downlink_mbps: Option<f32>, save_data: bool) -> Signal {
        Signal {
            effective_type: effective_type.map(str::to_owned),
            downlink_mbps,
            save_data,
        }
    }

    #[test]
    fn no_signal_at_all_defaults_to_good() {
        let s = signal(None, None, false);
        assert_eq!(classify(&s), Quality::Good);
        assert_eq!(choose(&s), Background::Image);
    }

    #[test]
    fn four_g_is_good_and_loads_an_image() {
        let s = signal(Some("4g"), None, false);
        assert_eq!(classify(&s), Quality::Good);
        assert_eq!(choose(&s), Background::Image);
    }

    #[test]
    fn three_g_and_below_are_poor() {
        for bucket in ["3g", "2g", "slow-2g"] {
            let s = signal(Some(bucket), None, false);
            assert_eq!(classify(&s), Quality::Poor, "bucket {bucket}");
            assert_eq!(choose(&s), Background::Shapes, "bucket {bucket}");
        }
    }

    #[test]
    fn an_unrecognised_effective_type_falls_through_to_downlink() {
        let fast = signal(Some("bluetooth"), Some(50.0), false);
        assert_eq!(classify(&fast), Quality::Good);

        let slow = signal(Some("bluetooth"), Some(0.5), false);
        assert_eq!(classify(&slow), Quality::Poor);

        // No downlink either → defaults to Good (LAN assumption).
        let neither = signal(Some("bluetooth"), None, false);
        assert_eq!(classify(&neither), Quality::Good);
    }

    #[test]
    fn downlink_is_only_consulted_without_an_effective_type() {
        let fast = signal(None, Some(10.0), false);
        assert_eq!(classify(&fast), Quality::Good);

        let slow = signal(None, Some(0.5), false);
        assert_eq!(classify(&slow), Quality::Poor);

        // effective_type, when present, wins even if it disagrees with downlink.
        let mixed = signal(Some("3g"), Some(10.0), false);
        assert_eq!(classify(&mixed), Quality::Poor);
    }

    #[test]
    fn downlink_threshold_is_inclusive() {
        assert_eq!(classify(&signal(None, Some(1.5), false)), Quality::Good);
        assert_eq!(classify(&signal(None, Some(1.499), false)), Quality::Poor);
    }

    #[test]
    fn save_data_always_wins() {
        let s = signal(Some("4g"), Some(100.0), true);
        assert_eq!(classify(&s), Quality::Poor);
        assert_eq!(choose(&s), Background::Shapes);
    }
}
