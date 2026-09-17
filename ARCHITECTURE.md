# CCOSEL architecture

A browser-hosted desktop environment served from one machine on a LAN. The shell (Rust + egui,
compiled to wasm) renders a windowed desktop in the browser. Apps ship as *separate* `.wasm`
modules, fetched lazily and evicted when memory is tight. Heavy work goes to the server.

Two constraints shape every decision below:

1. **An app module has its own linear memory and cannot touch the shell's `egui::Ui`.**
2. **The LAN is unreliable and slow.** Bytes on the wire are the scarce resource.

## Target layout

```
crates/
  ccosel-abi/             the guest<->shell contract. no_std, bytemuck only.   [BUILT]
  ccosel-sdk/             guest side: Ui facade, encoder, response lookup     [BUILT]
  ccosel-host/            decode + replay into egui::Ui. NO wasm runtime dep. [BUILT]
  ccosel-host-wasmtime/   native/server backend                               [BUILT]
  ccosel-proto/           client<->server RPC types (serde/postcard)
  ccosel-host-web/        wasm-bindgen + web-sys backend
  ccosel-transport/       WS codec, pending-call table, reconnect/resume, coalescing
  ccosel-cas/             content-defined chunking + hashing (client and server)
  ccosel-shell/           window manager, taskbar, launcher, app lifecycle
  ccosel-server/          axum binary
apps/                     SEPARATE cargo workspace
  file-browser/  compiler/
web/  docker/  xtask/
```

Three structural choices worth not undoing:

**`apps/` is its own workspace.** `[profile]` is workspace-global. Guests need `opt-level="z"`,
`lto="fat"`, `panic="abort"`; the shell wants `opt-level=3`. They cannot share a workspace.

**`ccosel-host` depends on neither wasm runtime.** Decode-and-replay is the hardest code here;
keeping it platform-free makes it testable against a byte array and an offscreen `egui::Context`,
with no wasm in the loop.

**`ccosel-shell` builds native and web.** Native (`eframe` + wasmtime) is the dev loop: real
debugger, one-second iteration. Web is the product.

## How apps render

Apps don't call egui. They *record* egui-shaped calls into a flat command buffer, which the
shell replays into a real `egui::Ui`. `ui.horizontal(|ui| ..)` becomes `BeginScope` / commands /
`EndScope`, and the host walks the stream with a `Vec<Ui>` — mechanically what `Ui::scope_dyn`
already does internally.

