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

/// On-disk base directory for admin uploads, `<site_root>/img/uploads`
/// (e.g. `html/img/uploads` on prod, `target/site/img/uploads` in dev).
/// Provided into server-fn context so server fns that touch upload files
/// — e.g. `delete_pdf_cover` unlinking a cover — resolve the same path the
/// axum upload handlers write to. Immutable for the process lifetime, so a
/// plain `Arc<str>` (no lock) is enough.
#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct UploadsDir(pub std::sync::Arc<str>);

#[cfg(feature = "ssr")]
impl UploadsDir {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Deployment / tenancy diagnostics — what the admin sees at the bottom of
/// /admin/settings. Resolved ONCE at server boot (the static parts) by the
/// binary's mode-detect; the per-request `subdomain` is filled in by the
/// diagnostics server fn from the live `X-Tenant` / Host header. Plain
/// (de)serializable so the server fn returns it straight to the admin page.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentInfo {
    /// `true` when the binary booted in in-process multi-tenant mode
    /// (≥2 `.env*`, distinct DBs). `false` = single-tenant (the real-shop
    /// default — davidspizzeria.de).
    pub multi_tenant: bool,
    /// Which `.env*` file(s) this process loaded. Single-tenant: one entry
    /// (or the `ENV_FILE=`/`<default .env>`); multi-tenant: every loaded slug.
    pub env_files: Vec<String>,
    /// The active tenant slug — DB filename stem in single-tenant
    /// ("davidspizzeria"), or the registry's tenant list in multi-tenant.
    pub tenant_slug: String,
    /// systemd/binary service identity (LEPTOS_OUTPUT_NAME, falls back to the
    /// DB stem) so the admin can tell which unit they're looking at.
    pub service_name: String,
    /// `DATABASE_URL` of the active DB (single-tenant) — confirms isolation.
    pub database_url: String,
    /// Bind address the process listens on (LEPTOS_SITE_ADDR).
    pub site_addr: String,
    /// The `Host` header of THIS request (e.g. "davidspizzeria.de").
    pub host: String,
    /// The nginx-supplied `X-Tenant` subdomain label for THIS request, if
    /// present (multi-tenant routing). Empty in single-tenant deploys.
    pub subdomain: String,
    /// The tenant THIS request actually resolved to (multi-tenant): slug, the
    /// REAL env filename it was loaded from, and its DATABASE_URL. Empty on the
    /// apex / single-tenant (use `tenant_slug`/`database_url` then).
    #[serde(default)]
    pub resolved_slug: String,
    #[serde(default)]
    pub resolved_env_file: String,
    #[serde(default)]
    pub resolved_db: String,
}

/// The per-request resolved tenant identity (slug + real env file + DB),
/// provided into context by the server's tenant router so the diagnostics
/// server fn can report WHICH tenant this request hit — without the frontend
/// crate depending on the server crate's `Tenant` type. Empty/None on the
/// apex or single-tenant.
#[cfg(feature = "ssr")]
#[derive(Clone, Default)]
pub struct ResolvedTenant {
    pub slug: String,
    pub env_file: String,
    pub database_url: String,
}

/// Boot-time deployment snapshot, provided into server-fn context so the
/// diagnostics server fn can read it without re-globbing the filesystem on
/// every request. The per-request `host`/`subdomain` are layered on top from
/// the live request headers. Same context-handle pattern as ThemeHandle.
#[cfg(feature = "ssr")]
#[derive(Clone, Default)]
pub struct DeploymentHandle(pub std::sync::Arc<DeploymentInfo>);

