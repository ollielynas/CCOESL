//! What kind of picture a file is, from its first bytes, and whether every browser shows it as
//! it is. Pure, so every case is tested without a converter.
//!
//! Only pictures are recognised here. Audio and video are left to ffprobe, which knows far more
//! containers than are worth matching by hand.

/// How a picture is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    /// Every browser shows it as it is. `animated` if it has more than one frame, so the
    /// browser plays it rather than the shell drawing its first frame.
    AsIs {
        content_type: &'static str,
        animated: bool,
    },
    /// Converted. `magick` is the ImageMagick coder to read it with, never left to ImageMagick
    /// to guess (that guess is how its worst security holes were reached), and `magick_first`
    /// says it reads this format better than ffmpeg does.
    Convert {
        magick: Option<&'static str>,
        magick_first: bool,
    },
    /// Drawn by resvg, in the server.
    Svg,
}

const fn convert(magick: &'static str, magick_first: bool) -> Picture {
    Picture::Convert {
        magick: Some(magick),
        magick_first,
    }
}

/// What `head` (the start of a file named with extension `ext`, lowercase, `""` for none) is,
/// or `None` if it isn't a picture this recognises. Magic bytes win over the name; the name
/// only decides formats that have no magic bytes (TGA, WBMP, PCX).
pub fn picture(head: &[u8], ext: &str) -> Option<Picture> {
    let starts = |magic: &[u8]| head.starts_with(magic);
    if starts(b"\xFF\xD8\xFF") {
        return Some(if jpeg_is_arithmetic(head) {
            // Arithmetic coding (SOF9 and up) is in the standard, but few browsers decode it.
            convert("jpeg", true)
        } else {
            Picture::AsIs {
                content_type: "image/jpeg",
                animated: false,
            }
        });
    }
    if starts(b"\x89PNG\r\n\x1a\n") {
        return Some(Picture::AsIs {
            content_type: "image/png",
            animated: png_is_animated(head),
        });
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Some(Picture::AsIs {
            content_type: "image/gif",
            animated: gif_frames(head) > 1,
        });
    }
    if starts(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        return Some(Picture::AsIs {
            content_type: "image/webp",
            animated: webp_is_animated(head),
        });
    }
    if let Some(brands) = ftyp_brands(head) {
        let has = |b: &[u8]| brands.iter().any(|x| x == b);
        if has(b"avis") {
            return Some(Picture::AsIs {
                content_type: "image/avif",
                animated: true,
            });
        }
        if has(b"avif") {
            return Some(Picture::AsIs {
                content_type: "image/avif",
                animated: false,
            });
        }
        if [
            "heic", "heix", "heim", "heis", "hevc", "hevx", "mif1", "msf1",
        ]
        .iter()
        .any(|b| has(b.as_bytes()))
        {
            return Some(convert("heic", false));
        }
        // Any other `ftyp` is an MP4, MOV, 3GP or the like: ffprobe's to decide.
        return None;
    }
    if starts(b"BM") && head.len() >= 26 {
        return Some(Picture::AsIs {
            content_type: "image/bmp",
            animated: false,
        });
    }
    if starts(b"\0\0\x01\0") {
        return Some(Picture::AsIs {
            content_type: "image/x-icon",
            animated: false,
        });
    }
    if starts(b"\0\0\x02\0") && ext == "cur" {
        return Some(convert("cur", false));
    }
    if starts(b"II*\0") || starts(b"MM\0*") {
        // A camera's raw file is a TIFF too; LibRaw, through ImageMagick, develops it.
        let coder = match ext {
            "dng" | "nef" | "cr2" | "arw" | "orf" | "rw2" | "pef" | "srw" | "raf" => "dng",
            _ => "tiff",
        };
        return Some(convert(coder, true));
    }
    if starts(b"8BPS") {
        return Some(convert("psd", true));
    }
    if starts(b"\xFF\x0A") || starts(b"\0\0\0\x0CJXL \r\n\x87\n") {
        // ImageMagick is often built without JPEG XL; ffmpeg usually has it.
        return Some(convert("jxl", false));
    }
    if starts(b"\0\0\0\x0CjP  \r\n\x87\n") {
        return Some(convert("jp2", false));
    }
    if starts(b"\xFF\x4F\xFF\x51") {
        return Some(convert("j2k", false));
    }
    if starts(b"\x76\x2F\x31\x01") {
        return Some(convert("exr", false));
    }
    if starts(b"#?RADIANCE") || starts(b"#?RGBE") {
        return Some(convert("hdr", false));
    }
    if starts(b"qoif") {
        return Some(convert("qoi", false));
    }
    if starts(b"DDS ") {
        return Some(convert("dds", false));
    }
    if starts(b"\x01\xDA") {
        return Some(convert("sgi", false));
    }
    if starts(b"/* XPM */") {
        return Some(convert("xpm", false));
    }
    if head.len() >= 3 && head[0] == b'P' && head[2].is_ascii_whitespace() {
        match head[1] {
            b'F' | b'f' => return Some(convert("pfm", false)),
            b'1'..=b'6' => return Some(convert("pnm", false)),
            b'7' => return Some(convert("pam", false)),
            _ => {}
        }
    }
    if is_svg(head) {
        return Some(Picture::Svg);
    }
    // Formats with no magic bytes worth the name, so the extension is all there is.
    match ext {
        "tga" | "icb" | "vda" | "vst" => Some(convert("tga", false)),
        "pcx" if head.first() == Some(&0x0A) => Some(convert("pcx", false)),
        "wbmp" if head.first() == Some(&0) => Some(convert("wbmp", false)),
        _ => None,
    }
}

