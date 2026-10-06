//! A picture's size and what it says about itself, for the Viewer's Details: what its camera
//! recorded (EXIF), and who made it and what it's of (EXIF, Windows' own fields, XMP and IPTC,
//! whichever it has).
//!
//! Read here rather than in the app because the app never holds the picture's bytes: the
//! browser decodes it in the shell. The readers look only at the file's header and metadata
//! blocks, never the pixels, so a 50 MB photo costs a little reading.
//!
//! Metadata is written by every kind of software and often wrong: a damaged entry, a date of
//! all zeros, a latitude of 200°, an f-number divided by zero. What can't be right is left out,
//! and one bad entry never costs the rest.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use ccosel_proto::fs::ImageInfoReply;
use exif::{Exif, In, Tag, Value};

/// The fields Windows writes for itself, which EXIF's tag list doesn't name.
const XP_TITLE: Tag = Tag(exif::Context::Tiff, 0x9C9B);
const XP_COMMENT: Tag = Tag(exif::Context::Tiff, 0x9C9C);
const XP_AUTHOR: Tag = Tag(exif::Context::Tiff, 0x9C9D);
const XP_KEYWORDS: Tag = Tag(exif::Context::Tiff, 0x9C9E);

/// How much of a file is searched for XMP and IPTC, which sit near the start.
const METADATA_BYTES: u64 = 1 << 20;

/// What `path` says about itself. Whatever can't be read is left out rather than failing the
/// whole answer: a screenshot has a size but no camera, and a damaged file may have neither.
pub fn read(path: &Path) -> ImageInfoReply {
    let size = imagesize::size(path).ok();
    let exif = read_exif(path);
    let mut head = Vec::new();
    if let Ok(f) = File::open(path) {
        let _ = f.take(METADATA_BYTES).read_to_end(&mut head);
    }
    let extra = Extra::from_bytes(&head);
    let turned = exif
        .as_ref()
        .is_some_and(|e| matches!(orientation_of(e), 5..=8));
    let (w, h) = (
        size.as_ref().and_then(|s| u32::try_from(s.width).ok()),
        size.as_ref().and_then(|s| u32::try_from(s.height).ok()),
    );
    ImageInfoReply {
        // As it's shown: a photo taken with the phone on its side is shown upright.
        width: if turned { h } else { w },
        height: if turned { w } else { h },
        fields: fields(exif.as_ref(), &extra),
    }
}

/// The EXIF in `path`, as much of it as can be read.
pub fn read_exif(path: &Path) -> Option<Exif> {
    let f = File::open(path).ok()?;
    exif::Reader::new()
        .continue_on_error(true)
        .read_from_container(&mut BufReader::new(f))
        .or_else(|e| e.distill_partial_result(|_| {}))
        .ok()
}

/// How the picture in `path` must be turned to be the right way up, as EXIF's orientation, 1 to
/// 8. 1 (as it is) if it doesn't say, or says something that isn't an orientation.
pub fn orientation(path: &Path) -> u32 {
    read_exif(path).map_or(1, |e| orientation_of(&e))
}

fn orientation_of(exif: &Exif) -> u32 {
    exif.get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .filter(|o| (1..=8).contains(o))
        .unwrap_or(1)
}

/// The details worth showing a person, labelled, in the order a photo app lists them.
pub fn fields(exif: Option<&Exif>, extra: &Extra) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut push = |label: &str, value: Option<String>| {
        if let Some(v) = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
            out.push((label.to_owned(), v));
        }
    };
    let e = |f: fn(&Exif) -> Option<String>| exif.and_then(f);
    push("Camera", e(camera));
    push("Lens", e(|x| text(x, Tag::LensModel)));
    push("Taken", e(taken));
    push("Exposure", e(|x| number(x, Tag::ExposureTime)));
    push("Aperture", e(|x| number(x, Tag::FNumber)));
    push("ISO", e(|x| number(x, Tag::PhotographicSensitivity)));
    push("Focal length", e(focal_length));
    push("Flash", e(|x| text(x, Tag::Flash)));
    push("Turned", e(turned));
    push("Location", e(location));
    push("Altitude", e(altitude));
    push(
        "Title",
        e(|x| windows_text(x, XP_TITLE)).or_else(|| extra.title.clone()),
    );
    push(
        "Description",
        e(|x| text(x, Tag::ImageDescription)).or_else(|| extra.description.clone()),
    );
    push(
        "Keywords",
        e(|x| windows_text(x, XP_KEYWORDS)).or_else(|| extra.keywords.clone()),
    );
    push(
        "Comment",
        e(|x| user_comment(x).or_else(|| windows_text(x, XP_COMMENT))),
    );
    push(
        "Artist",
        e(|x| text(x, Tag::Artist).or_else(|| windows_text(x, XP_AUTHOR)))
            .or_else(|| extra.artist.clone()),
    );
    push(
        "Copyright",
        e(|x| text(x, Tag::Copyright)).or_else(|| extra.copyright.clone()),
    );
    push("Software", e(|x| text(x, Tag::Software)));
    out
}

