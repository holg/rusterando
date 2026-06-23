//! Pure translation **gap computation** — the single source of truth shared by
//! the i18n management wasm (`rusterando-i18n-mgmt`) and the i18n TUI
//! (`rusterando-i18n-tui`), so the two can never disagree about what's
//! translated.
//!
//! "Gap" = a German source phrase a shop USES that has no translation for a
//! given locale. The COVERAGE (which (locale, german) pairs are translated) is
//! supplied by the caller via the [`Coverage`] trait — the wasm passes its
//! build-baked static slice; the TUI passes a set read from the
//! `menu_strings.<locale>.json` files (or a DB). This crate has no data, no I/O,
//! and no dependencies.

use std::collections::BTreeSet;

/// Source of "is this (locale, german) phrase translated?". Implementors decide
/// where the answer comes from (a baked slice, a HashSet read from JSON, …).
pub trait Coverage {
    fn is_covered(&self, locale: &str, german: &str) -> bool;
}

/// A [`Coverage`] backed by a slice of `(locale, german)` pairs **sorted by
/// `(locale, german)`** — binary-search lookup, no allocation. This is what the
/// mgmt wasm uses (its build-baked `COVERAGE`).
pub struct SliceCoverage<'a>(pub &'a [(&'a str, &'a str)]);

impl Coverage for SliceCoverage<'_> {
    fn is_covered(&self, locale: &str, german: &str) -> bool {
        self.0
            .binary_search_by(|(l, g)| (*l, *g).cmp(&(locale, german)))
            .is_ok()
    }
}

/// A [`Coverage`] backed by an owned set of `(locale, german)` strings — what
/// the TUI builds after reading the `menu_strings.<locale>.json` files. Order
/// doesn't matter (set membership).
pub struct SetCoverage(pub BTreeSet<(String, String)>);

impl Coverage for SetCoverage {
    fn is_covered(&self, locale: &str, german: &str) -> bool {
        // Borrowed lookup without allocating a (String, String) key.
        self.0.iter().any(|(l, g)| l == locale && g == german)
    }
}

impl SetCoverage {
    /// Build from an iterator of `(locale, german)`.
    pub fn from_pairs<I, L, G>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (L, G)>,
        L: Into<String>,
        G: Into<String>,
    {
        Self(
            pairs
                .into_iter()
                .map(|(l, g)| (l.into(), g.into()))
                .collect(),
        )
    }
}

/// Clean a phrase list: trim, drop empties, de-duplicate (stable order).
pub fn clean_phrases<'a>(phrases: &'a [&'a str]) -> Vec<&'a str> {
    let mut seen = BTreeSet::new();
    phrases
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .filter(|p| seen.insert(*p))
        .collect()
}

/// For each locale, the cleaned phrases NOT covered. Returns `(locale,
/// missing)` in the given locale order.
pub fn gaps<C: Coverage>(
    coverage: &C,
    phrases: &[&str],
    locales: &[&str],
) -> Vec<(String, Vec<String>)> {
    let cleaned = clean_phrases(phrases);
    locales
        .iter()
        .map(|loc| {
            let missing: Vec<String> = cleaned
                .iter()
                .filter(|p| !coverage.is_covered(loc, p))
                .map(|p| p.to_string())
                .collect();
            (loc.to_string(), missing)
        })
        .collect()
}

/// Build the gap-report JSON:
/// `{ "<locale>": ["missing phrase", …], …, "_summary": {phrases, locales,
/// total_missing} }`. A locale with `[]` is fully translated for this shop.
/// Hand-rolled so the core stays dependency-free (and identical to what the
/// wasm emitted before extraction).
pub fn gap_report_json<C: Coverage>(coverage: &C, phrases: &[&str], locales: &[&str]) -> String {
    let report = gaps(coverage, phrases, locales);
    let distinct = clean_phrases(phrases).len();

    let mut json = String::from("{");
    let mut total_missing = 0usize;
    for (i, (loc, missing)) in report.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        total_missing += missing.len();
        json.push_str(&json_str(loc));
        json.push(':');
        json.push('[');
        for (j, m) in missing.iter().enumerate() {
            if j > 0 {
                json.push(',');
            }
            json.push_str(&json_str(m));
        }
        json.push(']');
    }
    json.push_str(",\"_summary\":{\"phrases\":");
    json.push_str(&distinct.to_string());
    json.push_str(",\"locales\":");
    json.push_str(&locales.len().to_string());
    json.push_str(",\"total_missing\":");
    json.push_str(&total_missing.to_string());
    json.push_str("}}");
    json
}

/// Escape a string for a JSON string literal.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cov() -> SetCoverage {
        SetCoverage::from_pairs([
            ("en", "mit Tomaten und Mozzarella"),
            ("fr", "mit Tomaten und Mozzarella"),
            ("en", "mit Pilzen"),
        ])
    }

    #[test]
    fn covered_phrase_no_gap() {
        let g = gaps(&cov(), &["mit Tomaten und Mozzarella"], &["en"]);
        assert!(g[0].1.is_empty());
    }

    #[test]
    fn uncovered_phrase_reported() {
        let g = gaps(&cov(), &["unbekannte Zutat"], &["en", "fr"]);
        assert_eq!(g[0].1, vec!["unbekannte Zutat".to_string()]);
        assert_eq!(g[1].1, vec!["unbekannte Zutat".to_string()]);
    }

    #[test]
    fn dedup_and_blank() {
        let g = gaps(&cov(), &["mit Pilzen", " ", "mit Pilzen", ""], &["en"]);
        assert!(g[0].1.is_empty()); // covered + deduped
    }

    #[test]
    fn slice_coverage_matches_set() {
        // Sorted slice (the wasm's form) must agree with the set form.
        let slice = SliceCoverage(&[("en", "a"), ("en", "b"), ("fr", "a")]);
        assert!(slice.is_covered("en", "a"));
        assert!(!slice.is_covered("en", "z"));
        assert!(!slice.is_covered("de", "a"));
    }

    #[test]
    fn report_json_shape() {
        let j = gap_report_json(
            &cov(),
            &[
                "mit Pilzen\nunbekannt".split('\n').next().unwrap(),
                "unbekannt",
            ],
            &["en"],
        );
        assert!(j.starts_with('{'));
        assert!(j.contains("\"_summary\""));
        assert!(j.contains("\"unbekannt\""));
    }
}
