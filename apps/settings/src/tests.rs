//! Driven through the SDK's native harness, the same way `clock` and `file-browser` are: a
//! click reaches the app one frame after it happens, so `press` runs the frame that observes
//! the click and the one that redraws with its effect.

use ccosel_sdk::testing::Harness;

use super::*;

/// The bit `FrameCtx::dark_mode` reads. Kept as a local literal, matching
/// `ccosel_abi::frame::input_flags::DARK_MODE`, so this app's tests don't need `ccosel-abi` as
/// a dependency just to flip one flag.
const DARK_MODE_FLAG: u32 = 1;

fn settings() -> Harness<Settings> {
    Harness::new(Settings::default())
}

fn press(h: &mut Harness<Settings>, button: &str) {
    h.click(button);
    h.frame();
    h.frame();
}

#[test]
fn opens_on_appearance_with_every_section_button_present() {
    let mut h = settings();
    h.frame();
    assert!(h.has_label("Settings"));
    for section in Section::ALL {
        assert!(
            h.buttons().iter().any(|b| b.ends_with(section.label())),
            "missing a button for {}",
            section.label()
        );
    }
    // Selected section is marked, not just present.
    assert!(h.has_button("▸ Appearance"));
}

#[test]
fn switches_section_on_click() {
    let mut h = settings();
    h.frame();
    press(&mut h, "Performance");
    assert!(h.has_button("▸ Performance"));
    assert!(h.has_label("Trade-off: caps the frame rate."));
}

#[test]
fn appearance_reflects_the_real_dark_mode_flag_until_overridden() {
    let mut h = settings();
    h.ctx.flags = DARK_MODE_FLAG;
    h.frame();
    assert!(h.has_button("🌙 Dark"), "starts from the shell's real flag");

    press(&mut h, "🌙 Dark");
    assert!(h.has_button("☀ Light"), "clicking flips the local override");
    assert_eq!(h.app.dark_override, Some(false));
}

#[test]
fn appearance_starts_light_when_the_shell_says_light() {
    let mut h = settings();
    h.ctx.flags = 0;
    h.frame();
    assert!(h.has_button("☀ Light"));
}

#[test]
fn background_url_is_kept_locally() {
    let mut h = settings();
    h.frame();
    h.app.background_url.set("http://example.com/bg.png");
    h.frame();
    assert_eq!(h.app.background_url.as_str(), "http://example.com/bg.png");
}

#[test]
fn performance_toggles_flip_independently() {
    let mut h = settings();
    h.frame();
    press(&mut h, "Performance");
    assert!(h.app.prevent_tearing, "on by default");
    assert!(!h.app.save_battery);

    press(&mut h, "✓ Prevent screen tearing");
    assert!(!h.app.prevent_tearing);
    assert!(h.has_button("Prevent screen tearing"));

    press(&mut h, "Save battery");
    assert!(h.app.save_battery);
    assert!(h.has_button("✓ Save battery"));
    // The other toggle is untouched by flipping this one.
    assert!(!h.app.prevent_tearing);
}

#[test]
fn dock_lists_todays_catalog_by_name_and_toggles_independently() {
    let mut h = settings();
    h.frame();
    press(&mut h, "Dock");
    assert!(h.has_button("✓ 🗀 Files"));
    assert!(h.has_button("✓ ◴ Clock"));

    press(&mut h, "✓ 🗀 Files");
    assert!(!h.app.dock[0].shown);
    assert!(h.app.dock[1].shown, "Clock is untouched");
    assert!(h.has_button("🗀 Files"));
    assert!(h.has_button("✓ ◴ Clock"));
}

#[test]
fn account_starts_with_a_single_guest_account_selected() {
    let mut h = settings();
    h.frame();
    press(&mut h, "Account");
    assert!(h.has_button("▸ Guest"));
    assert_eq!(h.app.accounts, ["Guest"]);
    assert_eq!(h.app.active_account, 0);
}

#[test]
fn adding_an_account_makes_it_active() {
    let mut h = settings();
    h.frame();
    press(&mut h, "Account");

    h.app.new_account.set("Ada");
    h.frame();
    press(&mut h, "Add");

    assert_eq!(h.app.accounts, ["Guest", "Ada"]);
    assert_eq!(h.app.active_account, 1);
    assert!(h.has_button("▸ Ada"));
    assert!(h.has_button("Guest"), "no longer marked active");
    assert_eq!(
        h.app.new_account.as_str(),
        "",
        "the input is cleared after adding"
    );
}

#[test]
fn adding_a_blank_account_name_is_a_no_op() {
    let mut h = settings();
    h.frame();
    press(&mut h, "Account");

    h.app.new_account.set("   ");
    h.frame();
    press(&mut h, "Add");

    assert_eq!(h.app.accounts, ["Guest"], "a blank name is not added");
}

#[test]
fn switching_back_to_an_earlier_account_moves_the_marker() {
    let mut h = settings();
    h.frame();
    press(&mut h, "Account");
    h.app.new_account.set("Ada");
    h.frame();
    press(&mut h, "Add");

    press(&mut h, "Guest");
    assert_eq!(h.app.active_account, 0);
    assert!(h.has_button("▸ Guest"));
    assert!(h.has_button("Ada"));
}
