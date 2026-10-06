use ccosel_proto::fs::{
    DirEntry, DirListing, EntryKind, FileText, ImageInfo, ImageInfoReply, ListDir, MAX_TEXT_BYTES,
    ReadFile, Search, SearchHit, SearchReply, Shown, WebCopy, WebCopyStatus,
};
use ccosel_sdk::testing::{Harness, rpc_error};
use ccosel_sdk::{MediaKind, TextStyle, Vec2, icons};

use super::*;

fn entry(name: &str, kind: EntryKind, size: u64) -> DirEntry {
    DirEntry {
        name: name.to_owned(),
        kind,
        size,
        mtime_s: 0,
        writable: false,
    }
}

fn listing(entries: Vec<DirEntry>, truncated: bool) -> DirListing {
    DirListing { entries, truncated }
}

fn label(icon: &str, text: &str) -> String {
    format!("{icon}  {text}")
}

/// A Viewer opened on `path`, with its folder listed as holding `name` at `size`.
fn opened_on(path: &str, size: u64) -> Harness<Viewer> {
    let mut h = Harness::new(Viewer::default());
    h.launch(path);
    h.frame();
    assert!(h.has_label("Opening…"));
    let (_, name) = split(path);
    h.reply::<ListDir>(&listing(vec![entry(name, EntryKind::File, size)], false));
    h.frame();
    h
}

#[test]
fn the_window_is_called_by_its_files_name() {
    let mut h = Harness::new(Viewer::default());
    h.frame();
    assert_eq!(h.window_title(), None, "a search is just the Viewer");
    let h = opened_on("/Photos/beach day.jpg", 10);
    assert_eq!(h.window_title().as_deref(), Some("beach day.jpg"));
    let h = opened_on("/notes.txt", 10);
    assert_eq!(h.window_title().as_deref(), Some("notes.txt"));
}

#[test]
fn opened_from_the_menu_it_is_a_search() {
    let mut h = Harness::new(Viewer::default());
    h.frame();
    assert!(h.has_text("Find a file"));
    assert_eq!(h.text_fields(), [""]);
    assert_eq!(
        h.outstanding::<Search>(),
        0,
        "nothing searched before typing"
    );

    h.type_text(0, "a");
    h.frame();
    assert_eq!(
        h.outstanding::<Search>(),
        0,
        "one letter matches nearly everything"
    );
    assert!(h.has_text(
        "Type a name, or some words from inside a file, to find it anywhere you can see."
    ));
}

#[test]
fn a_search_result_opens_and_the_window_stays_on_it() {
    let mut h = Harness::new(Viewer::default());
    h.frame();
    h.type_text(0, "holiday");
    h.frame();
    assert!(h.has_label("Searching…"));
    h.reply::<Search>(&SearchReply {
        hits: vec![
            SearchHit {
                path: "/Photos/holiday.jpg".to_owned(),
                line: String::new(),
            },
            SearchHit {
                path: "/notes.txt".to_owned(),
                line: "plans for the holiday".to_owned(),
            },
        ],
        truncated: true,
    });
    h.frame();
    assert_eq!(
        h.selectables(),
        [
            ("holiday.jpg".to_owned(), false),
            ("notes.txt".to_owned(), false)
        ]
    );
    assert!(h.has_text("/Photos"));
    assert!(h.has_text("plans for the holiday"));
    assert!(h.has_text("More files match than are shown: try a longer search."));

    h.click("holiday.jpg");
    h.frame();
    assert_eq!(h.app.file.as_deref(), Some("/Photos/holiday.jpg"));
    h.frame();
    // From here the window is that file: no search, and no way back to one.
    assert!(h.text_fields().is_empty());
    assert!(!h.has_text("Find a file"));
    assert!(h.buttons().is_empty(), "nothing to go back with");
}

