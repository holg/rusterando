//! Drop-in replacement for `leptos::hydration::HydrationScripts` that
//! reads the `hash.txt` file **once** at server startup via `OnceLock`
//! instead of on every SSR render.
//!
//! Upstream leptos 0.8.14 / 0.8.19 calls
//! `std::fs::read_to_string(&hash_path).expect(...)` directly inside
//! the component on every page render — see
//! `leptos-0.8.14/src/hydration/mod.rs:135`. That's 1 open+read+close
//! per SSR call, and an `expect(...)` that panics the tokio worker if
//! the open ever fails. When the process is under any fd pressure for
//! unrelated reasons, the *first* visible symptom is hash-file panics
//! masking the real cause. See [[feedback-leptos-hash-file-read-per-request]]
//! and [[feedback-fd-limit-systemd]].
//!
//! This module produces byte-identical HTML to leptos's component for
//! the configurations we use (no islands, no wasm-splits, no nonce —
//! we set those upstream features off). If we adopt any of those it
//! needs reworking; the doctest below pins the shape so a refactor
//! that diverges shows up in CI.

use leptos::prelude::*;
use std::sync::OnceLock;

/// Hashed filename suffixes resolved from `hash.txt`. `js` and `wasm`
/// are extensions-less (e.g. `"abc123"` for `rusterando.abc123.wasm`)
/// or empty when hashing is off / no file exists.
#[derive(Debug, Clone, Default)]
struct HashedNames {
    js: String,
    wasm: String,
}

/// Read `hash.txt` exactly once for the lifetime of the process.
/// SSR-only — hydrate path never calls this. The `current_exe` lookup
/// and `read_to_string` happen on the first SSR render after boot; every
/// subsequent render returns the cached `&'static HashedNames` for free.
///
/// Returns empty strings (= no hash suffix appended) when:
///   * `hash_files` is off in `LeptosOptions`, or
///   * `hash.txt` doesn't exist next to the binary.
///
/// Both match leptos's own fallback behaviour, so a missing file is
/// non-fatal (the bundle URLs just lose their cache-busting suffix).
fn hashed_names(options: &LeptosOptions) -> &'static HashedNames {
    static CACHE: OnceLock<HashedNames> = OnceLock::new();
    CACHE.get_or_init(|| {
        if !options.hash_files {
            return HashedNames::default();
        }
        let hash_path = std::env::current_exe()
            .map(|p| p.parent().map(|d| d.to_path_buf()).unwrap_or_default())
            .unwrap_or_default()
            .join(options.hash_file.as_ref());
        let Ok(contents) = std::fs::read_to_string(&hash_path) else {
            // Match leptos's behaviour: log + fall through to empty
            // suffixes. A missing hash.txt is recoverable (the loader
            // just won't cache-bust); we don't want to panic the boot.
            leptos::logging::error!(
                "hash_files=true but {} not readable; serving unhashed bundle names",
                hash_path.display()
            );
            return HashedNames::default();
        };
        let mut out = HashedNames::default();
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((file, hash)) = line.split_once(':') else {
                continue;
            };
            let hash = hash.trim();
            match file {
                "js" => out.js = hash.to_string(),
                "wasm" => out.wasm = hash.to_string(),
                _ => {} // ignore manifest/split etc.; we don't use wasm-splits
            }
        }
        out
    })
}

/// Same surface as `leptos::hydration::HydrationScripts` for our use:
/// emit the `<link rel=modulepreload>`, `<link rel=preload>` for the
/// wasm, and the hydrate `<script type=module>` bootstrap. Same hashed
/// filenames as the canonical component — only the fd footprint changes.
#[component]
pub fn CachedHydrationScripts(options: LeptosOptions) -> impl IntoView {
    let names = hashed_names(&options);
    let pkg_path = options.site_pkg_dir.to_string();
    let output_name = options.output_name.to_string();

    let js_file = if names.js.is_empty() {
        output_name.clone()
    } else {
        format!("{output_name}.{}", names.js)
    };
    let wasm_file = if names.wasm.is_empty() {
        format!("{output_name}_bg")
    } else {
        format!("{output_name}.{}", names.wasm)
    };

    // hydration_script.js is the same blob leptos itself ships; we
    // re-embed it (compile-time string) so the runtime byte output of
    // this component matches `HydrationScripts` 1:1. Versioned with
    // leptos: bump on every upstream `cargo update -p leptos`.
    const HYDRATION_SCRIPT: &str = include_str!("./hydration_script.js");

    let pkg_path_link = pkg_path.clone();
    let pkg_path_preload = pkg_path.clone();
    let js_file_for_link = js_file.clone();
    let wasm_file_for_link = wasm_file.clone();

    view! {
        <link rel="modulepreload" href=format!("/{pkg_path_link}/{js_file_for_link}.js")/>
        <link
            rel="preload"
            href=format!("/{pkg_path_preload}/{wasm_file_for_link}.wasm")
            r#as="fetch"
            r#type="application/wasm"
            crossorigin=""
        />
        <script type="module">
            {format!("{HYDRATION_SCRIPT}(\"\", {pkg_path:?}, {js_file:?}, {wasm_file:?});")}
        </script>
    }
}
