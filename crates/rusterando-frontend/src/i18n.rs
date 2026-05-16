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

// The other 7 are only embedded when the `i18n` feature is on, so
// off-builds don't carry extra bytes.
#[cfg(feature = "i18n")]
mod tables_on {
    use super::*;
    static EN_JSON: &str = include_str!("../locales/en.json");
    static FR_JSON: &str = include_str!("../locales/fr.json");
    static IT_JSON: &str = include_str!("../locales/it.json");
    static ES_JSON: &str = include_str!("../locales/es.json");
    static PT_JSON: &str = include_str!("../locales/pt.json");
    static RU_JSON: &str = include_str!("../locales/ru.json");
    static CN_JSON: &str = include_str!("../locales/cn.json");

    static EN_TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    static FR_TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    static IT_TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    static ES_TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    static PT_TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    static RU_TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    static CN_TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();

    pub fn table_for(loc: Locale) -> Option<&'static HashMap<String, String>> {
        match loc {
            Locale::De => Some(de_table()),
            Locale::En => Some(EN_TABLE.get_or_init(|| build_table(EN_JSON, "en"))),
            Locale::Fr => Some(FR_TABLE.get_or_init(|| build_table(FR_JSON, "fr"))),
            Locale::It => Some(IT_TABLE.get_or_init(|| build_table(IT_JSON, "it"))),
            Locale::Es => Some(ES_TABLE.get_or_init(|| build_table(ES_JSON, "es"))),
            Locale::Pt => Some(PT_TABLE.get_or_init(|| build_table(PT_JSON, "pt"))),
            Locale::Ru => Some(RU_TABLE.get_or_init(|| build_table(RU_JSON, "ru"))),
            Locale::Cn => Some(CN_TABLE.get_or_init(|| build_table(CN_JSON, "cn"))),
        }
    }
}

// ---------------------------------------------------------------------------
// Active-locale context. Always available so server fns / SQL loaders
// can call `current_locale()` without `#[cfg]` gating — in off-builds
// it just always returns DEFAULT.
// ---------------------------------------------------------------------------

#[cfg(feature = "i18n")]
pub use ctx_on::*;

#[cfg(feature = "i18n")]
mod ctx_on {
    use super::Locale;
    use leptos::prelude::*;

    #[derive(Debug, Clone, Copy)]
    pub struct LocaleCtx(pub RwSignal<Locale>);

    /// Read the current locale from context. Falls back to DE if no
    /// context has been provided (defensive — every App() that uses
    /// the i18n feature must call provide_locale_ctx).
    pub fn current_locale() -> Locale {
        use_context::<LocaleCtx>()
            .map(|c| c.0.get())
            .unwrap_or(Locale::DEFAULT)
    }

    /// Install the locale context. Pass the initial locale resolved
    /// from the request path / cookie / default. Returns the signal
    /// so a locale switcher can write to it.
    pub fn provide_locale_ctx(initial: Locale) -> RwSignal<Locale> {
        let sig = RwSignal::new(initial);
        provide_context(LocaleCtx(sig));
        sig
    }
}

/// Stub for off-builds so callers can use `crate::i18n::current_locale()`
/// without a `#[cfg]` shim. Always returns `Locale::De`.
#[cfg(not(feature = "i18n"))]
pub fn current_locale() -> Locale {
    Locale::DEFAULT
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
// Translation lookup. Two implementations selected by feature flag.
// ---------------------------------------------------------------------------

/// Translation lookup. Missing keys return `⚠key` so a typo is loud
/// in the UI instead of silent (matches leptos_i18n's dev-mode hint).
#[cfg(not(feature = "i18n"))]
pub fn t(key: &str) -> String {
    de_table()
        .get(key)
        .cloned()
        .unwrap_or_else(|| format!("⚠{key}"))
}

/// Locale-aware lookup. Reads the active locale from context, falls
/// back to German for keys missing in the active locale's table.
#[cfg(feature = "i18n")]
pub fn t(key: &str) -> String {
    let loc = current_locale();
    if let Some(tab) = tables_on::table_for(loc) {
        if let Some(v) = tab.get(key) {
            return v.clone();
        }
    }
    // Fallback chain: active locale → DE → key-warning.
    de_table()
        .get(key)
        .cloned()
        .unwrap_or_else(|| format!("⚠{key}"))
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