#[test]
fn a_search_that_finds_nothing_or_fails_says_so() {
    let mut h = Harness::new(Viewer::default());
    h.frame();
    h.type_text(0, "zzz");
    h.frame();
    h.reply::<Search>(&SearchReply {
        hits: vec![],
        truncated: false,
    });
    h.frame();
    assert!(h.has_text("Nothing matches."));

    h.type_text(0, "qqq");
    h.frame();
    h.fail::<Search>(rpc_error::TIMEOUT);
    h.frame();
    assert!(h.has_label("timed out"));
}

/// The server's answer for a file it has finished with.
fn done(shown: Shown, size: Option<(u32, u32)>) -> WebCopyStatus {
    WebCopyStatus {
        finished: true,
        permille: Some(1000),
        error: None,
        shown: Some(shown),
        width: size.map(|s| s.0),
        height: size.map(|s| s.1),
    }
}

fn converting(shown: Option<Shown>, permille: Option<u16>) -> WebCopyStatus {
    WebCopyStatus {
        finished: false,
        permille,
        error: None,
        shown,
        width: None,
        height: None,
    }
}

/// `path` opened, and the server's answer about it given.
fn shown_as(path: &str, status: &WebCopyStatus) -> Harness<Viewer> {
    let mut h = opened_on(path, 1000);
    assert!(h.has_label("Opening…"));
    assert_eq!(h.outstanding::<WebCopy>(), 1, "{path}: asks the server");
    h.reply::<WebCopy>(status);
    h.frame();
    h
}

#[test]
fn a_picture_is_the_whole_window_shaped_like_it() {
    let h = shown_as(
        "/Photos/beach day.jpg",
        &done(Shown::Picture, Some((4032, 3024))),
    );
    assert_eq!(
        h.images(),
        ["/files/Photos/beach%20day.jpg?inline=1&as=web"]
    );
    assert_eq!(h.window_size(), Some(Vec2::new(4032.0, 3024.0)));
    // No header: its name and the rest are in its right-click menu.
    assert!(h.labels().is_empty(), "{:?}", h.labels());
    assert_eq!(
        h.context_menu_items(),
        [
            details(false),
            label(icons::DOWNLOAD_SIMPLE, "Download"),
            label(icons::LINK, "Share")
        ]
    );
    assert!(
        h.styled()
            .contains(&("beach day.jpg".to_owned(), TextStyle::STRONG))
    );
    assert!(
        h.styled()
            .contains(&("/Photos".to_owned(), TextStyle::WEAK))
    );
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        ccosel_sdk::REPAINT_ON_INPUT_ONLY
    );
}

#[test]
fn a_picture_with_no_known_size_asks_for_no_window_size() {
    let h = shown_as("/x/drawing.svg", &done(Shown::Picture, None));
    assert_eq!(h.images(), ["/files/x/drawing.svg?inline=1&as=web"]);
    assert_eq!(h.window_size(), None);
}

#[test]
fn a_moving_picture_is_played_by_the_browser() {
    let h = shown_as("/fun/cat.gif", &done(Shown::Animated, Some((200, 150))));
    assert!(h.images().is_empty(), "the shell would show it still");
    assert_eq!(
        h.media(),
        [(
            "/files/fun/cat.gif?inline=1&as=web".to_owned(),
            MediaKind::Picture
        )]
    );
    assert_eq!(h.window_size(), Some(Vec2::new(200.0, 150.0)));
    assert!(h.context_menu_items().contains(&details(false)));
}

#[test]
fn a_picture_still_being_made_says_so_without_a_header() {
    let mut h = shown_as("/raw/IMG.dng", &converting(Some(Shown::Picture), None));
    assert_eq!(h.labels(), ["Getting this ready to show…"]);
    assert!(h.open_urls().is_empty(), "no header for a picture");
    assert_eq!(h.app.wants_repaint_after_ms(), POLL_MS);
    h.frame();
    assert_eq!(h.outstanding::<WebCopy>(), 1, "asked again");
    h.reply::<WebCopy>(&done(Shown::Picture, Some((6000, 4000))));
    h.frame();
    assert_eq!(h.images(), ["/files/raw/IMG.dng?inline=1&as=web"]);
}

