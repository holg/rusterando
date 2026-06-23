// i18n-loader.js — thin per-wasm loader for the translation pack.
//
// Uses the generic makeWasmSplitLoader (wasm-split-loader.js, loaded first) to
// fetch + instantiate the pack and register window.__i18nPack. The Rust side
// (i18n.rs via rusterando_wasm_split::interop) calls __i18nPack.lookup /
// .menuLookup; App() triggers window.__loadI18nPack() on hydrate for a
// non-German locale, and re-renders on the i18n-pack-loaded event.
//
// DB-first reminder: the pack only fills empty cells; the DB always wins.

(function () {
  if (typeof window.makeWasmSplitLoader !== "function") {
    console.error("[i18n] wasm-split-loader.js not loaded before i18n-loader.js");
    return;
  }
  window.makeWasmSplitLoader({
    windowKey: "__i18nPack",
    loaderKey: "__loadI18nPack",
    unloadKey: "__unloadI18nPack",
    manifest: "/pkg/i18n/manifest.json",
    // The pack JS filename is baked into window.__appBootstrap.i18n_pack_js by
    // the SSR shell — used directly, no manifest fetch. (manifest is the
    // fallback only.)
    bakedJs: "i18n_pack_js",
    importBase: "/pkg/i18n/",
    // surface name → wasm-bindgen export name. The Rust interop calls
    // "lookup" and "menuLookup".
    exports: {
      lookup: "lookup",
      menuLookup: "menu_lookup",
      packLocales: "pack_locales",
      packHash: "pack_hash",
    },
    loadedEvent: "i18n-pack-loaded",
    unloadedEvent: "i18n-pack-unloaded",
  });
})();
