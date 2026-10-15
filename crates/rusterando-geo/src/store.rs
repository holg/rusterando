//! SQLite persistence: the import writes `geo_streets` / `geo_addresses`,
//! the server reads them once into a [`GeoIndex`] at startup, and the zone
//! definitions (`delivery_zones` + `geo_zone_streets`) are read per request.
//!
//! The tables are created by the workspace migration
//! `20260914000002_geo_addresses.sql`; [`SCHEMA_SQL`] mirrors it for tests.

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::import::ImportedStreet;
use crate::index::{GeoIndex, HouseNo, Street};
use crate::zones::{parse_geometry, GeometryKind, StreetOverride, ZoneDef};

/// `(id, name, priority, fee, min_order, eta, is_active, geometry_kind, geometry)`
type ZoneRow = (
    String,
    Option<String>,
    i64,
    i64,
    i64,
    i64,
    i64,
    String,
    Option<String>,
);
/// `(min lat, max lat, min lon, max lon)` — all NULL on an empty table.
type BboxRow = (Option<f64>, Option<f64>, Option<f64>, Option<f64>);

/// Sentinel range bounds so the composite UNIQUE index has no NULLs.
pub const RANGE_OPEN_FROM: i64 = 0;
pub const RANGE_OPEN_TO: i64 = 999_999;

/// Test-only schema (identical to the migration's geo tables).
pub const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS geo_streets (
  id          INTEGER PRIMARY KEY,
  ags         TEXT NOT NULL,
  street_key  TEXT NOT NULL,
  name        TEXT NOT NULL,
  name_norm   TEXT NOT NULL,
  postcode    TEXT NOT NULL,
  city        TEXT NOT NULL,
  UNIQUE (ags, street_key)
);
CREATE INDEX IF NOT EXISTS geo_streets_norm ON geo_streets(name_norm);
CREATE TABLE IF NOT EXISTS geo_addresses (
  street_id   INTEGER NOT NULL REFERENCES geo_streets(id) ON DELETE CASCADE,
  number      INTEGER NOT NULL,
  suffix      TEXT NOT NULL DEFAULT '',
  lat         REAL NOT NULL,
  lon         REAL NOT NULL,
  PRIMARY KEY (street_id, number, suffix)
);
CREATE TABLE IF NOT EXISTS geo_zone_streets (
  id          INTEGER PRIMARY KEY,
  zone_id     TEXT NOT NULL,
  street_id   INTEGER NOT NULL,
  number_from INTEGER NOT NULL DEFAULT 0,
  number_to   INTEGER NOT NULL DEFAULT 999999,
  UNIQUE (street_id, number_from, number_to)
);
";

