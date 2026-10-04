use ccosel_proto::fs::{
    DirEntry, DirListing, EntryKind, FileText, ImageInfo, ImageInfoReply, ListDir, MAX_TEXT_BYTES,
    ReadFile, Search, SearchHit, SearchReply, WebCopy, WebCopyStatus,
};
use ccosel_sdk::testing::{Harness, rpc_error};
use ccosel_sdk::{MediaKind, TextStyle, icons};

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
    assert_eq!(h.buttons(), [details(false)], "nothing to go back with");
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

#[test]
fn a_picture_is_drawn_by_the_shell_from_its_url() {
    let h = opened_on("/Photos/beach day.jpg", 1000);
    assert_eq!(h.images(), ["/files/Photos/beach%20day.jpg?inline=1"]);
    assert!(h.has_text("beach day.jpg"));
    assert!(h.has_text("/Photos"));
}

#[test]
fn video_audio_and_pdf_are_handed_to_the_browser() {
    for (path, kind) in [
        ("/v/clip.MP4", MediaKind::Video),
        ("/a/song.flac", MediaKind::Audio),
        ("/d/report.pdf", MediaKind::Document),
    ] {
        let h = opened_on(path, 1000);
        assert_eq!(h.media(), [(url::inline_file_url(path), kind)], "{path}");
    }
}

#[test]
fn apple_photos_and_audio_are_shown_from_the_servers_web_copy() {
    let h = opened_on("/Photos/IMG_0001.HEIC", 1000);
    assert_eq!(h.images(), ["/files/Photos/IMG_0001.HEIC?inline=1&as=web"]);
    let h = opened_on("/Scans/page.tif", 1000);
    assert_eq!(h.images(), ["/files/Scans/page.tif?inline=1&as=web"]);
    for path in ["/m/lossless.m4a", "/m/memo.caf", "/m/take.AIFF"] {
        let h = opened_on(path, 1000);
        assert_eq!(
            h.media(),
            [(
                format!("{}&as=web", url::inline_file_url(path)),
                MediaKind::Audio
            )],
            "{path}"
        );
    }
    assert!(!h.has_text("Safari"), "no browser note any more");
    assert_eq!(
        h.outstanding::<WebCopy>(),
        0,
        "only a video waits for its copy"
    );
}

fn converting(permille: Option<u16>) -> WebCopyStatus {
    WebCopyStatus {
        finished: false,
        permille,
        error: None,
    }
}

#[test]
fn an_apple_video_shows_its_conversion_then_plays_the_copy() {
    let mut h = opened_on("/Videos/IMG_0002.MOV", 1000);
    assert!(h.media().is_empty(), "no blank player while it converts");
    assert!(h.has_label("Converting this video so it plays in this browser…"));
    assert_eq!(h.app.wants_repaint_after_ms(), POLL_MS);

    h.reply::<WebCopy>(&converting(None));
    h.frame();
    assert!(h.has_label("Converting this video so it plays in this browser…"));
    h.frame();
    h.reply::<WebCopy>(&converting(Some(420)));
    h.frame();
    assert!(
        h.has_label("Converting this video so it plays in this browser… 42%"),
        "{:?}",
        h.labels()
    );
    h.frame();
    assert_eq!(h.outstanding::<WebCopy>(), 1, "asked again");
    h.reply::<WebCopy>(&WebCopyStatus {
        finished: true,
        permille: Some(1000),
        error: None,
    });
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
fn a_video_that_cannot_be_converted_says_why() {
    let mut h = opened_on("/Videos/broken.mov", 1000);
    h.reply::<WebCopy>(&WebCopyStatus {
        finished: true,
        permille: Some(1000),
        error: Some("ffmpeg couldn't convert it: invalid data".to_owned()),
    });
    h.frame();
    assert!(h.media().is_empty());
    assert!(h.has_label(
        "This video couldn't be converted to play here: ffmpeg couldn't convert it: invalid \
         data. Download it to watch it."
    ));

    let mut h = opened_on("/Videos/private.mov", 1000);
    h.fail::<WebCopy>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
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
    let mut h = opened_on("/program.exe", 500);
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
    assert_eq!(kind_of("/a/b.PNG"), Kind::Image);
    assert_eq!(kind_of("x.heic"), Kind::Image);
    assert_eq!(kind_of("x.mov"), Kind::Video);
    assert_eq!(kind_of("x.m4a"), Kind::Audio);
    assert_eq!(kind_of("x.pdf"), Kind::Pdf);
    assert_eq!(kind_of("x.md"), Kind::Other);
    assert_eq!(
        kind_of("/folder.jpg/README"),
        Kind::Other,
        "the folder's name doesn't count"
    );
    assert_eq!(kind_of("Makefile"), Kind::Other);
    assert!(needs_web_copy("/a/b.MOV"));
    assert!(needs_web_copy("/a/b.m4a"));
    assert!(!needs_web_copy("/a.mov/b.mp4"));
    assert!(!needs_web_copy("/a/b.jpg"));
    assert_eq!(shown_url("/a/b.jpg"), "/files/a/b.jpg?inline=1");
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

#[test]
fn a_pictures_details_are_asked_for_only_when_opened() {
    let mut h = opened_on("/Photos/beach.jpg", 3 << 20);
    assert!(h.has_button(&details(false)));
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

    h.click(&details(true));
    h.frame();
    h.frame();
    assert!(!h.app.details);
    assert!(!h.has_label("Apple iPhone 15 Pro"));
}

#[test]
fn a_picture_with_no_camera_details_says_so() {
    let mut h = opened_on("/shot.png", 10);
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

    let mut h = opened_on("/shot.png", 10);
    h.app.details = true;
    h.frame();
    h.fail::<ImageInfo>(rpc_error::DENIED);
    h.frame();
    assert!(h.has_label("permission denied"));
}

#[test]
fn only_pictures_have_details() {
    let h = opened_on("/v/clip.mp4", 10);
    assert!(!h.has_button(&details(false)));
}
