# Viewer

Look at almost any file on the server without changing it: pictures, music, videos, PDFs and
anything written in plain text.

## Opening a file

There are three ways in:

- **From Files.** Right-click a file and choose **Open with Viewer**. It opens in a new window.
- **From a link someone shared.** It opens the Viewer on its own in a browser tab, already
  showing the file. If you aren't signed in, you sign in first and then land on the file.
- **From the app menu.** The Viewer starts with a search box. Type at least two letters of a
  file's name, or some words from inside it, and click a result to open it. **← Back** returns
  to your results.

Opened on a file, **🔍 Search** at the top takes you to the search instead.

You only ever find and open files you are allowed to read.

## What you see

- **Pictures** fit the width of the window.
- **Music and videos** play in your browser's own player, with its usual controls: play, pause,
  seek, volume and full screen. Music keeps playing if another window covers the player.
- **PDFs** open in your browser's own PDF viewer, where you can scroll, zoom and search.
- **Spreadsheet data** (`.csv` and `.tsv` files) is shown as a table, with the first row as the
  column names. Columns line up, and a wide table scrolls sideways. Only the first 500 rows are
  shown; the full file is in **Show as text** and in the download. **Show as text** shows the file
  exactly as it is written, and **Show as a table** goes back.
- **Text files** are shown as they are, in a fixed-width font. You can select and copy from them,
  but not change them. Files over 1 MB are too big to show; download them instead.

Anything else says it can't be shown here, and you can still download it.

## Download and Share

- **Download** saves the file to your computer.
- **Share** copies a link that opens this file in the Viewer. Anyone who follows it has to sign
  in, and sees the file only if they are allowed to read it, so sharing a link never lets in
  someone who couldn't already see the file. A message at the bottom of the screen shows the
  link it copied. If your browser won't let it copy, the message shows the link so you can copy
  it yourself.

## Which files can be shown

✅ shown in every browser, ⚠️ shown only in some browsers (it says which), ❌ not shown yet:
download it instead.

What your browser can play decides most of this, so the same file can work on one computer and
not on another.

### Pictures

| Kind | Extensions | Shown |
|---|---|---|
| JPEG | `.jpg` `.jpeg` | ✅ |
| PNG | `.png` | ✅ |
| GIF | `.gif` | ✅ The first frame only: animations don't move |
| WebP | `.webp` | ✅ |
| AVIF | `.avif` | ✅ In current browsers |
| Bitmap | `.bmp` | ✅ |
| Icon | `.ico` | ✅ |
| Apple photos (HEIC/HEIF) | `.heic` `.heif` | ⚠️ Safari only |
| TIFF | `.tif` `.tiff` | ⚠️ Safari only |
| SVG drawing | `.svg` | ⚠️ Shown as its text, not as a picture |
| Camera RAW | `.cr2` `.cr3` `.nef` `.arw` `.dng` `.raf` `.orf` `.rw2` | ❌ |
| Photoshop | `.psd` | ❌ |
| JPEG XL | `.jxl` | ❌ |

### Video

| Kind | Extensions | Shown |
|---|---|---|
| MP4 (H.264) | `.mp4` `.m4v` | ✅ |
| WebM | `.webm` | ✅ |
| QuickTime | `.mov` | ⚠️ Plays where the video inside is H.264; iPhone (HEVC) and ProRes videos in Safari only |
| MP4 with HEVC/H.265 | `.mp4` | ⚠️ Safari, and some other browsers on some computers |
| Ogg video | `.ogv` | ⚠️ Not in Safari |
| Matroska | `.mkv` | ⚠️ Mostly Chrome and Edge |
| AVI | `.avi` | ❌ |
| Windows Media | `.wmv` `.asf` | ❌ |
| Flash video | `.flv` | ❌ |
| Mobile video | `.3gp` | ❌ |
| MPEG-2 | `.mpg` `.mpeg` `.ts` `.m2ts` | ❌ |

### Music and sound

| Kind | Extensions | Shown |
|---|---|---|
| MP3 | `.mp3` | ✅ |
| AAC | `.m4a` `.aac` | ✅ |
| WAV | `.wav` | ✅ |
| FLAC | `.flac` | ✅ |
| Ogg Vorbis | `.ogg` `.oga` | ✅ In current browsers |
| Opus | `.opus` | ✅ In current browsers |
| Apple Lossless (ALAC) | `.m4a` | ⚠️ Safari only |
| Core Audio | `.caf` | ⚠️ Safari only |
| AIFF | `.aif` `.aiff` | ⚠️ Safari only |
| Windows Media Audio | `.wma` | ❌ |
| MIDI | `.mid` `.midi` | ❌ |
| AMR (voice memos from older phones) | `.amr` | ❌ |

### Documents

| Kind | Extensions | Shown |
|---|---|---|
| PDF | `.pdf` | ✅ |
| Spreadsheet data | `.csv` `.tsv` | ✅ As a table, or as its text (up to 1 MB, first 500 rows) |
| Word | `.doc` `.docx` | ❌ |
| Excel | `.xls` `.xlsx` | ❌ |
| PowerPoint | `.ppt` `.pptx` | ❌ |
| OpenDocument | `.odt` `.ods` `.odp` | ❌ |
| Apple Pages, Numbers, Keynote | `.pages` `.numbers` `.key` | ❌ |
| Rich text | `.rtf` | ⚠️ Shown as its raw text, with the formatting codes |
| E-book | `.epub` `.mobi` | ❌ |

### Text and code

Shown as text, up to 1 MB:

| Kind | Extensions | Shown |
|---|---|---|
| Plain text and logs | `.txt` `.log` | ✅ |
| Markdown | `.md` | ✅ As its text; open it in Docs to see it formatted |
| Data and settings | `.json` `.yaml` `.yml` `.toml` `.xml` `.ini` `.conf` `.cfg` `.env` | ✅ |
| Web pages | `.html` `.htm` `.css` | ✅ As their source, never run |
| Program code | `.rs` `.py` `.js` `.ts` `.c` `.h` `.cpp` `.java` `.go` `.rb` `.php` `.swift` `.kt` `.cs` `.lua` `.sql` `.sh` and the like | ✅ No colouring yet |
| Files with no extension | `README` `Makefile` `Dockerfile` … | ✅ If they are text |
| Subtitles | `.srt` `.vtt` | ✅ As their text |
| 3D models in text form | `.obj` `.gltf` | ✅ As their text, not as a model |

### Everything else

| Kind | Extensions | Shown |
|---|---|---|
| Archives | `.zip` `.tar` `.gz` `.tgz` `.7z` `.rar` `.xz` `.bz2` | ❌ |
| Fonts | `.ttf` `.otf` `.woff` `.woff2` | ❌ |
| Disk images and installers | `.iso` `.dmg` `.exe` `.msi` `.deb` `.apk` | ❌ |
| 3D models (binary) | `.stl` `.glb` `.fbx` `.blend` | ❌ |
| Databases | `.sqlite` `.db` | ❌ |
| Any other file that isn't text | | ❌ |

Apple formats that only Safari shows today are planned to be converted on the server so that
every browser can show them.
