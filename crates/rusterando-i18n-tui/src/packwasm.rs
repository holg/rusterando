//! Execute the REAL built translation pack `.wasm` (source #2) from the TUI.
//!
//! The pack is compiled with wasm-bindgen (`--target web`), so its exports take
//! and return strings via pointers into linear memory, with the marshalling
//! normally done by the JS glue. There's no JS here, so we re-implement that
//! string ABI by hand against wasmtime. Verified against the pack's glue:
//!
//!   menu_lookup(locale, source):
//!     ptr0 = __wbindgen_malloc(len0, 1); write utf8(locale) @ ptr0
//!     ptr1 = __wbindgen_malloc(len1, 1); write utf8(source) @ ptr1
//!     (ret_ptr, ret_len) = menu_lookup(ptr0, len0, ptr1, len1)   // multi-value
//!     out = utf8(memory[ret_ptr .. ret_ptr+ret_len])
//!     __wbindgen_free(ret_ptr, ret_len, 1)
//!
//! The wasm imports exactly one host function,
//! `__wbindgen_init_externref_table`, which we stub as a no-op (the pure-data
//! pack doesn't use externrefs at runtime). Empty result string == "not in pack".

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use wasmtime::{
    Caller, Config, Engine, Extern, Func, Instance, Linker, Memory, Module, Store, Val,
};

pub struct PackWasm {
    store: Store<()>,
    memory: Memory,
    malloc: Func,
    free: Func,
    menu_lookup: Func,
    lookup: Func,
}

impl PackWasm {
    /// Load + instantiate the pack wasm at `path`.
    pub fn load(path: &Path) -> Result<Self> {
        // wasm-bindgen (--target web) emits a module that uses the
        // reference-types proposal (the externref table). wasmtime disallows GC
        // types by default, so enable reference types or instantiation fails
        // with "gc types are disallowed".
        let mut config = Config::new();
        config.wasm_reference_types(true);
        let engine = Engine::new(&config).context("wasmtime engine")?;
        let module = Module::from_file(&engine, path)
            .with_context(|| format!("load wasm {}", path.display()))?;
        let mut store = Store::new(&engine, ());

        let mut linker = Linker::new(&engine);
        // The only import: a no-op externref table init.
        linker.func_wrap(
            "./rusterando_i18n_pack_bg.js",
            "__wbindgen_init_externref_table",
            |_caller: Caller<'_, ()>| {},
        )?;
        // Be tolerant of a differently-named glue module (hash in the name) by
        // also registering common fallbacks is unnecessary here — the import
        // module string is stable (the _bg.js basename). If it ever differs,
        // instantiation errors clearly.

        let instance = linker
            .instantiate(&mut store, &module)
            .context("instantiate pack wasm")?;

        // wasm-bindgen emits a start function; run it if present.
        if let Some(start) = instance.get_func(&mut store, "__wbindgen_start") {
            start.call(&mut store, &[], &mut [])?;
        }

        let memory = get_memory(&instance, &mut store)?;
        let malloc = get_func(&instance, &mut store, "__wbindgen_malloc")?;
        let free = get_func(&instance, &mut store, "__wbindgen_free")?;
        let menu_lookup = get_func(&instance, &mut store, "menu_lookup")?;
        let lookup = get_func(&instance, &mut store, "lookup")?;

        Ok(Self {
            store,
            memory,
            malloc,
            free,
            menu_lookup,
            lookup,
        })
    }

    /// Menu translation (keyed by German source). `None` if the pack lacks it.
    pub fn menu_lookup(&mut self, locale: &str, german: &str) -> Result<Option<String>> {
        self.call_two_str(self.menu_lookup, locale, german)
    }

    /// Chrome translation (keyed by dotted key). `None` if the pack lacks it.
    #[allow(dead_code)]
    pub fn lookup(&mut self, locale: &str, key: &str) -> Result<Option<String>> {
        self.call_two_str(self.lookup, locale, key)
    }

    /// Call a `(str, str) -> str` wasm-bindgen export, marshalling both ways.
    fn call_two_str(&mut self, func: Func, a: &str, b: &str) -> Result<Option<String>> {
        let p0 = self.write_str(a)?;
        let p1 = self.write_str(b)?;

        // Multi-value return: (ret_ptr i32, ret_len i32).
        let mut results = [Val::I32(0), Val::I32(0)];
        func.call(
            &mut self.store,
            &[
                Val::I32(p0.0),
                Val::I32(p0.1),
                Val::I32(p1.0),
                Val::I32(p1.1),
            ],
            &mut results,
        )?;
        let ret_ptr = results[0].unwrap_i32();
        let ret_len = results[1].unwrap_i32();

        // Note: the input buffers were consumed by wasm-bindgen's own
        // free inside the export (it takes ownership of &str args via the
        // passStringToWasm contract), so we must NOT free p0/p1 ourselves.

        let out = self.read_str(ret_ptr, ret_len)?;
        // Free the returned string (the glue does __wbindgen_free(ptr,len,1)).
        if ret_ptr != 0 {
            self.free.call(
                &mut self.store,
                &[Val::I32(ret_ptr), Val::I32(ret_len), Val::I32(1)],
                &mut [],
            )?;
        }
        Ok(if out.is_empty() { None } else { Some(out) })
    }

    /// malloc(len,1) + write utf8; returns (ptr, len).
    fn write_str(&mut self, s: &str) -> Result<(i32, i32)> {
        let bytes = s.as_bytes();
        let len = bytes.len() as i32;
        let mut ret = [Val::I32(0)];
        self.malloc
            .call(&mut self.store, &[Val::I32(len), Val::I32(1)], &mut ret)?;
        let ptr = ret[0].unwrap_i32();
        self.memory
            .write(&mut self.store, ptr as usize, bytes)
            .context("write arg into wasm memory")?;
        Ok((ptr, len))
    }

    /// Read `len` utf8 bytes at `ptr`.
    fn read_str(&mut self, ptr: i32, len: i32) -> Result<String> {
        if ptr == 0 || len == 0 {
            return Ok(String::new());
        }
        let mut buf = vec![0u8; len as usize];
        self.memory
            .read(&self.store, ptr as usize, &mut buf)
            .context("read result from wasm memory")?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

fn get_memory(instance: &Instance, store: &mut Store<()>) -> Result<Memory> {
    match instance.get_export(&mut *store, "memory") {
        Some(Extern::Memory(m)) => Ok(m),
        _ => Err(anyhow!("pack wasm has no exported `memory`")),
    }
}

fn get_func(instance: &Instance, store: &mut Store<()>, name: &str) -> Result<Func> {
    instance
        .get_func(&mut *store, name)
        .ok_or_else(|| anyhow!("pack wasm missing export `{name}`"))
}
