//! Local address lookup (NRW cadastre) — the checkout-facing half.
//!
//! The pure logic lives in the `rusterando-geo` crate. This module owns:
//!
//! - [`GeoIndexHandle`]: the per-tenant `Arc<GeoIndex>` (reloadable) that the
//!   server puts into Leptos context next to the pool
//! - the admin switch `app_settings.address_lookup_mode`
//!   (`nominatim` = the existing geocoder + suburb routing, `local` = the
//!   cadastre index + geometry zones)
//! - the three server functions the checkout autocomplete calls
//! - [`ssr::LocalLookup`]: what `validate_address` needs to decide a
//!   delivery-zone locally
//!
//! Nothing here runs unless the tenant has an imported index AND the admin
//! switched the mode to `local`; otherwise every caller sees "not enabled"
//! and the old path is used unchanged.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreetSuggestion {
    pub id: u32,
    pub name: String,
    pub postcode: String,
    pub city: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedAddress {
    pub street_id: u32,
    pub street: String,
    pub house_number: String,
    pub postcode: String,
    pub city: String,
    pub lat: f64,
    pub lon: f64,
}

/// Admin-visible state of the local lookup for this tenant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GeoLookupStatus {
    /// `nominatim` | `local`
    pub mode: String,
    pub index_loaded: bool,
    pub streets: usize,
    pub addresses: usize,
    /// `GEO_BACKEND` configured for this tenant.
    pub backend_configured: bool,
}

#[cfg(feature = "ssr")]
pub mod ssr {
    use std::sync::{Arc, RwLock};

    use leptos::prelude::ServerFnError;
    use rusterando_geo::{GeoIndex, StreetOverride, ZoneDef, ZoneMatch};
    use sqlx::SqlitePool;

    /// Per-tenant index handle. `None` inside = no import yet. Reloadable
    /// (admin "Index neu laden" after `geo-import`) without a restart.
    #[derive(Clone, Default)]
    pub struct GeoIndexHandle {
        inner: Arc<RwLock<Option<Arc<GeoIndex>>>>,
        pub backend_configured: bool,
        /// The tenant's `GEO_*` env keys — the DEFAULTS; `app_settings`
        /// (`geo_ags`, `geo_postcode_map`) override them when non-empty.
        pub env_config: Arc<Option<rusterando_geo::GeoConfig>>,
    }

    impl GeoIndexHandle {
        pub fn new(index: Option<GeoIndex>, env_config: Option<rusterando_geo::GeoConfig>) -> Self {
            Self {
                inner: Arc::new(RwLock::new(index.map(Arc::new))),
                backend_configured: env_config.is_some(),
                env_config: Arc::new(env_config),
            }
        }
        pub fn get(&self) -> Option<Arc<GeoIndex>> {
            self.inner.read().ok().and_then(|g| g.clone())
        }
        pub fn set(&self, index: Option<GeoIndex>) {
            if let Ok(mut g) = self.inner.write() {
                *g = index.map(Arc::new);
            }
        }
        /// Load (or reload) from the tenant DB. Logs and keeps the old
        /// index on error.
        pub async fn reload(&self, pool: &SqlitePool) -> anyhow::Result<usize> {
            let idx = rusterando_geo::store::load_index(pool).await?;
            let n = idx.as_ref().map(|i| i.streets().len()).unwrap_or(0);
            self.set(idx);
            Ok(n)
        }
    }

    /// The admin switch.
    pub const MODE_LOCAL: &str = "local";
    pub const MODE_NOMINATIM: &str = "nominatim";

