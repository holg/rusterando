//! Tenant configuration from the per-shop `.env`:
//!
//! ```text
//! GEO_BACKEND=cadastre_nrw
//! GEO_AGS=05558024
//! GEO_POSTCODE_MAP=05558024:59348:Lüdinghausen
//! GEO_DATA_DIR=/var/lib/rusterando/<tenant>/geo
//! GEO_EXPECT_BBOX=51.68,51.86,7.30,7.58        # optional import sanity box
//! ```
//!
//! `GEO_BACKEND` unset → the tenant keeps the Nominatim behaviour and
//! [`GeoConfig::from_vars`] returns `None`.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct GeoConfig {
    pub backend: String,
    /// Municipality keys (AGS, 8 digits) in the delivery area.
    pub ags: Vec<String>,
    /// `AGS → (postcode, city)`. Phase 1: one postcode per AGS.
    pub postcode_map: HashMap<String, (String, String)>,
    /// Download cache dir for `geo-import --download`.
    pub data_dir: Option<String>,
    /// `(lat_min, lat_max, lon_min, lon_max)` the import centroid must fall
    /// into. Defaults to NRW.
    pub expect_bbox: (f64, f64, f64, f64),
}

/// North Rhine-Westphalia, generously.
pub const NRW_BBOX: (f64, f64, f64, f64) = (50.3, 52.6, 5.8, 9.5);

impl GeoConfig {
    /// `None` when `GEO_BACKEND` is unset/empty.
    pub fn from_vars(vars: &HashMap<String, String>) -> Option<Self> {
        let backend = vars.get("GEO_BACKEND")?.trim().to_string();
        if backend.is_empty() {
            return None;
        }
        let ags: Vec<String> = vars
            .get("GEO_AGS")
            .map(|s| {
                s.split(',')
                    .map(|a| a.trim().to_string())
                    .filter(|a| !a.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let mut postcode_map = HashMap::new();
        if let Some(m) = vars.get("GEO_POSTCODE_MAP") {
            for triple in m.split(',') {
                let mut it = triple.trim().splitn(3, ':');
                if let (Some(a), Some(plz), Some(city)) = (it.next(), it.next(), it.next()) {
                    postcode_map.insert(
                        a.trim().to_string(),
                        (plz.trim().to_string(), city.trim().to_string()),
                    );
                }
            }
        }
        let expect_bbox = vars
            .get("GEO_EXPECT_BBOX")
            .and_then(|s| {
                let v: Vec<f64> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (v.len() == 4).then(|| (v[0], v[1], v[2], v[3]))
            })
            .unwrap_or(NRW_BBOX);
        Some(Self {
            backend,
            ags,
            postcode_map,
            data_dir: vars.get("GEO_DATA_DIR").map(|s| s.trim().to_string()),
            expect_bbox,
        })
    }

    /// From the process environment (single-tenant deployments).
    pub fn from_env() -> Option<Self> {
        let vars: HashMap<String, String> = std::env::vars().collect();
        Self::from_vars(&vars)
    }

    pub fn postcode_city(&self, ags: &str) -> Option<&(String, String)> {
        self.postcode_map.get(ags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn unset_backend_is_none() {
        assert!(GeoConfig::from_vars(&vars(&[])).is_none());
        assert!(GeoConfig::from_vars(&vars(&[("GEO_BACKEND", " ")])).is_none());
    }

    #[test]
    fn parses_all_keys() {
        let c = GeoConfig::from_vars(&vars(&[
            ("GEO_BACKEND", "cadastre_nrw"),
            ("GEO_AGS", "05558024, 05558028"),
            (
                "GEO_POSTCODE_MAP",
                "05558024:59348:Lüdinghausen,05558028:59394:Nordkirchen",
            ),
            ("GEO_EXPECT_BBOX", "51.68,51.86,7.30,7.58"),
        ]))
        .unwrap();
        assert_eq!(c.ags, vec!["05558024", "05558028"]);
        assert_eq!(
            c.postcode_city("05558024"),
            Some(&("59348".to_string(), "Lüdinghausen".to_string()))
        );
        assert_eq!(c.expect_bbox, (51.68, 51.86, 7.30, 7.58));
        assert!(c.data_dir.is_none());
    }

    #[test]
    fn bad_bbox_falls_back_to_nrw() {
        let c = GeoConfig::from_vars(&vars(&[("GEO_BACKEND", "x"), ("GEO_EXPECT_BBOX", "1,2")]))
            .unwrap();
        assert_eq!(c.expect_bbox, NRW_BBOX);
    }
}
