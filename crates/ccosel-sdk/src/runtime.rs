//! The guest-side entry points.
//!
//! Everything here runs inside the app's own wasm module. The shell reaches it only through
//! the `extern "C"` exports that [`ccosel_app!`](crate::ccosel_app) generates — there is no
//! shared memory and no `wasm-bindgen`, which is what lets the identical binary run under
//! `wasmtime` on the server and under `WebAssembly.instantiate` in a browser.

use alloc::vec::Vec;
use ccosel_abi::{ABI_VERSION, FrameInput, FrameOutput, RespRecord};

use crate::App;
use crate::recorder::Recorder;
use crate::rpc::RpcCtx;
use crate::ui::Ui;

/// Holds an app and its recording state for the lifetime of the module.
pub struct Runtime<A: App> {
    app: A,
    rec: Recorder,
    out: FrameOutput,
    saved: ccosel_abi::Slice,
    responses: Vec<RespRecord>,
    rpc: RpcCtx,
}

impl<A: App> Runtime<A> {
    pub fn new(app: A) -> Self {
        Self {
            app,
            rec: Recorder::new(),
            out: FrameOutput::default(),
            saved: ccosel_abi::Slice::default(),
            responses: Vec::new(),
            rpc: RpcCtx::new(),
        }
    }

    /// Run one frame.
    ///
    /// # Safety
    /// `input_ptr` must point to a [`FrameInput`] written by the host, whose `resp_ptr` /
    /// `resp_len` describe a valid `RespRecord` array in this module's memory.
    pub unsafe fn frame(&mut self, input_ptr: u32) -> u32 {
        let input = unsafe { core::ptr::read(input_ptr as usize as *const FrameInput) };

        // Copy the response table in before running the app: the host's staging buffer is
        // freed the moment we return, and the recorder keeps these across the frame.
        self.responses.clear();
        if input.resp_len > 0 {
            let src = input.resp_ptr as usize as *const RespRecord;
            let n = input.resp_len as usize;
            self.responses
                .extend_from_slice(unsafe { core::slice::from_raw_parts(src, n) });
        }
        self.rec.set_responses(core::mem::take(&mut self.responses));

        // Age the request cache before the app runs, so anything it asks for this frame is
        // marked fresh and survives the sweep.
        self.rpc.begin_frame();
        {
            let ctx = crate::FrameCtx::from_input(&input);
            let mut ui = Ui::root(&mut self.rec, ctx, &self.rpc);
            self.app.update(&mut ui);
        }
        // Hand the shell anything the app asked for. Fire-and-forget: the replies come back
        // through `on_event` before some later frame, and wake the app then.
        self.flush_rpc();

        let cmds = self.rec.commands();
        self.out = FrameOutput {
            cmd_ptr: cmds.as_ptr() as usize as u32,
            cmd_len: cmds.len() as u32,
            wants_repaint_after_ms: self.app.wants_repaint_after_ms(),
            status: ccosel_abi::frame::status::OK,
        };
        // Sound because `self` outlives the call: the host reads this struct, copies the
        // command bytes out, and does not retain either pointer.
        &self.out as *const FrameOutput as usize as u32
    }

    pub fn abi_version(&self) -> u32 {
        ABI_VERSION
    }

    fn flush_rpc(&mut self) {
        for call in self.rpc.take_outbox() {
            host::rpc_call(call.call_id, call.method, &call.args);
        }
        for call_id in self.rpc.take_cancels() {
            host::rpc_cancel(call_id);
        }
    }

    /// Deliver a batch of events from the shell.
    ///
    /// # Safety
    /// `ptr`/`len` must describe a valid event batch written into this module's memory.
    pub unsafe fn on_event(&mut self, ptr: u32, len: u32) {
        if len == 0 {
            return;
        }
        let bytes = unsafe { core::slice::from_raw_parts(ptr as usize as *const u8, len as usize) };
        // A malformed batch is dropped whole rather than partially applied: half-applied
        // replies would leave slots in flight forever, and the app has no way to recover.
        if let Ok(events) = ccosel_abi::event::decode_batch(bytes) {
            for event in &events {
                self.rpc.deliver(event);
            }
        }
    }

    /// Access for tests: what the app has queued for the shell.
    pub fn rpc(&self) -> &RpcCtx {
        &self.rpc
    }

    /// Serialize the app's state for eviction. Stubbed until eviction lands, but the export
    /// exists in v0 because adding one later would break every module already cached by a
    /// client.
    pub fn save_state(&mut self) -> u32 {
        self.saved = ccosel_abi::Slice { ptr: 0, len: 0 };
        &self.saved as *const ccosel_abi::Slice as usize as u32
    }
}

/// A single-threaded cell for the module's global app instance.
///
/// wasm32 without shared memory is single-threaded by construction, so there is no concurrent
/// access to guard against. This exists because Rust 2024 forbids references to `static mut`,
/// not because there is a race to prevent.
pub struct GuestCell<T>(core::cell::UnsafeCell<Option<T>>);

// SAFETY: guests are single-threaded. If guests are ever moved to a shared-memory threading
// model, this is the thing that has to change.
unsafe impl<T> Sync for GuestCell<T> {}

