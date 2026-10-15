//! Delivery-zone evaluation for a resolved address.
//!
//! Precedence (first hit wins):
//! 1. explicit street override (`geo_zone_streets`), optionally limited to a
//!    house-number range
//! 2. active zones by ascending `priority`: `polygon` (point in GeoJSON
//!    Polygon/MultiPolygon) and `radius` (haversine from a centre)
//! 3. the single `default` zone, if the address lies in a configured AGS
//! 4. `None` → outside the delivery area
//!
//! Geometry is stored as text and parsed on read; [`parse_geometry`]
//! validates operator input (closed rings, `[lon, lat]` order sanity via a
//! bounding box) so the dashboard rejects it before it can silently drop
//! every address into the default zone.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GeometryKind {
    /// Not evaluated by geometry (street overrides can still point here).
    None,
    Polygon,
    Radius,
    /// Catch-all for every address of the imported municipalities.
    Default,
}

impl GeometryKind {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "polygon" => Self::Polygon,
            "radius" => Self::Radius,
            "default" => Self::Default,
            _ => Self::None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Polygon => "polygon",
            Self::Radius => "radius",
            Self::Default => "default",
        }
    }
}

/// A polygon ring as `(lat, lon)` points; first == last.
pub type Ring = Vec<(f64, f64)>;

#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// Each polygon: outer ring first, then holes.
    Polygons(Vec<Vec<Ring>>),
    Radius {
        lat: f64,
        lon: f64,
        metres: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ZoneDef {
    pub id: String,
    pub name: String,
    pub priority: i64,
    pub fee_cents: i64,
    pub min_order_cents: Option<i64>,
    pub eta_minutes: i64,
    pub active: bool,
    pub kind: GeometryKind,
    pub geometry: Option<Geometry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreetOverride {
    pub zone_id: String,
    pub street_id: u32,
    pub number_from: Option<u32>,
    pub number_to: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneMatch {
    pub zone_id: String,
    pub name: String,
    pub fee_cents: i64,
    pub min_order_cents: Option<i64>,
    pub eta_minutes: i64,
}

impl ZoneMatch {
    fn from(z: &ZoneDef) -> Self {
        Self {
            zone_id: z.id.clone(),
            name: z.name.clone(),
            fee_cents: z.fee_cents,
            min_order_cents: z.min_order_cents,
            eta_minutes: z.eta_minutes,
        }
    }
}

/// Evaluate the zone for an address.
///
/// - `point`: resolved `(lat, lon)`, if any
/// - `street`: `(street_id, house_number)` when the address came from the index
/// - `in_ags`: the address belongs to one of the tenant's configured
///   municipalities (true for every index hit; the caller decides for
///   geocoder fallbacks)
pub fn zone_for(
    point: Option<(f64, f64)>,
    street: Option<(u32, u32)>,
    in_ags: bool,
    zones: &[ZoneDef],
    overrides: &[StreetOverride],
) -> Option<ZoneMatch> {
    let active = |id: &str| zones.iter().find(|z| z.id == id && z.active);

    // 1. Street overrides.
    if let Some((street_id, number)) = street {
        let hit = overrides.iter().find(|o| {
            o.street_id == street_id
                && o.number_from.is_none_or(|f| number >= f)
                && o.number_to.is_none_or(|t| number <= t)
        });
        if let Some(o) = hit {
            if let Some(z) = active(&o.zone_id) {
                return Some(ZoneMatch::from(z));
            }
        }
    }

    // 2. Geometry, by priority (stable: ties keep table order).
    if let Some((lat, lon)) = point {
        let mut geo: Vec<&ZoneDef> = zones
            .iter()
            .filter(|z| z.active && matches!(z.kind, GeometryKind::Polygon | GeometryKind::Radius))
            .collect();
        geo.sort_by_key(|z| z.priority);
        for z in geo {
            let inside = match &z.geometry {
                Some(Geometry::Polygons(polys)) => {
                    polys.iter().any(|p| polygon_contains(p, lat, lon))
                }
                Some(Geometry::Radius {
                    lat: cl,
                    lon: cn,
                    metres,
                }) => haversine_m((*cl, *cn), (lat, lon)) <= *metres,
                None => false,
            };
            if inside {
                return Some(ZoneMatch::from(z));
            }
        }
    }

    // 3. Default zone.
    if in_ags {
        let mut defaults: Vec<&ZoneDef> = zones
            .iter()
            .filter(|z| z.active && z.kind == GeometryKind::Default)
            .collect();
        defaults.sort_by_key(|z| z.priority);
        if let Some(z) = defaults.first() {
            return Some(ZoneMatch::from(z));
        }
    }
    None
}

/// Great-circle distance in metres.
pub fn haversine_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let r = 6_371_000.0_f64;
    let (lat1, lon1) = (a.0.to_radians(), a.1.to_radians());
    let (lat2, lon2) = (b.0.to_radians(), b.1.to_radians());
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * r * h.sqrt().asin()
}

/// Ray casting on one ring. Points exactly on an edge count as inside.
pub fn ring_contains(ring: &[(f64, f64)], lat: f64, lon: f64) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (yi, xi) = ring[i];
        let (yj, xj) = ring[j];
        // Cast along +x (lon); toggle on each crossing of the horizontal
        // line y = lat.
        if (yi > lat) != (yj > lat) {
            let x_at = xj + (lat - yj) / (yi - yj) * (xi - xj);
            if lon < x_at {
                inside = !inside;
            } else if (lon - x_at).abs() < 1e-12 {
                return true;
            }
        }
        j = i;
    }
    inside
}

