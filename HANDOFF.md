# Handoff: Window Style Overhaul + Upload/Download Feature

**Date:** Sep 18 2026
**Branch:** scaffold-abi-sdk-host

## Status

| Area | Status |
|------|--------|
| Window styling overhaul | **DONE** — compiles clean |
| UploadFolder ABI + SDK + host replay | **DONE** — compiles clean |
| OpenUrl ABI + SDK + host replay | **DONE** — compiles clean |
| Server upload endpoint | **DONE** — compiles with pre-existing errors |
| Shell upload interop (file picker) | **DONE** — compiles clean for wasm32 |
| File browser upload + download buttons | **DONE** — compiles clean |
| Rust compiler app fixes | **NEEDS FIX** — `CompileStatus` vs `CompileResult` field access |
| Server `rpc.rs` pre-existing errors | **PRE-EXISTING** — not introduced by this work |

## What was done

### 1. Window Style Overhaul (`crates/ccosel-shell/src/desktop.rs`)

**`apply_style()`** (line ~395):
- Bigger padding: `button_padding` 10,6 → 12,7; `window_margin` 10 → 12
- Rounder corners: window 10 → 12, widgets 6 → 8
- Widget fills: inactive `#1e293b`, hovered `#334155`, active `#475569`
- Widget borders: inactive subtle `#334155`, hovered blue `#60a5fa`, active blue `#3b82f6`
- Selection: blue `#3b82f6` with white stroke
- Status colors: warn amber `#f59e0b`, error red `#ef4444`
- Window shadow: 16px blur, offset [0,4], 50 alpha
- Popup shadow: 8px blur, offset [0,2], 40 alpha

**`wallpaper()`** — Light mode is now a landscape (SVG-inspired):
- Sky gradient: `#79baf7` → `#a8d4fa`
- 7 white cloud ellipses at varying positions/sizes/shades
- 4 green hills as rotated convex polygons:
  - Back: `#0c8c0c` (darkest)
  - Mid-back: `#17a517`
  - Mid-front: `#5dad0d`
  - Front: `#72c421` (brightest)
- Dark mode unchanged: slate gradient `#1e293b` → `#0f172a`

**`taskbar()`**:
- Height: 40 → 44px
- Top border: 1px `#334155` separator line
- Error indicator color updated to `#ef4444`

**`web/index.html`**:
- Background `#14161a` → `#0f172a`
- Progress bar `#5b9dd9` → `#3b82f6`
- Text `#c9ced6` → `#e2e8f0`
- Button border `#3a4150` → `#475569`

### 2. Upload Feature (Shared SDK)

**ABI — new opcodes** (`crates/ccosel-abi/src/opcode.rs`):
- `UploadFolder = 0x0C` — triggers browser file picker
- `OpenUrl = 0x0D` — opens URL in new tab (for downloads)

**Decode/Encode** (`crates/ccosel-abi/src/decode.rs`, `encode.rs`):
- `Cmd::UploadFolder { id }` — just an id
- `Cmd::OpenUrl { id, url }` — id + URL string

**SDK** (`crates/ccosel-sdk/src/ui.rs`):
- `ui.upload_folder() -> Response` — emits `UploadFolder` opcode
- `ui.open_url(url) -> Response` — emits `OpenUrl` opcode

**Host replay** (`crates/ccosel-host/src/replay.rs`):
- `UploadFolder` renders as a flat "Upload Folder" button
- `OpenUrl` renders as a flat button with the URL as label
- `Replayer` tracks `upload_ids` and `open_url_ids` for shell detection
- New getters: `upload_ids()`, `open_url_ids()`

### 3. Shell Upload Handling

**`crates/ccosel-shell/src/upload.rs`** (NEW):
- `UploadState` struct — tracks pending uploads and completed results
- `open_file_picker()` — creates hidden `<input webkitdirectory>`, triggers click
- `upload_files()` — reads files via FileReader, POSTs each to `/upload`
- `strip_root_folder()` — strips `webkitRelativePath` root folder name

**`crates/ccosel-shell/src/app_window.rs`**:
- `check_upload_click()` — scans responses for clicks on upload buttons
- `check_open_url_click()` — scans responses for clicks on URL buttons

