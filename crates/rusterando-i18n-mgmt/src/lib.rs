//! Management `.wasm` — per-shop translation gap reporting.
//!
//! Given a shop's German menu phrases and the locales it cares about, reports
//! which phrases have NO translation in the i18n pack yet — a per-shop worklist
//! of what still needs translating. Loaded only in the admin UI (see the
//! /admin/translations page + the mgmt loader).
//!
//! Data model: the pack's MENU COVERAGE (which (locale, german) pairs have a
//! translation) is baked into a sorted static slice by build.rs, read from the
//! SAME generated/menu.json the pack ships — so the gap report can never
//! disagree with what the pack actually contains. Lookups are binary search; no
//! JSON parser in the wasm.
//!
//! The gap COMPUTATION lives in `rusterando-i18n-core` (shared with the i18n
//! TUI, so the two can't drift). This crate only bakes the coverage data and
//! adapts the string in/out interface JS needs.

use rusterando_i18n_core::SliceCoverage;
use wasm_bindgen::prelude::*;

// COVERAGE: &[(locale, german)] sorted.  COVERED_LOCALES: &[&str].
include!(concat!(env!("OUT_DIR"), "/coverage.rs"));

/// The pack's menu coverage as a `Coverage`, backed by the build-baked sorted
/// `COVERAGE` slice (binary search).
fn coverage() -> SliceCoverage<'static> {
    SliceCoverage(COVERAGE)
}

// ---------------------------------------------------------------------------
// wasm-bindgen surface.
// ---------------------------------------------------------------------------

/// Per-shop gap report.
///   `phrases_nl` — the shop's German phrases, one per line (the menu item
///                  names + descriptions + option labels the shop uses).
///   `locales_csv` — comma-separated locales to check (e.g. "en,fr,it"). Empty
///                   → check all locales the pack covers.
/// Returns JSON: `{ "locale": ["missing phrase", …], … , "_summary": {…} }`.
/// A locale with an empty array is fully translated for this shop.
#[wasm_bindgen]
pub fn gap_report(phrases_nl: &str, locales_csv: &str) -> String {
    let phrases: Vec<&str> = phrases_nl.split('\n').collect();
    let locales: Vec<&str> = if locales_csv.trim().is_empty() {
        COVERED_LOCALES.to_vec()
    } else {
        locales_csv
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect()
    };
    rusterando_i18n_core::gap_report_json(&coverage(), &phrases, &locales)
}

/// The locales the pack covers (JSON array), so the admin UI can default the
/// locale checkboxes to exactly what's available.
#[wasm_bindgen]
pub fn covered_locales() -> String {
    let mut s = String::from("[");
    for (i, l) in COVERED_LOCALES.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('"');
        s.push_str(l);
        s.push('"');
    }
    s.push(']');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_non_empty_and_sorted() {
        // The lookups (in i18n-core's SliceCoverage) rely on (locale, key) sort
        // order — guard it so a generator change can't silently break it.
        assert!(!COVERAGE.is_empty());
        for w in COVERAGE.windows(2) {
            assert!((w[0].0, w[0].1) <= (w[1].0, w[1].1), "coverage not sorted");
        }
        assert!(!COVERED_LOCALES.is_empty());
    }

    #[test]
    fn report_resolves_against_baked_coverage() {
        // A known-translated phrase has no gap; a fabricated one does. This
        // exercises the baked COVERAGE through i18n-core end to end.
        let out = gap_report("mit Tomaten und Mozzarella\nerfundene Zutat XYZ", "en");
        assert!(out.contains("\"_summary\""));
        assert!(out.contains("\"erfundene Zutat XYZ\"")); // missing
                                                          // The translated phrase must NOT appear as missing.
        assert!(!out.contains("\"mit Tomaten und Mozzarella\""));
    }

    #[test]
    fn covered_locales_is_json_array() {
        let l = covered_locales();
        assert!(l.starts_with('[') && l.ends_with(']'));
        assert!(l.contains("\"en\""));
    }
}