impl<T> GuestCell<T> {
    pub const fn new() -> Self {
        Self(core::cell::UnsafeCell::new(None))
    }

    /// # Safety
    /// No other borrow of the contents may be live.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_or_init(&self, init: impl FnOnce() -> T) -> &mut T {
        let slot = unsafe { &mut *self.0.get() };
        slot.get_or_insert_with(init)
    }
}

impl<T> Default for GuestCell<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Define the module's `extern "C"` exports for an app type implementing [`App`] + `Default`.
///
/// ```ignore
/// struct FileBrowser { /* .. */ }
/// impl ccosel_sdk::App for FileBrowser { /* .. */ }
/// ccosel_sdk::ccosel_app!(FileBrowser);
/// ```
///
/// This is a `macro_rules!` rather than a proc macro on purpose: a proc macro would put `syn`
/// and `quote` in every app's build graph for what is a fixed block of glue.
#[macro_export]
macro_rules! ccosel_app {
    ($app:ty) => {
        const _: () = {
            use $crate::runtime::{GuestCell, Runtime};

            static RUNTIME: GuestCell<Runtime<$app>> = GuestCell::new();

            fn runtime() -> &'static mut Runtime<$app> {
                // SAFETY: single-threaded guest; no other borrow is live across these calls.
                unsafe { RUNTIME.get_or_init(|| Runtime::new(<$app>::default())) }
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn ccosel_abi_version() -> u32 {
                $crate::ABI_VERSION
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn ccosel_alloc(len: u32, align: u32) -> u32 {
                $crate::runtime::guest_alloc(len, align)
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn ccosel_dealloc(ptr: u32, len: u32, align: u32) {
                $crate::runtime::guest_dealloc(ptr, len, align)
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn ccosel_init(_cfg_ptr: u32, _cfg_len: u32) -> u32 {
                runtime();
                0
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn ccosel_frame(input_ptr: u32) -> u32 {
                // SAFETY: the host wrote a valid FrameInput at this address.
                unsafe { runtime().frame(input_ptr) }
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn ccosel_on_event(ptr: u32, len: u32) {
                // SAFETY: the shell wrote a valid event batch at this address.
                unsafe { runtime().on_event(ptr, len) }
            }

            // Declared in v0 and stubbed. Adding exports later would break every module the
            // shell has already cached, so the shape is fixed now even though eviction and
            // state restore are not implemented yet.
            #[unsafe(no_mangle)]
            pub extern "C" fn ccosel_save_state() -> u32 {
                runtime().save_state()
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn ccosel_restore_state(_ptr: u32, _len: u32) -> u32 {
                0
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn ccosel_memory_pressure(_level: u32) {}
        };
    };
}

/// Allocate a host staging buffer inside guest memory. Used by [`ccosel_app!`].
pub fn guest_alloc(len: u32, align: u32) -> u32 {
    if len == 0 {
        return 0;
    }
    let Ok(layout) = core::alloc::Layout::from_size_align(len as usize, align.max(1) as usize)
    else {
        return 0;
    };
    // SAFETY: layout has non-zero size.
    let ptr = unsafe { alloc::alloc::alloc(layout) };
    ptr as usize as u32
}

/// Free a buffer from [`guest_alloc`]. Used by [`ccosel_app!`].
pub fn guest_dealloc(ptr: u32, len: u32, align: u32) {
    if ptr == 0 || len == 0 {
        return;
    }
    let Ok(layout) = core::alloc::Layout::from_size_align(len as usize, align.max(1) as usize)
    else {
        return;
    };
    // SAFETY: the host only passes back pointers it received from `guest_alloc` with the same
    // layout.
    unsafe { alloc::alloc::dealloc(ptr as usize as *mut u8, layout) }
}

/// The host imports, and their stand-ins off-wasm.
///
/// Splitting here is what lets the whole RPC client be tested natively — the SDK's tests drive
/// a real app through real cache logic and simply record what would have crossed the boundary.
mod host {
    #[cfg(target_arch = "wasm32")]
    #[link(wasm_import_module = "ccosel")]
    unsafe extern "C" {
        #[link_name = "rpc_call"]
        fn host_rpc_call(call_id: u32, method: u32, arg_ptr: u32, arg_len: u32);
        #[link_name = "rpc_cancel"]
        fn host_rpc_cancel(call_id: u32);
    }

    #[cfg(target_arch = "wasm32")]
    pub fn rpc_call(call_id: u32, method: u32, args: &[u8]) {
        // The host copies the arguments out synchronously inside the import, so this pointer
        // only has to stay valid for the duration of the call.
        unsafe {
            host_rpc_call(
                call_id,
                method,
                args.as_ptr() as usize as u32,
                args.len() as u32,
            )
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn rpc_cancel(call_id: u32) {
        unsafe { host_rpc_cancel(call_id) }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn rpc_call(_call_id: u32, _method: u32, _args: &[u8]) {}

    #[cfg(not(target_arch = "wasm32"))]
    pub fn rpc_cancel(_call_id: u32) {}
}