    pub async fn lookup_mode(db: &SqlitePool) -> String {
        sqlx::query_scalar::<_, String>(
            "SELECT value FROM app_settings WHERE key = 'address_lookup_mode'",
        )
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| v == MODE_LOCAL)
        .unwrap_or_else(|| MODE_NOMINATIM.to_string())
    }

    pub fn index_from_context() -> Option<Arc<GeoIndex>> {
        leptos::prelude::use_context::<GeoIndexHandle>().and_then(|h| h.get())
    }

    /// Everything `validate_address` needs when the local path is active.
    pub struct LocalLookup {
        pub index: Arc<GeoIndex>,
        pub zones: Vec<ZoneDef>,
        pub overrides: Vec<StreetOverride>,
        /// City names of the configured municipalities (from the index):
        /// display form for messages, normalised for the `in_ags` test on
        /// geocoder fallbacks.
        pub cities: Vec<String>,
        pub cities_norm: Vec<String>,
    }

    impl LocalLookup {
        /// `None` unless mode == local AND an index is loaded.
        pub async fn active(db: &SqlitePool) -> Option<LocalLookup> {
            if lookup_mode(db).await != MODE_LOCAL {
                return None;
            }
            let index = index_from_context()?;
            let (zones, overrides) = match rusterando_geo::store::load_zones(db).await {
                Ok(z) => z,
                Err(e) => {
                    log::warn!("local lookup: load zones failed ({e}) — falling back");
                    return None;
                }
            };
            let mut cities: Vec<String> = index.streets().iter().map(|s| s.city.clone()).collect();
            cities.sort();
            cities.dedup();
            let mut cities_norm: Vec<String> = cities
                .iter()
                .map(|c| crate::pages::locality::normalize(c))
                .collect();
            cities_norm.sort();
            cities_norm.dedup();
            Some(LocalLookup {
                index,
                zones,
                overrides,
                cities,
                cities_norm,
            })
        }

        pub fn in_ags(&self, city: &str) -> bool {
            let n = crate::pages::locality::normalize(city);
            !n.is_empty() && self.cities_norm.contains(&n)
        }

        /// Zone for a resolved cadastre address.
        pub fn zone_for_resolved(
            &self,
            street_id: u32,
            house_number: u32,
            lat: f64,
            lon: f64,
        ) -> Option<ZoneMatch> {
            rusterando_geo::zone_for(
                Some((lat, lon)),
                Some((street_id, house_number)),
                true,
                &self.zones,
                &self.overrides,
            )
        }

        /// Zone for a geocoder point (unverified address).
        pub fn zone_for_point(&self, lat: f64, lon: f64, city: &str) -> Option<ZoneMatch> {
            rusterando_geo::zone_for(
                Some((lat, lon)),
                None,
                self.in_ags(city),
                &self.zones,
                &self.overrides,
            )
        }
    }

    /// Whether the checkout should show the autocomplete (mode local +
    /// index loaded). Cheap: one setting read.
    pub async fn autocomplete_enabled(db: &SqlitePool) -> bool {
        lookup_mode(db).await == MODE_LOCAL && index_from_context().is_some()
    }

    pub async fn setting(db: &SqlitePool, key: &str) -> String {
        sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?1")
            .bind(key)
            .fetch_optional(db)
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    pub async fn upsert_setting(
        db: &SqlitePool,
        key: &str,
        value: &str,
        label: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO app_settings (key, value, label_de) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(key)
        .bind(value)
        .bind(label)
        .execute(db)
        .await
        .map(|_| ())
    }

    /// The tenant's effective geo config: env defaults from the handle,
    /// municipality list + postcode map overridden by `app_settings` when set.
    /// `(config, source)` where source is "db", "env" or "none".
    pub async fn effective_config(
        db: &SqlitePool,
    ) -> (Option<rusterando_geo::GeoConfig>, &'static str) {
        let handle = leptos::prelude::use_context::<GeoIndexHandle>();
        let env_cfg = handle.as_ref().and_then(|h| (*h.env_config).clone());
        let ags = setting(db, "geo_ags").await;
        let pmap = setting(db, "geo_postcode_map").await;
        if ags.trim().is_empty() {
            return (
                env_cfg,
                if handle.is_some() && env_cfg_is_some(&handle) {
                    "env"
                } else {
                    "none"
                },
            );
        }
        let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if let Some(e) = &env_cfg {
            if let Some(d) = &e.data_dir {
                vars.insert("GEO_DATA_DIR".into(), d.clone());
            }
            let b = e.expect_bbox;
            vars.insert(
                "GEO_EXPECT_BBOX".into(),
                format!("{},{},{},{}", b.0, b.1, b.2, b.3),
            );
        }
        // The sanity box follows the shop: "Alles aus Adresse einrichten"
        // stores one around the new location, otherwise the env default
        // (written for the original town) would veto every later import.
        let bbox = setting(db, "geo_expect_bbox").await;
        if !bbox.trim().is_empty() {
            vars.insert("GEO_EXPECT_BBOX".into(), bbox.trim().to_string());
        }
        vars.insert("GEO_BACKEND".into(), "cadastre_nrw".into());
        vars.insert("GEO_AGS".into(), ags);
        vars.insert("GEO_POSTCODE_MAP".into(), pmap);
        (rusterando_geo::GeoConfig::from_vars(&vars), "db")
    }

    fn env_cfg_is_some(h: &Option<GeoIndexHandle>) -> bool {
        h.as_ref().is_some_and(|h| h.env_config.is_some())
    }

    /// The in-process import: effective config + tenant zones → split files
    /// (or the cached zip) → `geo_streets`/`geo_addresses` → index reload.
    pub async fn run_import(db: &SqlitePool) -> Result<String, ServerFnError> {
        use rusterando_geo::import::{AddressSource, CadastreNrw, ImportResult};
        let handle = leptos::prelude::use_context::<GeoIndexHandle>()
            .ok_or_else(|| ServerFnError::new("Adressindex-Handle fehlt im Kontext"))?;
        let (cfg, _) = effective_config(db).await;
        let cfg = cfg.ok_or_else(|| {
            ServerFnError::new("Keine Gemeinden konfiguriert (GEO_AGS / Gemeinden-Liste).")
        })?;
        if cfg.ags.is_empty() {
            return Err(ServerFnError::new("Gemeinden-Liste ist leer."));
        }
        let dir = split_dir();
        let zip = dir.parent().map(|p| p.join("gebref_EPSG25832_ASCII.zip"));
        let sources: Vec<std::path::PathBuf> = {
            let split: Vec<_> = cfg
                .ags
                .iter()
                .map(|a| dir.join(format!("{a}.txt")))
                .collect();
            if split.iter().all(|p| p.is_file()) {
                split
            } else if let Some(z) = zip.filter(|z| z.is_file()) {
                vec![z]
            } else {
                return Err(ServerFnError::new(format!(
                    "Keine Adressdaten auf dem Server: weder Gemeindedateien unter {} noch das Export-Zip. Einmal `geo-import split --download --out-dir {}` ausführen.",
                    dir.display(),
                    dir.display()
                )));
            }
        };
        let (zones, overrides) = rusterando_geo::store::load_zones(db)
            .await
            .map_err(|e| ServerFnError::new(format!("load zones: {e}")))?;
        let cfg2 = cfg.clone();
        let mut result: ImportResult =
            tokio::task::spawn_blocking(move || -> anyhow::Result<ImportResult> {
                let mut merged: Option<ImportResult> = None;
                for p in &sources {
                    let r = CadastreNrw.build_index(&cfg2, p)?;
                    match &mut merged {
                        None => merged = Some(r),
                        Some(m) => m.merge(r),
                    }
                }
                Ok(merged.expect("at least one source"))
            })
            .await
            .map_err(|e| ServerFnError::new(format!("import task: {e}")))?
            .map_err(|e| ServerFnError::new(format!("Import: {e}")))?;
        result
            .summary
            .check(cfg.expect_bbox)
            .map_err(|e| ServerFnError::new(format!("Import abgebrochen: {e}")))?;
        result.retain_in_zones(&zones, &overrides);
        if result.summary.addresses == 0 {
            return Err(ServerFnError::new(
                "Keine Adresse liegt in den Lieferzonen — erst Zonen-Geometrie (Umkreis/Polygon/Standard) anlegen.",
            ));
        }
        let (ns, na) = rusterando_geo::store::replace_import(db, &result.streets)
            .await
            .map_err(|e| ServerFnError::new(format!("write: {e}")))?;
        handle
            .reload(db)
            .await
            .map_err(|e| ServerFnError::new(format!("Index laden: {e}")))?;
        Ok(format!(
            "{}gespeichert: {ns} Straßen, {na} Adressen — Index neu geladen.",
            result.summary.report()
        ))
    }

    /// `GEO_DATA_DIR` (download cache + split), default `./tmp/geo`.
    pub fn data_dir() -> std::path::PathBuf {
        let base = leptos::prelude::use_context::<GeoIndexHandle>()
            .and_then(|h| (*h.env_config).clone())
            .and_then(|c| c.data_dir)
            .unwrap_or_else(|| "./tmp/geo".to_string());
        std::path::PathBuf::from(base)
    }

    /// Where the split municipality files live: `<GEO_DATA_DIR>/nrw`.
    pub fn split_dir() -> std::path::PathBuf {
        data_dir().join("nrw")
    }

    /// The one background job this process runs for the NRW data: download
    /// (cached) + split. Process-wide because the data dir is shared by
    /// every tenant on a server; a second start while one runs is refused.
    /// Runs on the blocking pool — a proxied request would not survive the
    /// minute or two it takes, so the dashboard polls [`NrwJob::snapshot`].
    #[derive(Debug, Clone, Default)]
    pub struct NrwJob {
        pub running: bool,
        /// `None` = never ran in this process.
        pub ok: Option<bool>,
        pub log: Vec<String>,
        /// The last `\r`-prefixed progress line (download counter).
        pub progress: String,
    }

    fn nrw_job() -> &'static std::sync::Mutex<NrwJob> {
        static JOB: std::sync::OnceLock<std::sync::Mutex<NrwJob>> = std::sync::OnceLock::new();
        JOB.get_or_init(|| std::sync::Mutex::new(NrwJob::default()))
    }

    impl NrwJob {
        pub fn snapshot() -> NrwJob {
            nrw_job().lock().map(|j| j.clone()).unwrap_or_default()
        }

        /// Start the job unless one is running. `false` = already running.
        pub fn start(data_dir: std::path::PathBuf, force_download: bool) -> bool {
            {
                let Ok(mut j) = nrw_job().lock() else {
                    return false;
                };
                if j.running {
                    return false;
                }
                *j = NrwJob {
                    running: true,
                    ok: None,
                    log: vec![format!("NRW-Daten → {}", data_dir.display())],
                    progress: String::new(),
                };
            }
            tokio::task::spawn_blocking(move || {
                let mut push = |line: String| {
                    if let Ok(mut j) = nrw_job().lock() {
                        if let Some(p) = line.strip_prefix('\r') {
                            j.progress = p.to_string();
                        } else {
                            j.progress.clear();
                            j.log.push(line);
                        }
                    }
                };
                let res = rusterando_geo::import::prepare_nrw(&data_dir, force_download, &mut push);
                if let Ok(mut j) = nrw_job().lock() {
                    j.running = false;
                    j.progress.clear();
                    match res {
                        Ok(_) => {
                            j.ok = Some(true);
                            j.log.push(
                                "fertig — Gemeinden können jetzt gesucht/eingerichtet werden"
                                    .into(),
                            );
                        }
                        Err(e) => {
                            j.ok = Some(false);
                            j.log.push(format!("FEHLER: {e:#}"));
                        }
                    }
                }
            });
            true
        }
    }
}

