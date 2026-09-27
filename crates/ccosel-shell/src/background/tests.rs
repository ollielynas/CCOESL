//! Run under node by `cargo xtask test-wasm`: this crate only compiles for wasm32.

use wasm_bindgen_test::wasm_bindgen_test;

use super::*;

/// The file the desktop actually fetches. `include_bytes!` also makes a renamed or deleted
/// wallpaper a compile error rather than a silent 404 at runtime.
const SHIPPED: &[u8] = include_bytes!("../../../../web/wallpaper.jpg");

fn approx(a: egui::Rect, b: egui::Rect) -> bool {
    (a.min - b.min).length() < 1e-4 && (a.max - b.max).length() < 1e-4
}

#[wasm_bindgen_test]
fn the_url_the_desktop_fetches_is_the_file_that_ships() {
    assert_eq!(crate::desktop::WALLPAPER_URL, "/wallpaper.jpg");
}

/// The regression behind "fetched but never rendered": the shipped wallpaper was a JPEG while
/// only the PNG decoder was compiled in, so every decode failed.
#[wasm_bindgen_test]
fn the_shipped_wallpaper_decodes() {
    let image = decode_wallpaper(SHIPPED, 4096).expect("the shipped wallpaper must decode");
    assert_eq!(image.size, [2560, 1910]);
}

#[wasm_bindgen_test]
fn a_png_decodes_too() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let image = decode_wallpaper(&png, 4096).unwrap();
    assert_eq!(image.size, [4, 2]);
    assert_eq!(image.pixels[0], egui::Color32::from_rgb(10, 20, 30));
}

#[wasm_bindgen_test]
fn an_image_over_the_texture_limit_is_shrunk_keeping_its_shape() {
    let image = decode_wallpaper(SHIPPED, 1024).unwrap();
    assert_eq!(image.size[0], 1024);
    // 2560x1910 scaled to 1024 wide is 764 high; allow a pixel of rounding.
    assert!((763..=765).contains(&image.size[1]), "{:?}", image.size);
}

#[wasm_bindgen_test]
fn garbage_is_an_error_not_a_panic() {
    assert!(decode_wallpaper(b"not an image", 4096).is_err());
    // A cut-off download: the JPEG decoder is lenient and may fill in the rest. Either outcome
    // is fine; panicking is not.
    let _ = decode_wallpaper(&SHIPPED[..1000], 4096);
}

#[wasm_bindgen_test]
fn a_wide_image_on_a_narrower_screen_crops_the_sides() {
    // 2:1 image on a 1:1 screen: the middle half of the width shows, all of the height.
    let uv = cover_uv([2000, 1000], egui::vec2(500.0, 500.0));
    assert!(approx(
        uv,
        egui::Rect::from_min_max(egui::pos2(0.25, 0.0), egui::pos2(0.75, 1.0))
    ));
}

#[wasm_bindgen_test]
fn a_tall_image_on_a_wider_screen_crops_top_and_bottom() {
    // 1:1 image on a 2:1 screen: the middle half of the height shows.
    let uv = cover_uv([1000, 1000], egui::vec2(1000.0, 500.0));
    assert!(approx(
        uv,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.25), egui::pos2(1.0, 0.75))
    ));
}

#[wasm_bindgen_test]
fn cropping_never_stretches() {
    // Whatever the shapes, the visible part of the image has the screen's aspect ratio.
    for (image, screen) in [
        ([2560, 1910], egui::vec2(1280.0, 800.0)),
        ([2560, 1910], egui::vec2(390.0, 844.0)),
        ([640, 480], egui::vec2(3840.0, 1600.0)),
    ] {
        let uv = cover_uv(image, screen);
        let visible_aspect = (uv.width() * image[0] as f32) / (uv.height() * image[1] as f32);
        assert!(
            (visible_aspect - screen.x / screen.y).abs() < 1e-3,
            "{image:?} on {screen:?}"
        );
        assert!(uv.min.x >= 0.0 && uv.min.y >= 0.0 && uv.max.x <= 1.0 && uv.max.y <= 1.0);
    }
}

#[wasm_bindgen_test]
fn same_shape_uses_the_whole_image_and_degenerate_sizes_do_not_panic() {
    let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    assert!(approx(cover_uv([800, 600], egui::vec2(400.0, 300.0)), full));
    assert!(approx(cover_uv([0, 0], egui::vec2(400.0, 300.0)), full));
    assert!(approx(cover_uv([800, 600], egui::vec2(0.0, 0.0)), full));
}

/// The rest of the "never rendered" path: a decoded image, once a texture, is actually drawn,
/// filling the screen with the cropped part of the texture.
#[wasm_bindgen_test]
fn a_loaded_wallpaper_is_painted_over_the_whole_screen() {
    let ctx = egui::Context::default();
    let texture = ctx.load_texture(
        "wallpaper",
        decode_wallpaper(SHIPPED, 1024).unwrap(),
        egui::TextureOptions::default(),
    );
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    let input = egui::RawInput {
        screen_rect: Some(screen),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        paint_image(
            &ui.ctx().layer_painter(egui::LayerId::background()),
            screen,
            &texture,
        );
    });
    out.textures_delta.clear();

    let image = out
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Mesh(m) if m.texture_id == texture.id() => Some(m.clone()),
            _ => None,
        })
        .expect("the wallpaper texture was never drawn");
    let bounds = image.calc_bounds();
    assert_eq!(
        bounds, screen,
        "fills the screen exactly, no overflow, no gap"
    );
}

#[wasm_bindgen_test]
fn shapes_draw_something_in_both_themes() {
    for dark in [true, false] {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            draw_shapes(
                &ui.ctx().layer_painter(egui::LayerId::background()),
                ui.ctx().viewport_rect(),
                dark,
            );
        });
        out.textures_delta.clear();
        assert!(!out.shapes.is_empty());
    }
}