#[cfg(feature = "ssr")]
impl DeploymentHandle {
    pub fn new(info: DeploymentInfo) -> Self {
        Self(std::sync::Arc::new(info))
    }
    pub fn get(&self) -> DeploymentInfo {
        (*self.0).clone()
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

    // The receipt/printer keys (receipt_*, printer_theme) are edited in the
    // dedicated Bon-Editor (/admin/printer), not here, so they're filtered
    // out — one home for every Bon setting.
    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT key, value, label_de, hint_de FROM app_settings
         WHERE key NOT LIKE 'receipt_%' AND key <> 'printer_theme'
         ORDER BY key",
    )
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

/// Read ALL settings (no filtering) — for the Bon-Editor, which owns the
/// receipt_* + printer_theme keys that `list_settings` hides from the
/// generic settings page.
#[server(name = ListSettingsAll, prefix = "/api", endpoint = "list_settings_all")]
pub async fn list_settings_all() -> Result<Vec<SettingRow>, ServerFnError> {
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

/// Deployment / tenancy diagnostics for the admin settings page. Returns the
/// boot-time mode snapshot (`DeploymentHandle` from context) with the live
/// `Host` + `X-Tenant` headers of THIS request layered on. Admin only.
#[server(name = DeploymentInfoFn, prefix = "/api", endpoint = "deployment_info")]
pub async fn deployment_info() -> Result<DeploymentInfo, ServerFnError> {
    use axum::http::HeaderMap;
    use leptos_axum::extract;

    crate::pages::admin::require_admin().await?;

    // Boot snapshot (mode, env files, service, db). Absent on a printerless /
    // misconfigured boot → default (single-tenant, empty), never an error.
    let mut info = use_context::<DeploymentHandle>()
        .map(|h| h.get())
        .unwrap_or_default();

    // Layer the live request headers. nginx sets `X-Tenant` to the subdomain
    // label in multi-tenant deploys; `Host` is always present.
    if let Ok(headers) = extract::<HeaderMap>().await {
        info.host = headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        info.subdomain = headers
            .get("x-tenant")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
    }

    // The tenant THIS request resolved to (its real env file + DB), provided by
    // the tenant router. This is what makes the panel show flizza's details on
    // flizza's admin, not the boot/global snapshot.
    if let Some(rt) = use_context::<ResolvedTenant>() {
        info.resolved_slug = rt.slug;
        info.resolved_env_file = rt.env_file;
        info.resolved_db = rt.database_url;
    }

    Ok(info)
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
    if matches!(
        key.as_str(),
        "free_delivery_threshold_cents" | "giveaway_min_order_cents"
    ) {
        let n: i64 = value
            .trim()
            .parse()
            .map_err(|_| ServerFnError::new("Bitte eine ganze Zahl in Cent angeben."))?;
        if n < 0 {
            return Err(ServerFnError::new("Wert darf nicht negativ sein."));
        }
    }
    // Giveaway master toggle: 0/1 (true/false also accepted).
    if key == "giveaway_enabled" && !matches!(value.trim(), "0" | "1" | "true" | "false") {
        return Err(ServerFnError::new(
            "giveaway_enabled muss '0' / '1' (oder 'true' / 'false') sein.",
        ));
    }
    // Which order types the giveaway applies to.
    if key == "giveaway_order_types" && !matches!(value.trim(), "both" | "pickup" | "delivery") {
        return Err(ServerFnError::new(
            "giveaway_order_types muss 'both', 'pickup' oder 'delivery' sein.",
        ));
    }
    // PDF front-cover mode.
    if key == "pdf_cover_mode" && !matches!(value.trim(), "text" | "image") {
        return Err(ServerFnError::new(
            "pdf_cover_mode muss 'text' oder 'image' sein.",
        ));
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
    if key == "printer_theme" && !matches!(value.trim(), "rusterando-default" | "rusterando-rando")
    {
        return Err(ServerFnError::new(
            "printer_theme muss 'rusterando-default' oder 'rusterando-rando' sein.",
        ));
    }
    if key == "receipt_qr_mode" && !matches!(value.trim(), "order-url" | "static-url") {
        return Err(ServerFnError::new(
            "receipt_qr_mode muss 'order-url' oder 'static-url' sein.",
        ));
    }
    if key == "receipt_qr_static_url" {
        let v = value.trim();
        if !v.is_empty() && !(v.starts_with("http://") || v.starts_with("https://")) {
            return Err(ServerFnError::new(
                "receipt_qr_static_url muss mit http:// oder https:// beginnen (oder leer sein).",
            ));
        }
    }
    if matches!(key.as_str(), "receipt_show_allergens" | "receipt_show_logo")
        && !matches!(value.trim(), "0" | "1" | "true" | "false")
    {
        return Err(ServerFnError::new(
            "Wert muss '0' / '1' (oder 'true' / 'false') sein.",
        ));
    }
    if key == "receipt_header_text" && value.trim().chars().count() > 60 {
        return Err(ServerFnError::new(
            "Kopfzeile darf höchstens 60 Zeichen lang sein.",
        ));
    }
    if key == "receipt_footer_text" && value.trim().chars().count() > 120 {
        return Err(ServerFnError::new(
            "Fußzeile darf höchstens 120 Zeichen lang sein.",
        ));
    }
    // Badge/banner overrides: a generous cap (the allergen block holds a few
    // wrapped lines; the rest are short). Empty is always fine (= default).
    if key.starts_with("receipt_label_") && value.chars().count() > 200 {
        return Err(ServerFnError::new(
            "Text darf höchstens 200 Zeichen lang sein.",
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
    // orders_paused_until / force_open_until: empty (clear) or a UTC
    // 'YYYY-MM-DD HH:MM:SS' timestamp.
    if matches!(key.as_str(), "orders_paused_until" | "force_open_until")
        && !value.trim().is_empty()
        && chrono::NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%d %H:%M:%S").is_err()
    {
        return Err(ServerFnError::new(
            "Zeitstempel muss leer oder im Format 'YYYY-MM-DD HH:MM:SS' (UTC) sein.",
        ));
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

    // The value actually persisted. Normally the trimmed input verbatim; the
    // giveaway arm below may REWRITE it (resolving a menu_number to the
    // canonical item id).
    let mut store_value = value.trim().to_string();

    // Giveaway article: must reference a real menu_item, else the claimed free
    // line JOINs to nothing and silently vanishes from the cart. The admin
    // knows the *menu number* (e.g. 99), not the internal id (e.g.
    // `mc-004-i009`) — and on a reseeded/imported menu the two don't line up
    // (Davids' `mi-400` ≈ number 400 is a coincidence; flizza's are unrelated).
    // So accept EITHER: an exact `id`, or a `menu_number`, and store the
    // resolved canonical id. Verified here (needs the pool) so the admin gets a
    // clear error instead of a promo that does nothing.
    if key == "giveaway_item_id" {
        let input = value.trim();
        if input.is_empty() {
            return Err(ServerFnError::new("Artikel-ID darf nicht leer sein."));
        }
        // 1) Exact id match.
        let by_id: Option<(String,)> = sqlx::query_as("SELECT id FROM menu_items WHERE id = ?1")
            .bind(input)
            .fetch_optional(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("prüfe Artikel-ID: {e}")))?;
        // 2) Else by menu_number (what the admin sees on the menu / receipt).
        let resolved = match by_id {
            Some((id,)) => Some(id),
            None => {
                let by_num: Vec<(String,)> =
                    sqlx::query_as("SELECT id FROM menu_items WHERE menu_number = ?1")
                        .bind(input)
                        .fetch_all(&db)
                        .await
                        .map_err(|e| ServerFnError::new(format!("prüfe Artikelnummer: {e}")))?;
                match by_num.len() {
                    0 => None,
                    1 => Some(by_num[0].0.clone()),
                    n => {
                        return Err(ServerFnError::new(format!(
                            "Artikelnummer '{input}' ist {n}× vergeben — bitte stattdessen \
                             die eindeutige Artikel-ID angeben."
                        )));
                    }
                }
            }
        };
        match resolved {
            Some(id) => store_value = id,
            None => {
                return Err(ServerFnError::new(format!(
                    "Kein Artikel mit der ID oder Nummer '{input}' gefunden. Bitte eine \
                     gültige Artikel-ID (z. B. mc-004-i009) oder die Artikelnummer aus der \
                     Speisekarte angeben."
                )));
            }
        }
    }

    let res = sqlx::query(
        "UPDATE app_settings
         SET value = ?2, updated_at = CURRENT_TIMESTAMP
         WHERE key = ?1",
    )
    .bind(&key)
    .bind(&store_value)
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
        // Branding feeds the Restaurant JSON-LD (name/phone/email/address/geo)
        // — rebuild the cache so structured data reflects the edit.
        crate::pages::seo::rebuild_jsonld_cache().await;
        // Shop name/address/phone also print on the menu.pdf header/footer.
        crate::pages::push::rebuild_menu_pdf_cache().await;
    }
    // Any pdf_* setting (cover mode, tagline, hours, extras lines) changes the
    // rendered menu — rebuild the static cache so the offline fallback matches.
    if key.starts_with("pdf_") {
        crate::pages::push::rebuild_menu_pdf_cache().await;
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

    /// Resolved giveaway (free-Pizzabrötchen) configuration, read from the
    /// three `giveaway_*` app_settings rows. The single source of truth for
    /// "does this order qualify" — used by both the cart (to gate the offer
    /// button + nudge) and place_order (to enforce server-side). Defaults are
    /// the migration defaults: disabled, 20 € threshold, both order types, so
    /// a DB missing the rows behaves as "off".
    #[derive(Debug, Clone)]
    pub struct GiveawayConfig {
        pub enabled: bool,
        pub min_order_cents: i64,
        /// 'both' | 'pickup' | 'delivery' — which order types qualify.
        pub order_types: GiveawayOrderTypes,
        /// The menu_item.id added as the free line. Admin-editable
        /// (`giveaway_item_id` setting); defaults to `mi-400` (Pizzabrötchen).
        pub item_id: String,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum GiveawayOrderTypes {
        Both,
        Pickup,
        Delivery,
    }

    impl GiveawayOrderTypes {
        fn parse(s: &str) -> Self {
            match s.trim() {
                "pickup" => Self::Pickup,
                "delivery" => Self::Delivery,
                _ => Self::Both,
            }
        }
        /// Does this setting allow the given order type? `is_delivery` is the
        /// order's type (true = delivery, false = pickup).
        pub fn allows(&self, is_delivery: bool) -> bool {
            match self {
                Self::Both => true,
                Self::Pickup => !is_delivery,
                Self::Delivery => is_delivery,
            }
        }
    }

    pub async fn giveaway_config(db: &SqlitePool) -> GiveawayConfig {
        let enabled = {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value FROM app_settings WHERE key = 'giveaway_enabled'")
                    .fetch_optional(db)
                    .await
                    .ok()
                    .flatten();
            matches!(row.as_ref().map(|(v,)| v.as_str()), Some("1" | "true"))
        };
        let min_order_cents = get_int(db, "giveaway_min_order_cents").await.unwrap_or(0);
        let order_types = {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value FROM app_settings WHERE key = 'giveaway_order_types'")
                    .fetch_optional(db)
                    .await
                    .ok()
                    .flatten();
            GiveawayOrderTypes::parse(row.as_ref().map(|(v,)| v.as_str()).unwrap_or("both"))
        };
        let item_id = {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value FROM app_settings WHERE key = 'giveaway_item_id'")
                    .fetch_optional(db)
                    .await
                    .ok()
                    .flatten();
            row.map(|(v,)| v.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| crate::pages::cart::GIVEAWAY_ITEM_ID.to_string())
        };
        GiveawayConfig {
            enabled,
            min_order_cents,
            order_types,
            item_id,
        }
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

    /// Parse a setting holding a UTC `'YYYY-MM-DD HH:MM:SS'` deadline,
    /// returning it only if it's still in the future. Unset / unparseable
    /// / past all yield `None` — so expired deadlines stop applying with
    /// no cleanup needed. Shared by the snooze + force-open settings.
    async fn future_deadline(db: &SqlitePool, key: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        use chrono::{NaiveDateTime, TimeZone, Utc};
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = ?1")
                .bind(key)
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

    /// Active timed-snooze deadline (`orders_paused_until`).
    pub async fn orders_paused_until(db: &SqlitePool) -> Option<chrono::DateTime<chrono::Utc>> {
        future_deadline(db, "orders_paused_until").await
    }

    /// Active ad-hoc force-open deadline (`force_open_until`). While this
    /// is in the future, ordering is open regardless of the weekly
    /// schedule (the "Jetzt öffnen on a Ruhetag" override). `today_slots`
    /// reads this; the manual pause/snooze still beats it.
    pub async fn force_open_until(db: &SqlitePool) -> Option<chrono::DateTime<chrono::Utc>> {
        future_deadline(db, "force_open_until").await
    }

    /// The single source of truth for "are online orders paused right
    /// now?" — used by `place_order` and every customer-facing surface
    /// so they never disagree. Returns `(paused, resume_label)`:
    ///   * `paused`        — indefinite switch on, OR an unexpired snooze.
    ///   * `resume_label`  — local "HH:MM" the snooze ends, when the
    ///     pause is purely timed (None for indefinite, so the UI shows
    ///     the generic/custom message).
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

    /// Resolve the admin-editable receipt config (Bon-Editor) into the
    /// shared `ReceiptConfig` the layout consumes. Defaults match the
    /// historical hardcoded behaviour so an un-migrated DB is unchanged.
    pub async fn receipt_config(db: &SqlitePool) -> kitchen_protocol::receipt::ReceiptConfig {
        use kitchen_protocol::receipt::ReceiptLabels;
        let header_text = get_string(db, "receipt_header_text").await;
        let footer_text = get_string(db, "receipt_footer_text").await;
        let show_allergens = get_bool(db, "receipt_show_allergens", true).await;
        let show_logo = get_bool(db, "receipt_show_logo", true).await;

        // Batch-load all label overrides in one query, into a map.
        let label_rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM app_settings WHERE key LIKE 'receipt_label_%'")
                .fetch_all(db)
                .await
                .unwrap_or_default();
        let map: std::collections::HashMap<String, String> = label_rows.into_iter().collect();
        let g = |k: &str| map.get(k).cloned().unwrap_or_default();
        let labels = ReceiptLabels {
            channel_delivery: g("receipt_label_channel_delivery"),
            channel_pickup: g("receipt_label_channel_pickup"),
            channel_dinein: g("receipt_label_channel_dinein"),
            paid_badge: g("receipt_label_paid_badge"),
            unpaid_badge: g("receipt_label_unpaid_badge"),
            paid_confirm: g("receipt_label_paid_confirm"),
            paid_by: g("receipt_label_paid_by"),
            cash: g("receipt_label_cash"),
            collect_delivery: g("receipt_label_collect_delivery"),
            collect_pickup: g("receipt_label_collect_pickup"),
            collect_dinein: g("receipt_label_collect_dinein"),
            disclaimer: g("receipt_label_disclaimer"),
            qr_caption: g("receipt_label_qr_caption"),
            allergen_notice: g("receipt_label_allergen_notice"),
            reprint_banner: g("receipt_label_reprint_banner"),
            test_banner: g("receipt_label_test_banner"),
        };

        kitchen_protocol::receipt::ReceiptConfig {
            header_text,
            footer_text,
            show_allergens,
            show_logo,
            labels,
        }
    }

    /// Resolve the QR target URL for an order per `receipt_qr_mode`:
    ///
    ///   * "static-url" → the fixed `receipt_qr_static_url` (e.g. the
    ///     shop's homepage), or `None` if that's blank.
    ///   * anything else ("order-url", default) → `<public_url>/orders/<id>`
    ///     — the live order channel the customer can receive messages on.
    ///
    /// `None` disables the QR block.
    pub async fn resolve_qr_url(
        db: &SqlitePool,
        public_url_base: &str,
        order_id: &str,
    ) -> Option<String> {
        let mode = get_string(db, "receipt_qr_mode").await;
        if mode == "static-url" {
            let url = get_string(db, "receipt_qr_static_url").await;
            return (!url.is_empty()).then_some(url);
        }
        // order-url (default)
        let base = public_url_base.trim_end_matches('/');
        (!base.is_empty()).then(|| format!("{base}/orders/{order_id}"))
    }

    /// A trimmed string setting, empty when unset.
    async fn get_string(db: &SqlitePool, key: &str) -> String {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = ?1")
                .bind(key)
                .fetch_optional(db)
                .await
                .ok()
                .flatten();
        row.map(|(v,)| v.trim().to_string()).unwrap_or_default()
    }

    /// A boolean setting, `default` when unset/invalid.
    async fn get_bool(db: &SqlitePool, key: &str, default: bool) -> bool {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = ?1")
                .bind(key)
                .fetch_optional(db)
                .await
                .ok()
                .flatten();
        match row.map(|(v,)| v.trim().to_string()) {
            Some(v) if matches!(v.as_str(), "1" | "true") => true,
            Some(v) if matches!(v.as_str(), "0" | "false") => false,
            _ => default,
        }
    }
}