// ---------------------------------------------------------------------------
// Server functions used by the checkout autocomplete
// ---------------------------------------------------------------------------

#[server(name = SuggestStreets, prefix = "/api", endpoint = "suggest_streets")]
pub async fn suggest_streets(q: String) -> Result<Vec<StreetSuggestion>, ServerFnError> {
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    if !ssr::autocomplete_enabled(&db).await {
        return Ok(Vec::new());
    }
    let Some(idx) = ssr::index_from_context() else {
        return Ok(Vec::new());
    };
    Ok(idx
        .suggest_streets(&q)
        .into_iter()
        .map(|s| StreetSuggestion {
            id: s.id,
            name: s.name.clone(),
            postcode: s.postcode.clone(),
            city: s.city.clone(),
        })
        .collect())
}

#[server(name = SuggestHouseNumbers, prefix = "/api", endpoint = "suggest_house_numbers")]
pub async fn suggest_house_numbers(
    street_id: u32,
    q: String,
) -> Result<Vec<String>, ServerFnError> {
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    if !ssr::autocomplete_enabled(&db).await {
        return Ok(Vec::new());
    }
    let Some(idx) = ssr::index_from_context() else {
        return Ok(Vec::new());
    };
    Ok(idx.suggest_house_numbers(street_id, &q))
}

#[server(name = ResolveAddress, prefix = "/api", endpoint = "resolve_address")]
pub async fn resolve_address(
    street_id: u32,
    house_number: String,
) -> Result<Option<ResolvedAddress>, ServerFnError> {
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    if !ssr::autocomplete_enabled(&db).await {
        return Ok(None);
    }
    let Some(idx) = ssr::index_from_context() else {
        return Ok(None);
    };
    Ok(idx
        .resolve(street_id, &house_number)
        .map(|r| ResolvedAddress {
            street_id: r.street_id,
            street: r.street,
            house_number: r.house_number,
            postcode: r.postcode,
            city: r.city,
            lat: r.lat,
            lon: r.lon,
        }))
}

// ---------------------------------------------------------------------------
// Admin: status + switch + reload
// ---------------------------------------------------------------------------

#[server(name = GeoLookupStatusFn, prefix = "/api", endpoint = "geo_lookup_status")]
pub async fn geo_lookup_status() -> Result<GeoLookupStatus, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let handle = use_context::<ssr::GeoIndexHandle>();
    let idx = handle.as_ref().and_then(|h| h.get());
    Ok(GeoLookupStatus {
        mode: ssr::lookup_mode(&db).await,
        index_loaded: idx.is_some(),
        streets: idx.as_ref().map(|i| i.streets().len()).unwrap_or(0),
        addresses: idx.as_ref().map(|i| i.address_count()).unwrap_or(0),
        backend_configured: handle.map(|h| h.backend_configured).unwrap_or(false),
    })
}

