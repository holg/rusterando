//! Site-wide tunables (admin-editable, no code deploy needed).
//!
//! v1: free-delivery threshold. The table is intentionally generic so
//! we can grow it (holiday-mode banner, max-orders-per-hour, …) without
//! schema changes. Each consumer reads its own key with the SSR helper
//! `get_int` (or its sister for strings).

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use std::sync::{Arc, RwLock};

/// Server-side cached theme value. Wrapped in a newtype so context lookup
/// is unambiguous (`use_context::<ThemeHandle>()`). The value is one of
/// `"warm"` / `"dark"`. Seeded at server boot from `app_settings.theme`,
/// rewritten in-place by `update_setting` so subsequent SSR responses
/// pick up changes without a DB round-trip.
#[cfg(feature = "ssr")]
#[derive(Clone, Default)]
pub struct ThemeHandle(pub Arc<RwLock<String>>);

#[cfg(feature = "ssr")]
impl ThemeHandle {
    pub fn new(initial: String) -> Self {
        Self(Arc::new(RwLock::new(initial)))
    }
    pub fn get(&self) -> String {
        self.0
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|_| "warm".to_string())
    }
    pub fn set(&self, v: String) {
        if let Ok(mut g) = self.0.write() {
            *g = v;
        }
    }
}

/// Allowed theme values. Kept as an associated const so the validator
/// and the admin <select> stay in sync.
pub const ALLOWED_THEMES: &[&str] = &["warm", "dark"];

/// Public-facing snapshot — what the menu/checkout/home pages need to
/// render. Cached in checkout context so the form picks it up via the
/// existing `load_checkout_context` server fn.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicSettings {
    /// Free-delivery threshold in cents. `0` = never free.
    pub free_delivery_threshold_cents: i64,
}

/// Admin-facing row shape for the /admin/settings table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingRow {
    pub key: String,
    pub value: String,
    pub label_de: String,
    pub hint_de: Option<String>,
}

/// Read all settings for the admin page.
#[server(
    name = ListSettings,
    prefix = "/api",
    endpoint = "list_settings"
)]
pub async fn list_settings() -> Result<Vec<SettingRow>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows: Vec<(String, String, String, Option<String>)> =
        sqlx::query_as("SELECT key, value, label_de, hint_de FROM app_settings ORDER BY key")
            .fetch_all(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("load settings: {e}")))?;

    Ok(rows
        .into_iter()
        .map(|(key, value, label_de, hint_de)| SettingRow {
            key,
            value,
            label_de,
            hint_de,
        })
        .collect())
}

/// Update a single setting's value. Admin only.
#[server(
    name = UpdateSetting,
    prefix = "/api",
    endpoint = "update_setting"
)]
pub async fn update_setting(key: String, value: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    if key.trim().is_empty() {
        return Err(ServerFnError::new("Schlüssel fehlt."));
    }
    // Per-key validation.
    if key == "free_delivery_threshold_cents" {
        let n: i64 = value
            .trim()
            .parse()
            .map_err(|_| ServerFnError::new("Bitte eine ganze Zahl in Cent angeben."))?;
        if n < 0 {
            return Err(ServerFnError::new("Wert darf nicht negativ sein."));
        }
    }
    if key == "theme" && !ALLOWED_THEMES.contains(&value.trim()) {
        return Err(ServerFnError::new("Thema muss 'warm' oder 'dark' sein."));
    }
    if key == "stripe_mode" {
        // Only the two literal values are accepted. Refuse 'live'
        // when the matching env vars aren't set so the admin can't
        // flip into a broken state.
        let trimmed = value.trim();
        if !matches!(trimmed, "sandbox" | "live") {
            return Err(ServerFnError::new(
                "Stripe-Modus muss 'sandbox' oder 'live' sein.",
            ));
        }
        let mode = crate::stripe::StripeMode::from_setting(trimmed);
        let keys = crate::stripe::StripeKeys::for_mode(mode);
        if !keys.is_complete() {
            return Err(ServerFnError::new(format!(
                "Schlüssel für '{}' unvollständig — bitte {prefix}STRIPE_PUBLISH_KEY, \
                 {prefix}STRIPE_SECRET_KEY und {prefix}STRIPE_WEBHOOK_SECRET in .env setzen.",
                mode.label_de(),
                prefix = mode.env_prefix(),
            )));
        }
    }
    // Branding numeric fields — friendly error if non-parseable.
    if matches!(key.as_str(), "shop_lat" | "shop_lon" | "shop_tax_rate") && !value.trim().is_empty()
    {
        value
            .trim()
            .parse::<f64>()
            .map_err(|_| ServerFnError::new("Bitte eine Zahl angeben."))?;
    }
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let res = sqlx::query(
        "UPDATE app_settings
         SET value = ?2, updated_at = CURRENT_TIMESTAMP
         WHERE key = ?1",
    )
    .bind(&key)
    .bind(value.trim())
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("update setting: {e}")))?;

    if res.rows_affected() == 0 {
        return Err(ServerFnError::new("Einstellung nicht gefunden."));
    }

    // Write-through cache for the theme handle so the next request's SSR
    // sees the new value without a DB read.
    if key == "theme" {
        if let Some(h) = use_context::<ThemeHandle>() {
            h.set(value.trim().to_string());
        }
    }
    // Same for branding fields — every shop_* key updates the cached
    // BrandingHandle so subsequent renders pick up the new value.
    if key.starts_with("shop_") {
        if let Some(h) = use_context::<crate::branding::BrandingHandle>() {
            h.apply_kv(&key, value.trim());
        }
    }
    // Stripe mode flip is write-through too: the next checkout (and
    // the next webhook validation via the Active legacy route) sees
    // the new mode without a DB read.
    if key == "stripe_mode" {
        if let Some(h) = use_context::<crate::stripe::StripeModeHandle>() {
            h.set(crate::stripe::StripeMode::from_setting(value.trim()));
        }
    }
    Ok(())
}

#[cfg(feature = "ssr")]
pub mod ssr {
    use sqlx::SqlitePool;

    /// Read a setting as i64. Returns `None` when the key doesn't
    /// exist or the value can't be parsed — callers decide on a
    /// sensible default.
    pub async fn get_int(db: &SqlitePool, key: &str) -> Option<i64> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = ?1")
                .bind(key)
                .fetch_optional(db)
                .await
                .ok()
                .flatten();
        row.and_then(|(s,)| s.trim().parse().ok())
    }

    /// Free-delivery threshold (cents). 0 = feature disabled.
    pub async fn free_delivery_threshold_cents(db: &SqlitePool) -> i64 {
        get_int(db, "free_delivery_threshold_cents")
            .await
            .unwrap_or(0)
    }

    /// Boot-time theme read. Defaults to `"warm"` when the row is missing
    /// (fresh DBs without the theme migration applied) or when the value
    /// is anything other than the two allowed names.
    pub async fn theme(db: &SqlitePool) -> String {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = 'theme'")
                .fetch_optional(db)
                .await
                .ok()
                .flatten();
        match row.map(|(v,)| v) {
            Some(ref v) if super::ALLOWED_THEMES.contains(&v.as_str()) => v.clone(),
            _ => "warm".to_string(),
        }
    }
}
