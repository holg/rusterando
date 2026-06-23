//! asset-manifest — checksum the built WASM/JS bundle and emit a manifest.
//!
//! After cargo-leptos / wasm-bindgen produces `target/site/pkg/`, this scans
//! the dir for the `*.wasm` and `*.js` bundle files, computes their SHA-256
//! via `rusterando_shared::assets`, and writes:
//!   - `<pkg>/manifest.json`         (logical name → {sha256, file})
//!   - `<pkg>/<file>.sha256`         (one per asset, `<hex>  <file>` format)
//!
//! Reuses the shared hashing so the server and tooling agree on the format.
//!
//! Usage:
//!   asset-manifest [PKG_DIR]        (default: target/site/pkg)

use std::path::{Path, PathBuf};

use rusterando_shared::assets::AssetManifest;

fn main() -> std::io::Result<()> {
    let pkg = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/site/pkg"));

    if !pkg.is_dir() {
        eprintln!("pkg dir not found: {}", pkg.display());
        std::process::exit(1);
    }

    // Pick the primary bundle files. cargo-leptos emits `<name>.<hash>.wasm`
    // and `<name>.<hash>.js`; wasm-bindgen `<name>_bg.wasm` + `<name>.js`.
    // We take the largest .wasm and the matching .js (skip .d.ts, loaders,
    // already-compressed .br).
    let wasm = pick(&pkg, |n| n.ends_with(".wasm") && !n.ends_with(".d.ts"))?;
    let js = pick(&pkg, |n| {
        n.ends_with(".js") && !n.ends_with(".d.ts") && !n.contains("loader")
    })?;

    let mut m = AssetManifest::new();
    if let Some(w) = &wasm {
        let file = w.file_name().unwrap().to_string_lossy().into_owned();
        m.add_file("wasm", w, file)?;
    }
    if let Some(j) = &js {
        let file = j.file_name().unwrap().to_string_lossy().into_owned();
        m.add_file("js", j, file)?;
    }

    if m.entries.is_empty() {
        eprintln!("no .wasm/.js found in {}", pkg.display());
        std::process::exit(1);
    }

    m.write(pkg.join("manifest.json"))?;
    m.write_checksum_files(&pkg)?;

    for (name, e) in &m.entries {
        println!("{name:>4}  {}  {}", &e.sha256[..16], e.file);
    }
    println!("wrote {}/manifest.json + .sha256 files", pkg.display());
    Ok(())
}

/// Return the largest file in `dir` matching `pred` (largest = the real
/// bundle, not a stub/source-map sharing the extension).
fn pick(dir: &Path, pred: impl Fn(&str) -> bool) -> std::io::Result<Option<PathBuf>> {
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !pred(&name) {
            continue;
        }
        let len = entry.metadata()?.len();
        if best.as_ref().map(|(l, _)| len > *l).unwrap_or(true) {
            best = Some((len, entry.path()));
        }
    }
    Ok(best.map(|(_, p)| p))
}