/// `mode`: `local` | `nominatim`. Switching to `local` without a loaded
/// index is refused — the checkout would have nothing to look up.
#[server(name = SetAddressLookupMode, prefix = "/api", endpoint = "set_address_lookup_mode")]
pub async fn set_address_lookup_mode(mode: String) -> Result<(), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let mode = mode.trim().to_ascii_lowercase();
    if mode != ssr::MODE_LOCAL && mode != ssr::MODE_NOMINATIM {
        return Err(ServerFnError::new("Modus muss local oder nominatim sein"));
    }
    if mode == ssr::MODE_LOCAL && ssr::index_from_context().is_none() {
        return Err(ServerFnError::new(
            "Kein Adressindex geladen — erst `geo-import` ausführen und den Index neu laden.",
        ));
    }
    sqlx::query(
        "INSERT INTO app_settings (key, value, label_de) VALUES ('address_lookup_mode', ?1, 'Adressprüfung')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(&mode)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("save mode: {e}")))?;
    Ok(())
}

/// Reload the index from the tenant DB after a `geo-import` run. Returns
/// the street count.
#[server(name = ReloadGeoIndex, prefix = "/api", endpoint = "reload_geo_index")]
pub async fn reload_geo_index() -> Result<usize, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let handle = use_context::<ssr::GeoIndexHandle>()
        .ok_or_else(|| ServerFnError::new("Adressindex-Handle fehlt im Kontext"))?;
    handle
        .reload(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("Index laden: {e}")))
}

// ---------------------------------------------------------------------------
// Admin: street → zone assignments + preview (zones page)
// ---------------------------------------------------------------------------

/// One imported street with its (first) zone assignment, for the admin table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeoStreetAdminRow {
    pub id: u32,
    pub name: String,
    pub postcode: String,
    pub city: String,
    pub house_count: usize,
    /// Assigned zone id, `None` = follows geometry / default.
    pub zone_id: Option<String>,
    pub number_from: Option<u32>,
    pub number_to: Option<u32>,
}

#[server(name = ListGeoStreetsAdmin, prefix = "/api", endpoint = "list_geo_streets_admin")]
pub async fn list_geo_streets_admin() -> Result<Vec<GeoStreetAdminRow>, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let Some(idx) = ssr::index_from_context() else {
        return Ok(Vec::new());
    };
    let (_, overrides) = rusterando_geo::store::load_zones(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("load zones: {e}")))?;
    Ok(idx
        .streets()
        .iter()
        .map(|s| {
            let ov = overrides.iter().find(|o| o.street_id == s.id);
            GeoStreetAdminRow {
                id: s.id,
                name: s.name.clone(),
                postcode: s.postcode.clone(),
                city: s.city.clone(),
                house_count: idx.house_numbers(s.id).len(),
                zone_id: ov.map(|o| o.zone_id.clone()),
                number_from: ov.and_then(|o| o.number_from),
                number_to: ov.and_then(|o| o.number_to),
            }
        })
        .collect())
}

/// Assign a street (optionally a house-number range) to a zone; an empty
/// `zone_id` clears the assignment. Explicit assignments beat geometry.
#[server(name = SetGeoStreetZone, prefix = "/api", endpoint = "set_geo_street_zone")]
pub async fn set_geo_street_zone(
    street_id: u32,
    zone_id: String,
    number_from: Option<u32>,
    number_to: Option<u32>,
) -> Result<(), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let zone_id = zone_id.trim().to_string();
    if let (Some(f), Some(t)) = (number_from, number_to) {
        if f > t {
            return Err(ServerFnError::new("Hausnummer von > bis"));
        }
    }
    let mut tx = db
        .begin()
        .await
        .map_err(|e| ServerFnError::new(format!("tx: {e}")))?;
    sqlx::query("DELETE FROM geo_zone_streets WHERE street_id = ?1")
        .bind(street_id as i64)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("clear assignment: {e}")))?;
    if !zone_id.is_empty() {
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT id FROM delivery_zones WHERE id = ?1")
                .bind(&zone_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| ServerFnError::new(format!("zone lookup: {e}")))?;
        if exists.is_none() {
            return Err(ServerFnError::new("Zone nicht gefunden"));
        }
        sqlx::query(
            "INSERT INTO geo_zone_streets (zone_id, street_id, number_from, number_to)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&zone_id)
        .bind(street_id as i64)
        .bind(
            number_from
                .map(i64::from)
                .unwrap_or(rusterando_geo::store::RANGE_OPEN_FROM),
        )
        .bind(
            number_to
                .map(i64::from)
                .unwrap_or(rusterando_geo::store::RANGE_OPEN_TO),
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("assign: {e}")))?;
    }
    tx.commit()
        .await
        .map_err(|e| ServerFnError::new(format!("commit: {e}")))?;
    Ok(())
}

/// What the checkout would decide for `street + house number` right now.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ZonePreview {
    pub street: String,
    pub house_number: String,
    pub lat: f64,
    pub lon: f64,
    /// `None` = outside the delivery area.
    pub zone_name: Option<String>,
    pub fee_cents: Option<i64>,
}

#[server(name = PreviewGeoZone, prefix = "/api", endpoint = "preview_geo_zone")]
pub async fn preview_geo_zone(
    street_id: u32,
    house_number: String,
) -> Result<Option<ZonePreview>, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let Some(idx) = ssr::index_from_context() else {
        return Err(ServerFnError::new("Kein Adressindex geladen"));
    };
    let Some(r) = idx.resolve(street_id, &house_number) else {
        return Ok(None);
    };
    let (zones, overrides) = rusterando_geo::store::load_zones(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("load zones: {e}")))?;
    let number = rusterando_geo::normalize::parse_house_number(&r.house_number)
        .map(|(n, _)| n)
        .unwrap_or(0);
    let m = rusterando_geo::zone_for(
        Some((r.lat, r.lon)),
        Some((street_id, number)),
        true,
        &zones,
        &overrides,
    );
    Ok(Some(ZonePreview {
        street: r.street,
        house_number: r.house_number,
        lat: r.lat,
        lon: r.lon,
        zone_name: m.as_ref().map(|z| z.name.clone()),
        fee_cents: m.map(|z| z.fee_cents),
    }))
}

