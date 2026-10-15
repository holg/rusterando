//! The single normaliser for street names and house numbers.
//!
//! Applied identically to the cadastre names at import time and to the
//! customer's keystrokes at lookup time, so a prefix comparison on the
//! normalised forms is all the matcher needs.

/// Street-name normalisation:
///
/// 1. lowercase
/// 2. `ä→ae`, `ö→oe`, `ü→ue`, `ß→ss`
/// 3. `strasse → str` (reduce, never expand: `Hauptstr`, `Hauptstr.` and
///    `Hauptstraße` all become prefix-compatible)
/// 4. every non-alphanumeric char → space
/// 5. collapse whitespace, trim
pub fn normalize_street(s: &str) -> String {
    let folded = s
        .to_lowercase()
        .replace('ä', "ae")
        .replace('ö', "oe")
        .replace('ü', "ue")
        .replace('ß', "ss")
        .replace("strasse", "str");
    let spaced: String = folded
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// House-number parsing: lowercase, strip all whitespace, split into the
/// leading digits and whatever follows (`"12 a" → (12, "a")`, `"7" → (7, "")`,
/// `"12-14" → (12, "-14")`). `None` when there are no leading digits.
///
/// The suffix keeps punctuation deliberately: `12-14` and `12/1` are distinct
/// cadastre entries and must not collapse into `12`.
pub fn parse_house_number(s: &str) -> Option<(u32, String)> {
    let compact: String = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    let digits: String = compact.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let number: u32 = digits.parse().ok()?;
    let suffix = compact[digits.len()..].to_string();
    Some((number, suffix))
}

/// Normalised house-number string used for prefix matching in suggestions:
/// digits immediately followed by the lowercase suffix (`"12a"`).
pub fn normalize_house_number(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// Display form of a stored `(number, suffix)` pair (`12`, `12a`).
pub fn display_house_number(number: u32, suffix: &str) -> String {
    format!("{number}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn street_vectors_from_spec() {
        let cases = [
            ("Hermann-Löns-Straße", "hermann loens str"),
            ("Hauptstr.", "hauptstr"),
            ("Hauptstrasse", "hauptstr"),
            ("Bahnhofstraße", "bahnhofstr"),
            ("Am Markt", "am markt"),
            ("Von-Steinen-Str", "von steinen str"),
            ("  wasser  straße ", "wasser str"),
        ];
        for (input, want) in cases {
            assert_eq!(normalize_street(input), want, "input {input:?}");
        }
    }

    #[test]
    fn street_is_idempotent() {
        for s in ["Hermann-Löns-Straße", "Am Markt", "Bahnhofstr."] {
            let once = normalize_street(s);
            assert_eq!(normalize_street(&once), once);
        }
    }

    #[test]
    fn house_numbers() {
        assert_eq!(parse_house_number("12"), Some((12, String::new())));
        assert_eq!(parse_house_number("12a"), Some((12, "a".into())));
        assert_eq!(parse_house_number("12 A"), Some((12, "a".into())));
        assert_eq!(parse_house_number("12-14"), Some((12, "-14".into())));
        assert_eq!(parse_house_number(" 7 "), Some((7, String::new())));
        assert_eq!(parse_house_number("abc"), None);
        assert_eq!(parse_house_number(""), None);
        assert_eq!(normalize_house_number("12 A"), "12a");
        assert_eq!(display_house_number(12, "a"), "12a");
    }
}