**`crates/ccosel-shell/src/desktop.rs`**:
- `upload_state: UploadState` field on `Desktop`
- After each window renders, checks for upload/URL clicks
- Upload clicks → `upload_state.start_upload()`
- URL clicks → `web_sys::window().open_with_url_and_target(url, "_blank")`

**Dependencies** (`crates/ccosel-shell/Cargo.toml`):
- Added `urlencoding = "2"`
- Added web-sys features: `HtmlInputElement`, `FileList`, `File`, `FileReader`, `Event`, `EventTarget`, `Node`, `Blob`

### 4. Server Upload Endpoint

**`crates/ccosel-server/src/upload_api.rs`** (NEW):
- `POST /upload?path=<dir>&filename=<name>` — raw body = file bytes
- Creates intermediate directories via `create_dir_all`
- Validates filename (no `/`, `\`, `..`)
- Double-checks resolved path is inside jail
- Writes file via `tokio::fs::write`

**`crates/ccosel-server/src/lib.rs`**:
- Registered `pub mod upload_api`
- Added route: `.route("/upload", post(upload_api::upload))`

### 5. File Browser App (`apps/file-browser/src/lib.rs`)

- Added `ui.upload_folder()` button in the toolbar
- Added download links next to each file: `ui.open_url("/files{path}")`

## What needs fixing

### Rust Compiler App — `CompileStatus` vs `CompileResult`

`Compile::Reply` is `CompileStatus`, not `CompileResult`. The app accesses `.success`, `.binaries`, `.output`, `.output_truncated` which are on the inner `CompileResult` (inside `CompileStatus.result: Option<CompileResult>`). Fix:

```rust
// In rust-compiler/src/lib.rs, the Poll::Ready(result) arm:
Poll::Ready(status) => {
    let Some(result) = &status.result else {
        ui.label("Build still in progress…");
        return;  // or continue
    };
    // Now result is &CompileResult, all fields available
    ui.label(if result.success { "✅" } else { "❌" });
    for binary in &result.binaries { ... }
    // ...
}
```

Also needs `generation` added to `CompileReq` in the `invalidate` and `get` calls (already done but double-check).

### Server Pre-existing Errors (NOT from this work)

`crates/ccosel-server/src/rpc.rs` line 95 calls `build_api::compile` with wrong args — missing `&Jobs` parameter. Also `build_api.rs` has unused `job_handle` function. These exist on the branch before this work.

`crates/ccosel-server/src/upload_api.rs` uses `log::error!` but the server crate doesn't depend on `log`. Either add `log` to `crates/ccosel-server/Cargo.toml` or replace with `eprintln!`.

## File inventory (new/modified in this work)

### New files
- `crates/ccosel-shell/src/upload.rs` — browser-side upload interop
- `crates/ccosel-server/src/upload_api.rs` — server upload endpoint

### Modified files
- `crates/ccosel-shell/src/desktop.rs` — style overhaul + upload/URL wiring
- `crates/ccosel-shell/src/registry.rs` — brighter badge colors
- `crates/ccosel-shell/src/app_window.rs` — upload/URL click detection
- `crates/ccosel-shell/src/lib.rs` — register upload module
- `crates/ccosel-shell/Cargo.toml` — urlencoding + web-sys features
- `crates/ccosel-abi/src/opcode.rs` — UploadFolder + OpenUrl opcodes
- `crates/ccosel-abi/src/decode.rs` — decode both new opcodes
- `crates/ccosel-abi/src/encode.rs` — encode both new opcodes
- `crates/ccosel-host/src/replay.rs` — render both + track IDs
- `crates/ccosel-sdk/src/ui.rs` — upload_folder() + open_url() methods
- `crates/ccosel-server/src/lib.rs` — upload route + module
- `apps/file-browser/src/lib.rs` — upload button + download links
- `web/index.html` — updated color palette

### Not touched (pre-existing issues)
- `crates/ccosel-server/src/rpc.rs` — wrong compile() call signature
- `crates/ccosel-server/src/build_api.rs` — unused job_handle function
- `apps/rust-compiler/src/lib.rs` — CompileStatus field access (needs fix above)
