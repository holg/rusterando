//! /admin dashboard — tile grid + today's quick stats.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdminStats {
    pub active_orders: i64, // received + preparing + ready_for_pickup
    pub today_orders: i64,  // count placed today (any non-cancelled status)
    pub today_revenue_cents: i64,
}

#[server(
    name = LoadAdminStats,
    prefix = "/api",
    endpoint = "load_admin_stats"
)]
pub async fn load_admin_stats() -> Result<AdminStats, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Active-orders count zählt ALLE laufenden Bestellungen — Küche
    // muss auch Test-Bestellungen sehen, sonst übersieht man eine
    // gerade getestete Order, die noch durchläuft.
    let active: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM orders
         WHERE status IN ('received', 'preparing', 'ready_for_pickup')",
    )
    .fetch_one(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("active count: {e}")))?;

    // SQLite stores `created_at` as 'YYYY-MM-DD HH:MM:SS' (UTC). For day-of
    // we look at the local day; using the server's local time avoids timezone
    // confusion at midnight rollover.
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let from = format!("{today} 00:00:00");
    let to = format!("{today} 23:59:59");

    // Heutiger Umsatz dagegen NUR Live — sonst sieht der Inhaber einen
    // künstlich aufgeblähten Tagesumsatz, der aus eigenen Test-Bestellungen
    // resultiert.
    let today_row: (i64, Option<i64>) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(total_cents), 0)
         FROM orders
         WHERE created_at BETWEEN ?1 AND ?2
           AND status != 'cancelled'
           AND stripe_mode = 'live'",
    )
    .bind(&from)
    .bind(&to)
    .fetch_one(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("today stats: {e}")))?;

    Ok(AdminStats {
        active_orders: active.0,
        today_orders: today_row.0,
        today_revenue_cents: today_row.1.unwrap_or(0),
    })
}

#[component]
pub fn AdminHomePage() -> impl IntoView {
    let stats = Resource::new(|| (), |_| async move { load_admin_stats().await });

    view! {
        <AdminShell>
            <section class="admin-dashboard">
                <h1>"Übersicht"</h1>

                // Live shop-status banner — same 3-level status (and exact
                // text) the CUSTOMER sees, so staff aren't misled by a green
                // "open" while the shop is actually closed-until-16:00. Click
                // → /admin/hours to change it.
                <ShopStatusPanel/>

                // Fallback wraps in a <div> matching the resolved
                // <div class="stats-row"> shape. Mixing <p> with <div>
                // makes tachys' walker panic at the Suspense boundary
                // (failed_to_cast_element) — the runtime expects the
                // same element tag on both sides.
                <Suspense fallback=|| view! {
                    <div class="stats-row loading"><p class="loading">"Lädt…"</p></div>
                }>
                    {move || stats.get().map(|res| match res {
                        Err(e) => view! {
                            <div class="stats-row error"><p class="error">{format!("Fehler: {e}")}</p></div>
                        }.into_any(),
                        Ok(s) => view! { <Stats s/> }.into_any(),
                    })}
                </Suspense>

                <div class="admin-tiles">
                    <Tile
                        href="/admin/orders"
                        icon="🍕"
                        title="Bestellungen"
                        sub="Eingehende Bestellungen verwalten"
                        badge=Memo::new(move |_| {
                            stats.get().and_then(|r| r.ok())
                                .map(|s| s.active_orders).unwrap_or(0)
                        })/>
                    <Tile
                        href="/admin/menu"
                        icon="📋"
                        title="Speisekarte"
                        sub="Preise und Verfügbarkeit pflegen"
                        badge=Memo::new(|_| 0_i64)/>
                    <Tile
                        href="/admin/history"
                        icon="📊"
                        title="Buchhaltung"
                        sub="Vergangene Bestellungen, Tagesumsatz, CSV-Export"
                        badge=Memo::new(|_| 0_i64)/>
                    <Tile
                        href="/menu.pdf"
                        icon="📄"
                        title="Speisekarte als PDF"
                        sub="Druckbare Speisekarte herunterladen"
                        badge=Memo::new(|_| 0_i64)/>
                    <Tile
                        href="/admin/printer"
                        icon="🧾"
                        title="Bon-Vorschau"
                        sub="So sieht der Küchen-Bon gedruckt aus"
                        badge=Memo::new(|_| 0_i64)/>
                </div>

                // Live receipt preview on the overview: the exact ticket
                // layout the kitchen prints, rendered from the shared
                // `build_receipt` model. Links through to the full pane.
                <div class="admin-receipt-card">
                    <header class="card-head">
                        <h2>"Bon-Vorschau"</h2>
                        <a class="go" href="/admin/printer">"Ganze Vorschau →"</a>
                    </header>
                    <crate::pages::admin::printer_admin::ReceiptPreviewPane/>
                </div>

                // Deployment / tenant diagnostics at the bottom of the overview
                // — which shop/env/DB this admin session is editing, plus the
                // process mode. Same panel as /admin/settings.
                <crate::pages::admin::settings_admin::DeploymentPanel/>
            </section>
        </AdminShell>
    }
}

