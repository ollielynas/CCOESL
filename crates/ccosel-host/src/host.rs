//! The platform-split app-hosting interface.
//!
//! Two backends implement this: `ccosel-host-wasmtime` (native/server) and `ccosel-host-web`
//! (browser, via `WebAssembly.instantiate`). Everything above this trait is identical on both,
//! which is what lets the shell be developed natively and shipped to the web.
//!
//! Note the async/sync split: **compilation is async, instantiation is not.**
//!
//! Compilation has to be async because browsers refuse synchronous `new WebAssembly.Module` on
//! the main thread for anything over 4 KB, and every real app module is bigger than that.
//! Instantiation is deliberately *not* async — both `new WebAssembly.Instance(module, ..)` and
//! `wasmtime::Instance::new` are synchronous once a module is compiled, and making the whole
//! trait async would poison the render loop for no reason.

use ccosel_abi::{DecodeError, RespRecord};

#[derive(Debug)]
pub enum HostError {
    /// The module was built against a different ABI. The shell must refuse it rather than
    /// guess — a cached module from an older build is the expected way to hit this.
    AbiMismatch { expected: u32, found: u32 },
    MissingExport(&'static str),
    /// The guest trapped: panic, unreachable, out-of-bounds.
    Trap(String),
    /// The guest returned pointers outside its own memory.
    BadPointer,
    Decode(DecodeError),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AbiMismatch { expected, found } => {
                write!(f, "app built for ABI v{found}, shell speaks v{expected}")
            }
            Self::MissingExport(name) => write!(f, "app is missing export `{name}`"),
            Self::Trap(msg) => write!(f, "app trapped: {msg}"),
            Self::BadPointer => write!(f, "app returned a pointer outside its own memory"),
            Self::Decode(e) => write!(f, "malformed command stream: {e:?}"),
        }
    }
}

impl std::error::Error for HostError {}

impl From<DecodeError> for HostError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}

/// What the shell tells a guest about this frame.
pub struct FrameArgs<'a> {
    pub frame_index: u64,
    pub time_ms: f64,
    pub dt_ms: f32,
    pub pixels_per_point: f32,
    pub screen_size: [f32; 2],
    pub flags: u32,
    /// Responses from the *previous* frame, sorted by local id.
    pub responses: &'a [RespRecord],
    pub events: &'a [u8],
}

impl Default for FrameArgs<'_> {
    fn default() -> Self {
        Self {
            frame_index: 0,
            time_ms: 0.0,
            dt_ms: 16.0,
            pixels_per_point: 1.0,
            screen_size: [0.0, 0.0],
            flags: 0,
            responses: &[],
            events: &[],
        }
    }
}

/// One frame's worth of guest output, **copied out** of guest memory.
///
/// Owned rather than borrowed on purpose. On web, `memory.buffer` detaches the moment the
/// guest grows its heap, invalidating any view held across a call; on wasmtime,
/// `Memory::data_mut` borrows the store so you cannot re-enter the guest while holding it.
/// One rule satisfies both: copy at flush time, never hold a view across a guest call.
pub struct FrameResult {
    pub commands: Vec<u8>,
    pub wants_repaint_after_ms: u32,
    pub status: u32,
}

/// A loaded, running app.
pub trait AppInstance {
    fn frame(&mut self, args: &FrameArgs<'_>) -> Result<FrameResult, HostError>;
    fn on_event(&mut self, bytes: &[u8]) -> Result<(), HostError>;
    fn save_state(&mut self) -> Result<Vec<u8>, HostError>;
}

/// Compiles and instantiates app modules.
pub trait AppHost {
    type Module;
    type Instance: AppInstance;

    fn compile(
        &self,
        wasm: &[u8],
    ) -> impl std::future::Future<Output = Result<Self::Module, HostError>>;

    fn instantiate(&self, module: &Self::Module) -> Result<Self::Instance, HostError>;
}