#[test]
fn video_audio_and_pdf_are_handed_to_the_browser() {
    for (path, shown, kind) in [
        ("/v/clip.MP4", Shown::Video, MediaKind::Video),
        ("/a/song.flac", Shown::Audio, MediaKind::Audio),
    ] {
        let h = shown_as(path, &done(shown, None));
        assert_eq!(h.media(), [(shown_url(path), kind)], "{path}");
        assert!(h.has_text(split(path).1), "{path}: a header");
        assert_eq!(
            h.window_size(),
            None,
            "{path}: only pictures shape the window"
        );
    }
    let h = opened_on("/d/report.pdf", 1000);
    assert_eq!(h.outstanding::<WebCopy>(), 0, "a PDF needs no asking");
    assert_eq!(
        h.media(),
        [(url::inline_file_url("/d/report.pdf"), MediaKind::Document)]
    );
}

#[test]
fn a_video_shows_its_conversion_then_plays_the_copy() {
    let mut h = shown_as("/Videos/IMG_0002.MOV", &converting(None, None));
    assert!(h.media().is_empty(), "no blank player while it converts");
    assert!(h.has_label("Getting this ready to show…"));
    assert_eq!(h.app.wants_repaint_after_ms(), POLL_MS);

    h.frame();
    h.reply::<WebCopy>(&converting(Some(Shown::Video), Some(420)));
    h.frame();
    assert!(
        h.has_label("Converting this video so it plays in this browser… 42%"),
        "{:?}",
        h.labels()
    );
    assert!(h.has_text("Only the first time: once converted, it plays straight away."));
    assert!(h.has_text("IMG_0002.MOV"), "a video has its header");
    h.frame();
    assert_eq!(h.outstanding::<WebCopy>(), 1, "asked again");
    h.reply::<WebCopy>(&done(Shown::Video, Some((1920, 1080))));
    h.frame();
    assert_eq!(
        h.media(),
        [(
            "/files/Videos/IMG_0002.MOV?inline=1&as=web".to_owned(),
            MediaKind::Video
        )]
    );
    assert_eq!(
        h.app.wants_repaint_after_ms(),
        ccosel_sdk::REPAINT_ON_INPUT_ONLY
    );
}

#[test]
fn a_file_that_cannot_be_converted_says_why() {
    let h = shown_as(
        "/Videos/broken.avi",
        &WebCopyStatus {
            finished: true,
            permille: Some(1000),
            error: Some("it looks damaged, or isn't a kind of video this can read".to_owned()),
            shown: None,
            width: None,
            height: None,
        },
    );
    assert!(h.media().is_empty());
    assert!(h.has_label(
        "This can't be shown here: it looks damaged, or isn't a kind of video this can read. \
         Download it to open it."
    ));
    assert!(h.open_urls().contains(&(
        label(icons::DOWNLOAD_SIMPLE, "Download"),
        "/files/Videos/broken.avi".to_owned()
    )));

    let mut h = opened_on("/Videos/private.mov", 1000);
    h.fail::<WebCopy>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
}

/// Text called `.jpg`, or a file whose name says nothing: if the server finds it isn't a
/// picture, video or recording, it's shown as text.
#[test]
fn what_isnt_media_after_all_is_shown_as_text() {
    for path in ["/odd/notes.jpg", "/odd/README"] {
        let mut h = shown_as(path, &done(Shown::NotMedia, None));
        assert_eq!(h.outstanding::<ReadFile>(), 1, "{path}");
        h.reply::<ReadFile>(&FileText {
            text: "{\"a\": 1}".to_owned(),
            writable: false,
        });
        h.frame();
        assert_eq!(h.text_views(), ["{\"a\": 1}"], "{path}");
        assert!(h.images().is_empty());
    }
}

/// A file with no extension that is really a photo is shown as one.
#[test]
fn a_photo_with_no_extension_is_still_a_photo() {
    let h = shown_as("/inbox/scan", &done(Shown::Picture, Some((640, 480))));
    assert_eq!(h.images(), ["/files/inbox/scan?inline=1&as=web"]);
}

