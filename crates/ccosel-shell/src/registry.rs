//! What apps exist and where to get them.
//!
//! Static for now. This becomes `GET /manifest` — the one mutable URL in the app-delivery
//! path, mapping `app_id -> {content hash, size, icon, min_abi_version}` while every module
//! itself is served from an immutable content-addressed URL. Keeping the shape here now means
//! swapping the source later touches one function.

#[derive(Clone)]
pub struct AppEntry {
    pub id: &'static str,
    pub name: &'static str,
    /// A Phosphor glyph, drawn on `color` as a badge — see `theme::paint_badge`.
    pub icon: &'static str,
    /// The badge's background. Each app gets its own, so its windows and dock entries
    /// stay visually identifiable at a glance instead of blurring into one grey list.
    pub color: egui::Color32,
    /// Absolute, so it resolves the same from `/` and from `/app/{id}`.
    pub url: &'static str,
    pub default_size: [f32; 2],
}

/// Where an app opens on its own, filling the page with no desktop around it: `/app/{id}`.
/// The server answers every such path with the boot page, so an app gets one by being in
/// [`catalog`], with nothing else to wire up.
pub const SOLO_PREFIX: &str = "/app/";

/// The app id in a `/app/{id}` path, if `path` is one. A trailing slash is allowed, since
/// people type one; anything deeper than one segment is not an app page.
pub fn solo_id(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(SOLO_PREFIX)?;
    let id = rest.strip_suffix('/').unwrap_or(rest);
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

/// What an app's own page opens it on: the `open` parameter of the page's query string
/// (`?open=%2FDocs%2Fa.png`), decoded. `None` when there isn't one, or it isn't UTF-8.
pub fn solo_arg(search: &str) -> Option<String> {
    let query = search.strip_prefix('?').unwrap_or(search);
    let value = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("open="))?;
    percent_decode(value)
}

