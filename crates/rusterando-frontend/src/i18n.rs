//! i18n shim — single API for both German-only and multi-language builds.
//!
//! The crate ships with `feature = "i18n"` OFF by default. In that
//! state the `t!` macro looks up the German string from the embedded
//! `locales/de.json` at runtime via a once-built `HashMap` — zero
//! external deps, identical observable behaviour to hardcoded literals.
//!
//! With `--features i18n` the eight target languages (de/en/fr/it/
//! es/pt/ru/cn) are all bundled in. A locale context (`RwSignal<Locale>`)
//! drives the active language; `t!` reads it and looks up the matching
//! JSON, falling back to German for any key that hasn't been translated
//! yet. The locale is resolved on hydrate from:
//!     1. URL path prefix (`/en/`, `/it/`, …),
//!     2. the `dp_lang` cookie,
//!     3. the default (`Locale::De`).
//!
//! Why a hand-rolled shim instead of `leptos_i18n`?
//!   - The `leptos_i18n::t!` macro requires `t!(i18n, voucher.unknown)`
//!     (context + ident path). Our literal-key form composes more
//!     easily with feature-flagged builds.
//!   - We don't need the compile-time key validation in v1 — a tiny
//!     missing-key check on CI will do.
//!   - Davids' OSS forks that stay German-only pay zero compile cost
//!     for the 7 other locale files.
//!
//! Call sites: always `crate::t!("dotted.key")`. The macro is identical
//! between feature modes.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Locale enum — same in both feature modes so call sites that talk
// about the *type* (e.g. a future header dropdown) compile either way.
// In off-mode it's effectively a one-variant enum but we keep all eight
// so the type signature doesn't shift.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locale {
    De,
    En,
    Fr,
    It,
    Es,
    Pt,
    Ru,
    Cn,
}

impl Locale {
    pub const ALL: &'static [Locale] = &[
        Locale::De,
        Locale::En,
        Locale::Fr,
        Locale::It,
        Locale::Es,
        Locale::Pt,
        Locale::Ru,
        Locale::Cn,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Locale::De => "de",
            Locale::En => "en",
            Locale::Fr => "fr",
            Locale::It => "it",
            Locale::Es => "es",
            Locale::Pt => "pt",
            Locale::Ru => "ru",
            Locale::Cn => "cn",
        }
    }

    /// Human-readable label for a locale switcher (in the locale's own
    /// language so a visitor recognises "their" entry without
    /// guessing flag emoji).
    pub fn label(self) -> &'static str {
        match self {
            Locale::De => "Deutsch",
            Locale::En => "English",
            Locale::Fr => "Français",
            Locale::It => "Italiano",
            Locale::Es => "Español",
            Locale::Pt => "Português",
            Locale::Ru => "Русский",
            Locale::Cn => "中文",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "de" => Some(Locale::De),
            "en" => Some(Locale::En),
            "fr" => Some(Locale::Fr),
            "it" => Some(Locale::It),
            "es" => Some(Locale::Es),
            "pt" => Some(Locale::Pt),
            "ru" => Some(Locale::Ru),
            "cn" => Some(Locale::Cn),
            _ => None,
        }
    }

    pub const DEFAULT: Locale = Locale::De;
}

// ---------------------------------------------------------------------------
// Translation tables — built at first use from compile-time-embedded
// JSON.
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::OnceLock;

/// The hashed filename of the built translation-pack JS glue (e.g.
/// `rusterando_i18n_pack-<hash>.js`), read from `pkg/i18n/manifest.json` ONCE at
/// server boot and baked into `window.__appBootstrap.i18n_pack_js` by the SSR
/// shell. The client loader then `import()`s the pack DIRECTLY by this name,
/// with no runtime `manifest.json` fetch — so a stale/hashed/missing manifest
/// can't break loading, and the client knows exactly which file (and content
/// hash) to expect. Empty when no pack is built.
#[cfg(feature = "ssr")]
#[derive(Clone, Default)]
pub struct I18nPackInfo(pub std::sync::Arc<str>);

#[cfg(feature = "ssr")]
impl I18nPackInfo {
    pub fn get(&self) -> &str {
        &self.0
    }
}

/// Dot-flatten `{"a": {"b": "x"}}` to `("a.b", "x")` so call sites
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
        _ => {}
    }
}

fn build_table(json: &str, label: &str) -> HashMap<String, String> {
    let v: serde_json::Value = serde_json::from_str(json)
        .unwrap_or_else(|e| panic!("locales/{label}.json must be valid JSON: {e}"));
    let mut out = HashMap::new();
    flatten(&mut out, String::new(), &v);
    out
}

