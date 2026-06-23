//! Build script: bake the committed `generated/{chrome,menu}.json` translation
//! blobs into sorted `&'static [..]` slices via the shared wasm-split codegen,
//! so the runtime wasm carries NO JSON parser — just static data + binary
//! search. See rusterando_wasm_split::build.

use rusterando_wasm_split::build::Codegen;

fn main() {
    let mut g = Codegen::new();
    // CHROME / MENU: (locale, key, value) sorted by (locale, key).
    g.full_table("CHROME", "generated/chrome.json")
        .full_table("MENU", "generated/menu.json")
        .locales_union("LOCALES", &["generated/chrome.json", "generated/menu.json"])
        .literal_str("PACK_HASH", "generated/hash.txt")
        .write("i18n_data.rs");
}
