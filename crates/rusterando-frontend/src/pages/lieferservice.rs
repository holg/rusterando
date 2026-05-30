//! `/lieferservice/<area_slug>` — per-delivery-zone SEO landing page.
//!
//! Hyperlocal SEO: a dedicated page per active `delivery_zones` row, so the
//! shop can rank for "Pizza-Lieferservice <Ort>" — the local intent that
//! actually converts. Each page is self-canonical and carries genuinely
//! distinct content (fee, min-order, ETA, hours, address + an admin-editable
//! `seo_text` paragraph, falling back to a generated German template) so it's
//! not thin/duplicate. Unknown/inactive slug → HTTP 404.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

/// Everything the landing page needs for one zone, resolved server-side.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AreaPage {
    pub area_name: String,
    pub fee_cents: i64,
    pub min_order_cents: i64,
    pub eta_minutes: i64,
    /// Admin-written copy; empty → the page builds a generated paragraph.
    pub seo_text: String,
    pub shop_name: String,
    pub shop_address: String,
    pub shop_phone: String,
    /// Weekly hours as ("Mo–Fr"-style label, "11:30–14:30 · 17:00–22:00").
    pub hours_rows: Vec<(String, String)>,
}

#[server(name = LoadDeliveryArea, prefix = "/api", endpoint = "load_delivery_area")]
pub async fn load_delivery_area(slug: String) -> Result<Option<AreaPage>, ServerFnError> {
    use sqlx::{Row, SqlitePool};

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let want = rusterando_shared::models::seo_slug(&slug);
    if want.is_empty() {
        return Ok(None);
    }

    // Match an active zone by re-slugging its name (same fn the sitemap uses).
    let rows = sqlx::query(
        "SELECT COALESCE(name,'') AS name, fee_cents, min_order_cents, eta_minutes,
                COALESCE(seo_text,'') AS seo_text
         FROM delivery_zones WHERE is_active = 1",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load zone: {e}")))?;

    let Some(row) = rows.into_iter().find(|r| {
        let name: String = r.get("name");
        rusterando_shared::models::seo_slug(&name) == want
    }) else {
        return Ok(None);
    };

    let branding = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get())
        .unwrap_or_default();

    // Weekly hours: collapse each weekday's open shifts into one label.
    let hr = sqlx::query(
        "SELECT weekday, open_time, close_time, is_closed FROM opening_hours ORDER BY weekday, open_time",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load hours: {e}")))?;
    let mut hours_rows: Vec<(String, String)> = Vec::new();
    for wd in [1_i64, 2, 3, 4, 5, 6, 0] {
        let windows: Vec<String> = hr
            .iter()
            .filter(|r| r.get::<i64, _>("weekday") == wd && r.get::<i64, _>("is_closed") == 0)
            .map(|r| {
                let o: String = r.get("open_time");
                let c: String = r.get("close_time");
                format!("{o}–{c}")
            })
            .collect();
        let label = match wd {
            1 => "Montag",
            2 => "Dienstag",
            3 => "Mittwoch",
            4 => "Donnerstag",
            5 => "Freitag",
            6 => "Samstag",
            _ => "Sonntag",
        };
        let windows = if windows.is_empty() {
            "Ruhetag".to_string()
        } else {
            windows.join(" · ")
        };
        hours_rows.push((label.to_string(), windows));
    }

    Ok(Some(AreaPage {
        area_name: row.get("name"),
        fee_cents: row.get("fee_cents"),
        min_order_cents: row.get("min_order_cents"),
        eta_minutes: row.get("eta_minutes"),
        seo_text: row.get("seo_text"),
        shop_name: branding.display_name(),
        shop_address: branding.full_address(),
        shop_phone: branding.shop_phone,
        hours_rows,
    }))
}

/// Generated fallback paragraph when the admin hasn't written `seo_text`.
fn generated_copy(p: &AreaPage) -> String {
    let fee = if p.fee_cents == 0 {
        "ohne Liefergebühr".to_string()
    } else {
        format!("für {} Liefergebühr", format_eur(p.fee_cents))
    };
    let minord = if p.min_order_cents > 0 {
        format!(" ab einem Mindestbestellwert von {}", format_eur(p.min_order_cents))
    } else {
        String::new()
    };
    format!(
        "{shop} liefert frische Pizza, Pasta und mehr nach {area} — {fee}{minord}, \
         in der Regel innerhalb von ca. {eta} Minuten. Jetzt bequem online bestellen \
         und direkt nach Hause liefern lassen.",
        shop = p.shop_name,
        area = p.area_name,
        eta = p.eta_minutes,
    )
}

