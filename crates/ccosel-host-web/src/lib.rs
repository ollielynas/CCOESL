//! `AppHost` backed by the browser's own `WebAssembly` engine.
//!
//! This is the shipping path. Guests are plain `wasm32-unknown-unknown` cdylibs with no
//! `wasm-bindgen`, so there is no per-app JS shim to fetch and the browser JITs them at full
//! speed — the shell instantiates them directly and talks to their linear memory by hand.
//!
//! **The detachment rule.** A `WebAssembly.Memory`'s `ArrayBuffer` is *detached* whenever the
//! guest grows its heap, which silently invalidates any `Uint8Array` view over it. So every
//! access here re-reads `memory.buffer()` immediately before use and never holds a view across
//! a call into the guest. (The wasmtime backend obeys the same rule for a different reason:
//! `Memory::data_mut` borrows the store.) Getting this wrong produces corruption that only
//! appears once an app's allocations happen to cross a page boundary — so it is enforced
//! structurally, by having exactly one accessor for each direction.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use ccosel_abi::{FrameInput, FrameOutput, RespRecord, Slice, ABI_VERSION};
use ccosel_host::{AppHost, AppInstance, FrameArgs, FrameResult, HostError};
use js_sys::{Function, Object, Reflect, Uint8Array, WebAssembly};
use wasm_bindgen::prelude::*;

const FRAME_INPUT_SIZE: u32 = size_of::<FrameInput>() as u32;
const RESP_SIZE: u32 = size_of::<RespRecord>() as u32;
const STAGING_ALIGN: u32 = 8;

fn js_err(e: JsValue) -> HostError {
    HostError::Trap(
        e.as_string()
            .or_else(|| {
                Reflect::get(&e, &"message".into())
                    .ok()
                    .and_then(|m| m.as_string())
            })
            .unwrap_or_else(|| format!("{e:?}")),
    )
}

/// Shared between the instance and the closures it hands to the guest as imports.
/// A call the guest issued this frame. Mirrors `ccosel_host_wasmtime::OutboundCall`; the two
/// backends must agree on the import shapes exactly, and this pair has diverged before.
#[derive(Clone, Debug)]
pub struct OutboundCall {
    pub call_id: u32,
    pub method: u32,
    pub args: Vec<u8>,
}

#[derive(Default)]
struct Shared {
    memory: Option<WebAssembly::Memory>,
    log: Vec<(u32, String)>,
    outbox: Vec<OutboundCall>,
    cancels: Vec<u32>,
}

impl Shared {
    /// Copy bytes out of guest memory.
    ///
    /// Re-reads `buffer()` every call — see the detachment rule in the module docs.
    fn read(&self, ptr: u32, len: u32) -> Result<Vec<u8>, HostError> {
        let Some(memory) = &self.memory else {
            return Err(HostError::BadPointer);
        };
        let buffer = memory.buffer();
        let size = Reflect::get(&buffer, &"byteLength".into())
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if f64::from(ptr) + f64::from(len) > size {
            return Err(HostError::BadPointer);
        }
        let view = Uint8Array::new_with_byte_offset_and_length(&buffer, ptr, len);
        let mut out = vec![0u8; len as usize];
        view.copy_to(&mut out);
        Ok(out)
    }

    fn write(&self, ptr: u32, bytes: &[u8]) -> Result<(), HostError> {
        let Some(memory) = &self.memory else {
            return Err(HostError::BadPointer);
        };
        let buffer = memory.buffer();
        let size = Reflect::get(&buffer, &"byteLength".into())
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if f64::from(ptr) + bytes.len() as f64 > size {
            return Err(HostError::BadPointer);
        }
        let view = Uint8Array::new_with_byte_offset_and_length(&buffer, ptr, bytes.len() as u32);
        view.copy_from(bytes);
        Ok(())
    }
}

pub struct WebHost;

impl Default for WebHost {
    fn default() -> Self {
        Self::new()
    }
}

impl WebHost {
    pub fn new() -> Self {
        Self
    }
}

impl AppHost for WebHost {
    type Module = WebAssembly::Module;
    type Instance = WebInstance;

    /// Always async: browsers reject synchronous `new WebAssembly.Module` on the main thread
    /// for buffers over 4 KB, and every real app module is larger than that.
    async fn compile(&self, wasm: &[u8]) -> Result<WebAssembly::Module, HostError> {
        let bytes = Uint8Array::from(wasm);
        let promise = WebAssembly::compile(&bytes.into());
        let value = wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(js_err)?;
        value
            .dyn_into::<WebAssembly::Module>()
            .map_err(|_| HostError::Trap("WebAssembly.compile did not yield a Module".into()))
    }

