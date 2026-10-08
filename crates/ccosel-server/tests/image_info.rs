//! `ImageInfo`: a picture's size and EXIF details, read on the server for the Viewer.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_proto::server_error;
use ccosel_server::fs_api::Jail;
use exif::{Field, In, Rational, Tag, Value};

fn temp_jail() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-image-info-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn ascii(tag: Tag, s: &str) -> Field {
    Field {
        tag,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![s.as_bytes().to_vec()]),
    }
}

fn rational(tag: Tag, parts: &[(u32, u32)]) -> Field {
    Field {
        tag,
        ifd_num: In::PRIMARY,
        value: Value::Rational(
            parts
                .iter()
                .map(|&(num, denom)| Rational { num, denom })
                .collect(),
        ),
    }
}

/// A JPEG of `w`×`h` carrying `fields` as EXIF: just the headers, which is all either reader
/// looks at.
fn jpeg(w: u16, h: u16, fields: &[Field]) -> Vec<u8> {
    let mut writer = exif::experimental::Writer::new();
    for f in fields {
        writer.push_field(f);
    }
    let mut tiff = Cursor::new(Vec::new());
    writer.write(&mut tiff, false).unwrap();
    let tiff = tiff.into_inner();

    let mut out = vec![0xFF, 0xD8];
    let app1_len = u16::try_from(2 + 6 + tiff.len()).unwrap();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&app1_len.to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&tiff);
    // Start of frame: precision, height, width, one component.
    out.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&[0x01, 0x01, 0x11, 0x00]);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

/// A PNG header of `w`×`h`: a screenshot, with a size and no camera.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    out
}

fn value<'a>(fields: &'a [(String, String)], label: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(l, _)| l == label)
        .map(|(_, v)| v.as_str())
}

#[test]
fn a_photo_reports_its_size_camera_and_where_it_was_taken() {
    let dir = temp_jail();
    fs::write(
        dir.join("photo.jpg"),
        jpeg(
            640,
            480,
            &[
                ascii(Tag::Make, "Canon"),
                ascii(Tag::Model, "Canon EOS R5"),
                ascii(Tag::DateTimeOriginal, "2024:05:01 12:34:56"),
                rational(Tag::ExposureTime, &[(1, 125)]),
                rational(Tag::FNumber, &[(28, 10)]),
                rational(Tag::GPSLatitude, &[(51, 1), (30, 1), (0, 1)]),
                ascii(Tag::GPSLatitudeRef, "N"),
                rational(Tag::GPSLongitude, &[(0, 1), (7, 1), (30, 1)]),
                ascii(Tag::GPSLongitudeRef, "W"),
            ],
        ),
    )
    .unwrap();
    let jail = Jail::new(&dir).unwrap();
    let info = jail.image_info("/photo.jpg", None).unwrap();

    assert_eq!((info.width, info.height), (Some(640), Some(480)));
    let f = &info.fields;
    assert_eq!(
        value(f, "Camera"),
        Some("Canon EOS R5"),
        "the maker isn't said twice"
    );
    assert_eq!(f[0].0, "Camera", "in the order a photo app lists them");
    let taken = value(f, "Taken").unwrap();
    assert!(taken.contains("2024") && taken.contains("12:34"), "{taken}");
    assert!(value(f, "Exposure").unwrap().contains("1/125"), "{f:?}");
    assert!(value(f, "Aperture").unwrap().contains("2.8"), "{f:?}");
    assert_eq!(value(f, "Location"), Some("51.50000, -0.12500"));
}

#[test]
fn a_screenshot_has_a_size_and_no_camera_details() {
    let dir = temp_jail();
    fs::write(dir.join("shot.png"), png(1920, 1080)).unwrap();
    let info = Jail::new(&dir)
        .unwrap()
        .image_info("/shot.png", None)
        .unwrap();
    assert_eq!((info.width, info.height), (Some(1920), Some(1080)));
    assert!(info.fields.is_empty());
}

#[test]
fn something_that_isnt_a_picture_has_nothing_to_say() {
    let dir = temp_jail();
    fs::write(dir.join("notes.jpg"), b"not really a picture").unwrap();
    let info = Jail::new(&dir)
        .unwrap()
        .image_info("/notes.jpg", None)
        .unwrap();
    assert_eq!(info, Default::default());
}

#[test]
fn only_pictures_the_caller_may_read() {
    let dir = temp_jail();
    fs::create_dir_all(dir.join("private")).unwrap();
    fs::write(dir.join("private/.access"), "read: alice\nwrite: alice\n").unwrap();
    fs::write(dir.join("private/me.png"), png(1, 1)).unwrap();
    let jail = Jail::new(&dir).unwrap();
    assert!(jail.image_info("/private/me.png", Some("alice")).is_ok());
    assert_eq!(
        jail.image_info("/private/me.png", Some("bob")),
        Err(server_error::NOT_FOUND)
    );
    assert_eq!(
        jail.image_info("/private", Some("alice")),
        Err(server_error::NOT_FOUND),
        "a folder is not a picture"
    );
}

