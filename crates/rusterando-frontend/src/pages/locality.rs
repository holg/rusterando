//! Delivery-area configuration: the two-layer service model that
//! decides whether classify_zone accepts an address.
//!
//! ## The model in one paragraph
//!
//! An address passes if EITHER (A) Nominatim's resolved municipality
//! name matches one of the configured `served_municipalities` (with
//! per-suburb zone routing), OR (B) the resolved lat/lon falls inside
//! one of the configured `extra_circles`. A 30 km sanity wall in
//! classify_zone is a separate paranoia guard against Nominatim
//! mis-resolving to a homonym (e.g. "Lüdinghausen, Sachsen-Anhalt").
//!
//! ## Why this shape
//!
//! Geography ignores municipal boundaries. A far-edge Bauerschaft of
//! Seppenrade may sit at 5 km from the shop while an even farther
//! Bauerschaft of Nordkirchen sits at 4 km — yet the public promise
//! "wir liefern in Lüdinghausen" must keep its word for ALL addresses
//! Nominatim resolves under Lüdinghausen. The name-based layer (A)
//! delivers that promise. The distance-based layer (B) handles the
//! outliers David chooses to serve outside the named municipalities,
//! one circle at a time.
//!
//! ## Why this is a single JSON setting, not tables
//!
//! Everything in here is ~tens of rows for any plausible shop. The
//! whole config fits in a few KB. A single `app_settings` row gives
//! us atomic updates, trivial backup/restore, and zero migration
//! surface as the schema evolves. Two tables would be more work for
//! no win at this scale.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Fold a user-typed or Nominatim-returned string into a canonical
/// compare key: umlauts expanded, lower-cased, trailing ", Suffix"
/// stripped. Both sides of every match call get folded so the
/// comparison is independent of spelling/casing/decoration.
pub fn normalize(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .replace('ä', "ae")
        .replace('ö', "oe")
        .replace('ü', "ue")
        .replace('ß', "ss")
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Sentinel value for "any suburb not specifically routed elsewhere".
/// Used in `SuburbRoute::match_suburb` to mark the fallback row inside
/// a municipality's zone_routing list.
pub const SUBURB_WILDCARD: &str = "*";

/// One (suburb → zone) mapping inside a municipality's routing table.
/// `match_suburb == "*"` is the wildcard fallback row that catches any
/// suburb the earlier rows didn't pin to a specific zone.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuburbRoute {
    pub match_suburb: String,
    pub zone_id: String,
}

/// One served municipality: every Nominatim-returned `city`/`town`/
/// `municipality` value that should be treated as part of the shop's
/// promised delivery area, plus per-suburb zone routing inside it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServedMunicipality {
    /// Names Nominatim might return for this municipality. Folded for
    /// matching, so adding "Lüdinghausen" already covers "LÜDINGHAUSEN",
    /// "luedinghausen", " Lüdinghausen, NRW", etc. Multiple entries
    /// only needed when OSM genuinely uses different canonical names
    /// (rare).
    #[serde(default)]
    pub match_names: Vec<String>,
    /// Walked top-down at classify time. First match wins. Put more
    /// specific entries first; put the `"*"` wildcard last.
    #[serde(default)]
    pub zone_routing: Vec<SuburbRoute>,
}

/// One explicit "I serve this spot even though Nominatim doesn't put
/// it in any of my `served_municipalities`" rule. Useful for selected
/// Bauerschaften David chooses to extend the offer to.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ExtraCircle {
    /// Admin-facing label. Free-text; not used in matching.
    pub label: String,
    pub center_lat: f64,
    pub center_lon: f64,
    pub radius_km: f64,
    pub zone_id: String,
}

/// The full delivery-area config. Persisted as JSON in
/// `app_settings.shop_delivery_areas`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DeliveryAreas {
    #[serde(default)]
    pub served_municipalities: Vec<ServedMunicipality>,
    #[serde(default)]
    pub extra_circles: Vec<ExtraCircle>,
}

/// Parse the JSON value of `shop_delivery_areas`. Empty / invalid JSON
/// yields the default (empty) config — fail-open: better to let orders
/// through (the 30 km wall still applies) than to block every customer
/// because of a malformed config.
pub fn parse_delivery_areas(json: &str) -> DeliveryAreas {
    let s = json.trim();
    if s.is_empty() {
        return DeliveryAreas::default();
    }
    serde_json::from_str::<DeliveryAreas>(s).unwrap_or_default()
}

