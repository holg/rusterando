//! The in-memory lookup index: streets + house numbers per tenant.
//!
//! Built once at startup from the `geo_streets` / `geo_addresses` tables
//! (see [`crate::store`]) and shared as `Arc<GeoIndex>`. A tenant has a few
//! hundred streets and a few thousand addresses; brute force over a `Vec`
//! is faster than any clever structure at that size.

use crate::normalize::{display_house_number, normalize_house_number, normalize_street};

/// A street of the delivery area.
#[derive(Debug, Clone, PartialEq)]
pub struct Street {
    pub id: u32,
    /// Display form: original casing and umlauts.
    pub name: String,
    /// [`normalize_street`] of `name`.
    pub name_norm: String,
    pub postcode: String,
    pub city: String,
}

/// One building address on a street.
#[derive(Debug, Clone, PartialEq)]
pub struct HouseNo {
    pub number: u32,
    /// Lowercase, no whitespace; `""` for a plain number.
    pub suffix: String,
    pub lat: f64,
    pub lon: f64,
}

impl HouseNo {
    pub fn display(&self) -> String {
        display_house_number(self.number, &self.suffix)
    }
}

/// Exact resolution of `(street, house number)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub street_id: u32,
    pub street: String,
    pub house_number: String,
    pub postcode: String,
    pub city: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Default)]
pub struct GeoIndex {
    /// Sorted by `name_norm`, then `name`.
    streets: Vec<Street>,
    /// `street_id → house numbers`, each `Vec` sorted by `(number, suffix)`.
    numbers: std::collections::HashMap<u32, Vec<HouseNo>>,
}

/// Fuzzy matches (rank 2) need at least this many normalised characters, so
/// a two-letter query never drags in half the street list.
const FUZZY_MIN_LEN: usize = 3;
const FUZZY_THRESHOLD: f64 = 0.85;
const STREET_LIMIT: usize = 8;
const NUMBER_LIMIT: usize = 12;

impl GeoIndex {
    pub fn new(
        mut streets: Vec<Street>,
        numbers: std::collections::HashMap<u32, Vec<HouseNo>>,
    ) -> Self {
        streets.sort_by(|a, b| a.name_norm.cmp(&b.name_norm).then(a.name.cmp(&b.name)));
        let numbers = numbers
            .into_iter()
            .map(|(id, mut v)| {
                v.sort_by(|a, b| a.number.cmp(&b.number).then(a.suffix.cmp(&b.suffix)));
                (id, v)
            })
            .collect();
        Self { streets, numbers }
    }

    pub fn streets(&self) -> &[Street] {
        &self.streets
    }

    pub fn street(&self, id: u32) -> Option<&Street> {
        self.streets.iter().find(|s| s.id == id)
    }