    fn instantiate(&self, module: &WebAssembly::Module) -> Result<WebInstance, HostError> {
        let shared = Rc::new(RefCell::new(Shared::default()));
        let (imports, closures) = build_imports(&shared);

        // Synchronous on purpose: the module is already compiled, so this costs nothing and
        // keeps app launch out of the async machinery.
        let instance = WebAssembly::Instance::new(module, &imports).map_err(js_err)?;
        let exports = instance.exports();

        let memory = Reflect::get(&exports, &"memory".into())
            .map_err(js_err)?
            .dyn_into::<WebAssembly::Memory>()
            .map_err(|_| HostError::MissingExport("memory"))?;
        shared.borrow_mut().memory = Some(memory);

        let exports = Exports::lookup(&exports)?;

        let found = call0(&exports.abi_version)?;
        if found != ABI_VERSION {
            return Err(HostError::AbiMismatch {
                expected: ABI_VERSION,
                found,
            });
        }
        exports
            .init
            .call2(&JsValue::NULL, &0u32.into(), &0u32.into())
            .map_err(js_err)?;

        Ok(WebInstance {
            shared,
            exports,
            _closures: closures,
        })
    }
}

/// The host functions a guest may import.
///
/// This list is mirrored by the wasmtime backend, and every entry is a place the two can
/// drift — which is why it is kept short.
fn build_imports(shared: &Rc<RefCell<Shared>>) -> (Object, Vec<Box<dyn Any>>) {
    let ccosel = Object::new();

    let log_shared = shared.clone();
    let log = Closure::<dyn FnMut(u32, u32, u32)>::new(move |level: u32, ptr: u32, len: u32| {
        let text = log_shared
            .borrow()
            .read(ptr, len)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        if let Some(text) = text {
            log_shared.borrow_mut().log.push((level, text));
        }
    });
    let _ = Reflect::set(&ccosel, &"log".into(), log.as_ref());

    let now = Closure::<dyn FnMut() -> f64>::new(js_sys::Date::now);
    let _ = Reflect::set(&ccosel, &"now_ms".into(), now.as_ref());

    let rand_shared = shared.clone();
    let random = Closure::<dyn FnMut(u32, u32)>::new(move |ptr: u32, len: u32| {
        let mut buf = vec![0u8; len as usize];
        for b in &mut buf {
            *b = (js_sys::Math::random() * 256.0) as u8;
        }
        let _ = rand_shared.borrow().write(ptr, &buf);
    });
    let _ = Reflect::set(&ccosel, &"random".into(), random.as_ref());

    // Fire-and-forget: queue the call and return. Returning a value here would be a mistake —
    // a Worker-hosted guest could only read it via `Atomics.wait`, which needs cross-origin
    // isolation, which needs HTTPS, which a plain-HTTP LAN does not have. Guest-allocated call
    // ids are what let this return nothing.
    let rpc_shared = shared.clone();
    let rpc = Closure::<dyn FnMut(u32, u32, u32, u32)>::new(
        move |call_id: u32, method: u32, ptr: u32, len: u32| {
            let args = rpc_shared.borrow().read(ptr, len).unwrap_or_default();
            rpc_shared.borrow_mut().outbox.push(OutboundCall {
                call_id,
                method,
                args,
            });
        },
    );
    let _ = Reflect::set(&ccosel, &"rpc_call".into(), rpc.as_ref());

    let cancel_shared = shared.clone();
    let cancel = Closure::<dyn FnMut(u32)>::new(move |call_id: u32| {
        cancel_shared.borrow_mut().cancels.push(call_id);
    });
    let _ = Reflect::set(&ccosel, &"rpc_cancel".into(), cancel.as_ref());

    // Still a placeholder: incremental command flushing is not implemented.
    let flush = Closure::<dyn FnMut(u32, u32)>::new(|_: u32, _: u32| {});
    let _ = Reflect::set(&ccosel, &"cmd_flush".into(), flush.as_ref());

    // The closures must outlive the instance, or the guest ends up calling into a freed
    // function table. `Box<dyn Any>` erases their differing signatures into one keep-alive
    // vector while still running each one's real drop glue.
    let keep: Vec<Box<dyn Any>> = vec![
        Box::new(log),
        Box::new(now),
        Box::new(random),
        Box::new(rpc),
        Box::new(cancel),
        Box::new(flush),
    ];

    let imports = Object::new();
    let _ = Reflect::set(&imports, &"ccosel".into(), &ccosel);
    (imports, keep)
}

struct Exports {
    alloc: Function,
    dealloc: Function,
    init: Function,
    frame: Function,
    on_event: Function,
    save_state: Function,
    abi_version: Function,
}

impl Exports {
    fn lookup(exports: &Object) -> Result<Self, HostError> {
        fn get(exports: &Object, name: &'static str) -> Result<Function, HostError> {
            Reflect::get(exports, &name.into())
                .ok()
                .and_then(|v| v.dyn_into::<Function>().ok())
                .ok_or(HostError::MissingExport(name))
        }
        Ok(Self {
            alloc: get(exports, "ccosel_alloc")?,
            dealloc: get(exports, "ccosel_dealloc")?,
            init: get(exports, "ccosel_init")?,
            frame: get(exports, "ccosel_frame")?,
            on_event: get(exports, "ccosel_on_event")?,
            save_state: get(exports, "ccosel_save_state")?,
            abi_version: get(exports, "ccosel_abi_version")?,
        })
    }
}

