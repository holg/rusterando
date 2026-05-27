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

/// Runtime toggle for the multilingual UI. `'1'` = show the locale
/// switcher in the header AND serve `/<lang>/...` prefixed routes;
/// `'0'` = single-locale shop, those routes 404. Same write-through
/// pattern as ThemeHandle: seeded at boot from `app_settings`,
/// rewritten in place when the admin toggles via `update_setting`.
#[cfg(feature = "ssr")]
#[derive(Clone, Default)]
pub struct I18nHandle(pub Arc<RwLock<bool>>);

#[cfg(feature = "ssr")]
impl I18nHandle {
    pub fn new(initial: bool) -> Self {
        Self(Arc::new(RwLock::new(initial)))
    }
    pub fn get(&self) -> bool {
        self.0.read().map(|g| *g).unwrap_or(false)
    }
    pub fn set(&self, v: bool) {
        if let Ok(mut g) = self.0.write() {
            *g = v;
        }
    }
}

/// Runtime "pause online orders" switch. `'1'` = reject all online
/// orders (independent of opening hours) and show a closed banner;
/// `'0'` = normal operation gated only by opening hours. Same
/// write-through pattern as I18nHandle: seeded at boot from
/// `app_settings.orders_paused`, rewritten when the admin toggles via
/// `update_setting`. The optional customer-facing message
/// (`orders_paused_message`) is read from the DB on demand rather than
/// cached — it changes rarely and is only read on the low-traffic
/// closed path.
#[cfg(feature = "ssr")]
#[derive(Clone, Default)]
pub struct OrdersPausedHandle(pub Arc<RwLock<bool>>);

#[cfg(feature = "ssr")]
impl OrdersPausedHandle {
    pub fn new(initial: bool) -> Self {
        Self(Arc::new(RwLock::new(initial)))
    }
    pub fn get(&self) -> bool {
        self.0.read().map(|g| *g).unwrap_or(false)
    }
    pub fn set(&self, v: bool) {
        if let Ok(mut g) = self.0.write() {
            *g = v;
        }
    }
}

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
    if key == "i18n_enabled" && !matches!(value.trim(), "0" | "1" | "true" | "false") {
        return Err(ServerFnError::new(
            "i18n_enabled muss '0' / '1' (oder 'true' / 'false') sein.",
        ));
    }
    if key == "menu_category_overlay" && !matches!(value.trim(), "0" | "1" | "true" | "false") {
        return Err(ServerFnError::new(
            "menu_category_overlay muss '0' / '1' (oder 'true' / 'false') sein.",
        ));
    }
    if key == "orders_paused" && !matches!(value.trim(), "0" | "1" | "true" | "false") {
        return Err(ServerFnError::new(
            "orders_paused muss '0' / '1' (oder 'true' / 'false') sein.",
        ));
    }
    if key == "orders_paused_message" && value.trim().chars().count() > 200 {
        return Err(ServerFnError::new(
            "Hinweistext darf höchstens 200 Zeichen lang sein.",
        ));
    }
    // orders_paused_until: empty (clear) or a UTC 'YYYY-MM-DD HH:MM:SS'.
    if key == "orders_paused_until" && !value.trim().is_empty() {
        if chrono::NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%d %H:%M:%S").is_err() {
            return Err(ServerFnError::new(
                "orders_paused_until muss leer oder ein Zeitstempel 'YYYY-MM-DD HH:MM:SS' (UTC) sein.",
            ));
        }
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
    // i18n toggle — flip the runtime handle so the next request
    // immediately sees the new state (locale switcher visibility,
    // /<lang>/* guard middleware).
    if key == "i18n_enabled" {
        if let Some(h) = use_context::<I18nHandle>() {
            h.set(matches!(value.trim(), "1" | "true"));
        }
    }
    // Pause toggle — flip the runtime handle so the next order attempt
    // and the next page render immediately see the new state.
    if key == "orders_paused" {
        let on = matches!(value.trim(), "1" | "true");
        if let Some(h) = use_context::<OrdersPausedHandle>() {
            h.set(on);
        }
        // Turning the indefinite switch OFF also cancels any active
        // timed snooze, so "Wieder öffnen" fully reopens in one click.
        if !on {
            let _ = sqlx::query(
                "UPDATE app_settings SET value = '', updated_at = CURRENT_TIMESTAMP
                 WHERE key = 'orders_paused_until'",
            )
            .execute(&db)
            .await;
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

    /// Boot-time read of the i18n toggle. Defaults to `false` (single-
    /// locale shop) on missing rows or invalid values — matches the
    /// migration default and keeps Davids' deployment behaviour
    /// unchanged after upgrading.
    pub async fn i18n_enabled(db: &SqlitePool) -> bool {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = 'i18n_enabled'")
                .fetch_optional(db)
                .await
                .ok()
                .flatten();
        matches!(row.as_ref().map(|(v,)| v.as_str()), Some("1" | "true"))
    }

    /// Whether online orders are manually paused. Defaults to `false`
    /// (normal operation) on missing rows or invalid values — matches
    /// the migration default.
    pub async fn orders_paused(db: &SqlitePool) -> bool {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = 'orders_paused'")
                .fetch_optional(db)
                .await
                .ok()
                .flatten();
        matches!(row.as_ref().map(|(v,)| v.as_str()), Some("1" | "true"))
    }

    /// Optional customer-facing message shown when orders are paused.
    /// Empty string when unset — callers fall back to a generic text.
    pub async fn orders_paused_message(db: &SqlitePool) -> String {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = 'orders_paused_message'")
                .fetch_optional(db)
                .await
                .ok()
                .flatten();
        row.map(|(v,)| v.trim().to_string()).unwrap_or_default()
    }

    /// Active timed-snooze deadline, parsed from `orders_paused_until`
    /// (stored as a UTC `'YYYY-MM-DD HH:MM:SS'` string). Returns `None`
    /// when unset, unparseable, or already in the past — so an expired
    /// snooze simply stops blocking with no cleanup needed.
    pub async fn orders_paused_until(db: &SqlitePool) -> Option<chrono::DateTime<chrono::Utc>> {
        use chrono::{NaiveDateTime, TimeZone, Utc};
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = 'orders_paused_until'")
                .fetch_optional(db)
                .await
                .ok()
                .flatten();
        let s = row.map(|(v,)| v.trim().to_string()).unwrap_or_default();
        if s.is_empty() {
            return None;
        }
        let dt = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok()?;
        let until = Utc.from_utc_datetime(&dt);
        (until > Utc::now()).then_some(until)
    }

    /// The single source of truth for "are online orders paused right
    /// now?" — used by `place_order` and every customer-facing surface
    /// so they never disagree. Returns `(paused, resume_label)`:
    ///   * `paused`        — indefinite switch on, OR an unexpired snooze.
    ///   * `resume_label`  — local "HH:MM" the snooze ends, when the
    ///                       pause is purely timed (None for indefinite,
    ///                       so the UI shows the generic/custom message).
    pub async fn order_pause_state(db: &SqlitePool) -> (bool, Option<String>) {
        let indefinite = orders_paused(db).await;
        let until = orders_paused_until(db).await;
        match (indefinite, until) {
            (true, _) => (true, None),
            (false, Some(until)) => {
                let local = until.with_timezone(&chrono::Local);
                (true, Some(local.format("%H:%M").to_string()))
            }
            (false, None) => (false, None),
        }
    }
}