// German is always bundled. The off-mode build embeds only this one.
static DE_JSON: &str = include_str!("../locales/de.json");
static DE_TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();

fn de_table() -> &'static HashMap<String, String> {
    DE_TABLE.get_or_init(|| build_table(DE_JSON, "de"))
}

// NOTE: the 7 non-German chrome tables NO LONGER ship in this bundle. They live
// in the separate, on-demand translation `.wasm` pack (crate
// `rusterando-i18n-pack`, built by scripts/build-i18n-pack.sh) and are fetched
// + cached by hash in the browser only when a non-German locale is active. The
// server (SSR) and the pre-pack hydrate render both fall back to German here, so
// SSR and the initial hydrate DOM are byte-identical (no hydration mismatch);
// the pack swap happens reactively AFTER hydration. See
// feedback_i18n_db_first_wasm_overlay.

// ---------------------------------------------------------------------------
// Translation-pack bridge (client only). The loaded pack publishes its lookups
// on `window.__i18nPack` (see static/i18n-loader.js). On SSR there is no pack —
// these always return None, so SSR renders German.
// ---------------------------------------------------------------------------

/// The window key the i18n pack loader registers its surface under (see
/// static/i18n-loader.js → `window.__i18nPack`).
#[cfg(feature = "hydrate")]
const PACK_KEY: &str = "__i18nPack";

/// Chrome string from the loaded pack for `loc`/`key`. `None` on SSR, when no
/// pack is loaded, or when the pack lacks the key (caller falls back to German).
/// Uses the shared split-wasm interop (`rusterando_wasm_split::interop`).
#[cfg(feature = "hydrate")]
fn pack_lookup(loc: Locale, key: &str) -> Option<String> {
    rusterando_wasm_split::interop::call(PACK_KEY, "lookup", &[loc.code(), key])
}
#[cfg(not(feature = "hydrate"))]
fn pack_lookup(_loc: Locale, _key: &str) -> Option<String> {
    None
}

/// Menu translation (keyed by German source) from the loaded pack. Same
/// SSR/None semantics as `pack_lookup`.
#[cfg(feature = "hydrate")]
fn pack_menu_lookup(loc: Locale, german_source: &str) -> Option<String> {
    rusterando_wasm_split::interop::call(PACK_KEY, "menuLookup", &[loc.code(), german_source])
}
#[cfg(not(feature = "hydrate"))]
fn pack_menu_lookup(_loc: Locale, _german_source: &str) -> Option<String> {
    None
}

// ---------------------------------------------------------------------------
// Active-locale context. Always available; defaults to Locale::De when
// no context has been provided. The locale signal is set in App() based
// on the URL path; SSR + hydrate compute it identically.
// ---------------------------------------------------------------------------

use leptos::prelude::*;

#[derive(Debug, Clone, Copy)]
pub struct LocaleCtx(pub RwSignal<Locale>);

/// Read the current locale from context. Falls back to DE if no
/// context has been provided (defensive — every App() should call
/// `provide_locale_ctx`).
///
/// Uses `get_untracked()` so callers that just want a snapshot ("what
/// locale is this request / this render in?") don't register a reactive
/// dependency. Components that need to *react* to locale switches
/// should read `LocaleCtx`'s signal directly inside a `move ||` closure.
pub fn current_locale() -> Locale {
    use_context::<LocaleCtx>()
        .map(|c| c.0.get_untracked())
        .unwrap_or(Locale::DEFAULT)
}

/// Install the locale context. Pass the initial locale resolved
/// from the request path. Returns the signal so a locale switcher
/// can write to it.
pub fn provide_locale_ctx(initial: Locale) -> RwSignal<Locale> {
    let sig = RwSignal::new(initial);
    provide_context(LocaleCtx(sig));
    sig
}

/// Reactive "translation generation" counter. Bumped when the translation pack
/// finishes loading (or unloads), so every `t!()` / `t_menu()` call site that
/// sits inside a reactive view re-renders and picks up the now-available
/// translations. Read (tracked) inside `t()`/`t_menu()`; written by the pack
/// loader bridge (`bump_pack_generation`). Context-provided so it's per-app.
#[derive(Debug, Clone, Copy)]
pub struct PackGen(pub RwSignal<u32>);

/// Install the pack-generation signal. Call once in `App()` alongside
/// `provide_locale_ctx`. Returns the signal so the loader hookup can bump it.
pub fn provide_pack_gen() -> RwSignal<u32> {
    let sig = RwSignal::new(0u32);
    provide_context(PackGen(sig));
    sig
}