    pub fn house_numbers(&self, street_id: u32) -> &[HouseNo] {
        self.numbers
            .get(&street_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn address_count(&self) -> usize {
        self.numbers.values().map(Vec::len).sum()
    }

    /// Bounding box `(lat_min, lat_max, lon_min, lon_max)` of every address,
    /// or `None` for an empty index.
    pub fn bbox(&self) -> Option<(f64, f64, f64, f64)> {
        let mut it = self.numbers.values().flatten();
        let first = it.next()?;
        let mut b = (first.lat, first.lat, first.lon, first.lon);
        for h in it {
            b.0 = b.0.min(h.lat);
            b.1 = b.1.max(h.lat);
            b.2 = b.2.min(h.lon);
            b.3 = b.3.max(h.lon);
        }
        Some(b)
    }

    /// Street suggestions for a typed prefix.
    ///
    /// rank 0: normalised name starts with the query;
    /// rank 1: any whitespace-separated token starts with the query;
    /// rank 2: Jaro-Winkler > 0.85 (query of ≥ 3 chars only).
    /// Sorted by `(rank, name_norm.len(), name)`, at most 8.
    pub fn suggest_streets(&self, q: &str) -> Vec<&Street> {
        let q = normalize_street(q);
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<(u8, &Street)> = Vec::new();
        for s in &self.streets {
            let rank = if s.name_norm.starts_with(&q) {
                0
            } else if s.name_norm.split(' ').any(|tok| tok.starts_with(&q)) {
                1
            } else if q.len() >= FUZZY_MIN_LEN
                && strsim::jaro_winkler(&s.name_norm, &q) > FUZZY_THRESHOLD
            {
                2
            } else {
                continue;
            };
            hits.push((rank, s));
        }
        hits.sort_by(|(ra, a), (rb, b)| {
            ra.cmp(rb)
                .then(a.name_norm.len().cmp(&b.name_norm.len()))
                .then(a.name.cmp(&b.name))
        });
        hits.into_iter()
            .take(STREET_LIMIT)
            .map(|(_, s)| s)
            .collect()
    }

    /// Exact street by normalised name (for free-text input without a
    /// selected suggestion). Unique match only.
    pub fn find_street_exact(&self, name: &str) -> Option<&Street> {
        let n = normalize_street(name);
        if n.is_empty() {
            return None;
        }
        let mut it = self.streets.iter().filter(|s| s.name_norm == n);
        let first = it.next()?;
        if it.next().is_some() {
            return None;
        }
        Some(first)
    }

    /// Resolve free-text street input without a selection: an exact
    /// normalised name match, disambiguated by `postcode` when the same
    /// name exists in several municipalities; failing that, a UNIQUE
    /// prefix / token-prefix match (rank 0/1 of [`suggest_streets`]). Returns
    /// `None` when nothing or more than one street fits — the caller then
    /// asks the customer to pick from the suggestions.
    pub fn find_street_by_text(&self, text: &str, postcode: Option<&str>) -> Option<&Street> {
        let n = normalize_street(text);
        if n.is_empty() {
            return None;
        }
        fn pick<'a>(cands: Vec<&'a Street>, postcode: Option<&str>) -> Option<&'a Street> {
            match cands.len() {
                0 => None,
                1 => Some(cands[0]),
                _ => {
                    let plz = postcode.map(str::trim).filter(|p| !p.is_empty())?;
                    let mut by_plz = cands.into_iter().filter(|s| s.postcode == plz);
                    let first = by_plz.next()?;
                    by_plz.next().is_none().then_some(first)
                }
            }
        }
        let exact: Vec<&Street> = self.streets.iter().filter(|s| s.name_norm == n).collect();
        if !exact.is_empty() {
            return pick(exact, postcode);
        }
        let prefix: Vec<&Street> = self
            .streets
            .iter()
            .filter(|s| {
                s.name_norm.starts_with(&n) || s.name_norm.split(' ').any(|t| t.starts_with(&n))
            })
            .collect();
        pick(prefix, postcode)
    }

    /// The known house number closest to `number` on a street — the stand-in
    /// point for a number the cadastre doesn't have (new building, typo).
    /// Ties prefer the lower number.
    pub fn nearest_house(&self, street_id: u32, number: u32) -> Option<&HouseNo> {
        self.house_numbers(street_id)
            .iter()
            .min_by_key(|h| ((h.number as i64 - number as i64).unsigned_abs(), h.number))
    }

    /// House-number suggestions (display strings) whose normalised form
    /// starts with `q`; numeric order, then suffix; at most 12. An empty
    /// `q` returns the first 12 numbers of the street.
    pub fn suggest_house_numbers(&self, street_id: u32, q: &str) -> Vec<String> {
        let q = normalize_house_number(q);
        self.house_numbers(street_id)
            .iter()
            .filter(|h| q.is_empty() || h.display().starts_with(&q))
            .take(NUMBER_LIMIT)
            .map(HouseNo::display)
            .collect()
    }

