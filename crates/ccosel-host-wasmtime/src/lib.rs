//! `AppHost` backed by wasmtime, for the native dev shell and for server-hosted apps.
//!
//! The guest is a plain `wasm32-unknown-unknown` `cdylib` with no `wasm-bindgen`, so the exact
//! same `.wasm` the browser loads runs here unmodified. That is what makes the native shell a
//! real dev loop rather than a simulation.
//!
//! **Memory discipline:** every access goes through `Memory::read`/`Memory::write`, which take
//! and release the store borrow per call. Nothing ever holds a view into guest memory across a
//! guest call — on wasmtime that would deadlock the store borrow, and on web the buffer would
//! detach the moment the guest grew its heap.

use ccosel_abi::{ABI_VERSION, FrameInput, FrameOutput, RespRecord, Slice};
use ccosel_host::{AppHost, AppInstance, FrameArgs, FrameResult, HostError, OutboundCall};
use wasmtime::{Caller, Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

const FRAME_INPUT_SIZE: u32 = size_of::<FrameInput>() as u32;
const RESP_SIZE: u32 = size_of::<RespRecord>() as u32;
const STAGING_ALIGN: u32 = 8;

fn trap(e: impl std::fmt::Display) -> HostError {
    HostError::Trap(e.to_string())
}

/// Host-side state reachable from imported functions.
#[derive(Default)]
pub struct HostState {
    /// Lines the guest logged this frame, for the shell to surface.
    pub log: Vec<(u32, String)>,
    /// Calls issued this frame, drained by the shell after `frame()` returns.
    pub outbox: Vec<OutboundCall>,
    /// Calls the guest abandoned. Without these the shell's pending table accumulates entries
    /// the guest no longer tracks, and the app becomes permanently unevictable.
    pub cancels: Vec<u32>,
}

pub struct WasmtimeHost {
    engine: Engine,
}

impl Default for WasmtimeHost {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmtimeHost {
    pub fn new() -> Self {
        Self {
            engine: Engine::default(),
        }
    }
}

impl AppHost for WasmtimeHost {
    type Module = Module;
    type Instance = WasmtimeInstance;

    /// Cranelift compiles synchronously, so this future is ready on first poll. The signature
    /// is async only because the browser backend has no choice.
    async fn compile(&self, wasm: &[u8]) -> Result<Module, HostError> {
        Module::new(&self.engine, wasm).map_err(trap)
    }

    fn instantiate(&self, module: &Module) -> Result<WasmtimeInstance, HostError> {
        let mut store = Store::new(&self.engine, HostState::default());
        let mut linker = Linker::new(&self.engine);
        define_imports(&mut linker)?;

        let instance = linker.instantiate(&mut store, module).map_err(trap)?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or(HostError::MissingExport("memory"))?;

        let exports = Exports::lookup(&mut store, &instance)?;

        // Refuse a module built against a different ABI rather than guessing. A stale module
        // out of the client's IndexedDB cache is the expected way to reach this.
        let found = exports.abi_version.call(&mut store, ()).map_err(trap)?;
        if found != ABI_VERSION {
            return Err(HostError::AbiMismatch {
                expected: ABI_VERSION,
                found,
            });
        }

        exports.init.call(&mut store, (0, 0)).map_err(trap)?;

        Ok(WasmtimeInstance {
            store,
            memory,
            exports,
        })
    }
}

/// The host functions a guest may import.
///
/// Kept deliberately short — this list has to be mirrored by hand in the browser backend's JS
/// glue, and every entry is a place the two can drift.
fn define_imports(linker: &mut Linker<HostState>) -> Result<(), HostError> {
    linker
        .func_wrap(
            "ccosel",
            "log",
            |mut caller: Caller<'_, HostState>, level: u32, ptr: u32, len: u32| {
                let Some(wasmtime::Extern::Memory(mem)) = caller.get_export("memory") else {
                    return;
                };
                let mut buf = vec![0u8; len as usize];
                if mem.read(&mut caller, ptr as usize, &mut buf).is_ok() {
                    let msg = String::from_utf8_lossy(&buf).into_owned();
                    caller.data_mut().log.push((level, msg));
                }
            },
        )
        .map_err(trap)?;

    linker
        .func_wrap("ccosel", "now_ms", || -> f64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64() * 1000.0)
                .unwrap_or(0.0)
        })
        .map_err(trap)?;

    linker
        .func_wrap(
            "ccosel",
            "random",
            |mut caller: Caller<'_, HostState>, ptr: u32, len: u32| {
                let Some(wasmtime::Extern::Memory(mem)) = caller.get_export("memory") else {
                    return;
                };
                // Not cryptographic; guests needing real entropy ask the server.
                let mut seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos() as u64)
                    .unwrap_or(1);
                let mut buf = vec![0u8; len as usize];
                for b in &mut buf {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    *b = (seed >> 33) as u8;
                }
                let _ = mem.write(&mut caller, ptr as usize, &buf);
            },
        )
        .map_err(trap)?;

    // Fire-and-forget: queue the call and return. Never blocks, never answers synchronously,
    // because a synchronous answer is exactly what a Worker-hosted guest could not obtain.
    linker
        .func_wrap(
            "ccosel",
            "rpc_call",
            |mut caller: Caller<'_, HostState>, call_id: u32, method: u32, ptr: u32, len: u32| {
                let Some(wasmtime::Extern::Memory(mem)) = caller.get_export("memory") else {
                    return;
                };
                let mut args = vec![0u8; len as usize];
                if mem.read(&mut caller, ptr as usize, &mut args).is_ok() {
                    caller.data_mut().outbox.push(OutboundCall {
                        call_id,
                        method,
                        args,
                    });
                }
            },
        )
        .map_err(trap)?;
    linker
        .func_wrap(
            "ccosel",
            "rpc_cancel",
            |mut caller: Caller<'_, HostState>, call_id: u32| {
                caller.data_mut().cancels.push(call_id);
            },
        )
        .map_err(trap)?;
    // Still a placeholder: incremental command flushing is not implemented.
    linker
        .func_wrap("ccosel", "cmd_flush", |_: u32, _: u32| {})
        .map_err(trap)?;

    Ok(())
}