/// A text field as it was written, without the quotes EXIF's own formatting puts round it.
fn text(exif: &Exif, tag: Tag) -> Option<String> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    let s = match &field.value {
        Value::Ascii(parts) => parts
            .iter()
            .map(|p| {
                String::from_utf8_lossy(p)
                    .trim_end_matches('\0')
                    .trim()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => field.display_value().with_unit(exif).to_string(),
    };
    Some(s.trim().to_owned())
}

/// A number with its unit ("1/120 s", "f/1.8"), unless it is one no camera could record: a
/// fraction over zero, which would read "inf" or "NaN".
fn number(exif: &Exif, tag: Tag) -> Option<String> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    let sane = match &field.value {
        Value::Rational(r) => r.iter().all(|r| r.denom != 0),
        Value::SRational(r) => r.iter().all(|r| r.denom != 0),
        Value::Float(f) => f.iter().all(|f| f.is_finite()),
        Value::Double(f) => f.iter().all(|f| f.is_finite()),
        _ => true,
    };
    sane.then(|| field.display_value().with_unit(exif).to_string())
}

/// "Apple iPhone 15 Pro": the maker and model, without saying the maker twice when the model
/// already starts with it ("Canon" + "Canon EOS R5", "NIKON CORPORATION" + "NIKON D850").
fn camera(exif: &Exif) -> Option<String> {
    let make = text(exif, Tag::Make).unwrap_or_default();
    let model = text(exif, Tag::Model).unwrap_or_default();
    let brand = make.split_whitespace().next().unwrap_or("").to_lowercase();
    let joined =
        if make.is_empty() || (!brand.is_empty() && model.to_lowercase().starts_with(&brand)) {
            model
        } else if model.is_empty() {
            make
        } else {
            format!("{make} {model}")
        };
    (!joined.is_empty()).then_some(joined)
}

/// When it was taken, as people write it ("2023-06-15 14:30:45"), with the time zone it was
/// taken in when the camera recorded that. A date that can't be one (all zeros, a 13th month)
/// is left out.
fn taken(exif: &Exif) -> Option<String> {
    let (when, zone) = [
        (Tag::DateTimeOriginal, Tag::OffsetTimeOriginal),
        (Tag::DateTimeDigitized, Tag::OffsetTimeDigitized),
        (Tag::DateTime, Tag::OffsetTime),
    ]
    .into_iter()
    .find_map(|(date, zone)| Some((date_time(&text(exif, date)?)?, text(exif, zone))))?;
    Some(match zone.filter(|z| is_zone(z)) {
        Some(zone) => format!("{when} {zone}"),
        None => when,
    })
}

/// `2023:06:15 14:30:45` as `2023-06-15 14:30:45`, or `None` if it isn't a real date and time.
pub fn date_time(exif_date: &str) -> Option<String> {
    let (date, time) = exif_date.trim().split_once(' ')?;
    let mut d = date.split(':').map(|p| p.parse::<u32>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|p| p.parse::<u32>().ok());
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next()??);
    let valid = (1..=12).contains(&m) && (1..=31).contains(&day) && hh < 24 && mm < 60 && ss < 61;
    valid.then(|| format!("{y:04}-{m:02}-{day:02} {hh:02}:{mm:02}:{ss:02}"))
}

/// Whether `z` is a time zone as EXIF writes one: `+02:00`, `-05:30`.
fn is_zone(z: &str) -> bool {
    let b = z.as_bytes();
    b.len() == 6
        && matches!(b[0], b'+' | b'-')
        && b[3] == b':'
        && [1, 2, 4, 5].iter().all(|&i| b[i].is_ascii_digit())
}