#[test]
fn a_text_file_is_shown_read_only() {
    let mut h = opened_on("/notes.txt", 11);
    assert_eq!(h.outstanding::<ReadFile>(), 1);
    assert!(h.has_label("Loading…"));
    h.reply::<ReadFile>(&FileText {
        text: "hello\nworld".to_owned(),
        writable: true,
    });
    h.frame();
    assert_eq!(h.text_views(), ["hello\nworld"]);
    assert!(h.text_fields().is_empty(), "nothing to edit");
}

#[test]
fn an_empty_file_says_so() {
    let mut h = opened_on("/empty.txt", 0);
    h.reply::<ReadFile>(&FileText {
        text: String::new(),
        writable: false,
    });
    h.frame();
    assert!(h.has_text("This file is empty."));
}

#[test]
fn a_file_that_is_not_text_offers_the_download() {
    let mut h = shown_as("/program.exe", &done(Shown::NotMedia, None));
    h.fail::<ReadFile>(rpc_error::SERVER);
    h.frame();
    assert!(h.has_label("This kind of file can't be shown here. Download it to open it."));
    assert!(h.open_urls().contains(&(
        label(icons::DOWNLOAD_SIMPLE, "Download"),
        "/files/program.exe".to_owned()
    )));
}

#[test]
fn a_big_file_is_not_read_at_all() {
    let h = opened_on("/logs/huge.log", MAX_TEXT_BYTES as u64 + 1);
    assert_eq!(h.outstanding::<ReadFile>(), 0);
    assert!(h.has_label("This file is too big to show here (1 MB). Download it to open it."));
}

#[test]
fn download_and_share_point_at_the_file() {
    let h = opened_on("/Docs/a b.txt", 3);
    assert!(h.open_urls().contains(&(
        label(icons::DOWNLOAD_SIMPLE, "Download"),
        "/files/Docs/a%20b.txt".to_owned()
    )));
    assert_eq!(
        h.copy_links(),
        [(
            label(icons::LINK, "Share"),
            "/app/viewer?open=%2FDocs%2Fa%20b.txt".to_owned()
        )]
    );
}

#[test]
fn a_window_opened_on_a_file_has_no_search() {
    let h = opened_on("/notes.txt", 3);
    assert!(h.text_fields().is_empty());
    assert!(!h.has_text("Find a file"));
    assert!(h.buttons().is_empty(), "{:?}", h.buttons());
}

/// Another file opens in another window: a window never changes what it shows.
#[test]
fn a_window_stays_on_the_file_it_was_opened_for() {
    let mut v = Viewer::default();
    v.open("/a.txt");
    v.open("/b.txt");
    assert_eq!(v.file.as_deref(), Some("/a.txt"));
}

#[test]
fn a_missing_file_a_folder_and_a_failed_listing_each_say_so() {
    let mut h = Harness::new(Viewer::default());
    h.launch("/gone.txt");
    h.frame();
    h.reply::<ListDir>(&listing(
        vec![entry("other.txt", EntryKind::File, 1)],
        false,
    ));
    h.frame();
    assert!(h.has_label("This file isn't there any more, or you aren't allowed to see it."));

    let mut h = Harness::new(Viewer::default());
    h.launch("/Photos");
    h.frame();
    h.reply::<ListDir>(&listing(vec![entry("Photos", EntryKind::Dir, 0)], false));
    h.frame();
    assert!(h.has_label("That is a folder. Open it in Files to see what is in it."));

    let mut h = Harness::new(Viewer::default());
    h.launch("/secret/x.txt");
    h.frame();
    h.fail::<ListDir>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
    assert!(h.has_text("x.txt"), "with its header, to download it from");
}

/// A folder too big to list in full may leave the file out of the listing: it is opened anyway.
#[test]
fn a_file_missing_from_a_truncated_listing_is_still_tried() {
    let mut h = Harness::new(Viewer::default());
    h.launch("/big/zzz.txt");
    h.frame();
    h.reply::<ListDir>(&listing(vec![entry("aaa.txt", EntryKind::File, 1)], true));
    h.frame();
    assert_eq!(h.outstanding::<ReadFile>(), 1);
}

