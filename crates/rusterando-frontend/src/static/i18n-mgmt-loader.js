// i18n-mgmt-loader.js — thin per-wasm loader for the translation management
// wasm (admin only). Uses the generic makeWasmSplitLoader; registers
// window.__i18nMgmt. The Rust side (translations.rs via
// rusterando_wasm_split::interop::load_and_call) calls __i18nMgmt.gapReport.

(function () {
  if (typeof window.makeWasmSplitLoader !== "function") {
    console.error("[i18n-mgmt] wasm-split-loader.js not loaded before i18n-mgmt-loader.js");
    return;
  }
  window.makeWasmSplitLoader({
    windowKey: "__i18nMgmt",
    loaderKey: "__loadI18nMgmt",
    manifest: "/pkg/i18n-mgmt/manifest.json",
    importBase: "/pkg/i18n-mgmt/",
    exports: {
      gapReport: "gap_report",
      coveredLocales: "covered_locales",
    },
    loadedEvent: "i18n-mgmt-loaded",
  });
})();
