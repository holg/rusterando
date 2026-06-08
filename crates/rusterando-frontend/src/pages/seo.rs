//! Restaurant structured data (schema.org JSON-LD).
//!
//! Emits a `Restaurant` graph — name, address, geo, phone, opening hours, and
//! the FULL menu with prices (`hasMenu` → `MenuSection` → `MenuItem` →
//! `Offer`) — into `<head>` on the homepage and `/menu`. This is how a local
//! restaurant earns the Google rich result, and it gives Google every item +
//! price WITHOUT thin per-item HTML pages.
//!
//! Two deliberate properties:
//!   * **SSR-only.** The `<script>` is rendered only on the server (the raw
//!     HTML a crawler reads). On the hydrated client it renders NOTHING — no
//!     network fetch, no DOM work on navigation. A crawler never SPA-navigates
//!     our client routes, so emitting it client-side would be pure waste.
//!   * **Cached.** The string is built once at boot and stored in
//!     `JsonLdHandle` (an `Arc<RwLock<Arc<str>>>`, same pattern as
//!     `BrandingHandle`), read synchronously during SSR — no per-request DB
//!     query. It is rebuilt (`rebuild_jsonld_cache`) at the few admin edit
//!     paths that change its inputs (menu items/categories, branding `shop_*`,
//!     weekly opening hours) so prices can never go stale.
//!
//! IMPORTANT: if you add a new admin write path that changes a menu item,
//! category, branding field, or weekly opening-hours row, call
//! `rebuild_jsonld_cache().await` after the write or the structured data will
//! serve stale data.

#[cfg(feature = "ssr")]
use leptos::prelude::use_context;

#[cfg(feature = "ssr")]
mod ssr {
    use crate::branding::Branding;
    use serde_json::json;
    use sqlx::{Row, SqlitePool};
    use std::sync::{Arc, RwLock};

    /// Server-side cache of the rendered Restaurant JSON-LD string. Newtype so
    /// `use_context` disambiguates from other `Arc<RwLock<…>>`.
    #[derive(Clone, Default)]
    pub struct JsonLdHandle(pub Arc<RwLock<Arc<str>>>);

    impl JsonLdHandle {
        pub fn new(initial: String) -> Self {
            Self(Arc::new(RwLock::new(Arc::from(initial.as_str()))))
        }
        pub fn get(&self) -> Arc<str> {
            self.0
                .read()
                .map(|g| g.clone())
                .unwrap_or_else(|_| Arc::from(""))
        }
        pub fn set(&self, s: String) {
            if let Ok(mut g) = self.0.write() {
                *g = Arc::from(s.as_str());
            }
        }
    }

