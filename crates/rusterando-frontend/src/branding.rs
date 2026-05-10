//! Shop-contact data — name, address, phone, email, owner, tax + bank.
//!
//! Stored in `app_settings` as keys with the `shop_` prefix and read at
//! server boot into a [`BrandingHandle`] cached in Leptos context. Read
//! sites (PDF generation, email rendering, Impressum, home meta, admin
//! shell brand link, …) pull from the handle so changes via
//! `/admin/branding` propagate without a redeploy and without per-request
//! DB hits.
//!
//! Mirrors the [`crate::pages::settings::ThemeHandle`] pattern.

use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use std::sync::{Arc, RwLock};

/// Snapshot of all `shop_*` `app_settings` rows. The `Default` impl
/// gives a fresh fork sensible placeholders so a freshly-cloned
/// Rusterando boots with a working public site (just blank contact
/// details) rather than panicking.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Branding {
    pub shop_name: String,
    pub shop_phone: String,
    pub shop_email: String,
    pub shop_owner: String,
    pub shop_address_street: String,
    pub shop_address_plz_city: String,
    pub shop_city: String,
    pub shop_lat: f64,
    pub shop_lon: f64,
    pub shop_vat_id: String,
    pub shop_tax_rate: f64,
    pub shop_bank_name: String,
    pub shop_bank_iban: String,
    pub shop_bank_bic: String,
}

impl Branding {
    /// Header / label fallback. Returns `"Mein Restaurant"` if the
    /// admin hasn't filled in `shop_name` yet.
    pub fn display_name(&self) -> String {
        if self.shop_name.trim().is_empty() {
            "Mein Restaurant".to_string()
        } else {
            self.shop_name.clone()
        }
    }

    /// Comma-joined address suitable for inline rendering. Returns
    /// only the parts that are present; empty string when neither
    /// street nor PLZ+city is set.
    pub fn full_address(&self) -> String {
        let street = self.shop_address_street.trim();
        let plz = self.shop_address_plz_city.trim();
        match (street.is_empty(), plz.is_empty()) {
            (true, true) => String::new(),
            (false, true) => street.to_string(),
            (true, false) => plz.to_string(),
            (false, false) => format!("{street}, {plz}"),
        }
    }

    /// `mailto:`-friendly contact email. Falls back to `bestellung@<host>`
    /// when not set so user-agents and contact-link patterns still work.
    pub fn email_or_fallback(&self) -> String {
        if self.shop_email.trim().is_empty() {
            "kontakt@example.com".to_string()
        } else {
            self.shop_email.clone()
        }
    }
}

/// Server-side cached `Branding`. Wrapped in a newtype so `use_context`
/// disambiguates from any other `Arc<RwLock<...>>` someone might add.
#[cfg(feature = "ssr")]
#[derive(Clone, Default)]
pub struct BrandingHandle(pub Arc<RwLock<Branding>>);

#[cfg(feature = "ssr")]
impl BrandingHandle {
    pub fn new(initial: Branding) -> Self {
        Self(Arc::new(RwLock::new(initial)))
    }
    pub fn get(&self) -> Branding {
        self.0.read().map(|g| g.clone()).unwrap_or_default()
    }
    pub fn set(&self, b: Branding) {
        if let Ok(mut g) = self.0.write() {
            *g = b;
        }
    }
    /// Apply a single key=value update from `update_setting`.
    /// Skips unknown keys so non-`shop_*` rows pass through.
    pub fn apply_kv(&self, key: &str, value: &str) {
        if let Ok(mut g) = self.0.write() {
            apply_kv_inner(&mut g, key, value);
        }
    }
}

#[cfg(feature = "ssr")]
fn apply_kv_inner(b: &mut Branding, key: &str, value: &str) {
    match key {
        "shop_name" => b.shop_name = value.to_string(),
        "shop_phone" => b.shop_phone = value.to_string(),
        "shop_email" => b.shop_email = value.to_string(),
        "shop_owner" => b.shop_owner = value.to_string(),
        "shop_address_street" => b.shop_address_street = value.to_string(),
        "shop_address_plz_city" => b.shop_address_plz_city = value.to_string(),
        "shop_city" => b.shop_city = value.to_string(),
        "shop_lat" => b.shop_lat = value.parse().unwrap_or(0.0),
        "shop_lon" => b.shop_lon = value.parse().unwrap_or(0.0),
        "shop_vat_id" => b.shop_vat_id = value.to_string(),
        "shop_tax_rate" => b.shop_tax_rate = value.parse().unwrap_or(0.0),
        "shop_bank_name" => b.shop_bank_name = value.to_string(),
        "shop_bank_iban" => b.shop_bank_iban = value.to_string(),
        "shop_bank_bic" => b.shop_bank_bic = value.to_string(),
        _ => {}
    }
}

/// All `shop_*` keys, useful for the admin form layout + the boot loader.
pub const BRANDING_KEYS: &[&str] = &[
    "shop_name",
    "shop_phone",
    "shop_email",
    "shop_owner",
    "shop_address_street",
    "shop_address_plz_city",
    "shop_city",
    "shop_lat",
    "shop_lon",
    "shop_vat_id",
    "shop_tax_rate",
    "shop_bank_name",
    "shop_bank_iban",
    "shop_bank_bic",
];

#[cfg(feature = "ssr")]
pub mod ssr {
    use super::*;
    use sqlx::SqlitePool;

    /// Read every `shop_*` row in one query and assemble a `Branding`.
    /// `lat`/`lon` fall back to `PIZZERIA_LAT`/`PIZZERIA_LON` env vars
    /// when the DB has the placeholder `0` — matches the existing prod
    /// deployment where the geo lives in `.env`.
    pub async fn load_branding(db: &SqlitePool) -> Branding {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM app_settings WHERE key LIKE 'shop_%'")
                .fetch_all(db)
                .await
                .unwrap_or_default();
        let mut b = Branding::default();
        for (k, v) in rows {
            apply_kv_inner(&mut b, &k, &v);
        }
        // Env-var fallback for geo so Davids' existing deployment keeps
        // its lat/lon without an extra admin step.
        if b.shop_lat == 0.0 {
            b.shop_lat = std::env::var("PIZZERIA_LAT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
        }
        if b.shop_lon == 0.0 {
            b.shop_lon = std::env::var("PIZZERIA_LON")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
        }
        b
    }
}