// ---------------------------------------------------------------------------
// Admin: municipalities (DB overrides env), in-process import, shop location
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Municipality {
    pub ags: String,
    pub postcode: String,
    pub city: String,
    /// The split file `<data_dir>/nrw/<ags>.txt` exists on this server.
    pub split_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GeoMunicipalities {
    pub list: Vec<Municipality>,
    /// `db` (set in the dashboard), `env` (from the tenant's .env), `none`.
    pub source: String,
    /// `<data_dir>/nrw/index.csv` exists — `geo-import split` ran here.
    pub split_index_available: bool,
    pub data_dir: String,
}

#[server(name = GeoMunicipalitiesFn, prefix = "/api", endpoint = "geo_municipalities")]
pub async fn geo_municipalities() -> Result<GeoMunicipalities, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let (cfg, source) = ssr::effective_config(&db).await;
    let dir = ssr::split_dir();
    let list = cfg
        .as_ref()
        .map(|c| {
            c.ags
                .iter()
                .map(|a| {
                    let (postcode, city) = c.postcode_city(a).cloned().unwrap_or_default();
                    Municipality {
                        ags: a.clone(),
                        postcode,
                        city,
                        split_available: dir.join(format!("{a}.txt")).is_file(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(GeoMunicipalities {
        list,
        source: source.to_string(),
        split_index_available: dir.join("index.csv").is_file(),
        data_dir: dir
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    })
}

/// State of the NRW data on this server + the background job preparing it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NrwDataStatus {
    pub data_dir: String,
    /// The ~100 MB export is cached on disk.
    pub zip_cached: bool,
    pub zip_mb: u64,
    /// `<data_dir>/nrw/index.csv` exists.
    pub split_available: bool,
    /// … and carries bounding boxes (needed for the radius search).
    pub split_current: bool,
    pub municipalities: usize,
    pub running: bool,
    /// `None` = no job ran in this process yet.
    pub ok: Option<bool>,
    pub log: Vec<String>,
    pub progress: String,
}

#[cfg(feature = "ssr")]
fn nrw_data_status_now() -> NrwDataStatus {
    let data_dir = ssr::data_dir();
    let split = data_dir.join("nrw");
    let zip = rusterando_geo::import::nrw_zip_path(&data_dir);
    let zip_len = std::fs::metadata(&zip).map(|m| m.len()).unwrap_or(0);
    let index = rusterando_geo::import::read_split_index(&split).unwrap_or_default();
    let job = ssr::NrwJob::snapshot();
    NrwDataStatus {
        data_dir: data_dir.display().to_string(),
        zip_cached: zip_len > 1_000_000,
        zip_mb: zip_len / 1_000_000,
        split_available: !index.is_empty(),
        split_current: !index.is_empty() && index.iter().all(|m| m.bbox.is_some()),
        municipalities: index.len(),
        running: job.running,
        ok: job.ok,
        log: job.log,
        progress: job.progress,
    }
}

/// Current state of the NRW data + job (polled by the dashboard while a
/// job runs).
#[server(name = NrwDataStatusFn, prefix = "/api", endpoint = "nrw_data_status")]
pub async fn nrw_data_status() -> Result<NrwDataStatus, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    Ok(nrw_data_status_now())
}

/// Start the download (cached unless `force`) + split in the background.
/// Returns at once; poll [`nrw_data_status`].
#[server(name = PrepareNrwData, prefix = "/api", endpoint = "prepare_nrw_data")]
pub async fn prepare_nrw_data(force: bool) -> Result<NrwDataStatus, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    if !ssr::NrwJob::start(ssr::data_dir(), force) {
        return Err(ServerFnError::new(
            "Es läuft bereits eine Aufbereitung — bitte warten.",
        ));
    }
    Ok(nrw_data_status_now())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MunicipalityHit {
    pub ags: String,
    pub name: String,
    pub rows: u64,
}

/// Search the split index (`index.csv`) by municipality name.
#[server(name = SearchMunicipalities, prefix = "/api", endpoint = "search_municipalities")]
pub async fn search_municipalities(q: String) -> Result<Vec<MunicipalityHit>, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let dir = ssr::split_dir();
    let index = rusterando_geo::import::read_split_index(&dir).map_err(|_| {
        ServerFnError::new(
            "Gemeindeliste fehlt — zuerst „NRW-Daten herunterladen & aufbereiten“ (Abschnitt „NRW-Daten“).",
        )
    })?;
    let qn = crate::pages::locality::normalize(&q);
    let mut hits: Vec<MunicipalityHit> = index
        .into_iter()
        .filter(|m| {
            qn.is_empty()
                || crate::pages::locality::normalize(&m.name).contains(&qn)
                || m.ags.starts_with(&qn)
        })
        .map(|m| MunicipalityHit {
            ags: m.ags,
            name: m.name,
            rows: m.rows,
        })
        .collect();
    hits.sort_by(|a, b| a.name.cmp(&b.name));
    hits.truncate(12);
    Ok(hits)
}

/// Save the municipality list (same string formats as GEO_AGS /
/// GEO_POSTCODE_MAP). Empty `ags` clears the override — back to the env.
#[server(name = SetGeoMunicipalities, prefix = "/api", endpoint = "set_geo_municipalities")]
pub async fn set_geo_municipalities(
    ags: String,
    postcode_map: String,
) -> Result<(), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let ags_list: Vec<String> = ags
        .split(',')
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect();
    for a in &ags_list {
        if a.len() != 8 || !a.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ServerFnError::new(format!(
                "AGS {a:?} muss 8 Ziffern haben"
            )));
        }
    }
    for triple in postcode_map
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        let parts: Vec<&str> = triple.splitn(3, ':').collect();
        if parts.len() != 3 || parts[1].trim().len() != 5 || parts[2].trim().is_empty() {
            return Err(ServerFnError::new(format!(
                "Eintrag {triple:?} muss AGS:PLZ:Ort sein"
            )));
        }
        if !ags_list.iter().any(|a| a == parts[0].trim()) {
            return Err(ServerFnError::new(format!(
                "PLZ-Eintrag für unbekannte Gemeinde {}",
                parts[0]
            )));
        }
    }
    ssr::upsert_setting(&db, "geo_ags", &ags_list.join(","), "Gemeinden (AGS)")
        .await
        .map_err(|e| ServerFnError::new(format!("save: {e}")))?;
    ssr::upsert_setting(
        &db,
        "geo_postcode_map",
        postcode_map.trim(),
        "Gemeinden → PLZ",
    )
    .await
    .map_err(|e| ServerFnError::new(format!("save: {e}")))?;
    Ok(())
}

