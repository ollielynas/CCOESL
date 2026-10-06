# Viewer

Look at almost any file on the server without changing it: pictures, music, videos, PDFs and
anything written in plain text.

## Opening a file

There are three ways in:

- **From Files.** Right-click a file and choose **Open with Viewer**. It opens in a new window.
- **From a link someone shared.** It opens the Viewer on its own in a browser tab, already
  showing the file. If you aren't signed in, you sign in first and then land on the file.
- **From the app menu.** The Viewer starts with a search box. Type at least two letters of a
  file's name, or some words from inside it, and click a result to open it.

Each Viewer window shows the one file it was opened on, and stays on it, and is called by its
name. To look at another file, open it in a new window: from Files, or from the app menu.

You only ever find and open files you are allowed to read.

## What you see

- **Pictures** fill the window, and nothing else is in it. The window opens in the picture's
  shape: as big as the picture, or smaller if that wouldn't fit on your screen, and never so
  small or thin you can't use it. Resize it as you like; the picture keeps its shape. Pictures
  that move (animated GIFs, and animated PNG, WebP and AVIF) play.

  **Right-click the picture** for everything else: its name and folder, **Download**, **Share**
  and **ⓘ Details**. Details opens a small window showing what the picture says about itself:
  its size in pixels and the file's size, and, for a photo, what the camera recorded, such as the
  camera and lens, when it was taken, the exposure, aperture and ISO, and where it was taken (as
  latitude and longitude) if the camera saved that. **Close** puts it away. A screenshot or a
  downloaded picture usually has no camera details, and says so.
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

For a picture these are in its right-click menu; for anything else, at the top of the window.

- **Download** saves the file to your computer, exactly as it is, even when what you see is a
  converted copy (see below).
- **Share** copies a link that opens this file in the Viewer. Anyone who follows it has to sign
  in, and sees the file only if they are allowed to read it, so sharing a link never lets in
  someone who couldn't already see the file. A message at the bottom of the screen shows the
  link it copied. If your browser won't let it copy, the message shows the link so you can copy
  it yourself.

## Which files can be shown

✅ shown in every browser, ⚠️ shown only in some browsers (it says which), ❌ not shown yet:
download it instead.

The Viewer goes by what is really in a file, not only its name, so a photo saved without an
extension, or a video with the wrong one, is still shown as what it is.

### Pictures

| Kind | Extensions | Shown |
|---|---|---|
| JPEG | `.jpg` `.jpeg` | ✅ Turned the right way up, as the camera recorded |
| PNG, including animated | `.png` `.apng` | ✅ |
| GIF, including animated | `.gif` | ✅ |
| WebP, including animated | `.webp` | ✅ |
| AVIF, including animated | `.avif` | ✅ |
| Bitmap | `.bmp` | ✅ |
| Icon and cursor | `.ico` `.cur` | ✅ |
| Apple photos (HEIC/HEIF) | `.heic` `.heif` | ✅ converted |
| TIFF, including scans and fax pages | `.tif` `.tiff` | ✅ converted; the first page of a multi-page file |
| SVG drawing | `.svg` | ✅ drawn on the server (see below) |
| Camera RAW (DNG) | `.dng` | ✅ converted |
| Other camera RAW | `.cr2` `.cr3` `.nef` `.arw` `.orf` `.rw2` `.raf` | ⚠️ converted when the server can read that camera's files |
| Photoshop | `.psd` | ✅ converted, as the whole picture (not layer by layer) |
| JPEG XL | `.jxl` | ✅ converted |
| JPEG 2000 | `.jp2` `.j2k` | ✅ converted |
| HDR and OpenEXR | `.exr` `.hdr` `.pfm` | ✅ converted; very bright parts are shown as white |
| Older formats | `.tga` `.pcx` `.sgi` `.qoi` `.dds` `.ppm` `.pgm` `.pbm` `.pam` `.xpm` `.wbmp` | ✅ converted |

### Video

| Kind | Extensions | Shown |
|---|---|---|
| MP4 (H.264) | `.mp4` `.m4v` | ✅ |
| WebM | `.webm` | ✅ |
| QuickTime, including iPhone (HEVC) and ProRes videos | `.mov` | ✅ converted |
| MP4 with HEVC/H.265, 10-bit or HDR video | `.mp4` | ✅ converted; HDR is shown in ordinary brightness |
| Matroska | `.mkv` | ✅ converted |
| Ogg video | `.ogv` | ✅ converted |
| AVI, including DivX and Xvid | `.avi` | ✅ converted |
| Windows Media | `.wmv` `.asf` | ✅ converted |
| Flash video | `.flv` `.swf` | ✅ converted |
| Mobile video | `.3gp` | ✅ converted |
| MPEG-1, MPEG-2 and DVD | `.mpg` `.mpeg` `.vob` `.ts` `.m2ts` | ✅ converted |
| Professional | `.mxf` `.dv` | ✅ converted |
| RealMedia | `.rm` | ✅ converted |
| Raw video streams | `.h264` `.hevc` `.obu` `.ivf` `.y4m` | ✅ converted |

### Music and sound

| Kind | Extensions | Shown |
|---|---|---|
| MP3 | `.mp3` | ✅ |
| AAC | `.m4a` `.aac` | ✅ |
| WAV | `.wav` | ✅ |
| FLAC | `.flac` | ✅ |
| Ogg Vorbis and Opus | `.ogg` `.oga` `.opus` | ✅ |
| Apple Lossless (ALAC) | `.m4a` | ✅ converted |
| Core Audio | `.caf` | ✅ converted |
| AIFF | `.aif` `.aiff` | ✅ converted |
| Wave64 | `.w64` | ✅ converted |
| Windows Media Audio | `.wma` | ✅ converted |
| Dolby Digital | `.ac3` `.eac3` | ✅ converted |
| MPEG audio layer 2 | `.mp2` | ✅ converted |
| MIDI | `.mid` `.midi` | ❌ It's notes, not sound |

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

### Converted files

Browsers can show only some kinds of picture, video and sound. For everything else (the rows
above that say *converted*) the server makes a copy that every browser can show: a picture
becomes WebP, a video becomes an MP4, a recording becomes AAC. The Viewer shows the copy. The
file itself isn't changed, and **Download** still gives you the original.

The first time a video is opened it has to be converted, which takes a while for a long one. The
Viewer says how far along it is, and plays it when it's ready. After that, and for anyone else
who opens the same video, it plays straight away. A picture or recording takes a moment the
first time. If the file changes, it's converted again the next time it's opened.

An SVG drawing is drawn into a picture on the server rather than opened by your browser, so
nothing in it runs: an SVG can hold scripts and links, and pictures from other places, and the
Viewer shows none of them, only the drawing itself.

A file that is damaged, or only pretends to be a picture or video, says it can't be shown. If it
is really text, it is shown as text.