struct Exports {
    alloc: TypedFunc<(u32, u32), u32>,
    dealloc: TypedFunc<(u32, u32, u32), ()>,
    init: TypedFunc<(u32, u32), u32>,
    frame: TypedFunc<u32, u32>,
    on_event: TypedFunc<(u32, u32), ()>,
    save_state: TypedFunc<(), u32>,
    abi_version: TypedFunc<(), u32>,
}

impl Exports {
    fn lookup(store: &mut Store<HostState>, instance: &Instance) -> Result<Self, HostError> {
        fn get<P, R>(
            store: &mut Store<HostState>,
            instance: &Instance,
            name: &'static str,
        ) -> Result<TypedFunc<P, R>, HostError>
        where
            P: wasmtime::WasmParams,
            R: wasmtime::WasmResults,
        {
            instance
                .get_typed_func(store, name)
                .map_err(|_| HostError::MissingExport(name))
        }

        Ok(Self {
            alloc: get(store, instance, "ccosel_alloc")?,
            dealloc: get(store, instance, "ccosel_dealloc")?,
            init: get(store, instance, "ccosel_init")?,
            frame: get(store, instance, "ccosel_frame")?,
            on_event: get(store, instance, "ccosel_on_event")?,
            save_state: get(store, instance, "ccosel_save_state")?,
            abi_version: get(store, instance, "ccosel_abi_version")?,
        })
    }
}

pub struct WasmtimeInstance {
    store: Store<HostState>,
    memory: Memory,
    exports: Exports,
}

impl WasmtimeInstance {
    /// Log lines the guest emitted, drained.
    pub fn take_log(&mut self) -> Vec<(u32, String)> {
        std::mem::take(&mut self.store.data_mut().log)
    }

    fn read_bytes(&mut self, ptr: u32, len: u32) -> Result<Vec<u8>, HostError> {
        let mut buf = vec![0u8; len as usize];
        self.memory
            .read(&self.store, ptr as usize, &mut buf)
            .map_err(|_| HostError::BadPointer)?;
        Ok(buf)
    }
}

impl AppInstance for WasmtimeInstance {
    fn frame(&mut self, args: &FrameArgs<'_>) -> Result<FrameResult, HostError> {
        let resp_bytes = args.responses.len() as u32 * RESP_SIZE;
        let event_bytes = args.events.len() as u32;
        let total = FRAME_INPUT_SIZE + resp_bytes + event_bytes;

        // One staging allocation per frame holding the FrameInput, the response table and the
        // event bytes contiguously, so a frame costs a single alloc/dealloc pair rather than
        // three round trips into the guest.
        let base = self
            .exports
            .alloc
            .call(&mut self.store, (total, STAGING_ALIGN))
            .map_err(trap)?;
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

        let write = |m: &Memory, store: &mut Store<HostState>, at: u32, bytes: &[u8]| {
            m.write(store, at as usize, bytes)
                .map_err(|_| HostError::BadPointer)
        };

        write(
            &self.memory,
            &mut self.store,
            base,
            bytemuck::bytes_of(&input),
        )?;
        if resp_bytes > 0 {
            write(
                &self.memory,
                &mut self.store,
                resp_ptr,
                bytemuck::cast_slice(args.responses),
            )?;
        }
        if event_bytes > 0 {
            write(&self.memory, &mut self.store, event_ptr, args.events)?;
        }

        let out_ptr = self
            .exports
            .frame
            .call(&mut self.store, base)
            .map_err(trap)?;

        let out_bytes = self.read_bytes(out_ptr, size_of::<FrameOutput>() as u32)?;
        let out: FrameOutput = *bytemuck::from_bytes(&out_bytes);

        // Copy the commands out *before* freeing the staging buffer, and never hand a borrow
        // of guest memory back to the caller.
        let commands = self.read_bytes(out.cmd_ptr, out.cmd_len)?;

        self.exports
            .dealloc
            .call(&mut self.store, (base, total, STAGING_ALIGN))
            .map_err(trap)?;

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
        let ptr = self
            .exports
            .alloc
            .call(&mut self.store, (len, 1))
            .map_err(trap)?;
        self.memory
            .write(&mut self.store, ptr as usize, bytes)
            .map_err(|_| HostError::BadPointer)?;
        self.exports
            .on_event
            .call(&mut self.store, (ptr, len))
            .map_err(trap)?;
        self.exports
            .dealloc
            .call(&mut self.store, (ptr, len, 1))
            .map_err(trap)?;
        Ok(())
    }

    fn take_outbox(&mut self) -> Vec<OutboundCall> {
        std::mem::take(&mut self.store.data_mut().outbox)
    }

    fn take_cancels(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.store.data_mut().cancels)
    }

    fn save_state(&mut self) -> Result<Vec<u8>, HostError> {
        let slice_ptr = self
            .exports
            .save_state
            .call(&mut self.store, ())
            .map_err(trap)?;
        if slice_ptr == 0 {
            return Ok(Vec::new());
        }
        let raw = self.read_bytes(slice_ptr, size_of::<Slice>() as u32)?;
        let slice: Slice = *bytemuck::from_bytes(&raw);
        if slice.ptr == 0 || slice.len == 0 {
            return Ok(Vec::new());
        }
        self.read_bytes(slice.ptr, slice.len)
    }
}