fn field(tag: Tag, value: Value) -> Field {
    Field {
        tag,
        ifd_num: In::PRIMARY,
        value,
    }
}

/// `fields` read back as the Details would show them.
fn details(fields: &[Field]) -> Vec<(String, String)> {
    let dir = temp_jail();
    fs::write(dir.join("p.jpg"), jpeg(64, 48, fields)).unwrap();
    Jail::new(&dir)
        .unwrap()
        .image_info("/p.jpg", None)
        .unwrap()
        .fields
}

#[test]
fn dates_read_as_people_write_them_with_their_time_zone() {
    let f = details(&[
        ascii(Tag::DateTimeOriginal, "2023:06:15 14:30:45"),
        ascii(Tag::OffsetTimeOriginal, "+02:00"),
    ]);
    assert_eq!(value(&f, "Taken"), Some("2023-06-15 14:30:45 +02:00"));
    // A date that can't be one is left out, and the next one tried.
    let f = details(&[
        ascii(Tag::DateTimeOriginal, "0000:00:00 00:00:00"),
        ascii(Tag::DateTime, "1936:08:01 12:00:00"),
    ]);
    assert_eq!(value(&f, "Taken"), Some("1936-08-01 12:00:00"));
    assert_eq!(
        value(
            &details(&[ascii(Tag::DateTime, "2023:13:40 25:00:00")]),
            "Taken"
        ),
        None
    );
}

#[test]
fn where_it_was_taken_is_said_only_if_it_can_be_somewhere() {
    let at = |lat: u32, lon: u32, refs: bool| {
        let mut f = vec![
            rational(Tag::GPSLatitude, &[(lat, 1), (0, 1), (0, 1)]),
            rational(Tag::GPSLongitude, &[(lon, 1), (0, 1), (0, 1)]),
        ];
        if refs {
            f.push(ascii(Tag::GPSLatitudeRef, "S"));
            f.push(ascii(Tag::GPSLongitudeRef, "E"));
        }
        details(&f)
    };
    assert_eq!(
        value(&at(33, 151, true), "Location"),
        Some("-33.00000, 151.00000")
    );
    assert_eq!(
        value(&at(48, 2, false), "Location"),
        Some("48.00000, 2.00000 (north/south or east/west wasn't recorded)")
    );
    assert_eq!(value(&at(200, 999, true), "Location"), None, "past a pole");
    let zero = details(&[
        rational(Tag::GPSLatitude, &[(1, 0)]),
        rational(Tag::GPSLongitude, &[(1, 1)]),
    ]);
    assert_eq!(value(&zero, "Location"), None, "divided by zero");
}

#[test]
fn altitude_says_above_or_below_the_sea() {
    let alt = |reference: u8| {
        details(&[
            rational(Tag::GPSAltitude, &[(430, 1)]),
            field(Tag::GPSAltitudeRef, Value::Byte(vec![reference])),
        ])
    };
    assert_eq!(value(&alt(0), "Altitude"), Some("430 m above sea level"));
    assert_eq!(value(&alt(1), "Altitude"), Some("430 m below sea level"));
}

#[test]
fn numbers_no_camera_could_record_are_left_out() {
    let f = details(&[
        rational(Tag::FNumber, &[(1, 0)]),
        rational(Tag::ExposureTime, &[(1, 250)]),
    ]);
    assert_eq!(value(&f, "Aperture"), None);
    assert!(
        value(&f, "Exposure").unwrap().contains("1/250"),
        "and the rest kept"
    );
}

#[test]
fn the_camera_lens_and_turn_read_as_people_say_them() {
    let f = details(&[
        ascii(Tag::Make, "NIKON CORPORATION"),
        ascii(Tag::Model, "NIKON D850"),
        rational(Tag::FocalLength, &[(686, 100)]),
        field(Tag::FocalLengthIn35mmFilm, Value::Short(vec![24])),
        field(Tag::Orientation, Value::Short(vec![6])),
    ]);
    assert_eq!(value(&f, "Camera"), Some("NIKON D850"));
    assert_eq!(
        value(&f, "Focal length"),
        Some("6.86 mm (24 mm in 35 mm terms)")
    );
    assert!(value(&f, "Turned").unwrap().contains("quarter right"));
    let f = details(&[field(Tag::Orientation, Value::Short(vec![9]))]);
    assert_eq!(value(&f, "Turned"), None, "9 isn't an orientation");
}

/// A JPEG of 400×300 tagged with orientation 6 is shown 300×400, and says so.
#[test]
fn a_turned_photos_size_is_as_it_is_shown() {
    let dir = temp_jail();
    fs::write(
        dir.join("p.jpg"),
        jpeg(400, 300, &[field(Tag::Orientation, Value::Short(vec![6]))]),
    )
    .unwrap();
    let info = Jail::new(&dir).unwrap().image_info("/p.jpg", None).unwrap();
    assert_eq!((info.width, info.height), (Some(300), Some(400)));
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .flat_map(u16::to_le_bytes)
        .chain([0, 0])
        .collect()
}

