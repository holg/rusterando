//! Browser interop for calling a loaded split wasm.
//!
//! The loader (wasm-split-loader.js) instantiates a split wasm and registers a
//! surface object at `window.__<name>` with a `ready: bool` flag and string→
//! string functions. This module calls into that surface from Rust/wasm without
//! each consumer re-writing the js_sys plumbing.
//!
//! All failures (no window, not ready, missing fn, JS throw, non-string result)
//! collapse to `None` — a split wasm that isn't loaded must degrade gracefully,
//! never panic the page.
//!
//! Hydrate-only (needs the wasm-bindgen stack); enable the `interop` feature.

use wasm_bindgen::{JsCast, JsValue};

/// Get `window.__<window_key>` if it exists and is non-null.
fn surface(window_key: &str) -> Option<JsValue> {
    let win = web_sys::window()?;
    let s = js_sys::Reflect::get(&win, &JsValue::from_str(window_key)).ok()?;
    if s.is_undefined() || s.is_null() {
        None
    } else {
        Some(s)
    }
}

/// Is the split wasm at `window.__<window_key>` loaded (its `ready` flag true)?
pub fn is_ready(window_key: &str) -> bool {
    surface(window_key)
        .and_then(|s| js_sys::Reflect::get(&s, &JsValue::from_str("ready")).ok())
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Call `window.__<window_key>.<fn_name>(args...)` synchronously, returning the
/// string result. `None` if the wasm isn't ready, the fn is missing, it throws,
/// or returns a non-string / empty string. (Empty == "no value" by the split-
/// wasm convention, so the caller falls back.)
pub fn call(window_key: &str, fn_name: &str, args: &[&str]) -> Option<String> {
    let s = surface(window_key)?;
    // Gate on ready so we never call before init finishes.
    let ready = js_sys::Reflect::get(&s, &JsValue::from_str("ready"))
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !ready {
        return None;
    }
    let func: js_sys::Function = js_sys::Reflect::get(&s, &JsValue::from_str(fn_name))
        .ok()?
        .dyn_into()
        .ok()?;
    let js_args = js_sys::Array::new();
    for a in args {
        js_args.push(&JsValue::from_str(a));
    }
    let out = func.apply(&s, &js_args).ok()?;
    let out = out.as_string()?;
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Trigger `window.<loader_fn>()` (e.g. `__loadI18nPack`). The loader returns a
/// Promise; this awaits it so the split wasm is ready afterwards. No-op (Ok) if
/// the loader is absent — the caller then sees `is_ready == false` and falls
/// back. Returns whether the loader was found + invoked.
pub async fn load(loader_fn: &str) -> bool {
    use wasm_bindgen_futures::JsFuture;
    let Some(win) = web_sys::window() else {
        return false;
    };
    let Ok(loader) = js_sys::Reflect::get(&win, &JsValue::from_str(loader_fn)) else {
        return false;
    };
    let Ok(func) = loader.dyn_into::<js_sys::Function>() else {
        return false;
    };
    let Ok(promise) = func.call0(&win) else {
        return false;
    };
    if let Ok(p) = promise.dyn_into::<js_sys::Promise>() {
        let _ = JsFuture::from(p).await;
    }
    true
}

/// Convenience: ensure the split wasm is loaded (via `loader_fn`), then call
/// `window.__<window_key>.<fn_name>(args...)`. Returns the string result or an
/// error message describing where it failed.
pub async fn load_and_call(
    loader_fn: &str,
    window_key: &str,
    fn_name: &str,
    args: &[&str],
) -> Result<String, String> {
    if !load(loader_fn).await {
        return Err(format!("loader '{loader_fn}' not found"));
    }
    if !is_ready(window_key) {
        return Err(format!("'{window_key}' not ready after load"));
    }
    // `call` returns None on empty result; for an explicit call we still want
    // the (possibly empty) string, so re-do the call here returning the raw
    // string rather than the empty-as-None convenience.
    let win = web_sys::window().ok_or("no window")?;
    let s = js_sys::Reflect::get(&win, &JsValue::from_str(window_key))
        .map_err(|_| format!("{window_key} missing"))?;
    let func: js_sys::Function = js_sys::Reflect::get(&s, &JsValue::from_str(fn_name))
        .map_err(|_| format!("{fn_name} missing"))?
        .dyn_into()
        .map_err(|_| format!("{fn_name} not a function"))?;
    let js_args = js_sys::Array::new();
    for a in args {
        js_args.push(&JsValue::from_str(a));
    }
    let out = func
        .apply(&s, &js_args)
        .map_err(|_| format!("{fn_name} threw"))?;
    out.as_string().ok_or_else(|| "non-string result".into())
}