/// Does any of `m.match_names` match the Nominatim-returned city name?
pub fn municipality_matches(m: &ServedMunicipality, resolved_city: &str) -> bool {
    let n = normalize(resolved_city);
    if n.is_empty() {
        return false;
    }
    m.match_names.iter().any(|name| normalize(name) == n)
}

/// First `SuburbRoute` whose `match_suburb` matches `resolved_suburb`
/// (case-insensitively, umlaut-folded). Walks in order so admins can
/// put specific suburbs before the `"*"` wildcard row. Empty
/// `resolved_suburb` matches an empty `match_suburb` (city-proper rule)
/// AND falls through to `"*"` if no city-proper row is present.
pub fn route_suburb<'a>(
    routes: &'a [SuburbRoute],
    resolved_suburb: &str,
) -> Option<&'a SuburbRoute> {
    let n = normalize(resolved_suburb);
    // Pass 1: exact match (including the empty-suburb / city-proper rule).
    for r in routes {
        let rn = normalize(&r.match_suburb);
        if rn == SUBURB_WILDCARD {
            continue;
        }
        if rn == n {
            return Some(r);
        }
    }
    // Pass 2: wildcard fallback if present.
    routes
        .iter()
        .find(|r| r.match_suburb.trim() == SUBURB_WILDCARD)
}

/// PLZ → city autofill hint. PURELY UI sugar; the classifier never
/// looks at this. Lives in its own JSON setting so it can evolve
/// independently of the delivery-area config (and stay obviously
/// non-authoritative).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostcodeHint {
    pub plz: String,
    pub city: String,
}

pub fn parse_postcode_hints(json: &str) -> Vec<PostcodeHint> {
    let s = json.trim();
    if s.is_empty() {
        return Vec::new();
    }
    serde_json::from_str::<Vec<PostcodeHint>>(s).unwrap_or_default()
}

pub fn hint_lookup_plz<'a>(hints: &'a [PostcodeHint], plz: &str) -> Option<&'a PostcodeHint> {
    let p = plz.trim();
    if p.is_empty() {
        return None;
    }
    hints.iter().find(|h| h.plz.trim() == p)
}

pub fn hint_lookup_city<'a>(hints: &'a [PostcodeHint], city: &str) -> Option<&'a PostcodeHint> {
    if city.trim().is_empty() {
        return None;
    }
    let n = normalize(city);
    hints.iter().find(|h| normalize(&h.city) == n)
}

// ---------------------------------------------------------------------------
// Server fns — cart-drawer (autofill) + admin (CRUD)
// ---------------------------------------------------------------------------

/// Snapshot consumed by the cart drawer: just the autofill hints. The
/// classifier config (`DeliveryAreas`) is server-only.
#[server(
    name = GetPostcodeHints,
    prefix = "/api",
    endpoint = "get_postcode_hints"
)]
pub async fn get_postcode_hints() -> Result<Vec<PostcodeHint>, ServerFnError> {
    let hints = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get().postcode_hints)
        .unwrap_or_default();
    Ok(hints)
}

/// Snapshot the admin page edits.
#[server(
    name = GetDeliveryAreas,
    prefix = "/api",
    endpoint = "get_delivery_areas"
)]
pub async fn get_delivery_areas() -> Result<DeliveryAreas, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let areas = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get().delivery_areas)
        .unwrap_or_default();
    Ok(areas)
}

