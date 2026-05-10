//! Bookkeeping page: list of all orders in a date range, day-by-day totals,
//! per-item stats, CSV export.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;
use crate::pages::order::status_label_de;

/// One row from order_items used to build the human-readable line summary.
/// Tuple: (menu_number_snapshot, name_snapshot, quantity, options_json, extras_json).
#[cfg(feature = "ssr")]
type ItemSummaryRow = (Option<String>, String, i64, String, Option<String>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRow {
    pub id: String,
    pub order_number: String,
    pub status: String,
    pub contact_name: String,
    pub created_at: String,
    pub pickup_label: String,
    pub items_summary: String,
    pub total_cents: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayTotal {
    pub date: String, // YYYY-MM-DD
    pub orders: i64,
    pub revenue_cents: i64, // only counts non-cancelled
    pub cancelled: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemStat {
    pub name: String,
    pub variant: Option<String>, // size label or NULL
    pub quantity: i64,
    pub revenue_cents: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistoryReport {
    pub from: String,
    pub to: String,
    pub orders: Vec<HistoryRow>,
    pub days: Vec<DayTotal>,
    pub items: Vec<ItemStat>,
    pub grand_revenue_cents: i64,
    pub grand_orders: i64,
    pub cancelled_count: i64,
}

#[server(
    name = ListAdminHistory,
    prefix = "/api",
    endpoint = "list_admin_history"
)]
pub async fn list_admin_history(
    from: String, // YYYY-MM-DD inclusive
    to: String,   // YYYY-MM-DD inclusive
) -> Result<HistoryReport, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    // Basic shape validation; SQLite parses date strings lexicographically.
    if from.len() != 10 || to.len() != 10 {
        return Err(ServerFnError::new("Datum im Format YYYY-MM-DD erwartet"));
    }
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // SQLite stored `created_at` as 'YYYY-MM-DD HH:MM:SS'. Use BETWEEN with end-of-day.
    let from_ts = format!("{from} 00:00:00");
    let to_ts = format!("{to} 23:59:59");

    let order_rows =
        sqlx::query_as::<_, (String, String, String, String, String, Option<String>, i64)>(
            "SELECT id, order_number, status, contact_name, created_at, scheduled_for, total_cents
         FROM orders
         WHERE created_at BETWEEN ?1 AND ?2
         ORDER BY created_at DESC",
        )
        .bind(&from_ts)
        .bind(&to_ts)
        .fetch_all(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("orders query: {e}")))?;

    // Build rows + day totals.
    let mut orders = Vec::with_capacity(order_rows.len());
    let mut days: std::collections::BTreeMap<String, DayTotal> = std::collections::BTreeMap::new();
    let mut grand_revenue = 0_i64;
    let mut grand_orders = 0_i64;
    let mut cancelled = 0_i64;

    for (id, order_number, status, contact_name, created_at, scheduled_for, total_cents) in
        order_rows
    {
        let day = created_at.get(..10).unwrap_or("").to_string();
        let entry = days.entry(day.clone()).or_insert(DayTotal {
            date: day.clone(),
            orders: 0,
            revenue_cents: 0,
            cancelled: 0,
        });
        entry.orders += 1;
        if status == "cancelled" {
            entry.cancelled += 1;
            cancelled += 1;
        } else {
            entry.revenue_cents += total_cents;
            grand_revenue += total_cents;
        }
        grand_orders += 1;

        // Per-order items summary (compact text). Same shape as the kitchen board.
        let items: Vec<ItemSummaryRow> = sqlx::query_as(
            "SELECT menu_number_snapshot, name_snapshot, quantity, options_json, extras_json
             FROM order_items WHERE order_id = ?1 ORDER BY id",
        )
        .bind(&id)
        .fetch_all(&db)
        .await
        .unwrap_or_default();

        let items_summary = items
            .into_iter()
            .map(|(num, n, q, opts, extras_json)| {
                let v: serde_json::Value = serde_json::from_str(&opts).unwrap_or_default();
                let variant = v.get("size_label").and_then(|x| x.as_str()).unwrap_or("");
                let prefix = num
                    .as_deref()
                    .map(|x| x.trim())
                    .filter(|x| !x.is_empty())
                    .map(|x| format!("{x}. "))
                    .unwrap_or_default();
                let extras_label = extras_json
                    .as_deref()
                    .and_then(|j| {
                        serde_json::from_str::<Vec<rusterando_shared::models::CartExtra>>(j).ok()
                    })
                    .filter(|v| !v.is_empty())
                    .map(|v| {
                        let labels = v
                            .into_iter()
                            .map(|e| e.label)
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!(" + {labels}")
                    })
                    .unwrap_or_default();
                if variant.is_empty() {
                    format!("{q}× {prefix}{n}{extras_label}")
                } else {
                    format!("{q}× {prefix}{n} ({variant}){extras_label}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");

        let pickup_label = match scheduled_for {
            Some(s) => format!("Heute {}", s.split_whitespace().nth(1).unwrap_or(&s)),
            None => "ASAP".to_string(),
        };

        orders.push(HistoryRow {
            id,
            order_number,
            status,
            contact_name,
            created_at,
            pickup_label,
            items_summary,
            total_cents,
        });
    }

    // Per-item stats across the whole range (excluding cancelled orders).
    let item_rows = sqlx::query_as::<_, (String, String, i64, i64)>(
        "SELECT oi.name_snapshot, oi.options_json,
                SUM(oi.quantity)         AS qty,
                SUM(oi.line_total_cents) AS revenue
         FROM order_items oi
         JOIN orders o ON o.id = oi.order_id
         WHERE o.created_at BETWEEN ?1 AND ?2
           AND o.status != 'cancelled'
         GROUP BY oi.name_snapshot, oi.options_json
         ORDER BY qty DESC",
    )
    .bind(&from_ts)
    .bind(&to_ts)
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("items query: {e}")))?;

    let items: Vec<ItemStat> = item_rows
        .into_iter()
        .map(|(name, opts, qty, revenue)| {
            let v: serde_json::Value = serde_json::from_str(&opts).unwrap_or_default();
            let variant = v
                .get("size_label")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            ItemStat {
                name,
                variant,
                quantity: qty,
                revenue_cents: revenue,
            }
        })
        .collect();

    Ok(HistoryReport {
        from,
        to,
        orders,
        days: days.into_values().collect(),
        items,
        grand_revenue_cents: grand_revenue,
        grand_orders,
        cancelled_count: cancelled,
    })
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

// chrono's `wasmbind` feature is enabled on the frontend crate, so
// `Local::now()` works both during SSR and after hydration in the
// browser — no separate cfg branches needed.
fn today_iso() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn iso_offset_days(days: i64) -> String {
    let n = chrono::Local::now() + chrono::Duration::days(days);
    n.format("%Y-%m-%d").to_string()
}

#[component]
pub fn AdminHistoryPage() -> impl IntoView {
    let from = RwSignal::new(iso_offset_days(-7));
    let to = RwSignal::new(today_iso());

    let report = Resource::new(
        move || (from.get(), to.get()),
        |(from, to)| async move { list_admin_history(from, to).await },
    );

    let csv_href = move || format!("/admin/history.csv?from={}&to={}", from.get(), to.get());

    view! {
        <AdminShell>
            <section class="admin-history">
                <div class="page-bar">
                    <h1>"Buchhaltung — Bestellverlauf"</h1>
                </div>

            <form class="filter" on:submit=move |ev| ev.prevent_default()>
                <label>
                    <span>"Von"</span>
                    <input type="date"
                        prop:value=move || from.get()
                        on:change=move |ev| from.set(event_target_value(&ev))/>
                </label>
                <label>
                    <span>"Bis"</span>
                    <input type="date"
                        prop:value=move || to.get()
                        on:change=move |ev| to.set(event_target_value(&ev))/>
                </label>
                <div class="quick">
                    <button type="button" on:click=move |_| { from.set(today_iso()); to.set(today_iso()); }>"Heute"</button>
                    <button type="button" on:click=move |_| { from.set(iso_offset_days(-7)); to.set(today_iso()); }>"7 Tage"</button>
                    <button type="button" on:click=move |_| { from.set(iso_offset_days(-30)); to.set(today_iso()); }>"30 Tage"</button>
                </div>
                <a class="btn ghost" href=csv_href download="bestellungen.csv">"⬇ CSV exportieren"</a>
            </form>

                <Suspense fallback=|| view! { <p>"Lädt…"</p> }>
                    {move || report.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(r) => view! { <Report r/> }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn Report(r: HistoryReport) -> impl IntoView {
    view! {
        <div class="summary-tiles">
            <div class="tile">
                <span class="label">"Bestellungen"</span>
                <strong class="value">{r.grand_orders}</strong>
                <span class="sub">{r.cancelled_count} " storniert"</span>
            </div>
            <div class="tile">
                <span class="label">"Umsatz"</span>
                <strong class="value">{format_eur(r.grand_revenue_cents)}</strong>
                <span class="sub">{format!("{} – {}", r.from, r.to)}</span>
            </div>
            <div class="tile">
                <span class="label">"Ø Bestellung"</span>
                <strong class="value">
                    {if r.grand_orders > r.cancelled_count {
                        let n = r.grand_orders - r.cancelled_count;
                        format_eur(r.grand_revenue_cents / n.max(1))
                    } else {
                        "—".into()
                    }}
                </strong>
            </div>
        </div>

        <h2>"Tagesübersicht"</h2>
        {if r.days.is_empty() {
            view! { <p class="empty">"Keine Bestellungen im Zeitraum."</p> }.into_any()
        } else {
            view! {
                <table class="hist-table">
                    <thead>
                        <tr>
                            <th>"Datum"</th>
                            <th>"Bestellungen"</th>
                            <th>"Storniert"</th>
                            <th>"Umsatz"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {r.days.into_iter().map(|d| view! {
                            <tr>
                                <td>{d.date}</td>
                                <td>{d.orders}</td>
                                <td class="muted">{d.cancelled}</td>
                                <td class="amt"><strong>{format_eur(d.revenue_cents)}</strong></td>
                            </tr>
                        }).collect_view()}
                    </tbody>
                </table>
            }.into_any()
        }}

        <h2>"Verkaufte Artikel"</h2>
        {if r.items.is_empty() {
            view! { <p class="empty">"—"</p> }.into_any()
        } else {
            view! {
                <table class="hist-table">
                    <thead>
                        <tr>
                            <th>"Artikel"</th>
                            <th>"Variante"</th>
                            <th>"Anzahl"</th>
                            <th>"Umsatz"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {r.items.into_iter().map(|i| view! {
                            <tr>
                                <td>{i.name}</td>
                                <td class="muted">{i.variant.unwrap_or_else(|| "—".into())}</td>
                                <td><strong>{i.quantity}</strong></td>
                                <td class="amt">{format_eur(i.revenue_cents)}</td>
                            </tr>
                        }).collect_view()}
                    </tbody>
                </table>
            }.into_any()
        }}

        <h2>"Bestellungen"</h2>
        {if r.orders.is_empty() {
            view! { <p class="empty">"—"</p> }.into_any()
        } else {
            view! {
                <table class="hist-table">
                    <thead>
                        <tr>
                            <th>"Nr."</th>
                            <th>"Datum"</th>
                            <th>"Kunde"</th>
                            <th>"Status"</th>
                            <th>"Artikel"</th>
                            <th>"Summe"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {r.orders.into_iter().map(|o| view! {
                            <tr>
                                <td>
                                    <a href=format!("/admin/orders/{}", o.id)>{o.order_number}</a>
                                </td>
                                <td class="muted">{o.created_at}</td>
                                <td>{o.contact_name}</td>
                                <td>
                                    <span class="status-pill" data-status=o.status.clone()>
                                        {status_label_de(&o.status)}
                                    </span>
                                </td>
                                <td class="items-cell">{o.items_summary}</td>
                                <td class="amt"><strong>{format_eur(o.total_cents)}</strong></td>
                            </tr>
                        }).collect_view()}
                    </tbody>
                </table>
            }.into_any()
        }}
    }
}