#[component]
pub fn DeliveryAreaPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let slug = move || params.read().get("area_slug").unwrap_or_default();
    // Blocking so an unknown slug sets HTTP 404 before the head flushes and
    // the canonical/meta land in <head>.
    let data = Resource::new_blocking(slug.clone(), |s| async move { load_delivery_area(s).await });

    let base = crate::pages::home::site_url();
    let base = base.trim_end_matches('/').to_string();

    // Canonical is derivable from the slug alone, so emit it OUTSIDE the
    // Suspense (synchronously) — that's what guarantees it lands in <head>.
    // A canonical set inside the resolved Suspense branch resolves after the
    // head is already streamed and silently never appears (same SSR-head
    // timing lesson as the JSON-LD). For an unknown slug the page 404s, so a
    // self-canonical on a 404 is harmless.
    let canonical = format!(
        "{base}/lieferservice/{}",
        rusterando_shared::models::seo_slug(&slug())
    );

    view! {
        <leptos_meta::Link rel="canonical" href=canonical/>
        <Suspense fallback=|| view! { <p class="loading">{crate::t!("common.loading")}</p> }>
            {move || {
                data.get().map(move |res| match res {
                    Err(e) => view! { <p class="error">{format!("{}: {e}", crate::t!("common.error"))}</p> }.into_any(),
                    Ok(None) => {
                        #[cfg(feature = "ssr")]
                        if let Some(resp) = use_context::<leptos_axum::ResponseOptions>() {
                            resp.set_status(http::StatusCode::NOT_FOUND);
                        }
                        view! { <p class="error">{crate::t!("errors.page_not_found")}</p> }.into_any()
                    }
                    Ok(Some(p)) => {
                        let copy = if p.seo_text.trim().is_empty() {
                            generated_copy(&p)
                        } else {
                            p.seo_text.clone()
                        };
                        let desc = format!(
                            "Pizza-Lieferservice {ort}: online bestellen, Lieferung in ca. {eta} Min. \
                             Lieferung {fee}.",
                            ort = p.area_name,
                            eta = p.eta_minutes,
                            fee = if p.fee_cents == 0 { "kostenlos".to_string() } else { format_eur(p.fee_cents) },
                        );
                        let hours = p.hours_rows.clone();
                        view! {
                            // Canonical is emitted once outside this Suspense
                            // (see above). Here we only add the per-area meta
                            // description (best-effort head injection).
                            <leptos_meta::Meta name="description" content=desc/>
                            <section class="lieferservice">
                                <h1>{crate::t!("lieferservice.title").replace("{area}", &p.area_name)}</h1>
                                <p class="lead">{copy}</p>
                                <ul class="liefer-facts">
                                    <li><strong>{crate::t!("lieferservice.delivery_fee")} ": "</strong>
                                        {if p.fee_cents == 0 { crate::t!("lieferservice.fee_free") } else { format_eur(p.fee_cents) }}</li>
                                    {(p.min_order_cents > 0).then(|| view! {
                                        <li><strong>{crate::t!("lieferservice.min_order")} ": "</strong>{format_eur(p.min_order_cents)}</li>
                                    })}
                                    <li><strong>{crate::t!("lieferservice.delivery_eta")} ": "</strong>{crate::t!("lieferservice.eta_minutes").replace("{n}", &p.eta_minutes.to_string())}</li>
                                </ul>
                                <a class="btn primary" href="/menu">{crate::t!("home.order_now")}</a>
                                <h2>{crate::t!("header.opening_hours")}</h2>
                                <table class="liefer-hours">
                                    {hours.into_iter().map(|(day, win)| view! {
                                        <tr><th>{day}</th><td>{win}</td></tr>
                                    }).collect_view()}
                                </table>
                                {(!p.shop_address.is_empty()).then(|| view! {
                                    <p class="muted">{format!("{} · {}", p.shop_name, p.shop_address)}</p>
                                })}
                            </section>
                        }.into_any()
                    }
                })
            }}
        </Suspense>
    }
}