    /// Build the `Restaurant` JSON-LD string from the DB + a `Branding`
    /// snapshot. Pure inputs (no leptos context) so it runs at boot AND from
    /// any server fn. Empty string on a DB error → the component emits nothing.
    pub async fn build_restaurant_jsonld(db: &SqlitePool, branding: &Branding) -> String {
        let base = crate::pages::home::site_url();
        let base = base.trim_end_matches('/').to_string();

        // openingHoursSpecification — one per open weekday shift.
        const DAY: [&str; 7] = [
            "Sunday",
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
        ];
        let opening: Vec<serde_json::Value> = match sqlx::query(
            "SELECT weekday, open_time, close_time FROM opening_hours WHERE is_closed = 0 ORDER BY weekday, open_time",
        )
        .fetch_all(db)
        .await
        {
            Ok(rows) => rows
                .iter()
                .filter_map(|r| {
                    let day = DAY.get(r.get::<i64, _>("weekday") as usize)?;
                    let opens: String = r.get("open_time");
                    let closes: String = r.get("close_time");
                    Some(json!({
                        "@type": "OpeningHoursSpecification",
                        "dayOfWeek": day, "opens": opens, "closes": closes,
                    }))
                })
                .collect(),
            Err(_) => Vec::new(),
        };

        // hasMenu — sections (active categories) → listed items → offers.
        let cat_name = crate::i18n::coalesce_col("name", "");
        let cats = sqlx::query(&format!(
            "SELECT id, {cat_name} AS name FROM menu_categories WHERE is_active = 1 ORDER BY sort_order"
        ))
        .fetch_all(db)
        .await
        .unwrap_or_default();

        let item_name = crate::i18n::coalesce_col("name", "");
        let item_desc = crate::i18n::coalesce_col("description", "");
        let items = sqlx::query(&format!(
            "SELECT category_id, {item_name} AS name, {item_desc} AS description,
                    price_small_cents, price_large_cents
             FROM menu_items WHERE is_listed = 1
             ORDER BY sort_order, CAST(COALESCE(menu_number,'') AS INTEGER), menu_number"
        ))
        .fetch_all(db)
        .await
        .unwrap_or_default();

        let offer = |cents: i64| {
            json!({
                "@type": "Offer",
                "price": format!("{}.{:02}", cents / 100, (cents % 100).abs()),
                "priceCurrency": "EUR",
            })
        };
        let sections: Vec<serde_json::Value> = cats
            .iter()
            .map(|c| {
                let cid: String = c.get("id");
                let cname: String = c.get("name");
                let menu_items: Vec<serde_json::Value> = items
                    .iter()
                    .filter(|it| it.get::<String, _>("category_id") == cid)
                    .map(|it| {
                        let mut offers = vec![offer(it.get::<i64, _>("price_small_cents"))];
                        if let Some(large) = it.get::<Option<i64>, _>("price_large_cents") {
                            offers.push(offer(large));
                        }
                        let mut node = json!({
                            "@type": "MenuItem",
                            "name": it.get::<String, _>("name"),
                            "offers": offers,
                        });
                        if let Some(d) = it
                            .get::<Option<String>, _>("description")
                            .filter(|d| !d.trim().is_empty())
                        {
                            node["description"] = json!(d);
                        }
                        node
                    })
                    .collect();
                json!({ "@type": "MenuSection", "name": cname, "hasMenuItem": menu_items })
            })
            .collect();

        let mut node = json!({
            "@context": "https://schema.org",
            "@type": "Restaurant",
            "name": branding.display_name(),
            "url": base,
            "servesCuisine": ["Pizza", "Pasta", "Italienisch"],
            "priceRange": "€€",
            "acceptsReservations": false,
            "hasMenu": { "@type": "Menu", "hasMenuSection": sections },
        });
        if !branding.shop_phone.trim().is_empty() {
            node["telephone"] = json!(branding.shop_phone);
        }
        if !branding.shop_email.trim().is_empty() {
            node["email"] = json!(branding.shop_email);
        }
        if !branding.shop_address_street.trim().is_empty()
            || !branding.shop_address_plz_city.trim().is_empty()
        {
            let plz_city = branding.shop_address_plz_city.trim();
            let (postal, locality) = plz_city
                .split_once(' ')
                .map(|(p, c)| (p.to_string(), c.to_string()))
                .unwrap_or_else(|| (String::new(), plz_city.to_string()));
            node["address"] = json!({
                "@type": "PostalAddress",
                "streetAddress": branding.shop_address_street,
                "postalCode": postal,
                "addressLocality": locality,
                "addressCountry": "DE",
            });
        }
        if branding.shop_lat != 0.0 || branding.shop_lon != 0.0 {
            node["geo"] = json!({
                "@type": "GeoCoordinates",
                "latitude": branding.shop_lat,
                "longitude": branding.shop_lon,
            });
        }
        if !opening.is_empty() {
            node["openingHoursSpecification"] = json!(opening);
        }
        serde_json::to_string(&node).unwrap_or_default()
    }
}

#[cfg(feature = "ssr")]
pub use ssr::{build_restaurant_jsonld, JsonLdHandle};

/// Rebuild + store the cached JSON-LD string from the current DB + branding.
/// Call from any admin server fn after a write that changes a menu item,
/// category, branding `shop_*` field, or weekly opening-hours row. No-op if
/// the handle/db/branding aren't in context (e.g. unit tests).
#[cfg(feature = "ssr")]
pub async fn rebuild_jsonld_cache() {
    use sqlx::SqlitePool;
    let (Some(db), Some(handle)) = (use_context::<SqlitePool>(), use_context::<JsonLdHandle>())
    else {
        return;
    };
    let branding = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get())
        .unwrap_or_default();
    let json = build_restaurant_jsonld(&db, &branding).await;
    handle.set(json);
}

/// Client/SSR-agnostic no-op so callers in non-ssr server-fn stubs compile.
#[cfg(not(feature = "ssr"))]
pub async fn rebuild_jsonld_cache() {}

// NOTE: the JSON-LD `<script>` is emitted directly in the `shell()` `<head>`
// (see app.rs), NOT via a body component. A body-level leptos_meta `Script`
// keeps a hydrate-tracked placeholder, so rendering it on SSR but nothing on
// hydrate trips the tachys walker (same class of bug as the rejected
// `<Title>` component). Baking it into `<head>` in the shell — which runs
// SSR-only and lives outside the hydrate walk — is the robust path, exactly
// like the `<title>` and bootstrap-blob already there.