#[test]
fn opening_on_nothing_changes_nothing_and_a_bare_name_is_from_the_top() {
    let mut v = Viewer::default();
    v.open("");
    assert_eq!(v.file, None);
    v.open("notes.txt");
    assert_eq!(v.file.as_deref(), Some("/notes.txt"));
}

#[test]
fn kinds_follow_the_extension_whatever_its_case() {
    for path in [
        "/a/b.PNG", "x.heic", "x.svg", "x.dng", "x.exr", "x.mov", "x.AVI", "x.wmv", "x.m4a",
        "x.wma", "x.ac3",
    ] {
        assert_eq!(kind_of(path), Kind::Media, "{path}");
    }
    assert_eq!(kind_of("x.pdf"), Kind::Pdf);
    assert_eq!(kind_of("x.TSV"), Kind::Table);
    assert_eq!(kind_of("x.md"), Kind::Text);
    assert_eq!(kind_of("x.json"), Kind::Text);
    assert_eq!(
        kind_of("x.ts"),
        Kind::Unknown,
        "TypeScript or a TV recording"
    );
    assert_eq!(kind_of("x.exe"), Kind::Unknown);
    assert_eq!(
        kind_of("/folder.jpg/README"),
        Kind::Unknown,
        "the folder's name doesn't count"
    );
    assert_eq!(kind_of("Makefile"), Kind::Unknown);
    assert_eq!(shown_url("/a/b.jpg"), "/files/a/b.jpg?inline=1&as=web");
}

#[test]
fn paths_split_into_folder_and_name() {
    assert_eq!(split("/a/b.txt"), ("/a", "b.txt"));
    assert_eq!(split("/b.txt"), ("/", "b.txt"));
    assert_eq!(split("b.txt"), ("/", "b.txt"));
}

#[test]
fn sizes_read_as_people_say_them() {
    assert_eq!(human_size(12), "12 bytes");
    assert_eq!(human_size(2048), "2 KB");
    assert_eq!(human_size(5 << 20), "5 MB");
    assert_eq!(human_size(3 << 30), "3 GB");
}

fn cells(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|c| (*c).to_owned()).collect())
        .collect()
}

#[test]
fn csv_is_split_the_way_spreadsheets_write_it() {
    assert_eq!(
        parse_table("name,age\r\nAda,36\nBob,\n", ','),
        cells(&[&["name", "age"], &["Ada", "36"], &["Bob", ""]])
    );
    assert_eq!(
        parse_table("\"Smith, J\",\"says \"\"hi\"\"\"\n\"two\nlines\",x", ','),
        cells(&[&["Smith, J", "says \"hi\""], &["two\nlines", "x"]]),
        "quoted cells keep separators, quotes and line breaks"
    );
    assert_eq!(
        parse_table("a\tb,c\n", '\t'),
        cells(&[&["a", "b,c"]]),
        "a TSV splits on tabs only"
    );
    assert!(parse_table("", ',').is_empty());
    assert_eq!(separator("/x/data.TSV"), '\t');
    assert_eq!(separator("/x/data.csv"), ',');
}

#[test]
fn a_long_cell_is_clipped() {
    assert_eq!(clip("short"), "short");
    let long = "é".repeat(MAX_CELL + 5);
    let clipped = clip(&long);
    assert_eq!(clipped.chars().count(), MAX_CELL + 1);
    assert!(clipped.ends_with('…'));
}

fn opened_csv(text: &str) -> Harness<Viewer> {
    let mut h = opened_on("/data/people.csv", text.len() as u64);
    h.reply::<ReadFile>(&FileText {
        text: text.to_owned(),
        writable: false,
    });
    h.frame();
    h
}

