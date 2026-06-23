// wasm-split-loader.js — generic lazy loader for a hand-split wasm.
//
// The reusable JS half of the split-wasm pattern (Rust half:
// rusterando-wasm-split crate; shell half: scripts/build-wasm-split.sh). A
// thin per-wasm loader calls makeWasmSplitLoader({...}) to get a load function
// and a window-registered surface that the Rust/wasm side talks to via
// rusterando_wasm_split::interop.
//
// Surface registered at window[cfg.windowKey]:
//   ready            : bool
//   <each exported wasm-bindgen fn, snake_case>  — called as (…stringArgs) →
//                      String (the wasm-bindgen export, passed through verbatim)
//   plus a generic   call(fnName, ...args) passthrough.
//
// Hashing: the loader fetches manifest.json (small, cacheable) for the hashed
// JS filename, so it never hardcodes a hash. The hashed wasm is cached
// immutably — a returning visitor whose hash matches fetches nothing.
//
// cfg = {
//   windowKey:  "__i18nPack",                 // window.<windowKey> surface
//   loaderKey:  "__loadI18nPack",             // window.<loaderKey> = load fn
//   manifest:   "/pkg/i18n/manifest.json",
//   importBase: "/pkg/i18n/",                 // dir the hashed js lives in
//   exports:    {lookup:"lookup", menuLookup:"menu_lookup"},  // surface→wasm fn
//                // (an array ["a","b"] is shorthand for identity {a:"a",b:"b"})
//   loadedEvent:"i18n-pack-loaded",           // dispatched on window when ready
// }

function makeWasmSplitLoader(cfg) {
  let loaded = false;
  let loading = null;
  window[cfg.windowKey] = window[cfg.windowKey] || { ready: false };

  async function load() {
    if (loaded) return window[cfg.windowKey];
    if (loading) return loading;

    loading = (async () => {
      try {
        // Prefer the pack JS filename BAKED into window.__appBootstrap by the
        // SSR shell (cfg.bakedJs names which bootstrap key holds it). This
        // avoids a runtime manifest.json fetch — which can 404 if the prod
        // build re-hashed the manifest — so we always know exactly which file
        // (and content hash) to load. Fall back to fetching the manifest only
        // when the baked name is absent.
        let jsFile = null;
        var manifest = {};
        var boot = window.__appBootstrap || {};
        if (cfg.bakedJs && boot[cfg.bakedJs]) {
          jsFile = boot[cfg.bakedJs];
        } else {
          const mResp = await fetch(cfg.manifest, { cache: "force-cache" });
          if (!mResp.ok) throw new Error(cfg.windowKey + " manifest HTTP " + mResp.status);
          manifest = await mResp.json();
          jsFile = manifest.js;
        }

        const mod = await import(cfg.importBase + jsFile);
        await mod.default(); // init wasm-bindgen module

        // Build the surface: ready flag, a generic call(), and a wrapper per
        // declared export (so Rust can call them by their wasm-bindgen names).
        const surface = {
          ready: true,
          manifest: manifest,
          call: function (fnName) {
            const args = Array.prototype.slice.call(arguments, 1);
            try {
              return mod[fnName].apply(null, args) || "";
            } catch (e) {
              console.error("[" + cfg.windowKey + "] " + fnName + " failed:", e);
              return "";
            }
          },
        };
        // Normalise exports to a {surfaceName: wasmFnName} map.
        let exportMap = {};
        if (Array.isArray(cfg.exports)) {
          (cfg.exports || []).forEach(function (n) { exportMap[n] = n; });
        } else if (cfg.exports) {
          exportMap = cfg.exports;
        }
        Object.keys(exportMap).forEach(function (surfaceName) {
          const wasmFn = exportMap[surfaceName];
          surface[surfaceName] = function () {
            const args = Array.prototype.slice.call(arguments);
            try {
              return mod[wasmFn].apply(null, args) || "";
            } catch (e) {
              console.error("[" + cfg.windowKey + "] " + surfaceName + " failed:", e);
              return "";
            }
          };
        });
        window[cfg.windowKey] = surface;

        loaded = true;
        if (cfg.loadedEvent) {
          window.dispatchEvent(new CustomEvent(cfg.loadedEvent));
        }
        return surface;
      } catch (err) {
        console.error("[" + cfg.windowKey + "] load failed:", err);
        loading = null; // allow retry
        return window[cfg.windowKey];
      }
    })();
    return loading;
  }

  function unload() {
    loaded = false;
    loading = null;
    window[cfg.windowKey] = { ready: false };
    if (cfg.unloadedEvent) {
      window.dispatchEvent(new CustomEvent(cfg.unloadedEvent));
    }
  }

  window[cfg.loaderKey] = load;
  if (cfg.unloadKey) window[cfg.unloadKey] = unload;
  return { load: load, unload: unload };
}

window.makeWasmSplitLoader = makeWasmSplitLoader;