/// Run the address import in-process from the split files (or the cached
/// zip) with the tenant's effective config and zones, then reload the
/// index. Returns the import summary.
#[server(name = ImportGeoAddresses, prefix = "/api", endpoint = "import_geo_addresses")]
pub async fn import_geo_addresses() -> Result<String, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    ssr::run_import(&db).await
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ShopLocation {
    pub street: String,
    pub plz_city: String,
    pub lat: f64,
    pub lon: f64,
    /// `db` when app_settings holds coordinates, `env` when they come from
    /// PIZZERIA_LAT/LON, `none` when unset.
    pub source: String,
}

#[server(name = ShopLocationFn, prefix = "/api", endpoint = "shop_location")]
pub async fn shop_location() -> Result<ShopLocation, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let b = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get())
        .unwrap_or_default();
    let db_lat: f64 = ssr::setting(&db, "shop_lat")
        .await
        .trim()
        .parse()
        .unwrap_or(0.0);
    let db_lon: f64 = ssr::setting(&db, "shop_lon")
        .await
        .trim()
        .parse()
        .unwrap_or(0.0);
    let source = if db_lat != 0.0 || db_lon != 0.0 {
        "db"
    } else if b.shop_lat != 0.0 || b.shop_lon != 0.0 {
        "env"
    } else {
        "none"
    };
    Ok(ShopLocation {
        street: b.shop_address_street,
        plz_city: b.shop_address_plz_city,
        lat: b.shop_lat,
        lon: b.shop_lon,
        source: source.into(),
    })
}

/// Save the shop address + coordinates (app_settings shop_*), refreshing
/// the cached branding so tours, zone checks and JSON-LD use them at once.
#[server(name = SetShopLocation, prefix = "/api", endpoint = "set_shop_location")]
pub async fn set_shop_location(
    street: String,
    plz_city: String,
    lat: f64,
    lon: f64,
) -> Result<(), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    if !(lat == 0.0 && lon == 0.0)
        && !((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon))
    {
        return Err(ServerFnError::new(
            "Koordinaten außerhalb des gültigen Bereichs",
        ));
    }
    let city = plz_city
        .trim()
        .split_once(' ')
        .map(|(_, c)| c.trim().to_string())
        .unwrap_or_default();
    for (k, v, label) in [
        (
            "shop_address_street",
            street.trim().to_string(),
            "Straße + Hausnummer",
        ),
        (
            "shop_address_plz_city",
            plz_city.trim().to_string(),
            "PLZ + Ort",
        ),
        ("shop_lat", format!("{lat}"), "Breitengrad"),
        ("shop_lon", format!("{lon}"), "Längengrad"),
    ] {
        ssr::upsert_setting(&db, k, &v, label)
            .await
            .map_err(|e| ServerFnError::new(format!("save {k}: {e}")))?;
    }
    if !city.is_empty() {
        ssr::upsert_setting(&db, "shop_city", &city, "Ort")
            .await
            .map_err(|e| ServerFnError::new(format!("save shop_city: {e}")))?;
    }
    if let Some(h) = use_context::<crate::branding::BrandingHandle>() {
        h.set(crate::branding::ssr::load_branding(&db).await);
    }
    Ok(())
}

/// Resolve the shop's own address through the local index and save the
/// coordinates. Needs the shop's municipality in the index.
#[server(name = GeocodeShopAddress, prefix = "/api", endpoint = "geocode_shop_address")]
pub async fn geocode_shop_address() -> Result<ShopLocation, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let loc = shop_location().await?;
    let idx = ssr::index_from_context()
        .ok_or_else(|| ServerFnError::new("Kein Adressindex geladen — erst importieren."))?;
    let street_text = loc.street.trim();
    // "Wolfsberger Str. 18" → ("Wolfsberger Str.", "18"); the number is the
    // last whitespace-separated token starting with a digit.
    let (street_name, number) = match street_text.rsplit_once(' ') {
        Some((s, n)) if n.chars().next().is_some_and(|c| c.is_ascii_digit()) => {
            (s.trim(), n.trim())
        }
        _ => {
            return Err(ServerFnError::new(
                "Straße + Hausnummer des Shops eintragen (z. B. „Wolfsberger Str. 18“).",
            ))
        }
    };
    let plz = loc
        .plz_city
        .trim()
        .split(' ')
        .next()
        .unwrap_or("")
        .to_string();
    let st = idx
        .find_street_by_text(street_name, Some(&plz))
        .ok_or_else(|| {
            ServerFnError::new(format!(
                "Straße {street_name:?} nicht im Adressindex (Gemeinde importiert?)."
            ))
        })?;
    let r = idx.resolve(st.id, number).ok_or_else(|| {
        ServerFnError::new(format!(
            "Hausnummer {number} in {} nicht im Kataster.",
            st.name
        ))
    })?;
    set_shop_location(loc.street.clone(), loc.plz_city.clone(), r.lat, r.lon).await?;
    Ok(ShopLocation {
        street: loc.street,
        plz_city: loc.plz_city,
        lat: r.lat,
        lon: r.lon,
        source: "db".into(),
    })
}