/// Register a reactive dependency on the pack generation (no-op outside a
/// reactive context, and when the context isn't installed). Called at the top
/// of `t()`/`t_menu()` so a pack load re-renders translated text.
fn track_pack_gen() {
    if let Some(PackGen(sig)) = use_context::<PackGen>() {
        // `.get()` registers the dependency; the value itself is unused.
        let _ = sig.get();
    }
}

/// Bump the pack generation so all `t!()` views re-render. Called by the loader
/// bridge after the pack's `i18n-pack-loaded` / `-unloaded` event.
#[cfg(feature = "hydrate")]
pub fn bump_pack_generation() {
    if let Some(PackGen(sig)) = use_context::<PackGen>() {
        sig.update(|n| *n = n.wrapping_add(1));
    }
}

/// Hydrate-side: kick off the translation-pack load and bump the generation
/// when it arrives. Idempotent-ish (the loader de-dupes concurrent loads). Wires
/// a one-shot `i18n-pack-loaded` listener to `bump_pack_generation` so views
/// re-render, and (defensively) bumps immediately if the pack is already ready
/// (e.g. a previous page on this SPA session already loaded it).
#[cfg(feature = "hydrate")]
pub fn hydrate_load_pack_on_ready() {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    let Some(win) = web_sys::window() else {
        return;
    };

    // Already loaded (SPA navigation within the session) → bump now so this
    // page renders translated immediately.
    if rusterando_wasm_split::interop::is_ready(PACK_KEY) {
        bump_pack_generation();
    }

    // Listen for the loader's completion event → bump so views swap to the
    // translated strings. We leak the closure (one per page boot) so it outlives
    // this fn — cheap and bounded (a page only boots App once).
    let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
        bump_pack_generation();
    });
    let _ = win.add_event_listener_with_callback("i18n-pack-loaded", cb.as_ref().unchecked_ref());
    let _ = win.add_event_listener_with_callback("i18n-pack-unloaded", cb.as_ref().unchecked_ref());
    cb.forget();

    // Trigger the load via the shared interop (calls window.__loadI18nPack()).
    // Fire-and-forget; the event listener above handles the re-render on ready.
    leptos::task::spawn_local(async {
        let _ = rusterando_wasm_split::interop::load("__loadI18nPack").await;
    });
}

/// Server fn exposing the runtime i18n toggle to SSR + hydrate.
/// Reads `app_settings.i18n_enabled` via the I18nHandle that the
/// server crate provides at boot. Defaults to `false` if the handle
/// is missing (older binaries / fresh DB).
#[server(name = GetI18nEnabled, prefix = "/api", endpoint = "i18n_enabled")]
pub async fn get_i18n_enabled() -> Result<bool, ServerFnError> {
    Ok(use_context::<crate::pages::settings::I18nHandle>()
        .map(|h| h.get())
        .unwrap_or(false))
}

/// Build a SQL fragment that resolves the right per-locale column with
/// a fallback to the canonical DE column. Used by menu / extras / option
/// loaders to pick `name_en` (etc.) when the active locale is non-default.
///
///   coalesce_col("name", "menu") → "name"             (DE, no-op)
///   coalesce_col("name", "menu") → "COALESCE(name_en, name)"  (EN)
///
/// `_table_alias` is reserved for a future variant that prefixes the
/// column with an alias (e.g. `m.name_en`); v1 doesn't need it because
/// every translatable column we have lives in an un-aliased FROM.
pub fn coalesce_col(base: &str, _table_alias: &str) -> String {
    let loc = current_locale();
    if loc == Locale::DEFAULT {
        base.to_string()
    } else {
        format!("COALESCE({base}_{lang}, {base})", lang = loc.code())
    }
}

// ---------------------------------------------------------------------------
// Translation lookup.
// ---------------------------------------------------------------------------

/// Locale-aware chrome lookup. Resolution order (DB-first model — chrome has no
/// DB layer, so it's pack-then-German):
///   1. German locale → the bundled `de_table()` (both SSR + hydrate).
///   2. Non-German → the loaded translation pack (client only); if the pack
///      isn't loaded yet (SSR, or before the fetch completes) → German.
///   3. Missing key everywhere → `⚠key` to make typos loud.
///
/// Because the pack is client-only, SSR and the initial hydrate render both
/// produce German for a non-German locale — identical DOM, no hydration
/// mismatch. The translated text appears after the pack loads (the locale
/// switch / boot triggers the fetch and a re-render).
pub fn t(key: &str) -> String {
    track_pack_gen(); // re-render this view when the pack loads/unloads
    let loc = current_locale();
    if loc != Locale::DEFAULT {
        if let Some(v) = pack_lookup(loc, key) {
            return v;
        }
    }
    // German base (the always-bundled fallback), then key-warning.
    de_table()
        .get(key)
        .cloned()
        .unwrap_or_else(|| format!("⚠{key}"))
}

