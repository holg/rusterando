//! i18n shim — single API for both German-only and multi-language builds.
//!
//! The crate ships with `feature = "i18n"` OFF by default. In that
//! state the `t!` macro looks up the German string from the embedded
//! `locales/de.json` at runtime via a once-built `HashMap` — zero
//! external deps, identical observable behaviour to hardcoded literals.
//!
//! With `--features i18n` the same `t!` calls switch to the full
//! leptos_i18n machinery: URL-prefix routing (/en/menu, /it/menu),
//! cookie-backed locale memory, runtime locale switch via a header
//! dropdown. The eight target languages (de/en/fr/it/es/pt/ru/cn)
//! live in `locales/<lang>.json` and are bundled into the wasm.
//!
//! Why the shim instead of just always using leptos_i18n?
//!   - Davids' production build stays bit-identical to today.
//!   - The OSS forks that want only German pay zero compile-time
//!     cost for the 7 other locale files + the locale-switcher UI.
//!   - The `t!` call sites are stable across both modes, so a future
//!     decision to flip the default doesn't touch every view! block.

#[cfg(not(feature = "i18n"))]
mod de_only {
    //! Compile-time embedded German translations. The de.json file
    //! is `include_str!`'d so the binary carries no filesystem
    //! dependency; the HashMap is built lazily on first `t()` call
    //! and stays for the process lifetime.
    use std::collections::HashMap;
    use std::sync::OnceLock;

    static DE_JSON: &str = include_str!("../locales/de.json");
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();

    fn table() -> &'static HashMap<String, String> {
        TABLE.get_or_init(|| {
            let v: serde_json::Value = serde_json::from_str(DE_JSON)
                .expect("locales/de.json must be valid JSON at build time");
            let mut out = HashMap::new();
            flatten(&mut out, String::new(), &v);
            out
        })
    }

    /// Dot-flatten `{"a": {"b": "x"}}` to `("a.b", "x")` so callers
    /// can use compact `t!("voucher.unknown")` keys.
    fn flatten(out: &mut HashMap<String, String>, prefix: String, v: &serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, child) in map {
                    let next = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    flatten(out, next, child);
                }
            }
            serde_json::Value::String(s) => {
                out.insert(prefix, s.clone());
            }
            _ => { /* skip non-string leaves */ }
        }
    }

    /// Translation lookup. Missing keys return the key itself so a
    /// typo is loud in the UI instead of silent. (Matches the
    /// behaviour leptos_i18n gives in dev mode.)
    pub fn t(key: &str) -> String {
        table()
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("⚠{key}"))
    }
}

#[cfg(feature = "i18n")]
mod i18n_on {
    //! Wires up leptos_i18n. The `i18n.toml` config at the crate root
    //! tells the proc-macro which locales exist; this re-export gives
    //! the rest of the crate a single import path.
    pub use leptos_i18n::*;
    // TODO when feature lands: `leptos_i18n::load_locales!();` plus
    // a `provide_i18n_context()` call in App() driven off the URL prefix
    // + cookie.
}

#[cfg(not(feature = "i18n"))]
pub use de_only::t;

#[cfg(feature = "i18n")]
pub use i18n_on::*;

#[cfg(feature = "i18n")]
pub fn t(key: &str) -> String {
    // Placeholder until the leptos_i18n wiring lands. For now we
    // also fall back to the German bundle so `crate::t!` resolves
    // identically in both feature modes; the URL-prefix + cookie
    // locale switch will replace this body in a follow-up commit
    // without changing any call site.
    let _ = key;
    de_only_for_on::t(key)
}

#[cfg(feature = "i18n")]
mod de_only_for_on {
    // Re-export of the de_only shim — kept callable while the i18n
    // feature is still being wired up. Once leptos_i18n is fully
    // integrated this module goes away.
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static DE_JSON: &str = include_str!("../locales/de.json");
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    fn table() -> &'static HashMap<String, String> {
        TABLE.get_or_init(|| {
            let v: serde_json::Value = serde_json::from_str(DE_JSON).expect("de.json");
            let mut out = HashMap::new();
            flatten(&mut out, String::new(), &v);
            out
        })
    }
    fn flatten(out: &mut HashMap<String, String>, prefix: String, v: &serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, child) in map {
                    let next = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    flatten(out, next, child);
                }
            }
            serde_json::Value::String(s) => {
                out.insert(prefix, s.clone());
            }
            _ => {}
        }
    }
    pub fn t(key: &str) -> String {
        table()
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("⚠{key}"))
    }
}

/// Macro shim. Both feature modes accept the same `t!("dotted.key")`
/// form. Off → de_only::t() lookup. On → i18n::t() (currently the
/// same; locale switching to land in a follow-up commit).
#[macro_export]
macro_rules! t {
    ($key:literal) => {
        $crate::i18n::t($key)
    };
}
