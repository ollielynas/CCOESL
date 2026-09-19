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
  ccosel-host-wasmtime/   dev/test backend only, never ships                  [BUILT]
  ccosel-host-web/        browser backend - THE shipping path                  [BUILT]
  ccosel-shell/           desktop: wallpaper, windows, taskbar, launcher       [BUILT]
  xtask/                  build pipeline + size budgets                        [BUILT]
  ccosel-proto/           client<->server RPC types (serde/postcard)          [BUILT]
  ccosel-transport/       pending-call table, coalescing, deadlines            [BUILT]
  ccosel-server/          axum: hosts the shell and answers /rpc               [BUILT]
  ccosel-cas/             content-defined chunking + hashing (client and server)
apps/                     SEPARATE cargo workspace
  file-browser/  clock/
web/      hand-written loader page
docker/
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
to Web Workers later (so a runaway app can't wedge the desktop).

It is *intended* to also let the shell re-tessellate a cached command buffer at display rate
while only re-running guests that need it. **That gate is not implemented yet** — see
`AppWindow::ui` in `crates/ccosel-shell/src/app_window.rs`, which calls `instance.frame()`
unconditionally. Today `wants_repaint_after_ms` controls whether egui repaints at all, not
which guests run, so one animating app re-runs every other app at its rate.

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

## The desktop

`ccosel-shell` owns the desktop. Apps never position their own window; they fill one the
desktop gives them, and the shell keeps the geometry, z-order and focus. Window *instances* are
numbered separately from app ids, so two Files windows get independent egui state instead of
fighting over scroll position and focus.

Compiled `WebAssembly.Module`s are cached per app id, so a second window of the same app skips
the fetch and the compile — the expensive half of a launch. That cache is where IndexedDB will
slot in to make it free across sessions too.

Closing a window drops the instance. That is the only way to reclaim a guest's memory: wasm
linear memory cannot shrink, so a live instance holds its high-water mark forever.

### Verified in a real browser

Driven through headless Chrome over CDP: two apps launched from the taskbar, a window dragged,
the stopwatch run to 7.3s **with no input at all** (proving `wants_repaint_after_ms` drives
re-runs), a lap recorded, a second Files window opened with independent state and an
auto-numbered title, a buried window raised from the taskbar, and a window closed — with the
byte counter halving from 1124 to 562 B/frame as its app went away. No console errors.

## Talking to the server

Apps are synchronous and batch-only; RPC is not. The bridge is a **request cache keyed by the
request itself**, which suits immediate mode: an app already re-declares its whole UI every
frame, so it re-declares its data dependencies the same way.

```rust
match ui.rpc().get::<ListDir>(&ListDirReq { path: &self.path }) {
    Poll::Pending     => ui.label("Loading…"),
    Poll::Failed(e)   => ui.label(e.message()),
    Poll::Ready(list) => { /* draw it */ }
}
```

An app writes no call ids, no "have I asked yet" flag, no `on_event` handler, no cancellation,
and no stale-reply check. Changing `self.path` *is* the re-request, because it changes the key.
That is the point: navigate `/a → /b` while `list_dir(/a)` is in flight and it resolves second,
and a hand-rolled state machine shows the wrong directory unless the author remembered to
compare ids. Here the key already moved on, so **the app cannot express the bug.**

Three rules hold the rest together:

- **Guests allocate their own call ids**, and `rpc_call` returns nothing. A shell-allocated id
  would have to come back synchronously, and a Worker-hosted guest could only read it via
  `Atomics.wait` — which needs cross-origin isolation, hence HTTPS, which the LAN does not
  have. Fire-and-forget is the only shape that keeps guests movable off the main thread.
- **The shell owns the pending-call table.** A window can close or suspend with calls
  outstanding without losing track of them; guests only *name* their calls.
- **Exactly one terminal event per call** — success, error, timeout or supersede, unless
  cancelled, which delivers nothing. So no app needs a "might wait forever" branch, which is
  the branch app authors forget to write.

`ccosel-abi` carries RPC payloads as **opaque bytes** and does not depend on `ccosel-proto`, so
adding a method never bumps `ABI_VERSION` and never invalidates a module a client has cached.

### HTTP, not a WebSocket — for now

The plan called for two WebSockets. That is premature while every call is request/response:
a socket would carry nothing a POST does not, while costing reconnect, resume-from-cursor, an
app-level heartbeat (browsers cannot observe ping/pong) and a pre-open queue. `POST /rpc` takes
a *batch*, so coalescing still pays, and the envelope is unchanged when a socket does arrive.
It earns its place with the first real push: compile progress, or filesystem change events.

### Verified against the real server

Headless Chrome against `ccosel-server`: the File Browser lists `data/shared`, entering
`Documents` shows its contents, and going back **costs no request at all** — boot 1 POST,
navigate 2, return 2. Three windows (two File Browsers plus the Clock) totalled 2 posts.

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
- [x] File Browser v0 as an actual `.wasm` guest
- [x] `ccosel-host-web` — the shipping backend, tested under a real WebAssembly engine in node
- [x] `ccosel-shell` + hand-written loader — **the File Browser runs in a browser tab, and
      clicking a file updates it**, verified end-to-end in headless Chrome
- [x] Desktop environment — wallpaper, draggable/resizable/closable windows, taskbar with
      click-to-raise, app launcher, live per-frame byte counter
- [x] Multiple concurrent apps, and multiple instances of the same app with independent state
- [x] Async RPC end to end — the File Browser lists a **real server directory** in a browser
- [ ] Per-app repaint gating (see the note under "How apps render" — currently every guest
      re-runs every egui frame)
- [ ] Upload/download with content-defined chunking; IndexedDB module cache; app eviction

## Running it

```
cargo xtask build-web      # both workspaces + wasm-bindgen, reports wire sizes
cargo xtask serve          # http://127.0.0.1:8777/

cargo test                                             # 32 native tests
cargo test -p ccosel-host-web --target wasm32-unknown-unknown   # 4, under node
cargo run -p ccosel-host-wasmtime --example dev_shell   # native dev loop
```

`dev_shell` loads the same `.wasm` the browser fetches — no `wasm-bindgen`, no JS glue — so a
bug reproduced there is the bug that would happen in the browser, with a real debugger.

### Measured wire sizes

|  | raw | gzip |
|---|---|---|
| shell.wasm | 5.7 MB | 1.8 MB |
| shell.js | 82 KB | 11 KB |
| **file-browser.wasm** | **27 KB** | **11 KB** |

The app number is the one that matters: it is paid per app, repeatedly, whereas the shell is
fetched once and cached. An app that linked its own eframe would be ~1 MB compressed instead of
11 KB — that ratio is the entire justification for the command-stream design. `xtask` fails the
build if an app module exceeds 100 KB gzipped. `wasm-opt -Oz` is not yet installed and is the
remaining lever on shell size.

Build order and rationale: see the plan at `~/.claude/plans/read-through-teh-readme-mellow-pond.md`.