/// `s` with `%XX` escapes (and `+`, a space in a query) turned back into what they stand for.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = s.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// The page that opens `app` on its own, on `arg`: what an `OpenApp` button opens on an app's
/// own page, where there is no desktop to open a window on.
pub fn solo_url(app: &str, arg: &str) -> String {
    let mut url = format!("{SOLO_PREFIX}{app}?open=");
    for &b in arg.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

/// The catalog entry for `id`.
pub fn find(id: &str) -> Option<AppEntry> {
    catalog().into_iter().find(|e| e.id == id)
}

pub fn catalog() -> Vec<AppEntry> {
    vec![
        AppEntry {
            id: "file-browser",
            name: "Files",
            icon: egui_phosphor::regular::FOLDER,
            color: egui::Color32::from_rgb(0x3b, 0x82, 0xf6),
            url: "/dist/file-browser.wasm",
            default_size: [420.0, 320.0],
        },
        AppEntry {
            id: "clock",
            name: "Clock",
            icon: egui_phosphor::regular::CLOCK,
            color: egui::Color32::from_rgb(0xf5, 0x9e, 0x0b),
            url: "/dist/clock.wasm",
            default_size: [240.0, 200.0],
        },
        AppEntry {
            id: "server-dashboard",
            name: "Server",
            icon: egui_phosphor::regular::CHART_LINE,
            color: egui::Color32::from_rgb(0x10, 0xb9, 0x81),
            url: "/dist/server-dashboard.wasm",
            default_size: [380.0, 560.0],
        },
        AppEntry {
            id: "rust-compiler",
            name: "Compiler",
            icon: egui_phosphor::regular::HAMMER,
            color: egui::Color32::from_rgb(0xea, 0x58, 0x0c),
            url: "/dist/rust-compiler.wasm",
            default_size: [480.0, 420.0],
        },
        AppEntry {
            id: "account",
            name: "Account",
            icon: egui_phosphor::regular::USER_CIRCLE,
            color: egui::Color32::from_rgb(0x8b, 0x5c, 0xf6),
            url: "/dist/account.wasm",
            default_size: [240.0, 160.0],
        },
        AppEntry {
            id: "viewer",
            name: "Viewer",
            icon: egui_phosphor::regular::EYE,
            color: egui::Color32::from_rgb(0x06, 0xb6, 0xd4),
            url: "/dist/viewer.wasm",
            default_size: [640.0, 520.0],
        },
        AppEntry {
            id: "docs",
            name: "Docs",
            icon: egui_phosphor::regular::BOOK_OPEN,
            color: egui::Color32::from_rgb(0x8b, 0x5c, 0xf6),
            url: "/dist/docs.wasm",
            default_size: [560.0, 560.0],
        },
        AppEntry {
            id: "spreadsheet",
            name: "Spreadsheet",
            icon: egui_phosphor::regular::TABLE,
            color: egui::Color32::from_rgb(0x16, 0xa3, 0x4a),
            url: "/dist/spreadsheet.wasm",
            default_size: [720.0, 600.0],
        },
    ]
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    #[wasm_bindgen_test]
    fn solo_id_reads_the_app_from_its_path() {
        assert_eq!(solo_id("/app/clock"), Some("clock"));
        assert_eq!(solo_id("/app/clock/"), Some("clock"));
        assert_eq!(solo_id("/app/file-browser"), Some("file-browser"));
    }

    #[wasm_bindgen_test]
    fn solo_id_is_none_off_an_app_page() {
        assert_eq!(solo_id("/"), None);
        assert_eq!(solo_id("/index.html"), None);
        assert_eq!(solo_id("/app"), None);
        assert_eq!(solo_id("/app/"), None);
        assert_eq!(solo_id("/app/clock/extra"), None);
        assert_eq!(solo_id("/apps/clock"), None);
    }

    /// The other half of "every app gets a page automatically": each catalog entry can be
    /// reached at its own `/app/{id}`, and loads its module from there.
    #[wasm_bindgen_test]
    fn every_app_has_a_page_that_finds_it() {
        for entry in catalog() {
            let path = format!("{SOLO_PREFIX}{}", entry.id);
            let id = solo_id(&path).unwrap_or_else(|| panic!("{path} is not an app page"));
            assert_eq!(find(id).map(|e| e.id), Some(entry.id));
            assert!(
                entry.url.starts_with('/'),
                "{}: module url {} would resolve under {path}",
                entry.id,
                entry.url
            );
        }
    }

    /// The Docs page listing each app's own page. A new app in the catalog fails this until it
    /// is listed there, so the page can't silently fall behind.
    #[wasm_bindgen_test]
    fn every_app_page_is_listed_in_the_docs() {
        let page = include_str!("../../../data/shared/Docs/app-links.md");
        for entry in catalog() {
            let link = format!("]({SOLO_PREFIX}{})", entry.id);
            assert!(
                page.contains(&link),
                "data/shared/Docs/app-links.md doesn't link to {SOLO_PREFIX}{}",
                entry.id
            );
        }
    }

    #[wasm_bindgen_test]
    fn solo_arg_reads_what_the_page_opens() {
        assert_eq!(
            solo_arg("?open=%2FDocs%2Fa%20b.png").as_deref(),
            Some("/Docs/a b.png")
        );
        assert_eq!(solo_arg("?x=1&open=%2Fa").as_deref(), Some("/a"));
        assert_eq!(solo_arg("open=%C3%BC+x").as_deref(), Some("ü x"));
        assert_eq!(solo_arg(""), None);
        assert_eq!(solo_arg("?other=1"), None);
        assert_eq!(solo_arg("?open=%zz"), None, "a broken escape opens nothing");
        assert_eq!(
            solo_arg("?open=%FF"),
            None,
            "nor does a path that isn't UTF-8"
        );
    }

    #[wasm_bindgen_test]
    fn solo_url_round_trips_through_solo_arg() {
        for arg in ["/Docs/a b.png", "/x?y=1&z#w", "/ü/日本.txt", ""] {
            let url = solo_url("viewer", arg);
            let (path, search) = url.split_once('?').unwrap();
            assert_eq!(solo_id(path), Some("viewer"));
            assert_eq!(solo_arg(search).as_deref(), Some(arg), "{url}");
        }
    }

    #[wasm_bindgen_test]
    fn find_rejects_an_unknown_app() {
        assert!(find("no-such-app").is_none());
    }
}