/// Inside the outer ring and not inside any hole.
pub fn polygon_contains(polygon: &[Ring], lat: f64, lon: f64) -> bool {
    let Some(outer) = polygon.first() else {
        return false;
    };
    ring_contains(outer, lat, lon) && !polygon[1..].iter().any(|h| ring_contains(h, lat, lon))
}

/// Parse and validate operator-supplied geometry text for `kind`.
///
/// - `polygon`: a GeoJSON `Polygon`, `MultiPolygon`, or a `Feature` wrapping
///   one. Coordinates are `[lon, lat]` (GeoJSON order). Every ring must be
///   closed (first == last) and have ≥ 4 points. With `bbox =
///   (lat_min, lat_max, lon_min, lon_max)` every point must fall inside it —
///   the check that catches a `[lat, lon]` mix-up.
/// - `radius`: `"lat,lon,metres"`.
/// - `none` / `default`: no geometry; returns `Ok(None)`.
pub fn parse_geometry(
    kind: GeometryKind,
    text: Option<&str>,
    bbox: Option<(f64, f64, f64, f64)>,
) -> Result<Option<Geometry>, String> {
    let text = text.map(str::trim).filter(|t| !t.is_empty());
    match kind {
        GeometryKind::None | GeometryKind::Default => Ok(None),
        GeometryKind::Radius => {
            let t = text.ok_or("Umkreis braucht \"lat,lon,meter\"")?;
            let v: Vec<f64> = t
                .split(',')
                .map(|x| {
                    x.trim()
                        .parse::<f64>()
                        .map_err(|_| format!("ungültige Zahl in \"{t}\""))
                })
                .collect::<Result<_, _>>()?;
            if v.len() != 3 {
                return Err("Umkreis braucht genau \"lat,lon,meter\"".into());
            }
            if v[2] <= 0.0 {
                return Err("Umkreis: Meter müssen > 0 sein".into());
            }
            if let Some(b) = bbox {
                if !in_bbox(b, v[0], v[1]) {
                    return Err(format!(
                        "Umkreis-Mittelpunkt ({:.5}, {:.5}) liegt außerhalb des Liefergebiets — lat,lon vertauscht?",
                        v[0], v[1]
                    ));
                }
            }
            Ok(Some(Geometry::Radius {
                lat: v[0],
                lon: v[1],
                metres: v[2],
            }))
        }
        GeometryKind::Polygon => {
            let t = text.ok_or("Polygon braucht GeoJSON")?;
            let json: serde_json::Value =
                serde_json::from_str(t).map_err(|e| format!("GeoJSON ungültig: {e}"))?;
            let geom = if json.get("type").and_then(|t| t.as_str()) == Some("Feature") {
                json.get("geometry")
                    .cloned()
                    .ok_or("Feature ohne geometry")?
            } else {
                json
            };
            let ty = geom.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let coords = geom.get("coordinates").ok_or("GeoJSON ohne coordinates")?;
            let polys_json: Vec<&serde_json::Value> = match ty {
                "Polygon" => vec![coords],
                "MultiPolygon" => coords
                    .as_array()
                    .ok_or("MultiPolygon.coordinates ist kein Array")?
                    .iter()
                    .collect(),
                other => {
                    return Err(format!(
                        "GeoJSON-Typ {other:?} — erwartet Polygon oder MultiPolygon"
                    ))
                }
            };
            let mut polys = Vec::new();
            for p in polys_json {
                let rings_json = p.as_array().ok_or("Polygon ist kein Array von Ringen")?;
                if rings_json.is_empty() {
                    return Err("Polygon ohne Ring".into());
                }
                let mut rings = Vec::new();
                for r in rings_json {
                    let pts = r.as_array().ok_or("Ring ist kein Array")?;
                    let mut ring: Ring = Vec::with_capacity(pts.len());
                    for pt in pts {
                        let xy = pt.as_array().ok_or("Punkt ist kein [lon, lat]")?;
                        if xy.len() < 2 {
                            return Err("Punkt braucht [lon, lat]".into());
                        }
                        let lon = xy[0].as_f64().ok_or("lon ist keine Zahl")?;
                        let lat = xy[1].as_f64().ok_or("lat ist keine Zahl")?;
                        if let Some(b) = bbox {
                            if !in_bbox(b, lat, lon) {
                                return Err(format!(
                                    "Punkt [lon {lon:.5}, lat {lat:.5}] liegt außerhalb des Liefergebiets — GeoJSON ist [lon, lat], nicht [lat, lon]"
                                ));
                            }
                        }
                        ring.push((lat, lon));
                    }
                    if ring.len() < 4 {
                        return Err("Ring braucht mindestens 4 Punkte (geschlossen)".into());
                    }
                    if ring.first() != ring.last() {
                        return Err(
                            "Ring ist nicht geschlossen (erster Punkt ≠ letzter Punkt)".into()
                        );
                    }
                    rings.push(ring);
                }
                polys.push(rings);
            }
            Ok(Some(Geometry::Polygons(polys)))
        }
    }
}