/// Replace the whole DeliveryAreas config. Validates zone_id references
/// (must point at an active delivery_zones row) so a typo here can't
/// silently route customers to a non-existent zone.
#[server(
    name = SetDeliveryAreas,
    prefix = "/api",
    endpoint = "set_delivery_areas"
)]
pub async fn set_delivery_areas(areas: DeliveryAreas) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Collect every zone_id referenced in the new config.
    let mut referenced: Vec<String> = Vec::new();
    for m in &areas.served_municipalities {
        for r in &m.zone_routing {
            let z = r.zone_id.trim();
            if !z.is_empty() {
                referenced.push(z.to_string());
            }
        }
    }
    for c in &areas.extra_circles {
        let z = c.zone_id.trim();
        if !z.is_empty() {
            referenced.push(z.to_string());
        }
    }
    referenced.sort();
    referenced.dedup();

    // Reject if any referenced zone is missing / inactive — protect
    // David from saving a config that would silently reject customers.
    for zid in &referenced {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM delivery_zones WHERE id = ?1 AND is_active = 1")
                .bind(zid)
                .fetch_optional(&db)
                .await
                .map_err(|e| ServerFnError::new(format!("validate zone_id: {e}")))?;
        if row.is_none() {
            return Err(ServerFnError::new(format!(
                "Zone '{zid}' existiert nicht (oder ist inaktiv). Bitte unter /admin/zones anlegen."
            )));
        }
    }

    // Light input cleaning: drop fully-empty suburb-routes and trim
    // strings. Doesn't enforce uniqueness or ordering — admin sets order
    // explicitly in the editor.
    let mut cleaned = DeliveryAreas::default();
    for m in areas.served_municipalities {
        let names: Vec<String> = m
            .match_names
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if names.is_empty() {
            continue;
        }
        let routes: Vec<SuburbRoute> = m
            .zone_routing
            .into_iter()
            .map(|r| SuburbRoute {
                match_suburb: r.match_suburb.trim().to_string(),
                zone_id: r.zone_id.trim().to_string(),
            })
            .filter(|r| !r.zone_id.is_empty())
            .collect();
        cleaned.served_municipalities.push(ServedMunicipality {
            match_names: names,
            zone_routing: routes,
        });
    }
    for c in areas.extra_circles {
        let label = c.label.trim().to_string();
        let zid = c.zone_id.trim().to_string();
        if zid.is_empty() {
            continue;
        }
        if !c.radius_km.is_finite() || c.radius_km <= 0.0 {
            return Err(ServerFnError::new(format!(
                "Kreis '{label}': Radius muss > 0 sein."
            )));
        }
        if !c.center_lat.is_finite() || !c.center_lon.is_finite() {
            return Err(ServerFnError::new(format!(
                "Kreis '{label}': Koordinaten ungültig."
            )));
        }
        cleaned.extra_circles.push(ExtraCircle {
            label,
            center_lat: c.center_lat,
            center_lon: c.center_lon,
            radius_km: c.radius_km,
            zone_id: zid,
        });
    }

    let json = serde_json::to_string(&cleaned)
        .map_err(|e| ServerFnError::new(format!("encode delivery areas: {e}")))?;
    crate::pages::settings::update_setting("shop_delivery_areas".to_string(), json).await?;
    Ok(())
}

/// Replace the postcode-hint list (PLZ → city autofill). Pure UI sugar,
/// no zone validation needed — wrong values just produce a misleading
/// placeholder, never a wrong classification.
#[server(
    name = SetPostcodeHints,
    prefix = "/api",
    endpoint = "set_postcode_hints"
)]
pub async fn set_postcode_hints(hints: Vec<PostcodeHint>) -> Result<(), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let mut cleaned: Vec<PostcodeHint> = Vec::with_capacity(hints.len());
    for h in hints.into_iter() {
        let plz = h.plz.trim().to_string();
        let city = h.city.trim().to_string();
        if plz.is_empty() || city.is_empty() {
            continue;
        }
        if plz.len() != 5 || !plz.chars().all(|c| c.is_ascii_digit()) {
            return Err(ServerFnError::new(format!(
                "PLZ '{plz}' ist keine gültige 5-stellige Postleitzahl."
            )));
        }
        cleaned.push(PostcodeHint { plz, city });
    }
    let json = serde_json::to_string(&cleaned)
        .map_err(|e| ServerFnError::new(format!("encode postcode hints: {e}")))?;
    crate::pages::settings::update_setting("shop_postcode_hints".to_string(), json).await?;
    Ok(())
}

