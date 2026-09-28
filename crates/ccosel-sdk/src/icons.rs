//! Icons an app can put in any text: a button, a label, a styled run.
//!
//! The shell draws text with the [Phosphor](https://phosphoricons.com) icon font as a fallback,
//! so these private-use codepoints render as icons, the same set the shell uses for its own
//! chrome. Prefer them to emoji, which the shell's fonts mostly lack and draw as empty boxes.
//! Only the ones apps have needed so far are here; add more from `egui_phosphor::regular` as
//! they are wanted. A test checks every one against that crate, so a typo cannot slip in.
//!
//! They are `&str`s, so they cost nothing in a module that does not use them.

pub const FOLDER: &str = "\u{E24A}";
pub const FOLDER_OPEN: &str = "\u{E256}";
pub const FILE_TEXT: &str = "\u{E23A}";
pub const BOOK_OPEN: &str = "\u{E0E6}";
pub const USERS: &str = "\u{E4D6}";
pub const HOUSE: &str = "\u{E2C2}";
pub const MAGNIFYING_GLASS: &str = "\u{E30C}";
pub const ARROW_LEFT: &str = "\u{E058}";
pub const PENCIL_SIMPLE: &str = "\u{E3B4}";
pub const FLOPPY_DISK: &str = "\u{E248}";
pub const CHECK: &str = "\u{E182}";
pub const TRASH: &str = "\u{E4A6}";
pub const EYE: &str = "\u{E220}";
pub const EYE_SLASH: &str = "\u{E224}";
pub const LOCK_SIMPLE: &str = "\u{E308}";
pub const DOWNLOAD_SIMPLE: &str = "\u{E20C}";
pub const FILE_PLUS: &str = "\u{E236}";
pub const FOLDER_PLUS: &str = "\u{E258}";
pub const LINK: &str = "\u{E2E2}";
pub const PAPERCLIP: &str = "\u{E39A}";
pub const IMAGE: &str = "\u{E2CA}";
pub const ARROW_CLOCKWISE: &str = "\u{E036}";
pub const TEXT_H: &str = "\u{E6BA}";
pub const TEXT_B: &str = "\u{E5BE}";
pub const TEXT_ITALIC: &str = "\u{E5C0}";
pub const LIST_BULLETS: &str = "\u{E2F2}";
pub const LIST_NUMBERS: &str = "\u{E2F6}";
pub const CHECK_SQUARE: &str = "\u{E186}";
pub const SQUARE: &str = "\u{E45E}";
pub const CODE_BLOCK: &str = "\u{EAFE}";
pub const QUOTES: &str = "\u{E660}";

#[cfg(test)]
mod tests {
    #[test]
    fn every_icon_is_the_phosphor_glyph_it_is_named_for() {
        use egui_phosphor::regular as p;
        assert_eq!(super::FOLDER, p::FOLDER);
        assert_eq!(super::FOLDER_OPEN, p::FOLDER_OPEN);
        assert_eq!(super::FILE_TEXT, p::FILE_TEXT);
        assert_eq!(super::BOOK_OPEN, p::BOOK_OPEN);
        assert_eq!(super::USERS, p::USERS);
        assert_eq!(super::HOUSE, p::HOUSE);
        assert_eq!(super::MAGNIFYING_GLASS, p::MAGNIFYING_GLASS);
        assert_eq!(super::ARROW_LEFT, p::ARROW_LEFT);
        assert_eq!(super::PENCIL_SIMPLE, p::PENCIL_SIMPLE);
        assert_eq!(super::FLOPPY_DISK, p::FLOPPY_DISK);
        assert_eq!(super::CHECK, p::CHECK);
        assert_eq!(super::TRASH, p::TRASH);
        assert_eq!(super::EYE, p::EYE);
        assert_eq!(super::EYE_SLASH, p::EYE_SLASH);
        assert_eq!(super::LOCK_SIMPLE, p::LOCK_SIMPLE);
        assert_eq!(super::DOWNLOAD_SIMPLE, p::DOWNLOAD_SIMPLE);
        assert_eq!(super::FILE_PLUS, p::FILE_PLUS);
        assert_eq!(super::FOLDER_PLUS, p::FOLDER_PLUS);
        assert_eq!(super::LINK, p::LINK);
        assert_eq!(super::PAPERCLIP, p::PAPERCLIP);
        assert_eq!(super::IMAGE, p::IMAGE);
        assert_eq!(super::ARROW_CLOCKWISE, p::ARROW_CLOCKWISE);
        assert_eq!(super::TEXT_H, p::TEXT_H);
        assert_eq!(super::TEXT_B, p::TEXT_B);
        assert_eq!(super::TEXT_ITALIC, p::TEXT_ITALIC);
        assert_eq!(super::LIST_BULLETS, p::LIST_BULLETS);
        assert_eq!(super::LIST_NUMBERS, p::LIST_NUMBERS);
        assert_eq!(super::CHECK_SQUARE, p::CHECK_SQUARE);
        assert_eq!(super::SQUARE, p::SQUARE);
        assert_eq!(super::CODE_BLOCK, p::CODE_BLOCK);
        assert_eq!(super::QUOTES, p::QUOTES);
    }
}
