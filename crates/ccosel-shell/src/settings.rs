//! The user's settings, as the shell applies them.
//!
//! The Settings app edits them; the shell is what makes them true. It fetches them once at
//! boot, and after that it never waits on the server: when the app sends `SetSettings`, the
//! shell applies the new value in the frame the call goes out, and the server only stores it.
//!
//! With login off there is nobody to store them for, so the server refuses (`NO_USER`) and
//! the shell keeps them in this browser's `localStorage` instead.

use ccosel_abi::frame::input_flags;
use ccosel_connection::Background;
use ccosel_proto::settings::{BackgroundChoice, Settings, ThemeChoice};

use crate::desktop::WALLPAPER_URL;

const STORAGE_KEY: &str = "ccosel.settings";

/// Which background to draw. `detected` is what the connection alone would choose.
///
/// Save data wins over everything: someone who asked for less traffic gets no image, even
/// one they picked.
pub fn background(s: &Settings, detected: Background) -> Background {
    if s.save_data {
        return Background::Shapes;
    }
    match s.background {
        BackgroundChoice::Auto => detected,
        BackgroundChoice::Photo => Background::Image,
        BackgroundChoice::Plain => Background::Shapes,
        BackgroundChoice::Custom if s.custom_image.trim().is_empty() => Background::Shapes,
        BackgroundChoice::Custom => Background::Image,
    }
}

/// Where the background image comes from, when [`background`] says to draw one.
pub fn wallpaper_url(s: &Settings) -> String {
    match s.background {
        BackgroundChoice::Custom if !s.custom_image.trim().is_empty() => {
            let path = s.custom_image.trim();
            let path = path.strip_prefix('/').unwrap_or(path);
            format!("/files/{path}")
        }
        _ => WALLPAPER_URL.to_owned(),
    }
}

/// The environment flags every app is handed each frame.
pub fn frame_flags(dark: bool, s: &Settings) -> u32 {
    let mut flags = 0;
    if dark {
        flags |= input_flags::DARK_MODE;
    }
    if s.reduce_motion {
        flags |= input_flags::REDUCED_MOTION;
    }
    if s.save_data {
        flags |= input_flags::LOW_BANDWIDTH;
    }
    flags
}

pub fn theme_preference(choice: ThemeChoice) -> egui::ThemePreference {
    match choice {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    }
}

/// How long egui animations take. Zero turns them off: hover lifts, fades and window
/// open/close all snap straight to their end state.
pub fn animation_time(s: &Settings) -> f32 {
    if s.reduce_motion {
        0.0
    } else {
        egui::Style::default().animation_time
    }
}

/// Apply what egui itself owns: the theme, and animation.
pub fn apply(ctx: &egui::Context, s: &Settings) {
    ctx.set_theme(theme_preference(s.theme));
    let time = animation_time(s);
    ctx.all_styles_mut(|style| style.animation_time = time);
}

/// Settings kept in this browser, for when login is off.
pub fn load_local() -> Option<Settings> {
    let text = storage()?.get_item(STORAGE_KEY).ok()??;
    decode_hex(&text).and_then(|bytes| postcard::from_bytes(&bytes).ok())
}

pub fn save_local(s: &Settings) {
    let (Some(storage), Ok(bytes)) = (storage(), postcard::to_allocvec(s)) else {
        return;
    };
    // Best effort: storage can be full or disabled, and the settings still apply this session.
    let _ = storage.set_item(STORAGE_KEY, &encode_hex(&bytes));
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests;