#[test]
fn windows_own_fields_and_the_comment_are_read() {
    let f = details(&[
        field(
            Tag(exif::Context::Tiff, 0x9C9B),
            Value::Byte(utf16("Holiday")),
        ),
        field(
            Tag(exif::Context::Tiff, 0x9C9E),
            Value::Byte(utf16("sea;sand")),
        ),
        field(Tag(exif::Context::Tiff, 0x9C9D), Value::Byte(utf16("Zoë"))),
        field(
            Tag::UserComment,
            Value::Undefined(b"ASCII\0\0\0caf\xc3\xa9 time".to_vec(), 0),
        ),
    ]);
    assert_eq!(value(&f, "Title"), Some("Holiday"));
    assert_eq!(value(&f, "Keywords"), Some("sea;sand"));
    assert_eq!(value(&f, "Artist"), Some("Zoë"));
    assert_eq!(value(&f, "Comment"), Some("café time"));
}

#[test]
fn xmp_and_iptc_fill_in_what_exif_doesnt_say() {
    let xmp = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator><rdf:Seq><rdf:li>Ada</rdf:li></rdf:Seq></dc:creator><dc:rights><rdf:Alt><rdf:li xml:lang="x-default">All mine</rdf:li></rdf:Alt></dc:rights></rdf:Description></rdf:RDF></x:xmpmeta>"#;
    // Photoshop's resource 0x0404: IIM caption and two keywords.
    let mut iim = Vec::new();
    for (ds, v) in [
        (120u8, "A caption"),
        (25, "nature"),
        (25, "sea"),
        (80, "Bob"),
    ] {
        iim.extend([0x1C, 2, ds]);
        iim.extend((v.len() as u16).to_be_bytes());
        iim.extend(v.as_bytes());
    }
    let mut head = b"8BIM\x04\x04\0\0".to_vec();
    head.extend((iim.len() as u32).to_be_bytes());
    head.extend(&iim);
    head.extend(xmp.as_bytes());
    let extra = ccosel_server::image_info::Extra::from_bytes(&head);
    assert_eq!(extra.artist.as_deref(), Some("Ada"), "XMP wins over IPTC");
    assert_eq!(extra.copyright.as_deref(), Some("All mine"));
    assert_eq!(extra.description.as_deref(), Some("A caption"));
    assert_eq!(extra.keywords.as_deref(), Some("nature, sea"));
    // EXIF first, then these.
    let f = ccosel_server::image_info::fields(None, &extra);
    assert_eq!(value(&f, "Artist"), Some("Ada"));
    assert_eq!(
        ccosel_server::image_info::Extra::from_bytes(b"<x:xmpmeta broken"),
        Default::default()
    );
}

/// The media test pack's EXIF folder, read for real. Opt-in, like the other pack tests:
/// `CCOSEL_TEST_MEDIA_PACK=<pack> cargo test -p ccosel-server --test image_info`.
#[test]
fn the_media_packs_exif_tests() {
    let Some(pack) = std::env::var_os("CCOSEL_TEST_MEDIA_PACK") else {
        eprintln!("skipped: set CCOSEL_TEST_MEDIA_PACK to read the pack's EXIF tests");
        return;
    };
    let dir = PathBuf::from(pack).join("images/exif_tests");
    if !dir.is_dir() {
        eprintln!("skipped: this pack has no images/exif_tests");
        return;
    }
    let read = |rel: &str| ccosel_server::image_info::read(&dir.join(rel)).fields;
    let f = read("gps/gps_south_west_sydney.jpg");
    assert!(value(&f, "Location").unwrap().starts_with("-33.8"));
    let f = read("camera_makernote/full_camera_nikon.jpg");
    assert_eq!(value(&f, "Camera"), Some("NIKON D850"));
    let f = read("timestamps/with_subsec_and_tz.jpg");
    assert_eq!(value(&f, "Taken"), Some("2023-06-15 14:30:45 +02:00"));
    assert_eq!(
        value(&read("timestamps/invalid_date_zeros.jpg"), "Taken"),
        None
    );
    assert_eq!(
        value(&read("malformed/iso_max_uint16.jpg"), "ISO"),
        Some("65535"),
        "the rest of a damaged block"
    );
    assert_eq!(
        value(&read("malformed/gps_out_of_range.jpg"), "Location"),
        None
    );
    assert_eq!(
        value(&read("malformed/rational_zero_denominator.jpg"), "Aperture"),
        None
    );
    assert_eq!(
        value(&read("encoding/xp_fields_utf16.jpg"), "Title"),
        Some("Windows XP Title field")
    );
    let f = read("stripped_vs_tagged/with_xmp_and_iptc.jpg");
    assert_eq!(value(&f, "Artist"), Some("XMP Creator"));
    assert_eq!(value(&f, "Description"), Some("IPTC Caption"));
    assert!(read("stripped_vs_tagged/all_metadata_stripped.jpg").is_empty());
    // Every file in it, damaged or not, reads without trouble.
    for entry in walk(&dir) {
        let _ = ccosel_server::image_info::read(&entry);
    }
}

fn walk(d: &std::path::Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for e in fs::read_dir(d).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            v.extend(walk(&p));
        } else {
            v.push(p);
        }
    }
    v
}