/// "6.9 mm (26 mm in 35 mm terms)": what a phone's tiny lens is like on a full-frame camera,
/// which is how most people know focal lengths.
fn focal_length(exif: &Exif) -> Option<String> {
    let real = number(exif, Tag::FocalLength);
    let full_frame = exif
        .get_field(Tag::FocalLengthIn35mmFilm, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .filter(|&mm| mm > 0);
    match (real, full_frame) {
        (Some(real), Some(ff)) => Some(format!("{real} ({ff} mm in 35 mm terms)")),
        (Some(real), None) => Some(real),
        (None, Some(ff)) => Some(format!("{ff} mm in 35 mm terms")),
        (None, None) => None,
    }
}

/// How the camera says to turn the picture, which the Viewer has done, in words. Nothing when
/// it's already the right way up.
fn turned(exif: &Exif) -> Option<String> {
    Some(
        match orientation_of(exif) {
            2 => "Mirrored",
            3 => "Upside down",
            4 => "Mirrored top to bottom",
            5 => "On its side, mirrored",
            6 => "On its side (turned a quarter right to show it)",
            7 => "On its side, mirrored the other way",
            8 => "On its side (turned a quarter left to show it)",
            _ => return None,
        }
        .to_owned(),
    )
}

/// Where it was taken, as decimal latitude and longitude ("51.50072, -0.12462"), from EXIF's
/// degrees, minutes and seconds with their N/S and E/W. Somewhere that can't be (past a pole,
/// or round the world twice) is left out; without the N/S or E/W, it says it can't tell.
fn location(exif: &Exif) -> Option<String> {
    let (lat, lat_known) = coordinate(exif, Tag::GPSLatitude, Tag::GPSLatitudeRef, b'S')?;
    let (lon, lon_known) = coordinate(exif, Tag::GPSLongitude, Tag::GPSLongitudeRef, b'W')?;
    if lat.abs() > 90.0 || lon.abs() > 180.0 {
        return None;
    }
    Some(if lat_known && lon_known {
        format!("{lat:.5}, {lon:.5}")
    } else {
        format!("{lat:.5}, {lon:.5} (north/south or east/west wasn't recorded)")
    })
}

/// Degrees, signed by the reference tag, and whether there was one.
fn coordinate(exif: &Exif, tag: Tag, reference: Tag, negative: u8) -> Option<(f64, bool)> {
    let Value::Rational(dms) = &exif.get_field(tag, In::PRIMARY)?.value else {
        return None;
    };
    if dms.iter().any(|r| r.denom == 0) {
        return None;
    }
    let part = |i: usize| dms.get(i).map_or(0.0, |r| r.to_f64());
    let degrees = part(0) + part(1) / 60.0 + part(2) / 3600.0;
    if !degrees.is_finite() {
        return None;
    }
    let reference = match exif.get_field(reference, In::PRIMARY).map(|f| &f.value) {
        Some(Value::Ascii(r)) => r.first().and_then(|r| r.first()).copied(),
        _ => None,
    };
    let degrees = if reference == Some(negative) {
        -degrees
    } else {
        degrees
    };
    Some((degrees, reference.is_some()))
}

/// "430 m above sea level", or below it when the reference says so (the Dead Sea).
fn altitude(exif: &Exif) -> Option<String> {
    let Value::Rational(r) = &exif.get_field(Tag::GPSAltitude, In::PRIMARY)?.value else {
        return None;
    };
    let r = r.first().filter(|r| r.denom != 0)?;
    let below = exif
        .get_field(Tag::GPSAltitudeRef, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        == Some(1);
    let metres = r.to_f64();
    let shown = if metres.fract() == 0.0 {
        format!("{metres:.0}")
    } else {
        format!("{metres:.1}")
    };
    Some(format!(
        "{shown} m {} sea level",
        if below { "below" } else { "above" }
    ))
}

/// One of the fields Windows writes for itself (XPTitle, XPKeywords, ...): UTF-16 in bytes.
fn windows_text(exif: &Exif, tag: Tag) -> Option<String> {
    let bytes = match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Byte(b) | Value::Undefined(b, _) => b.clone(),
        _ => return None,
    };
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// The UserComment field: eight bytes naming its character set, then the text.
fn user_comment(exif: &Exif) -> Option<String> {
    let Value::Undefined(bytes, _) = &exif.get_field(Tag::UserComment, In::PRIMARY)?.value else {
        return None;
    };
    let (code, body) = bytes.split_at_checked(8)?;
    let text = match code {
        b"UNICODE\0" => {
            // UTF-16 in the file's own byte order, which a byte-order mark says if present.
            let big = body.starts_with(&[0xFE, 0xFF])
                || (!body.starts_with(&[0xFF, 0xFE]) && body.first() == Some(&0));
            let units: Vec<u16> = body
                .chunks_exact(2)
                .map(|c| {
                    if big {
                        u16::from_be_bytes([c[0], c[1]])
                    } else {
                        u16::from_le_bytes([c[0], c[1]])
                    }
                })
                .filter(|&u| u != 0xFEFF)
                .collect();
            String::from_utf16_lossy(&units)
        }
        // ASCII, or "undefined", which in practice is ASCII or UTF-8.
        b"ASCII\0\0\0" | [0, 0, 0, 0, 0, 0, 0, 0] => String::from_utf8_lossy(body).into_owned(),
        _ => return None,
    };
    Some(text.trim_end_matches(['\0', ' ']).to_owned())
}

/// What a picture says in XMP and IPTC, the other two ways software records who made a
/// picture and what it is: used where EXIF doesn't say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extra {
    pub title: Option<String>,
    pub description: Option<String>,
    pub keywords: Option<String>,
    pub artist: Option<String>,
    pub copyright: Option<String>,
}

impl Extra {
    /// From the start of a file: XMP wherever its packet is, and IPTC where Photoshop keeps it.
    /// XMP wins where both say.
    pub fn from_bytes(head: &[u8]) -> Self {
        let xmp = xmp(head);
        let iptc = iptc(head);
        let or = |a: Option<String>, b: Option<String>| a.or(b);
        Self {
            title: or(xmp.title, iptc.title),
            description: or(xmp.description, iptc.description),
            keywords: or(xmp.keywords, iptc.keywords),
            artist: or(xmp.artist, iptc.artist),
            copyright: or(xmp.copyright, iptc.copyright),
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The Dublin Core fields of the first XMP packet in `head`.
fn xmp(head: &[u8]) -> Extra {
    let parse = || -> Option<Extra> {
        let start = find(head, b"<x:xmpmeta")?;
        let len = find(&head[start..], b"</x:xmpmeta>")? + b"</x:xmpmeta>".len();
        let xml = std::str::from_utf8(&head[start..start + len]).ok()?;
        // No DTD, so no entities to expand: roxmltree refuses them unless asked.
        let doc = roxmltree::Document::parse(xml).ok()?;
        const DC: &str = "http://purl.org/dc/elements/1.1/";
        // Every `rdf:li` under the `dc:<name>` element (a list, an alternative, or a bag),
        // joined; or its own text if it has none.
        let get = |name: &str| -> Option<String> {
            let node = doc
                .descendants()
                .find(|n| n.tag_name().namespace() == Some(DC) && n.tag_name().name() == name)?;
            let items: Vec<&str> = node
                .descendants()
                .filter(|n| n.tag_name().name() == "li")
                .filter_map(|n| n.text())
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect();
            let joined = if items.is_empty() {
                node.text().unwrap_or("").trim().to_owned()
            } else {
                items.join(", ")
            };
            (!joined.is_empty()).then_some(joined)
        };
        Some(Extra {
            title: get("title"),
            description: get("description"),
            keywords: get("subject"),
            artist: get("creator"),
            copyright: get("rights"),
        })
    };
    parse().unwrap_or_default()
}

/// The IPTC fields Photoshop keeps in its `8BIM` resource 0x0404.
fn iptc(head: &[u8]) -> Extra {
    let mut out = Extra::default();
    let Some(at) = find(head, b"8BIM\x04\x04") else {
        return out;
    };
    // The resource: type and id (6 bytes), a padded Pascal-string name, a 4-byte size, data.
    let mut i = at + 6;
    let Some(&name_len) = head.get(i) else {
        return out;
    };
    i += 1 + usize::from(name_len);
    if i % 2 == 1 {
        i += 1;
    }
    let Some(size) = head.get(i..i + 4) else {
        return out;
    };
    let size = u32::from_be_bytes([size[0], size[1], size[2], size[3]]) as usize;
    i += 4;
    let data = &head[i..(i + size).min(head.len())];
    // IIM datasets: 0x1C, record, dataset, a 2-byte length, the value.
    let mut keywords = Vec::new();
    let mut j = 0;
    while j + 5 <= data.len() && data[j] == 0x1C {
        let (record, dataset) = (data[j + 1], data[j + 2]);
        let len = usize::from(u16::from_be_bytes([data[j + 3], data[j + 4]]));
        let Some(value) = data.get(j + 5..j + 5 + len) else {
            break;
        };
        let value = String::from_utf8_lossy(value).trim().to_owned();
        if record == 2 && !value.is_empty() {
            match dataset {
                5 => out.title = Some(value),
                25 => keywords.push(value),
                80 => out.artist = Some(value),
                116 => out.copyright = Some(value),
                120 => out.description = Some(value),
                _ => {}
            }
        }
        j += 5 + len;
    }
    if !keywords.is_empty() {
        out.keywords = Some(keywords.join(", "));
    }
    out
}