#[test]
fn spreadsheet_data_is_shown_as_a_table() {
    let mut h = opened_csv("name,age\nAda,36\nBob,41\n");
    assert!(h.styled().contains(&("name".to_owned(), TextStyle::STRONG)));
    assert!(h.has_label("Ada") && h.has_label("41"));
    assert!(h.has_text("2 rows"));
    assert!(h.text_views().is_empty());

    h.click("Show as text");
    h.frame();
    h.frame();
    assert!(h.app.as_text);
    assert_eq!(h.text_views(), ["name,age\nAda,36\nBob,41\n"]);
    assert!(!h.has_label("Ada"));

    h.click("Show as a table");
    h.frame();
    h.frame();
    assert!(h.has_label("Ada"));
}

#[test]
fn a_one_row_table_says_row() {
    let h = opened_csv("name\nAda\n");
    assert!(h.has_text("1 row"));
}

#[test]
fn a_big_table_shows_its_first_rows_and_says_so() {
    let mut text = String::from("n\n");
    for i in 0..MAX_ROWS + 10 {
        text.push_str(&format!("{i}\n"));
    }
    let h = opened_csv(&text);
    assert!(h.has_text(&format!("The first {MAX_ROWS} of {} rows", MAX_ROWS + 10)));
    assert!(h.has_label(&format!("{}", MAX_ROWS - 1)));
    assert!(
        !h.has_label(&format!("{MAX_ROWS}")),
        "no further than the cap"
    );
}

fn details(show: bool) -> String {
    if show {
        label(icons::INFO, "Hide details")
    } else {
        label(icons::INFO, "Details")
    }
}

fn picture(path: &str, size: u64) -> Harness<Viewer> {
    let mut h = opened_on(path, size);
    h.reply::<WebCopy>(&done(Shown::Picture, Some((4032, 3024))));
    h.frame();
    h
}

#[test]
fn a_pictures_details_open_from_its_menu_in_a_window() {
    let mut h = picture("/Photos/beach.jpg", 3 << 20);
    assert_eq!(h.outstanding::<ImageInfo>(), 0, "not until asked");

    h.click(&details(false));
    h.frame();
    h.frame();
    assert!(h.app.details);
    assert!(h.has_label("Reading the details…"));
    h.reply::<ImageInfo>(&ImageInfoReply {
        width: Some(4032),
        height: Some(3024),
        fields: vec![
            ("Camera".to_owned(), "Apple iPhone 15 Pro".to_owned()),
            ("Location".to_owned(), "51.50000, -0.12500".to_owned()),
        ],
    });
    h.frame();
    assert!(h.has_label("4032 × 3024 pixels"));
    assert!(h.has_label("3 MB"));
    assert!(h.has_label("Apple iPhone 15 Pro"));
    assert!(
        h.styled()
            .contains(&("Location".to_owned(), TextStyle::STRONG))
    );
    assert!(!h.has_text("No camera details in this picture."));
    assert_eq!(h.images().len(), 1, "the picture is still there");
    assert!(h.context_menu_items().contains(&details(true)));

    h.click("Close");
    h.frame();
    h.frame();
    assert!(!h.app.details);
    assert!(!h.has_label("Apple iPhone 15 Pro"));

    // And the menu entry toggles it too.
    h.click(&details(false));
    h.frame();
    h.frame();
    h.click(&details(true));
    h.frame();
    h.frame();
    assert!(!h.app.details);
}

#[test]
fn a_picture_with_no_camera_details_says_so() {
    let mut h = picture("/shot.png", 10);
    h.app.details = true;
    h.frame();
    h.reply::<ImageInfo>(&ImageInfoReply {
        width: None,
        height: None,
        fields: vec![],
    });
    h.frame();
    assert!(h.has_text("No camera details in this picture."));
    assert!(h.has_label("10 bytes"));
    assert!(!h.has_label("Dimensions"));

    let mut h = picture("/shot.png", 10);
    h.app.details = true;
    h.frame();
    h.fail::<ImageInfo>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
}

#[test]
fn only_pictures_have_details() {
    let h = shown_as("/v/clip.mp4", &done(Shown::Video, Some((320, 240))));
    assert!(!h.has_button(&details(false)));
    assert!(h.context_menu_items().is_empty());
}