    /// Exact `(number, suffix)` match. No fuzziness: an unknown number is the
    /// caller's problem (unverified address path).
    pub fn resolve(&self, street_id: u32, house_number: &str) -> Option<Resolved> {
        let (number, suffix) = crate::normalize::parse_house_number(house_number)?;
        let street = self.street(street_id)?;
        let h = self
            .house_numbers(street_id)
            .iter()
            .find(|h| h.number == number && h.suffix == suffix)?;
        Some(Resolved {
            street_id,
            street: street.name.clone(),
            house_number: h.display(),
            postcode: street.postcode.clone(),
            city: street.city.clone(),
            lat: h.lat,
            lon: h.lon,
        })
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn street(id: u32, name: &str) -> Street {
        Street {
            id,
            name: name.to_string(),
            name_norm: normalize_street(name),
            postcode: "59348".into(),
            city: "Lüdinghausen".into(),
        }
    }

    /// ~20 streets, a few numbers on the first two.
    pub fn index() -> GeoIndex {
        let names = [
            "Hauptstraße",
            "Am Markt",
            "Bahnhofstraße",
            "Hermann-Löns-Straße",
            "Mühlenstraße",
            "Münsterstraße",
            "Olfener Straße",
            "Seppenrader Straße",
            "Wolfsberger Straße",
            "Tüllinghofer Straße",
            "Klosterstraße",
            "Marktstraße",
            "Kirchplatz",
            "Borgmühle",
            "Berenbrock",
            "Ondrup",
            "Westrup",
            "Elvert",
            "Bechtrup",
            "Am Hagen",
        ];
        let streets: Vec<Street> = names
            .iter()
            .enumerate()
            .map(|(i, n)| street(i as u32 + 1, n))
            .collect();
        let mut numbers = std::collections::HashMap::new();
        numbers.insert(
            1,
            vec![
                HouseNo {
                    number: 12,
                    suffix: "a".into(),
                    lat: 51.7701,
                    lon: 7.4401,
                },
                HouseNo {
                    number: 1,
                    suffix: "".into(),
                    lat: 51.77,
                    lon: 7.44,
                },
                HouseNo {
                    number: 12,
                    suffix: "".into(),
                    lat: 51.7702,
                    lon: 7.4402,
                },
                HouseNo {
                    number: 120,
                    suffix: "".into(),
                    lat: 51.771,
                    lon: 7.441,
                },
                HouseNo {
                    number: 2,
                    suffix: "".into(),
                    lat: 51.7703,
                    lon: 7.4403,
                },
            ],
        );
        numbers.insert(
            2,
            vec![HouseNo {
                number: 3,
                suffix: "".into(),
                lat: 51.768,
                lon: 7.433,
            }],
        );
        GeoIndex::new(streets, numbers)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::index;
    use super::*;

    #[test]
    fn prefix_ranks_first() {
        let idx = index();
        let s = idx.suggest_streets("hauptstr");
        assert_eq!(s[0].name, "Hauptstraße");
    }

    #[test]
    fn token_prefix_finds_am_markt() {
        let idx = index();
        let s = idx.suggest_streets("markt");
        assert_eq!(s[0].name, "Marktstraße", "rank 0 prefix first");
        assert!(
            s.iter().any(|x| x.name == "Am Markt"),
            "token match present"
        );
    }

    #[test]
    fn fuzzy_typo_finds_hauptstrasse() {
        let idx = index();
        let s = idx.suggest_streets("haubtstr");
        assert!(s.iter().any(|x| x.name == "Hauptstraße"), "{s:?}");
    }

    #[test]
    fn short_query_never_fuzzy() {
        let idx = index();
        let s = idx.suggest_streets("xx");
        assert!(s.is_empty(), "{s:?}");
        assert!(idx.suggest_streets("").is_empty());
    }

    #[test]
    fn umlauts_and_dots_are_prefix_compatible() {
        let idx = index();
        assert_eq!(idx.suggest_streets("Mühlenstr.")[0].name, "Mühlenstraße");
        assert_eq!(idx.suggest_streets("muehlen")[0].name, "Mühlenstraße");
        assert_eq!(
            idx.suggest_streets("Hermann-Löns")[0].name,
            "Hermann-Löns-Straße"
        );
    }

    #[test]
    fn at_most_eight() {
        let idx = index();
        // "str" is a token of many names.
        assert!(idx.suggest_streets("str").len() <= 8);
    }

    #[test]
    fn house_numbers_sorted_and_prefixed() {
        let idx = index();
        assert_eq!(
            idx.suggest_house_numbers(1, ""),
            vec!["1", "2", "12", "12a", "120"]
        );
        assert_eq!(idx.suggest_house_numbers(1, "12"), vec!["12", "12a", "120"]);
        assert_eq!(idx.suggest_house_numbers(1, "12 A"), vec!["12a"]);
        assert!(idx.suggest_house_numbers(99, "").is_empty());
    }

    #[test]
    fn resolve_is_exact() {
        let idx = index();
        let r = idx.resolve(1, "12 a").unwrap();
        assert_eq!(r.house_number, "12a");
        assert_eq!(r.street, "Hauptstraße");
        assert_eq!(r.postcode, "59348");
        assert!(idx.resolve(1, "13").is_none());
        assert!(idx.resolve(1, "12b").is_none());
        assert!(idx.resolve(1, "abc").is_none());
        assert!(idx.resolve(42, "1").is_none());
    }

    #[test]
    fn free_text_street_resolution() {
        let idx = index();
        // exact
        assert_eq!(idx.find_street_by_text("Hauptstr.", None).unwrap().id, 1);
        // unique prefix
        assert_eq!(
            idx.find_street_by_text("Bahnhof", None).unwrap().name,
            "Bahnhofstraße"
        );
        // unique token prefix
        assert_eq!(
            idx.find_street_by_text("Hagen", None).unwrap().name,
            "Am Hagen"
        );
        // ambiguous prefix ("Am " matches Am Markt + Am Hagen)
        assert!(idx.find_street_by_text("am", None).is_none());
        assert!(idx.find_street_by_text("", None).is_none());
        assert!(idx.find_street_by_text("zzz", None).is_none());
    }

    #[test]
    fn same_name_in_two_towns_is_disambiguated_by_postcode() {
        let mut streets = vec![
            super::fixtures::street(1, "Bahnhofstr."),
            super::fixtures::street(2, "Bahnhofstr."),
        ];
        streets[1].postcode = "59394".into();
        streets[1].city = "Nordkirchen".into();
        let idx = GeoIndex::new(streets, std::collections::HashMap::new());
        assert!(idx.find_street_by_text("Bahnhofstraße", None).is_none());
        assert_eq!(
            idx.find_street_by_text("Bahnhofstraße", Some("59394"))
                .unwrap()
                .id,
            2
        );
        assert_eq!(
            idx.find_street_by_text("Bahnhofstraße", Some("59348"))
                .unwrap()
                .id,
            1
        );
        assert!(idx
            .find_street_by_text("Bahnhofstraße", Some("00000"))
            .is_none());
    }

    #[test]
    fn nearest_house_for_unknown_number() {
        let idx = index();
        // street 1 has 1, 2, 12, 12a, 120
        assert_eq!(idx.nearest_house(1, 13).unwrap().number, 12);
        assert_eq!(idx.nearest_house(1, 60).unwrap().number, 12);
        assert_eq!(idx.nearest_house(1, 999).unwrap().number, 120);
        assert!(idx.nearest_house(42, 1).is_none());
    }

    #[test]
    fn exact_street_lookup() {
        let idx = index();
        assert_eq!(idx.find_street_exact("Hauptstr.").unwrap().id, 1);
        assert!(idx.find_street_exact("Haupt").is_none());
    }
}
