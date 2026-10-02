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