/// Everything from the shop's address + a delivery radius:
/// 1. the town name → municipality (split index)
/// 2. that municipality's file → the shop's coordinates (saved)
/// 3. every municipality with at least one address within the radius →
///    Gemeinden-Liste (postcodes kept where known; the home town gets the
///    address' postcode; others are left for the operator)
/// 4. if no zone has geometry yet: the default zone (else the dearest
///    active one) becomes a radius around the shop
/// 5. import + index reload
#[server(name = SetupFromAddress, prefix = "/api", endpoint = "setup_from_address")]
pub async fn setup_from_address(
    street: String,
    plz_city: String,
    radius_m: f64,
    recenter_zones: bool,
) -> Result<String, ServerFnError> {
    use rusterando_geo::import::{read_split_index, AddressSource, CadastreNrw};
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    if !(500.0..=50_000.0).contains(&radius_m) {
        return Err(ServerFnError::new(
            "Lieferradius zwischen 500 und 50000 m angeben",
        ));
    }
    // The typed address is the input of this whole procedure, so it is
    // persisted here (coordinates are recomputed below) — a separate
    // "save" round-trip would race with this call.
    set_shop_location(street.clone(), plz_city.clone(), 0.0, 0.0).await?;
    let loc = ShopLocation {
        street: street.trim().to_string(),
        plz_city: plz_city.trim().to_string(),
        lat: 0.0,
        lon: 0.0,
        source: "db".into(),
    };
    let (street_name, number) = match loc.street.trim().rsplit_once(' ') {
        Some((s, n)) if n.chars().next().is_some_and(|c| c.is_ascii_digit()) => {
            (s.trim().to_string(), n.trim().to_string())
        }
        _ => {
            return Err(ServerFnError::new(
                "Straße + Hausnummer des Shops eintragen (z. B. „Marktstraße 1“).",
            ))
        }
    };
    let (plz, city) = match loc.plz_city.trim().split_once(' ') {
        Some((p, c)) if p.len() == 5 && !c.trim().is_empty() => {
            (p.to_string(), c.trim().to_string())
        }
        _ => {
            return Err(ServerFnError::new(
                "PLZ + Ort des Shops eintragen (z. B. „48249 Dülmen“).",
            ))
        }
    };
    let dir = ssr::split_dir();
    // No split, or one without bounding boxes: fetch + split in the
    // background and tell the operator to come back — this call cannot
    // wait a minute or two behind a proxy.
    if !rusterando_geo::import::split_index_current(&dir) {
        let job = ssr::NrwJob::snapshot();
        let msg = if job.running {
            "Die NRW-Daten werden gerade aufbereitet — Fortschritt im Abschnitt „NRW-Daten“; danach erneut klicken."
        } else if ssr::NrwJob::start(ssr::data_dir(), false) {
            "NRW-Daten fehlen oder sind veraltet — Download + Aufbereitung wurden gestartet (Abschnitt „NRW-Daten“, ein bis zwei Minuten). Danach erneut klicken."
        } else {
            "NRW-Daten fehlen — Aufbereitung konnte nicht gestartet werden."
        };
        return Err(ServerFnError::new(msg));
    }
    let index = read_split_index(&dir)
        .map_err(|e| ServerFnError::new(format!("Gemeindeliste lesen: {e}")))?;
    let city_n = crate::pages::locality::normalize(&city);
    let homes: Vec<_> = index
        .iter()
        .filter(|m| crate::pages::locality::normalize(&m.name) == city_n)
        .collect();
    let home = match homes.as_slice() {
        [h] => (*h).clone(),
        [] => {
            return Err(ServerFnError::new(format!(
                "Gemeinde {city:?} nicht in der NRW-Liste gefunden."
            )))
        }
        many => {
            return Err(ServerFnError::new(format!(
                "Mehrere Gemeinden heißen {city:?}: {}",
                many.iter()
                    .map(|m| m.ags.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
    };
    // Known postcodes are reused: the env defaults first, then whatever the
    // dashboard holds (a municipality that left the list and comes back
    // keeps its postcode that way), then the typed one for the home town.
    let mut postcodes: std::collections::HashMap<String, (String, String)> =
        use_context::<ssr::GeoIndexHandle>()
            .and_then(|h| (*h.env_config).clone())
            .map(|c| c.postcode_map)
            .unwrap_or_default();
    let (prev_cfg, _) = ssr::effective_config(&db).await;
    if let Some(c) = prev_cfg {
        postcodes.extend(
            c.postcode_map
                .into_iter()
                .filter(|(_, (p, _))| !p.is_empty()),
        );
    }
    postcodes.insert(home.ags.clone(), (plz.clone(), home.name.clone()));

    // Blocking part: geocode the shop in its own municipality, then scan the
    // bbox candidates for any address inside the radius.
    let home2 = home.clone();
    let candidates: Vec<rusterando_geo::import::MunicipalityInfo> = index.clone();
    let postcodes2 = postcodes.clone();
    let (lat, lon, kept): (f64, f64, Vec<(String, String)>) =
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let cfg_for = |ags: &str, name: &str| {
                let mut vars = std::collections::HashMap::new();
                vars.insert("GEO_BACKEND".to_string(), "cadastre_nrw".to_string());
                vars.insert("GEO_AGS".to_string(), ags.to_string());
                let (p, c) = postcodes2
                    .get(ags)
                    .cloned()
                    .unwrap_or_else(|| (String::new(), name.to_string()));
                vars.insert("GEO_POSTCODE_MAP".to_string(), format!("{ags}:{p}:{c}"));
                rusterando_geo::GeoConfig::from_vars(&vars).expect("config")
            };
            let home_file = dir.join(format!("{}.txt", home2.ags));
            let r = CadastreNrw
                .build_index(&cfg_for(&home2.ags, &home2.name), &home_file)
                .map_err(|e| format!("Gemeindedatei {}: {e}", home_file.display()))?;
            let idx = r.into_index();
            let st = idx
                .find_street_by_text(&street_name, Some(&plz))
                .ok_or_else(|| {
                    format!("Straße {street_name:?} nicht in {} gefunden.", home2.name)
                })?;
            let hit = idx.resolve(st.id, &number).ok_or_else(|| {
                format!(
                    "Hausnummer {number} in {} ({}) nicht im Kataster.",
                    st.name, home2.name
                )
            })?;
            let (lat, lon) = (hit.lat, hit.lon);
            let mut kept: Vec<(String, String)> = vec![(home2.ags.clone(), home2.name.clone())];
            for m in candidates
                .iter()
                .filter(|m| m.ags != home2.ags && m.may_reach(lat, lon, radius_m))
            {
                let file = dir.join(format!("{}.txt", m.ags));
                if !file.is_file() {
                    continue;
                }
                let r = match CadastreNrw.build_index(&cfg_for(&m.ags, &m.name), &file) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let inside = r.streets.iter().flat_map(|s| s.numbers.iter()).any(|h| {
                    rusterando_geo::zones::haversine_m((lat, lon), (h.lat, h.lon)) <= radius_m
                });
                if inside {
                    kept.push((m.ags.clone(), m.name.clone()));
                }
            }
            Ok((lat, lon, kept))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("setup task: {e}")))?
        .map_err(ServerFnError::new)?;

    // Save location, municipalities, postcode map.
    set_shop_location(loc.street.clone(), loc.plz_city.clone(), lat, lon).await?;
    let ags_csv = kept
        .iter()
        .map(|(a, _)| a.clone())
        .collect::<Vec<_>>()
        .join(",");
    let pmap_csv = kept
        .iter()
        .filter_map(|(a, name)| {
            let (p, c) = postcodes
                .get(a)
                .cloned()
                .unwrap_or_else(|| (String::new(), name.clone()));
            (!p.is_empty()).then(|| format!("{a}:{p}:{c}"))
        })
        .collect::<Vec<_>>()
        .join(",");
    ssr::upsert_setting(&db, "geo_ags", &ags_csv, "Gemeinden (AGS)")
        .await
        .map_err(|e| ServerFnError::new(format!("save: {e}")))?;
    ssr::upsert_setting(&db, "geo_postcode_map", &pmap_csv, "Gemeinden → PLZ")
        .await
        .map_err(|e| ServerFnError::new(format!("save: {e}")))?;
    // Import sanity box around the new location: radius plus a 5 km margin
    // (the kept addresses' centroid must fall inside it).
    let margin_m = radius_m + 5_000.0;
    let dlat = margin_m / 111_000.0;
    let dlon = margin_m / (111_000.0 * lat.to_radians().cos().max(0.1));
    let bbox = format!(
        "{:.4},{:.4},{:.4},{:.4}",
        lat - dlat,
        lat + dlat,
        lon - dlon,
        lon + dlon
    );
    ssr::upsert_setting(&db, "geo_expect_bbox", &bbox, "Import-Prüfbox")
        .await
        .map_err(|e| ServerFnError::new(format!("save: {e}")))?;

    // Zone geometry: only when nothing has geometry yet.
    let (zones, _) = rusterando_geo::store::load_zones(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("load zones: {e}")))?;
    let has_geometry = zones
        .iter()
        .any(|z| z.active && z.kind != rusterando_geo::GeometryKind::None);
    let mut zone_note = String::new();
    // Radius zones still centred on a previous location would silently
    // filter the new municipalities away — move them on request (keeping
    // their radius), otherwise say so instead of guessing.
    for z in zones.iter().filter(|z| z.active) {
        if let Some(rusterando_geo::zones::Geometry::Radius {
            lat: zl,
            lon: zo,
            metres,
        }) = &z.geometry
        {
            let dist = rusterando_geo::zones::haversine_m((lat, lon), (*zl, *zo));
            if dist <= 1_000.0 {
                continue;
            }
            if recenter_zones {
                sqlx::query("UPDATE delivery_zones SET geometry = ?2 WHERE id = ?1")
                    .bind(&z.id)
                    .bind(format!("{lat:.6},{lon:.6},{metres:.0}"))
                    .execute(&db)
                    .await
                    .map_err(|e| ServerFnError::new(format!("zone: {e}")))?;
                zone_note.push_str(&format!(
                    "Zone „{}“: Umkreis {metres:.0} m auf den Shop verschoben (vorher {:.1} km entfernt)\n",
                    z.name,
                    dist / 1_000.0
                ));
            } else {
                zone_note.push_str(&format!(
                    "Achtung: Zone „{}“ ist ein Umkreis ({metres:.0} m) um einen Punkt {:.1} km vom Shop entfernt — Geometrie prüfen oder „Umkreis-Zonen verschieben“ ankreuzen\n",
                    z.name,
                    dist / 1_000.0
                ));
            }
        }
    }
    if !has_geometry {
        let target = zones
            .iter()
            .filter(|z| z.active)
            .max_by_key(|z| (z.kind == rusterando_geo::GeometryKind::Default, z.fee_cents));
        if let Some(z) = target {
            sqlx::query("UPDATE delivery_zones SET geometry_kind = 'radius', geometry = ?2, priority = 100 WHERE id = ?1")
                .bind(&z.id)
                .bind(format!("{lat:.6},{lon:.6},{radius_m:.0}"))
                .execute(&db)
                .await
                .map_err(|e| ServerFnError::new(format!("zone: {e}")))?;
            zone_note = format!("Zone „{}“ → Umkreis {radius_m:.0} m um den Shop\n", z.name);
        } else {
            zone_note =
                "Keine aktive Zone — bitte eine Zone anlegen, sonst ist nichts lieferbar\n".into();
        }
    }
    let missing: Vec<&str> = kept
        .iter()
        .filter(|(a, _)| postcodes.get(a).map(|p| p.0.is_empty()).unwrap_or(true))
        .map(|(_, n)| n.as_str())
        .collect();
    let mut report =
        format!(
        "Shop: {} — {lat:.6}, {lon:.6} ({}, AGS {})\nGemeinden im Radius {radius_m:.0} m: {}\n{}",
        loc.street,
        home.name,
        home.ags,
        kept.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(", "),
        zone_note
    );
    if !missing.is_empty() {
        report.push_str(&format!(
            "PLZ fehlt noch für: {} (unten eintragen, speichern, erneut importieren)\n",
            missing.join(", ")
        ));
    }
    match ssr::run_import(&db).await {
        Ok(summary) => report.push_str(&summary),
        Err(e) => report.push_str(&format!("Import: {e}")),
    }
    Ok(report)
}