fn as_u32(v: JsValue) -> u32 {
    v.as_f64().unwrap_or(0.0) as u32
}

fn call0(f: &Function) -> Result<u32, HostError> {
    f.call0(&JsValue::NULL).map(as_u32).map_err(js_err)
}

pub struct WebInstance {
    shared: Rc<RefCell<Shared>>,
    exports: Exports,
    /// Keep-alive for the import closures; dropping these would leave the guest calling into
    /// freed function tables.
    _closures: Vec<Box<dyn Any>>,
}

impl WebInstance {
    pub fn take_log(&mut self) -> Vec<(u32, String)> {
        std::mem::take(&mut self.shared.borrow_mut().log)
    }

    /// Calls the guest issued during the last `frame()`, drained.
    pub fn take_outbox(&mut self) -> Vec<OutboundCall> {
        std::mem::take(&mut self.shared.borrow_mut().outbox)
    }

    /// Calls the guest abandoned during the last `frame()`, drained.
    pub fn take_cancels(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.shared.borrow_mut().cancels)
    }

    fn alloc(&self, len: u32, align: u32) -> Result<u32, HostError> {
        self.exports
            .alloc
            .call2(&JsValue::NULL, &len.into(), &align.into())
            .map(as_u32)
            .map_err(js_err)
    }

    fn dealloc(&self, ptr: u32, len: u32, align: u32) -> Result<(), HostError> {
        self.exports
            .dealloc
            .call3(&JsValue::NULL, &ptr.into(), &len.into(), &align.into())
            .map(|_| ())
            .map_err(js_err)
    }
}

impl AppInstance for WebInstance {
    fn frame(&mut self, args: &FrameArgs<'_>) -> Result<FrameResult, HostError> {
        let resp_bytes = args.responses.len() as u32 * RESP_SIZE;
        let event_bytes = args.events.len() as u32;
        let total = FRAME_INPUT_SIZE + resp_bytes + event_bytes;

        // One staging allocation per frame holding the FrameInput, the response table and the
        // events contiguously, so a frame costs a single alloc/dealloc pair.
        let base = self.alloc(total, STAGING_ALIGN)?;
        if base == 0 {
            return Err(HostError::BadPointer);
        }
        let resp_ptr = base + FRAME_INPUT_SIZE;
        let event_ptr = resp_ptr + resp_bytes;

        let input = FrameInput {
            frame_index: args.frame_index,
            time_ms: args.time_ms,
            dt_ms: args.dt_ms,
            pixels_per_point: args.pixels_per_point,
            screen_size: args.screen_size,
            abi_version: ABI_VERSION,
            resp_ptr: if resp_bytes == 0 { 0 } else { resp_ptr },
            resp_len: args.responses.len() as u32,
            event_ptr: if event_bytes == 0 { 0 } else { event_ptr },
            event_len: event_bytes,
            flags: args.flags,
            _pad: [0; 2],
        };

        {
            let shared = self.shared.borrow();
            shared.write(base, bytemuck::bytes_of(&input))?;
            if resp_bytes > 0 {
                shared.write(resp_ptr, bytemuck::cast_slice(args.responses))?;
            }
            if event_bytes > 0 {
                shared.write(event_ptr, args.events)?;
            }
        }

        let out_ptr = self
            .exports
            .frame
            .call1(&JsValue::NULL, &base.into())
            .map(as_u32)
            .map_err(js_err)?;

        // Borrow *after* the guest call: the buffer may have detached during it.
        let (commands, out) = {
            let shared = self.shared.borrow();
            let raw = shared.read(out_ptr, size_of::<FrameOutput>() as u32)?;
            let out: FrameOutput = *bytemuck::from_bytes(&raw);
            (shared.read(out.cmd_ptr, out.cmd_len)?, out)
        };

        self.dealloc(base, total, STAGING_ALIGN)?;

        Ok(FrameResult {
            commands,
            wants_repaint_after_ms: out.wants_repaint_after_ms,
            status: out.status,
        })
    }

    fn on_event(&mut self, bytes: &[u8]) -> Result<(), HostError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let len = bytes.len() as u32;
        let ptr = self.alloc(len, 1)?;
        self.shared.borrow().write(ptr, bytes)?;
        self.exports
            .on_event
            .call2(&JsValue::NULL, &ptr.into(), &len.into())
            .map_err(js_err)?;
        self.dealloc(ptr, len, 1)
    }

    fn save_state(&mut self) -> Result<Vec<u8>, HostError> {
        let slice_ptr = call0(&self.exports.save_state)?;
        if slice_ptr == 0 {
            return Ok(Vec::new());
        }
        let shared = self.shared.borrow();
        let raw = shared.read(slice_ptr, size_of::<Slice>() as u32)?;
        let slice: Slice = *bytemuck::from_bytes(&raw);
        if slice.ptr == 0 || slice.len == 0 {
            return Ok(Vec::new());
        }
        shared.read(slice.ptr, slice.len)
    }
}
