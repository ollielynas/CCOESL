//! Run under node by `cargo xtask test-wasm`: this crate only compiles for wasm32.

use wasm_bindgen_test::wasm_bindgen_test;

use super::*;

fn with(background: BackgroundChoice) -> Settings {
    Settings {
        background,
        ..Settings::default()
    }
}

#[wasm_bindgen_test]
fn automatic_follows_the_connection() {
    let s = with(BackgroundChoice::Auto);
    assert_eq!(background(&s, Background::Image), Background::Image);
    assert_eq!(background(&s, Background::Shapes), Background::Shapes);
}

#[wasm_bindgen_test]
fn photo_and_plain_ignore_the_connection() {
    for detected in [Background::Image, Background::Shapes] {
        assert_eq!(
            background(&with(BackgroundChoice::Photo), detected),
            Background::Image
        );
        assert_eq!(
            background(&with(BackgroundChoice::Plain), detected),
            Background::Shapes
        );
    }
}

#[wasm_bindgen_test]
fn save_data_never_draws_an_image() {
    for choice in [
        BackgroundChoice::Auto,
        BackgroundChoice::Photo,
        BackgroundChoice::Custom,
    ] {
        let s = Settings {
            background: choice,
            custom_image: "/home/alice/beach.jpg".to_owned(),
            save_data: true,
            ..Settings::default()
        };
        assert_eq!(background(&s, Background::Image), Background::Shapes);
    }
}

#[wasm_bindgen_test]
fn a_custom_image_comes_from_the_users_files() {
    let s = Settings {
        background: BackgroundChoice::Custom,
        custom_image: " /home/alice/beach.jpg ".to_owned(),
        ..Settings::default()
    };
    assert_eq!(background(&s, Background::Shapes), Background::Image);
    assert_eq!(wallpaper_url(&s), "/files/home/alice/beach.jpg");
}

#[wasm_bindgen_test]
fn a_custom_background_with_no_image_is_plain() {
    let s = with(BackgroundChoice::Custom);
    assert_eq!(background(&s, Background::Image), Background::Shapes);
    assert_eq!(wallpaper_url(&s), WALLPAPER_URL);
}

#[wasm_bindgen_test]
fn flags_tell_apps_what_the_user_asked_for() {
    let none = Settings::default();
    assert_eq!(frame_flags(false, &none), 0);
    assert_eq!(frame_flags(true, &none), input_flags::DARK_MODE);

    let both = Settings {
        reduce_motion: true,
        save_data: true,
        ..Settings::default()
    };
    assert_eq!(
        frame_flags(false, &both),
        input_flags::REDUCED_MOTION | input_flags::LOW_BANDWIDTH
    );
}

#[wasm_bindgen_test]
fn reduce_motion_turns_animation_off_and_back_on() {
    let ctx = egui::Context::default();
    let on = Settings {
        reduce_motion: true,
        ..Settings::default()
    };
    // Both themes: each keeps its own style, and switching theme must not bring motion back.
    let time = |theme| ctx.style_of(theme).animation_time;
    apply(&ctx, &on);
    assert_eq!(time(egui::Theme::Dark), 0.0);
    assert_eq!(time(egui::Theme::Light), 0.0);
    apply(&ctx, &Settings::default());
    assert!(time(egui::Theme::Dark) > 0.0);
    assert!(time(egui::Theme::Light) > 0.0);
}

#[wasm_bindgen_test]
fn the_theme_choice_reaches_egui() {
    let ctx = egui::Context::default();
    for (choice, theme) in [
        (ThemeChoice::Light, egui::Theme::Light),
        (ThemeChoice::Dark, egui::Theme::Dark),
    ] {
        apply(
            &ctx,
            &Settings {
                theme: choice,
                ..Settings::default()
            },
        );
        assert_eq!(ctx.theme(), theme);
    }
}

#[wasm_bindgen_test]
fn hex_round_trips_and_rejects_junk() {
    let bytes = [0u8, 1, 0x7f, 0xff];
    assert_eq!(decode_hex(&encode_hex(&bytes)).as_deref(), Some(&bytes[..]));
    assert_eq!(decode_hex("abc"), None);
    assert_eq!(decode_hex("zz"), None);
}