/// Server-side helper used by `validate_address` to geocode a single
/// admin-typed address ("📍 Adresse → Koordinaten" button on
/// /admin/localities). Re-uses the customer-facing Nominatim path so
/// rate-limit + AUP behaviour is identical.
#[server(
    name = GeocodeForAdmin,
    prefix = "/api",
    endpoint = "geocode_for_admin"
)]
pub async fn geocode_for_admin(query: String) -> Result<(f64, f64), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let q = query.trim();
    if q.is_empty() {
        return Err(ServerFnError::new("Bitte Adresse oder Ort eingeben."));
    }
    // Reuse the public Nominatim wrapper. Pass the whole query as
    // street; the helper concatenates with the other fields so passing
    // empty postcode/city is fine — Nominatim resolves free-form
    // addresses gracefully.
    let hit = crate::pages::order::ssr::nominatim_lookup(q, "", "", "")
        .await
        .map_err(|e| ServerFnError::new(format!("Geocoder-Fehler: {e}")))?;
    let Some(found) = hit else {
        return Err(ServerFnError::new(
            "Adresse nicht gefunden. Bitte präziser eingeben (Straße + Ort).",
        ));
    };
    let lat: f64 = found
        .lat
        .parse()
        .map_err(|_| ServerFnError::new("Geocoder lieferte ungültige Koordinate."))?;
    let lon: f64 = found
        .lon
        .parse()
        .map_err(|_| ServerFnError::new("Geocoder lieferte ungültige Koordinate."))?;
    Ok((lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generic test municipality. Has two name variants (one with an
    /// umlaut, one folded) so the umlaut-fold path gets exercised, and
    /// three routing rules: a specific suburb, the city-proper, and the
    /// `*` wildcard fallback.
    fn sample_municipality() -> ServedMunicipality {
        ServedMunicipality {
            match_names: vec!["Ünterstadt".into(), "Uenterstadt".into()],
            zone_routing: vec![
                SuburbRoute {
                    match_suburb: "Oberdorf".into(),
                    zone_id: "zone-outer".into(),
                },
                SuburbRoute {
                    match_suburb: "Ünterstadt".into(),
                    zone_id: "zone-center".into(),
                },
                SuburbRoute {
                    match_suburb: "*".into(),
                    zone_id: "zone-fallback".into(),
                },
            ],
        }
    }

    #[test]
    fn normalize_folds_umlauts_and_case() {
        assert_eq!(normalize("Ünterstadt"), "uenterstadt");
        assert_eq!(normalize("  ÜNTERSTADT, Bundesland "), "uenterstadt");
        assert_eq!(normalize("Uenterstadt"), "uenterstadt");
    }

    #[test]
    fn municipality_match_is_fold_insensitive() {
        let m = sample_municipality();
        assert!(municipality_matches(&m, "Ünterstadt"));
        assert!(municipality_matches(&m, "uenterstadt"));
        assert!(municipality_matches(&m, "UENTERSTADT"));
        assert!(!municipality_matches(&m, "Anderstown"));
        assert!(!municipality_matches(&m, ""));
    }

    #[test]
    fn suburb_routing_specific_then_wildcard() {
        let m = sample_municipality();
        // Specific match wins.
        let r = route_suburb(&m.zone_routing, "Oberdorf").unwrap();
        assert_eq!(r.zone_id, "zone-outer");
        // Empty suburb falls through to the `*` wildcard since none of
        // the specific rules use an empty match_suburb.
        let r = route_suburb(&m.zone_routing, "").unwrap();
        assert_eq!(r.zone_id, "zone-fallback");
        // Unknown suburb → wildcard.
        let r = route_suburb(&m.zone_routing, "Unknownville").unwrap();
        assert_eq!(r.zone_id, "zone-fallback");
        // Umlaut variant of the city-proper rule.
        let r = route_suburb(&m.zone_routing, "Ünterstadt").unwrap();
        assert_eq!(r.zone_id, "zone-center");
    }

    #[test]
    fn suburb_routing_no_wildcard_rejects_unknown() {
        let routes = vec![SuburbRoute {
            match_suburb: "Centertown".into(),
            zone_id: "zone-specific".into(),
        }];
        assert!(route_suburb(&routes, "Centertown").is_some());
        assert!(route_suburb(&routes, "Othertown").is_none());
    }

    #[test]
    fn parse_round_trip() {
        let cfg = DeliveryAreas {
            served_municipalities: vec![sample_municipality()],
            extra_circles: vec![ExtraCircle {
                label: "Outlying Hamlets".into(),
                center_lat: 51.7,
                center_lon: 7.5,
                radius_km: 2.5,
                zone_id: "zone-fallback".into(),
            }],
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back = parse_delivery_areas(&s);
        assert_eq!(back, cfg);
    }

    #[test]
    fn parse_empty_or_bad_yields_default() {
        assert_eq!(parse_delivery_areas(""), DeliveryAreas::default());
        assert_eq!(parse_delivery_areas("not json"), DeliveryAreas::default());
    }

    #[test]
    fn postcode_hint_lookups() {
        let hints = vec![PostcodeHint {
            plz: "12345".into(),
            city: "Ünterstadt".into(),
        }];
        assert!(hint_lookup_plz(&hints, "12345").is_some());
        assert!(hint_lookup_plz(&hints, "99999").is_none());
        assert!(hint_lookup_city(&hints, "Uenterstadt").is_some());
        assert!(hint_lookup_city(&hints, "uenterstadt").is_some());
        assert!(hint_lookup_city(&hints, "Anderstown").is_none());
    }
}