/// Replace the tenant's whole street/address set in one transaction.
/// Street ids are reassigned; `geo_zone_streets` rows are re-pointed by
/// `(ags, street_key)` so operator street assignments survive a re-import.
/// Returns `(streets, addresses)` written.
pub async fn replace_import(
    pool: &SqlitePool,
    streets: &[ImportedStreet],
) -> anyhow::Result<(usize, usize)> {
    let mut tx = pool.begin().await?;
    // Remember old ids → (ags, key) so overrides can be re-linked.
    let old: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, ags, street_key FROM geo_streets")
            .fetch_all(&mut *tx)
            .await?;
    let old_by_id: HashMap<i64, (String, String)> =
        old.into_iter().map(|(id, a, k)| (id, (a, k))).collect();
    let overrides: Vec<(i64, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, zone_id, street_id, number_from, number_to FROM geo_zone_streets",
    )
    .fetch_all(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM geo_addresses")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM geo_streets")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM geo_zone_streets")
        .execute(&mut *tx)
        .await?;

    let mut new_ids: HashMap<(String, String), i64> = HashMap::new();
    let mut n_addr = 0usize;
    for s in streets {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO geo_streets (ags, street_key, name, name_norm, postcode, city)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING id",
        )
        .bind(&s.ags)
        .bind(&s.street_key)
        .bind(&s.name)
        .bind(&s.name_norm)
        .bind(&s.postcode)
        .bind(&s.city)
        .fetch_one(&mut *tx)
        .await?;
        new_ids.insert((s.ags.clone(), s.street_key.clone()), id);
        for h in &s.numbers {
            sqlx::query(
                "INSERT OR IGNORE INTO geo_addresses (street_id, number, suffix, lat, lon)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(id)
            .bind(h.number as i64)
            .bind(&h.suffix)
            .bind(h.lat)
            .bind(h.lon)
            .execute(&mut *tx)
            .await?;
            n_addr += 1;
        }
    }
    // Re-link overrides whose street still exists.
    let mut relinked = 0usize;
    for (_, zone_id, old_street, from, to) in overrides {
        let Some(key) = old_by_id.get(&old_street) else {
            continue;
        };
        let Some(new_id) = new_ids.get(key) else {
            continue;
        };
        sqlx::query(
            "INSERT OR IGNORE INTO geo_zone_streets (zone_id, street_id, number_from, number_to)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&zone_id)
        .bind(new_id)
        .bind(from)
        .bind(to)
        .execute(&mut *tx)
        .await?;
        relinked += 1;
    }
    tx.commit().await?;
    if relinked > 0 {
        tracing::info!("re-linked {relinked} street→zone assignments after re-import");
    }
    Ok((streets.len(), n_addr))
}

/// Load the runtime index. `Ok(None)` when no streets are imported (the
/// tenant then keeps the geocoder path).
pub async fn load_index(pool: &SqlitePool) -> anyhow::Result<Option<GeoIndex>> {
    let rows: Vec<(i64, String, String, String, String)> =
        sqlx::query_as("SELECT id, name, name_norm, postcode, city FROM geo_streets")
            .fetch_all(pool)
            .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let streets: Vec<Street> = rows
        .into_iter()
        .map(|(id, name, name_norm, postcode, city)| Street {
            id: id as u32,
            name,
            name_norm,
            postcode,
            city,
        })
        .collect();
    let addr: Vec<(i64, i64, String, f64, f64)> =
        sqlx::query_as("SELECT street_id, number, suffix, lat, lon FROM geo_addresses")
            .fetch_all(pool)
            .await?;
    let mut numbers: HashMap<u32, Vec<HouseNo>> = HashMap::new();
    for (sid, number, suffix, lat, lon) in addr {
        numbers.entry(sid as u32).or_default().push(HouseNo {
            number: number as u32,
            suffix,
            lat,
            lon,
        });
    }
    Ok(Some(GeoIndex::new(streets, numbers)))
}

/// Zone definitions + street overrides, read fresh (they change in the
/// dashboard; the address index does not).
pub async fn load_zones(pool: &SqlitePool) -> anyhow::Result<(Vec<ZoneDef>, Vec<StreetOverride>)> {
    let rows: Vec<ZoneRow> = sqlx::query_as(
        "SELECT id, name, priority, fee_cents, min_order_cents, eta_minutes, is_active,
                    geometry_kind, geometry
             FROM delivery_zones ORDER BY priority, id",
    )
    .fetch_all(pool)
    .await?;
    let zones = rows
        .into_iter()
        .map(
            |(id, name, priority, fee, min_order, eta, active, kind, geometry)| {
                let kind = GeometryKind::parse(&kind);
                let geometry = match parse_geometry(kind, geometry.as_deref(), None) {
                    Ok(g) => g,
                    Err(e) => {
                        tracing::warn!(
                            "zone {id}: stored geometry unusable ({e}) — treated as none"
                        );
                        None
                    }
                };
                ZoneDef {
                    name: name.unwrap_or_else(|| id.clone()),
                    id,
                    priority,
                    fee_cents: fee,
                    min_order_cents: (min_order > 0).then_some(min_order),
                    eta_minutes: eta,
                    active: active != 0,
                    kind,
                    geometry,
                }
            },
        )
        .collect();
    let ov: Vec<(String, i64, i64, i64)> =
        sqlx::query_as("SELECT zone_id, street_id, number_from, number_to FROM geo_zone_streets")
            .fetch_all(pool)
            .await?;
    let overrides = ov
        .into_iter()
        .map(|(zone_id, street_id, from, to)| StreetOverride {
            zone_id,
            street_id: street_id as u32,
            number_from: (from > RANGE_OPEN_FROM).then_some(from as u32),
            number_to: (to < RANGE_OPEN_TO).then_some(to as u32),
        })
        .collect();
    Ok((zones, overrides))
}

/// Bounding box of the imported addresses, for GeoJSON validation.
pub async fn address_bbox(pool: &SqlitePool) -> anyhow::Result<Option<(f64, f64, f64, f64)>> {
    let row: Option<BboxRow> =
        sqlx::query_as("SELECT MIN(lat), MAX(lat), MIN(lon), MAX(lon) FROM geo_addresses")
            .fetch_optional(pool)
            .await?;
    Ok(match row {
        Some((Some(a), Some(b), Some(c), Some(d))) => Some((a, b, c, d)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::CadastreNrw;
    use crate::sample;

    async fn pool() -> SqlitePool {
        let p = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for stmt in SCHEMA_SQL
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(stmt).execute(&p).await.unwrap();
        }
        // Minimal delivery_zones shape (the real one comes from migrations).
        sqlx::query(
            "CREATE TABLE delivery_zones (id TEXT PRIMARY KEY, name TEXT, priority INTEGER NOT NULL DEFAULT 100,
             fee_cents INTEGER NOT NULL, min_order_cents INTEGER NOT NULL DEFAULT 0, eta_minutes INTEGER NOT NULL DEFAULT 30,
             is_active INTEGER NOT NULL DEFAULT 1, geometry_kind TEXT NOT NULL DEFAULT 'none', geometry TEXT)",
        )
        .execute(&p)
        .await
        .unwrap();
        p
    }

    #[tokio::test]
    async fn import_round_trips_through_sqlite() {
        let p = pool().await;
        let cfg = sample::config();
        let r = CadastreNrw::import_reader(&cfg, sample::SAMPLE_UTF8.as_bytes()).unwrap();
        let (ns, na) = replace_import(&p, &r.streets).await.unwrap();
        assert_eq!(
            (ns as u64, na as u64),
            (sample::STREETS_LH, sample::ROWS_KEPT_LH)
        );
        let idx = load_index(&p).await.unwrap().unwrap();
        assert_eq!(idx.streets().len() as u64, sample::STREETS_LH);
        assert_eq!(idx.address_count() as u64, sample::ROWS_KEPT_LH);
        let s = idx.suggest_streets("hermann");
        assert_eq!(s[0].name, "Hermann-Löns-Straße");
        let r = idx.resolve(s[0].id, "3").unwrap();
        assert!(
            (r.lat - 51.77).abs() < 0.02 && (r.lon - 7.43).abs() < 0.02,
            "{r:?}"
        );
        assert!(address_bbox(&p).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn empty_store_gives_no_index() {
        let p = pool().await;
        assert!(load_index(&p).await.unwrap().is_none());
        assert!(address_bbox(&p).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn overrides_survive_reimport_and_zones_load() {
        let p = pool().await;
        let cfg = sample::config();
        let r = CadastreNrw::import_reader(&cfg, sample::SAMPLE_UTF8.as_bytes()).unwrap();
        replace_import(&p, &r.streets).await.unwrap();
        sqlx::query("INSERT INTO delivery_zones (id, name, priority, fee_cents, geometry_kind) VALUES ('c','Centre',10,150,'none'), ('d','Outer',100,250,'default')")
            .execute(&p).await.unwrap();
        let idx = load_index(&p).await.unwrap().unwrap();
        let am_markt = idx.find_street_exact("Am Markt").unwrap().id as i64;
        sqlx::query("INSERT INTO geo_zone_streets (zone_id, street_id, number_from, number_to) VALUES ('c', ?1, 0, 999999)")
            .bind(am_markt).execute(&p).await.unwrap();
        // Re-import: ids may change, the assignment must follow the street.
        let r2 = CadastreNrw::import_reader(&cfg, sample::SAMPLE_UTF8.as_bytes()).unwrap();
        replace_import(&p, &r2.streets).await.unwrap();
        let idx2 = load_index(&p).await.unwrap().unwrap();
        let am_markt2 = idx2.find_street_exact("Am Markt").unwrap().id;
        let (zones, ov) = load_zones(&p).await.unwrap();
        assert_eq!(zones.len(), 2);
        assert_eq!(zones[0].id, "c");
        assert_eq!(ov.len(), 1);
        assert_eq!(ov[0].street_id, am_markt2);
        assert_eq!((ov[0].number_from, ov[0].number_to), (None, None));
        let m = crate::zones::zone_for(None, Some((am_markt2, 2)), true, &zones, &ov).unwrap();
        assert_eq!(m.fee_cents, 150);
        let m =
            crate::zones::zone_for(None, Some((am_markt2 + 1000, 2)), true, &zones, &ov).unwrap();
        assert_eq!(m.fee_cents, 250);
    }
}