Apps are **strictly batch-only**: a frame is emitted in full, with no blocking call back into
the shell. That costs a one-frame delay on widget responses and buys two things: apps can move
to Web Workers later (so a runaway app can't wedge the desktop), and the shell can re-tessellate
a *cached* command buffer at display rate while only re-running guests that need it.

**The one-frame delay is invisible because the shell owns all egui state** — hover, focus, drag,
scroll offsets, text buffers, window geometry, textures. The guest owns only its model. It is
the shell that draws the interaction, so nothing the user sees is ever stale; the guest merely
learns the committed result a frame later. *A guest storing a scroll offset or a window rect is
a bug.*

Consequences worth knowing before writing an app:
- Tooltips are declarative (`Tooltip { id, text }` emitted unconditionally). Never branch on
  `hovered()` in order to draw.
- Sliders: the shell owns the live value during a drag; the guest sees the committed one.
- Text edit uses a delta protocol — the buffer is not resent each frame.
- `available_width` is last frame's. Use the shell-side responsive helpers, or add hysteresis,
  or layout will oscillate.

## Keeping modules small — the whole thesis

A guest must **not** link egui. The SDK is a recorder, not a renderer, and value types
(`Vec2`, `Rect`, `Color32`) are copied into `ccosel-abi` rather than pulled from `emath`.

| | raw | brotli |
|---|---|---|
| The shell (egui + eframe + glow) | 3.5–6 MB | 1.0–1.6 MB |
| An app that linked its own eframe | 2.5–5 MB | 0.8–1.4 MB |
| **SDK-only guest, target** | **≤250 KB** | **≤80 KB** |

That gap is why the command stream exists. **Any guest dependency over ~200 KB of wasm becomes
a server RPC instead** — syntax highlighting runs on the server and returns
`Vec<(range, style_id)>`. The project's own thesis, applied to its dependency graph.

Corollary: the shell is now the largest single download, so it gets a hand-written loader with a
progress bar and resume-on-failure, not a framework default page.

## Network

Two WebSockets (`/ws/control` for RPC, `/ws/events` for push) because a single multiplexed
socket head-of-line blocks — one retransmit on a lossy LAN would stall keystroke acks behind a
diagnostics dump. Bulk transfer goes over HTTP regardless.

- Everything content-addressed under `/cas/{blake3}` and served `immutable`. `/manifest` is the
  only URL ever revalidated.
- **IndexedDB module cache**, not the Cache API: `http://192.168.x.x` is not a secure context,
  so service workers and the Cache API are unavailable. IndexedDB works on plain HTTP and can
  hold compiled `WebAssembly.Module` objects where structured-clone allows. Net effect: a module
  version is fetched once, ever. HTTPS is a later upgrade, not a prerequisite.
- postcard on the wire; brotli precompressed at build time; RPC coalesced in an 8–16 ms window.
- Server push is rate-limited and lossy. Compile progress is last-value-wins at ≤4 Hz; full logs
  are fetched over HTTP on demand. Streaming raw build output line-by-line over a bad LAN is the
  fastest way to make this feel broken.
- Upload is chunked, content-hashed and deduplicated: send the hash list, server replies with
  what it lacks. Re-uploading a project after a one-file edit transfers one chunk.
- **Anything over ~2s is a Job, not an RPC.** `POST /jobs` with an idempotency key, progress
  events carrying a monotonic `seq`, resume-from-cursor on reconnect. One mechanism covers both
  "socket dropped mid-compile" and "app evicted mid-compile".
- App-level heartbeat (15s/45s). Browser JS cannot observe WebSocket ping/pong, so without it
  half-open TCP connections go undetected.

## App lifecycle

wasm linear memory **cannot shrink**. A guest that once allocated 300 MB holds it forever, so
"unload" can only mean dropping the whole instance and restoring from a serialized `State`.
Do not attempt whole-linear-memory snapshotting — it requires restoring `__stack_pointer`,
mutable globals and table entries, and is too fragile to rely on.

The **shell** owns the pending-RPC table, not the guest, so a call survives its caller being
evicted; results are delivered via `ccosel_on_event` in order before the next frame.

## Replay, concretely

`validate` proves the buffer is balanced and within the depth limit **before** anything is
drawn. That up-front pass is what lets the renderer index the scope tree and then recurse
through egui's ordinary closure containers (`ui.horizontal(|ui| ..)`) instead of hand-managing
a `Vec<Ui>` stack — there is no host stack to corrupt, and a bad frame is discarded whole with
the previous frame left standing.

Tooltips are resolved in a pre-pass and attached on the way past, which is why they cost
nothing despite responses being a frame stale.

## Status

- [x] `ccosel-abi` v0 — opcodes, frame structs, codec, validator, hostile-input tests
- [x] `ccosel-sdk` — `Ui` facade, recorder, `Text`; native harness asserts the byte stream
- [x] `ccosel-host` — decode + replay into `egui::Ui`, tested against an offscreen context
- [x] `cargo run -p ccosel-host --example replay_demo` — SDK -> bytes -> egui, natively
- [x] `ccosel-host-wasmtime` — real modules over a real memory boundary, response write-back
- [x] File Browser v0 as an actual `.wasm` guest: **27.7 KB raw, 11.8 KB gzipped**
- [ ] `ccosel-server` skeleton, `ccosel-transport`
- [ ] `ccosel-host-web`, then CAS upload/download, then `xtask`

## Running it

```
cargo test                                            # 28 tests, both workspaces
cargo run -p ccosel-host-wasmtime --example dev_shell  # the native dev loop
cd apps && cargo build --release --target wasm32-unknown-unknown
```

`dev_shell` loads the same `.wasm` the browser will fetch — no `wasm-bindgen`, no JS glue — so
a bug reproduced there is the bug that would happen in the browser, with a real debugger.

Build order and rationale: see the plan at `~/.claude/plans/read-through-teh-readme-mellow-pond.md`.