/// Whether `head` looks like an SVG document: XML (or a bare tag) with an `<svg` element near the
/// start.
fn is_svg(head: &[u8]) -> bool {
    let start = &head[..head.len().min(4096)];
    let text = String::from_utf8_lossy(start);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    trimmed.starts_with('<') && text.contains("<svg")
}

/// The brands of an ISO media file's `ftyp` box (major brand first), or `None` if `head` doesn't
/// start with one.
fn ftyp_brands(head: &[u8]) -> Option<Vec<[u8; 4]>> {
    if head.get(4..8) != Some(b"ftyp") {
        return None;
    }
    let size = u32::from_be_bytes(head.get(..4)?.try_into().ok()?) as usize;
    let end = size.min(head.len());
    let mut brands = vec![head.get(8..12)?.try_into().ok()?];
    let mut at = 16;
    while at + 4 <= end {
        brands.push(head[at..at + 4].try_into().ok()?);
        at += 4;
    }
    Some(brands)
}

/// Whether a JPEG's frame is arithmetic-coded (SOF9 to SOF15, but not DAC at 0xCC).
fn jpeg_is_arithmetic(head: &[u8]) -> bool {
    let mut at = 2;
    while at + 4 <= head.len() {
        if head[at] != 0xFF {
            return false;
        }
        let marker = head[at + 1];
        match marker {
            // Padding before a marker.
            0xFF => {
                at += 1;
                continue;
            }
            0xC0..=0xC3 | 0xC5..=0xC7 => return false,
            0xC9..=0xCB | 0xCD..=0xCF => return true,
            // Start of scan: past the header, and no frame seen.
            0xDA => return false,
            _ => {}
        }
        let len = usize::from(u16::from_be_bytes([head[at + 2], head[at + 3]]));
        at += 2 + len;
    }
    false
}

/// Whether a PNG is an APNG: an `acTL` chunk before the first `IDAT`.
fn png_is_animated(head: &[u8]) -> bool {
    let mut at = 8;
    while at + 8 <= head.len() {
        let len = u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
        match &head[at + 4..at + 8] {
            b"acTL" => return true,
            b"IDAT" => return false,
            _ => {}
        }
        at = at.saturating_add(12).saturating_add(len as usize);
    }
    false
}

/// Whether a WebP is animated: the animation flag in its `VP8X` header.
fn webp_is_animated(head: &[u8]) -> bool {
    head.get(12..16) == Some(b"VP8X") && head.get(20).is_some_and(|flags| flags & 0x02 != 0)
}

/// How many frames a GIF has, counting no further than two, within the bytes there are.
pub fn gif_frames(gif: &[u8]) -> usize {
    let Some(&packed) = gif.get(10) else {
        return 0;
    };
    let mut at = 13;
    if packed & 0x80 != 0 {
        at += 3 << ((packed & 0x07) + 1);
    }
    let mut frames = 0;
    // Skips a run of data sub-blocks starting at `at`, returning where they end.
    let skip_blocks = |mut at: usize| -> Option<usize> {
        loop {
            let len = usize::from(*gif.get(at)?);
            at += 1;
            if len == 0 {
                return Some(at);
            }
            at += len;
        }
    };
    while let Some(&block) = gif.get(at) {
        match block {
            // Image descriptor, optional local colour table, LZW minimum size, image data.
            0x2C => {
                frames += 1;
                if frames > 1 {
                    break;
                }
                let Some(&flags) = gif.get(at + 9) else {
                    break;
                };
                at += 10;
                if flags & 0x80 != 0 {
                    at += 3 << ((flags & 0x07) + 1);
                }
                match skip_blocks(at + 1) {
                    Some(next) => at = next,
                    None => break,
                }
            }
            // Extension: label, then sub-blocks.
            0x21 => match skip_blocks(at + 2) {
                Some(next) => at = next,
                None => break,
            },
            // Trailer, or something that isn't a GIF block.
            _ => break,
        }
    }
    frames
}