/// Full-width shop open/closed banner for the dashboard. Three-level colour
/// + the exact customer-facing reason text, linking to /admin/hours. Initial
/// value from the `shop_status` server fn (OnceResource, hydration-safe in
/// its own <Suspense> with a matching fallback shape — same pattern as the
/// AdminShell chip); the `/api/live/shop` SSE flips it live.
#[component]
fn ShopStatusPanel() -> impl IntoView {
    use rusterando_shared::models::ShopLevel;

    let init = OnceResource::new(crate::pages::menu::shop_status());
    let (live, set_live) = signal::<Option<(ShopLevel, String)>>(None);
    crate::utils::subscribe_shop_status(set_live);

    view! {
        <Suspense fallback=|| view! {
            <a class="shop-status-panel opens-soon" href="/admin/hours">
                <span class="dot"></span>
                <span class="txt">"…"</span>
            </a>
        }>
            {move || {
                // Live SSE override wins over the initial resource value.
                let (level, reason) = live.get().unwrap_or_else(|| {
                    init.get()
                        .and_then(|r| r.ok())
                        .unwrap_or((ShopLevel::Open, String::new()))
                });
                let (cls, label) = match level {
                    ShopLevel::Open => ("shop-status-panel open", "🟢 Online-Bestellung läuft".to_string()),
                    ShopLevel::OpensLater => {
                        let r = if reason.is_empty() { "Öffnet später".to_string() } else { reason.clone() };
                        ("shop-status-panel opens-soon", format!("🟡 {r}"))
                    }
                    ShopLevel::Closed => {
                        let r = if reason.is_empty() { "Geschlossen".to_string() } else { reason.clone() };
                        ("shop-status-panel closed", format!("🔴 {r}"))
                    }
                };
                view! {
                    <a class=cls href="/admin/hours" title="Öffnungszeiten verwalten">
                        <span class="dot"></span>
                        <span class="txt">{label}</span>
                        <span class="go">"Öffnungszeiten →"</span>
                    </a>
                }
            }}
        </Suspense>
    }
}

#[component]
fn Stats(s: AdminStats) -> impl IntoView {
    let today = chrono::Local::now().format("%d.%m.%Y").to_string();
    view! {
        <div class="stats-row">
            <div class="stat-tile">
                <span class="label">"Offene Bestellungen"</span>
                <strong class="value">{s.active_orders}</strong>
                <span class="sub">"in Warteschlange"</span>
            </div>
            <div class="stat-tile">
                <span class="label">"Heute"</span>
                <strong class="value">{s.today_orders}</strong>
                <span class="sub">{format!("Bestellungen am {today}")}</span>
            </div>
            <div class="stat-tile">
                <span class="label">"Umsatz heute"</span>
                <strong class="value">{format_eur(s.today_revenue_cents)}</strong>
                <span class="sub">"ohne Stornierungen"</span>
            </div>
        </div>
    }
}

#[component]
fn Tile(
    href: &'static str,
    icon: &'static str,
    title: &'static str,
    sub: &'static str,
    badge: Memo<i64>,
) -> impl IntoView {
    let is_external = href.starts_with("http") || href.ends_with(".pdf");
    // The badge <span> is always rendered, visibility flips via class
    // toggle. Previously we returned `Some(view!{...})` vs `None`,
    // which gave SSR (resource resolved → Some) and the first hydrate
    // tick (memo seeded → 0 → None) different DOM shapes. tachys
    // panicked in `failed_to_cast_element` because it expected a
    // <span> at the position SSR put one but got nothing.
    view! {
        <a class="admin-tile" href=href
           target=if is_external { "_blank" } else { "_self" }
           rel=if is_external { "noopener" } else { "" }>
            <span class="icon">{icon}</span>
            <span class="title">
                {title}
                <span class=move || {
                    if badge.get() > 0 { "tile-badge" } else { "tile-badge hidden" }
                }>{move || badge.get()}</span>
            </span>
            <span class="sub">{sub}</span>
        </a>
    }
}