fn in_bbox(b: (f64, f64, f64, f64), lat: f64, lon: f64) -> bool {
    lat >= b.0 && lat <= b.1 && lon >= b.2 && lon <= b.3
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lüdinghausen-ish square around the town centre.
    const CENTRE_POLY: &str = r#"{"type":"Polygon","coordinates":[[[7.42,51.76],[7.46,51.76],[7.46,51.78],[7.42,51.78],[7.42,51.76]]]}"#;
    const BBOX: (f64, f64, f64, f64) = (51.68, 51.86, 7.30, 7.58);

    fn zone(id: &str, prio: i64, fee: i64, kind: GeometryKind, geom: Option<&str>) -> ZoneDef {
        ZoneDef {
            id: id.into(),
            name: id.into(),
            priority: prio,
            fee_cents: fee,
            min_order_cents: None,
            eta_minutes: 30,
            active: true,
            kind,
            geometry: parse_geometry(kind, geom, Some(BBOX)).unwrap(),
        }
    }

    fn fixture() -> Vec<ZoneDef> {
        vec![
            zone("centre", 10, 150, GeometryKind::Polygon, Some(CENTRE_POLY)),
            zone("outer", 100, 250, GeometryKind::Default, None),
        ]
    }

    #[test]
    fn inside_polygon_is_centre() {
        let m = zone_for(Some((51.77, 7.44)), None, true, &fixture(), &[]).unwrap();
        assert_eq!((m.zone_id.as_str(), m.fee_cents), ("centre", 150));
    }

    #[test]
    fn outside_polygon_inside_ags_is_default() {
        let m = zone_for(Some((51.80, 7.50)), None, true, &fixture(), &[]).unwrap();
        assert_eq!((m.zone_id.as_str(), m.fee_cents), ("outer", 250));
    }

    #[test]
    fn outside_ags_is_none() {
        assert!(zone_for(Some((51.80, 7.50)), None, false, &fixture(), &[]).is_none());
        // no point, no street, not in AGS
        assert!(zone_for(None, None, false, &fixture(), &[]).is_none());
    }

    #[test]
    fn street_override_beats_polygon() {
        let ov = [StreetOverride {
            zone_id: "outer".into(),
            street_id: 7,
            number_from: None,
            number_to: None,
        }];
        // Point is inside the centre polygon, but street 7 is routed outer.
        let m = zone_for(Some((51.77, 7.44)), Some((7, 3)), true, &fixture(), &ov).unwrap();
        assert_eq!(m.zone_id, "outer");
    }

    #[test]
    fn house_number_range_override_applies_only_within_range() {
        let ov = [StreetOverride {
            zone_id: "centre".into(),
            street_id: 7,
            number_from: Some(1),
            number_to: Some(20),
        }];
        let z = fixture();
        let far = Some((51.80, 7.50)); // outside the polygon
        assert_eq!(
            zone_for(far, Some((7, 12)), true, &z, &ov).unwrap().zone_id,
            "centre"
        );
        assert_eq!(
            zone_for(far, Some((7, 21)), true, &z, &ov).unwrap().zone_id,
            "outer"
        );
    }

    #[test]
    fn inactive_zone_ignored() {
        let mut z = fixture();
        z[0].active = false;
        let m = zone_for(Some((51.77, 7.44)), None, true, &z, &[]).unwrap();
        assert_eq!(m.zone_id, "outer");
        // An override to an inactive zone falls through too.
        let ov = [StreetOverride {
            zone_id: "centre".into(),
            street_id: 1,
            number_from: None,
            number_to: None,
        }];
        assert_eq!(
            zone_for(Some((51.77, 7.44)), Some((1, 1)), true, &z, &ov)
                .unwrap()
                .zone_id,
            "outer"
        );
    }

    #[test]
    fn priority_tie_break_lower_wins() {
        let mut z = fixture();
        z.push(zone(
            "wide",
            5,
            199,
            GeometryKind::Radius,
            Some("51.77,7.44,5000"),
        ));
        // Both centre (prio 10) and wide (prio 5) contain the point.
        let m = zone_for(Some((51.77, 7.44)), None, true, &z, &[]).unwrap();
        assert_eq!(m.zone_id, "wide");
        z.iter_mut().find(|x| x.id == "wide").unwrap().priority = 50;
        let m = zone_for(Some((51.77, 7.44)), None, true, &z, &[]).unwrap();
        assert_eq!(m.zone_id, "centre");
    }

    #[test]
    fn no_default_zone_means_not_deliverable() {
        let z = vec![zone(
            "centre",
            10,
            150,
            GeometryKind::Polygon,
            Some(CENTRE_POLY),
        )];
        assert!(zone_for(Some((51.80, 7.50)), None, true, &z, &[]).is_none());
    }

    #[test]
    fn geojson_unclosed_ring_rejected() {
        let bad = r#"{"type":"Polygon","coordinates":[[[7.42,51.76],[7.46,51.76],[7.46,51.78],[7.42,51.78]]]}"#;
        let e = parse_geometry(GeometryKind::Polygon, Some(bad), Some(BBOX)).unwrap_err();
        assert!(e.contains("nicht geschlossen"), "{e}");
    }

    #[test]
    fn geojson_lat_lon_mixup_detected_by_bbox() {
        let swapped = r#"{"type":"Polygon","coordinates":[[[51.76,7.42],[51.76,7.46],[51.78,7.46],[51.78,7.42],[51.76,7.42]]]}"#;
        let e = parse_geometry(GeometryKind::Polygon, Some(swapped), Some(BBOX)).unwrap_err();
        assert!(e.contains("[lon, lat]"), "{e}");
        // Without a bbox it parses (nothing to compare against).
        assert!(parse_geometry(GeometryKind::Polygon, Some(swapped), None).is_ok());
    }

    #[test]
    fn geojson_feature_and_multipolygon_with_hole() {
        let mp = r#"{"type":"Feature","properties":{},"geometry":{"type":"MultiPolygon","coordinates":[[[[7.42,51.76],[7.46,51.76],[7.46,51.78],[7.42,51.78],[7.42,51.76]],[[7.43,51.765],[7.44,51.765],[7.44,51.775],[7.43,51.775],[7.43,51.765]]]]}}"#;
        let g = parse_geometry(GeometryKind::Polygon, Some(mp), Some(BBOX))
            .unwrap()
            .unwrap();
        let Geometry::Polygons(p) = g else { panic!() };
        assert!(polygon_contains(&p[0], 51.77, 7.45));
        assert!(!polygon_contains(&p[0], 51.77, 7.435), "inside the hole");
    }

    #[test]
    fn radius_parsing() {
        assert!(parse_geometry(GeometryKind::Radius, Some("51.77,7.44,1500"), Some(BBOX)).is_ok());
        assert!(parse_geometry(GeometryKind::Radius, Some("7.44,51.77,1500"), Some(BBOX)).is_err());
        assert!(parse_geometry(GeometryKind::Radius, Some("51.77,7.44"), None).is_err());
        assert!(parse_geometry(GeometryKind::Radius, Some("51.77,7.44,0"), None).is_err());
        assert_eq!(
            parse_geometry(GeometryKind::Default, Some("junk"), None).unwrap(),
            None
        );
    }

    #[test]
    fn haversine_sanity() {
        // ~1 degree of latitude ≈ 111 km
        let d = haversine_m((51.0, 7.0), (52.0, 7.0));
        assert!((d - 111_195.0).abs() < 200.0, "{d}");
    }
}
