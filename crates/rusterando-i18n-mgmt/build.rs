//! Build script: bake the i18n pack's MENU COVERAGE (which (locale, german)
//! pairs have a translation) into a sorted static slice via the shared
//! wasm-split codegen. Source of truth is the SAME file the pack ships
//! (`crates/rusterando-i18n-pack/generated/menu.json`), so the gap report can
//! never disagree with the pack. No JSON parser in the wasm.

use rusterando_wasm_split::build::Codegen;

fn main() {
    let menu = "../rusterando-i18n-pack/generated/menu.json";
    let mut g = Codegen::new();
    // COVERAGE: (locale, german) for every non-empty translation in the pack.
    g.key_table("COVERAGE", menu)
        .locales_union("COVERED_LOCALES", &[menu])
        .write("coverage.rs");
}
