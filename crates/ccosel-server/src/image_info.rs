//! A picture's size and what its camera recorded (EXIF), for the Viewer's Details.
//!
//! Read here rather than in the app because the app never holds the picture's bytes: the
//! browser decodes it in the shell. Both readers look only at the file's header and metadata
//! blocks, never the pixels, so a 50 MB photo costs a few kilobytes of reading.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use ccosel_proto::fs::ImageInfoReply;
use exif::{Exif, In, Tag, Value};

/// What `path` says about itself. Whatever can't be read is left out rather than failing the
/// whole answer: a screenshot has a size but no camera, and a damaged file may have neither.
pub fn read(path: &Path) -> ImageInfoReply {
    let size = imagesize::size(path).ok();
    let exif = File::open(path).ok().and_then(|f| {
        exif::Reader::new()
            .read_from_container(&mut BufReader::new(f))
            .ok()
    });
    ImageInfoReply {
        width: size.as_ref().and_then(|s| u32::try_from(s.width).ok()),
        height: size.as_ref().and_then(|s| u32::try_from(s.height).ok()),
        fields: exif.as_ref().map(fields).unwrap_or_default(),
    }
}

/// The details worth showing a person, labelled, in the order a photo app lists them.
pub fn fields(exif: &Exif) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut push = |label: &str, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            out.push((label.to_owned(), v));
        }
    };
    push("Camera", camera(exif));
    push("Lens", text(exif, Tag::LensModel));
    push(
        "Taken",
        text(exif, Tag::DateTimeOriginal).or_else(|| text(exif, Tag::DateTime)),
    );
    push("Exposure", text(exif, Tag::ExposureTime));
    push("Aperture", text(exif, Tag::FNumber));
    push("ISO", text(exif, Tag::PhotographicSensitivity));
    push("Focal length", text(exif, Tag::FocalLength));
    push("Flash", text(exif, Tag::Flash));
    push("Orientation", text(exif, Tag::Orientation));
    push("Location", location(exif));
    push("Altitude", text(exif, Tag::GPSAltitude));
    push("Description", text(exif, Tag::ImageDescription));
    push("Artist", text(exif, Tag::Artist));
    push("Copyright", text(exif, Tag::Copyright));
    push("Software", text(exif, Tag::Software));
    out
}

/// A field as people read it: text without the quotes EXIF's own formatting puts round it,
/// anything else with its unit ("1/120 s", "f/1.8", "26 mm").
fn text(exif: &Exif, tag: Tag) -> Option<String> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    let s = match &field.value {
        Value::Ascii(parts) => parts
            .iter()
            .map(|p| String::from_utf8_lossy(p).trim().to_owned())
            .collect::<Vec<_>>()
            .join(" "),
        _ => field.display_value().with_unit(exif).to_string(),
    };
    Some(s.trim().to_owned())
}

/// "Apple iPhone 15 Pro": the maker and model, without saying the maker twice when the model
/// already starts with it ("Canon" + "Canon EOS R5").
fn camera(exif: &Exif) -> Option<String> {
    let make = text(exif, Tag::Make).unwrap_or_default();
    let model = text(exif, Tag::Model).unwrap_or_default();
    let joined = if make.is_empty() || model.to_lowercase().starts_with(&make.to_lowercase()) {
        model
    } else if model.is_empty() {
        make
    } else {
        format!("{make} {model}")
    };
    (!joined.is_empty()).then_some(joined)
}

/// Where it was taken, as decimal latitude and longitude ("51.50072, -0.12462"), from EXIF's
/// degrees, minutes and seconds with their N/S and E/W.
fn location(exif: &Exif) -> Option<String> {
    let lat = coordinate(exif, Tag::GPSLatitude, Tag::GPSLatitudeRef, b'S')?;
    let lon = coordinate(exif, Tag::GPSLongitude, Tag::GPSLongitudeRef, b'W')?;
    Some(format!("{lat:.5}, {lon:.5}"))
}

fn coordinate(exif: &Exif, tag: Tag, reference: Tag, negative: u8) -> Option<f64> {
    let Value::Rational(dms) = &exif.get_field(tag, In::PRIMARY)?.value else {
        return None;
    };
    let part = |i: usize| dms.get(i).map_or(0.0, |r| r.to_f64());
    let degrees = part(0) + part(1) / 60.0 + part(2) / 3600.0;
    if !degrees.is_finite() {
        return None;
    }
    let south_or_west = match exif.get_field(reference, In::PRIMARY).map(|f| &f.value) {
        Some(Value::Ascii(r)) => r.first().and_then(|r| r.first()) == Some(&negative),
        _ => false,
    };
    Some(if south_or_west { -degrees } else { degrees })
}