/// Menu/shop-content resolver (DB-first). `db_value` is what the server already
/// resolved via `coalesce_col` SQL — i.e. the DB `_<lang>` cell if filled, else
/// the German base. `german_source` is the canonical German value for the same
/// field (so the pack can be keyed by it).
///
/// Order: if the DB already gave a real translation (`db_value != german_source`,
/// i.e. the `_<lang>` override cell was filled) → keep it (the DB always wins).
/// Otherwise the cell was empty → ask the pack; if it has the string, use it;
/// else fall back to the German `db_value`. On SSR / no pack, returns `db_value`
/// unchanged, so first paint is the DB value and the pack swaps in client-side.
pub fn t_menu(db_value: &str, german_source: &str) -> String {
    track_pack_gen(); // re-render when the pack loads/unloads
    let loc = current_locale();
    if loc == Locale::DEFAULT {
        return db_value.to_string();
    }
    // DB override present → it wins (DB-first).
    if db_value != german_source {
        return db_value.to_string();
    }
    // Empty cell → pack fills it; else German.
    pack_menu_lookup(loc, german_source).unwrap_or_else(|| db_value.to_string())
}

// ---------------------------------------------------------------------------
// Locale resolution from path / cookie / Accept-Language. Only the
// path part runs in both SSR and hydrate; the cookie + Accept-Language
// pieces are SSR-only (they need the request).
// ---------------------------------------------------------------------------

/// Strip a known locale prefix from a path. Returns (locale, rest).
/// "/en/menu" → (Some(Locale::En), "/menu"). "/menu" → (None, "/menu").
/// Unknown two-letter prefixes pass through unchanged so a future
/// /es-mx style locale doesn't accidentally match.
pub fn split_locale_prefix(path: &str) -> (Option<Locale>, &str) {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    let (first, rest) = match trimmed.split_once('/') {
        Some((a, b)) => (a, b),
        None => (trimmed, ""),
    };
    match Locale::parse(first) {
        Some(loc) => {
            // Re-attach a leading slash to the rest for callers that
            // pass the result back to the router.
            let rest_with_slash = if rest.is_empty() {
                "/"
            } else {
                &path[first.len() + 1..]
            };
            (Some(loc), rest_with_slash)
        }
        None => (None, path),
    }
}

/// Build a path with a (different) locale prefix. The default locale
/// is always served without a prefix to stay SEO-canonical. An
/// existing prefix is always stripped first so the swap is correct
/// regardless of starting state.
pub fn with_locale_prefix(loc: Locale, path: &str) -> String {
    let p = if path.starts_with('/') { path } else { "/" };
    let (_, rest) = split_locale_prefix(p);
    if loc == Locale::DEFAULT {
        rest.to_string()
    } else {
        format!("/{}{}", loc.code(), rest)
    }
}

// ---------------------------------------------------------------------------
// Macro shim. Both feature modes accept the same `t!("dotted.key")`.
// ---------------------------------------------------------------------------

#[macro_export]
macro_rules! t {
    ($key:literal) => {
        $crate::i18n::t($key)
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn de_table_loads() {
        let t = de_table();
        // A known key from de.json.
        assert_eq!(
            t.get("errors.page_not_found").map(String::as_str),
            Some("Seite nicht gefunden.")
        );
    }

    #[test]
    fn split_prefix_default() {
        assert_eq!(split_locale_prefix("/menu"), (None, "/menu"));
        assert_eq!(split_locale_prefix("/"), (None, "/"));
        assert_eq!(split_locale_prefix(""), (None, ""));
    }

    #[test]
    fn split_prefix_known_locale() {
        let (loc, rest) = split_locale_prefix("/en/menu");
        assert_eq!(loc, Some(Locale::En));
        assert_eq!(rest, "/menu");

        let (loc, rest) = split_locale_prefix("/it");
        assert_eq!(loc, Some(Locale::It));
        assert_eq!(rest, "/");
    }

    #[test]
    fn split_prefix_unknown_pass_through() {
        let (loc, rest) = split_locale_prefix("/admin/menu");
        assert_eq!(loc, None);
        assert_eq!(rest, "/admin/menu");
    }

    #[test]
    fn with_prefix_default_strips() {
        assert_eq!(with_locale_prefix(Locale::De, "/menu"), "/menu");
        assert_eq!(with_locale_prefix(Locale::De, "/en/menu"), "/menu");
    }

    #[test]
    fn with_prefix_non_default_adds() {
        assert_eq!(with_locale_prefix(Locale::En, "/menu"), "/en/menu");
        assert_eq!(with_locale_prefix(Locale::It, "/en/menu"), "/it/menu");
    }
}
