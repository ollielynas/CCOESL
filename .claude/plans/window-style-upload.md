# Plan: Window Style Overhaul + Shared Upload SDK Feature

## Context

Two parallel tasks:
1. **Window styling** — The current desktop uses muted dark colors (#1a1f2b → #101218 wallpaper, subtle widget styling). The user wants brighter, higher-contrast, more modern visuals.
2. **Upload feature** — The rust-compiler app only works with directories already on the server. The user wants to upload folders from their local machine, with a progress bar. This should be a shared SDK feature usable by any app, supporting both file picker and drag-and-drop.

---

## Part 1: Window Style Overhaul

All changes are in `crates/ccosel-shell/src/desktop.rs` (the single styling entry point) plus `web/index.html` for the loader.

### 1.1 Wallpaper — brighter, more saturated gradient

**File:** `crates/ccosel-shell/src/desktop.rs` — `wallpaper()` (line 186)

Current dark mode: `#1a1f2b` → `#101218` (very muted navy).  
New dark mode: richer, more saturated gradient with a subtle color shift.

```
Top:    #1e293b (slate-800)  →  Bottom: #0f172a (slate-900)
```

Light mode: `#e2e8f0` → `#cbd5e1` (slate-200 → slate-300).

### 1.2 Global style — brighter accents, better contrast

**File:** `crates/ccosel-shell/src/desktop.rs` — `apply_style()` (line 326)

- Increase `item_spacing` from `(8, 8)` to `(8, 8)` (keep)
- Increase `button_padding` from `(10, 6)` to `(12, 7)` for a more substantial feel
- Window margin from `10` to `12`
- Window corner radius from `10` to `12` (slightly more modern)
- Widget corner radius from `6` to `8`

Add to `apply_style()`:
- Set `visuals.widgets.inactive.weak_bg_fill` to a subtle tinted background (`#1e293b`)
- Set `visuals.widgets.hovered.weak_bg_fill` to a brighter hover state (`#334155`)
- Set `visuals.widgets.active.weak_bg_fill` to an even brighter active state (`#475569`)
- Set `visuals.widgets.inactive.bg_stroke` to a subtle border (`#334155`, 1px)
- Set `visuals.widgets.hovered.bg_stroke` to a brighter border (`#60a5fa`, 1px) — blue accent on hover
- Set `visuals.widgets.active.bg_stroke` to a strong border (`#3b82f6`, 1px)
- Set `visuals.selection` to a vibrant blue (`#3b82f6`)
- Set `visuals.warn_fg_color` to amber (`#f59e0b`)
- Set `visuals.error_fg_color` to red (`#ef4444`)
- Set `visuals.extreme_bg_color` to near-black (`#020617`)
- Set `visuals.faint_bg_color` to very subtle (`#1e293b`)

### 1.3 App badge colors — more vibrant

**File:** `crates/ccosel-shell/src/registry.rs` — `catalog()` (line 21)

Current colors are already reasonable but can be punchier:
- Files: `#4c8bf5` → `#3b82f6` (blue-500, cleaner)
- Clock: `#f2a13a` → `#f59e0b` (amber-500)
- Rust Compiler: `#ce422b` → `#ef4444` (red-500)

### 1.4 Taskbar — enhanced visual treatment

**File:** `crates/ccosel-shell/src/desktop.rs` — `taskbar()` (line 226)

- Add a subtle top border/stroke to the taskbar panel for separation from windows
- Slightly increase height from `40.0` to `44.0` for a more modern feel
- The "Apps" menu button can use a brighter background

### 1.5 Window rendering — add subtle shadow/border

**File:** `crates/ccosel-shell/src/desktop.rs` — `ui()` method (line 134)

Wrap each `egui::Window` with:
- `.shadow(egui::epaint::Shadow { offset: vec2(0, 4), blur: 12, spread: 0, color: Color32::from_black_alpha(40) })`
- This gives windows a floating, elevated appearance

### 1.6 HTML loader — match the new palette

**File:** `web/index.html`

- Update `background: #14161a` → `#0f172a` (matches wallpaper bottom)
- Update progress bar fill from `#5b9dd9` → `#3b82f6` (matching accent blue)
- Update button border from `#3a4150` → `#475569`
- Update button background from `#232833` → `#1e293b`

---

## Part 2: Shared Upload SDK Feature

### Architecture

The upload feature bypasses the normal RPC pipeline because file data cannot travel through the `no_std` guest wasm. Instead:

1. **App** calls `ui.upload_folder()` which emits an `UploadFolder` opcode
2. **Shell** (host-side) intercepts the opcode, opens a native file picker (`<input webkitdirectory>`) and/or listens for drag-and-drop on the canvas
3. **Shell** uploads files to `POST /upload` with progress tracking
4. **Shell** sends an event back to the guest with the server-side path of the uploaded directory
5. **App** reads the path from the response and navigates to it

Drag-and-drop is handled entirely by the shell's JS/wasm-bindgen layer — the app doesn't need to know whether the user used a picker or dropped files.

### 2.1 ABI — new opcode

**File:** `crates/ccosel-abi/src/opcode.rs`

Add `UploadFolder = 0x0C` to `OpCode`.

**File:** `crates/ccosel-abi/src/decode.rs`

Add decoding for `UploadFolder { id }`.

**File:** `crates/ccosel-abi/src/encode.rs`

Add encoding for `UploadFolder`.

### 2.2 Host replay — handle the opcode

**File:** `crates/ccosel-host/src/replay.rs`

When `Cmd::UploadFolder { id }` is encountered, record the id in a pending-uploads list that the `AppInstance` can read. The shell's web backend translates this into a DOM file picker interaction.

### 2.3 SDK — new `upload_folder()` method

**File:** `crates/ccosel-sdk/src/ui.rs`

Add:
```rust
pub fn upload_folder(&mut self) -> Response {
    let id = self.auto_id();
    self.rec.push(&Cmd::UploadFolder { id });
    self.response(id)
}
```

The `Response` indicates whether an upload completed this frame (the shell delivers the result as a response record).

### 2.4 Shell — file picker + drag-and-drop handling

**File:** `crates/ccosel-shell/src/lib.rs` (or a new `upload.rs`)

The shell maintains upload state per-window:
- `pending_upload: Option<u64>` — the widget id waiting for a result
- `upload_progress: Option<UploadProgress>` — bytes sent / total

When the replay encounters `UploadFolder`:
1. Store the widget id
2. On the next frame, trigger a hidden `<input type="file" webkitdirectory>` via wasm-bindgen
3. When files are selected, read them into `Vec<u8>` chunks
4. POST each chunk to `POST /upload?path=<server_path>&index=<i>&total=<n>`
5. Track progress and feed it back as a response record

For drag-and-drop:
- Register a `dragover`/`drop` event listener on the canvas element
- On drop, extract the `DataTransferItemList`, read entries recursively
- Feed the same upload pipeline

### 2.5 Server — upload endpoint

**File:** `crates/ccosel-server/src/lib.rs`

Add `POST /upload` route:
```rust
.route("/upload", post(upload))
```

**File:** `crates/ccosel-server/src/upload_api.rs` (new)

```rust
pub async fn upload(
    State(state): State<AppState>,
    Query(params): Query<UploadParams>,
    body: Bytes,
) -> Response
```

- `UploadParams`: `path: String` (jail-relative destination), `filename: String`, `index: u32`, `total: u32`
- First chunk (`index == 0`): create the directory structure
- Each chunk: write bytes to `<jail>/<path>/<filename>`
- Last chunk (`index == total - 1`): return success with the server path
- Validate all paths stay within the jail (same as `fs_api::Jail::resolve`)

**File:** `crates/ccosel-server/Cargo.toml`

Add ` multer ` or use raw `Bytes` for multipart handling. Actually, since we're sending raw chunks (not multipart), `Bytes` from axum is sufficient.

### 2.6 Protocol — upload types

**File:** `crates/ccosel-proto/src/lib.rs`

No new `Method` needed — upload goes over HTTP `POST /upload`, not through the RPC batch. The upload is a direct HTTP endpoint, similar to how downloads work over `GET /files`.

### 2.7 Rust Compiler app — integrate upload

**File:** `apps/rust-compiler/src/lib.rs`

Add an "Upload Folder" button:
```rust
if ui.button("Upload Folder").clicked() {
    upload = true;
}
ui.tooltip("Upload a Rust project from your computer");

if upload {
    ui.upload_folder();
}
```

After upload completes, navigate to the uploaded path and show the directory listing.

### 2.8 File Browser app — optional integration

**File:** `apps/file-browser/src/lib.rs`

Could also add upload support, but is lower priority. The rust-compiler is the primary use case.

---

## Implementation Order

### Phase 1: Style overhaul (no new opcodes, no server changes)
1. Update `apply_style()` in `desktop.rs`
2. Update `wallpaper()` colors in `desktop.rs`
3. Update app badge colors in `registry.rs`
4. Update `taskbar()` in `desktop.rs`
5. Add window shadows in `desktop.rs`
6. Update `web/index.html` loader colors

### Phase 2: Upload infrastructure
7. Add `UploadFolder` opcode to `ccosel-abi`
8. Add `upload_folder()` to SDK `Ui`
9. Add upload endpoint to server (`upload_api.rs`)
10. Add upload route to server router

### Phase 3: Shell upload handling
11. Add file picker JS interop (wasm-bindgen + web-sys)
12. Add drag-and-drop event listener on canvas
13. Implement chunked upload logic in shell
14. Wire upload progress back to app via response records

### Phase 4: App integration
15. Update rust-compiler app with upload button
16. Test end-to-end: upload → directory listing → build → download

---

## Files to modify

| File | Change |
|------|--------|
| `crates/ccosel-shell/src/desktop.rs` | Style overhaul: colors, shadows, taskbar |
| `crates/ccosel-shell/src/registry.rs` | Brighter app badge colors |
| `web/index.html` | Loader palette update |
| `crates/ccosel-abi/src/opcode.rs` | Add `UploadFolder` opcode |
| `crates/ccosel-abi/src/decode.rs` | Decode `UploadFolder` |
| `crates/ccosel-abi/src/encode.rs` | Encode `UploadFolder` |
| `crates/ccosel-sdk/src/ui.rs` | Add `upload_folder()` method |
| `crates/ccosel-host/src/replay.rs` | Handle `UploadFolder` opcode |
| `crates/ccosel-server/src/lib.rs` | Add `/upload` route |
| `crates/ccosel-server/src/upload_api.rs` | **New** — upload endpoint |
| `crates/ccosel-server/Cargo.toml` | May need `multer` for multipart |
| `apps/rust-compiler/src/lib.rs` | Add upload button |

## Verification

1. `cargo test` — all existing tests pass (32 native tests)
2. `cargo test -p ccosel-host-web --target wasm32-unknown-unknown` — 4 wasm tests pass
3. `cargo xtask build-web` — builds successfully, apps stay under 100KB gzipped
4. Manual: `cargo xtask serve`, open in browser, verify:
   - Brighter, more vibrant wallpaper and widget styling
   - Window shadows give depth
   - Taskbar has better separation
   - Upload button in rust-compiler opens file picker
   - Drag-and-drop a folder onto the rust-compiler window
   - Progress bar shows upload progress
   - Uploaded directory appears in the file browser
   - Build succeeds after upload
