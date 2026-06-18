//! Kitchen view: list of pending+active orders, with one-click status transitions.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;
use crate::pages::order::status_label_de;

/// (menu_number_snapshot, name_snapshot, quantity, options_json, extras_json,
///  removals_json)
#[cfg(feature = "ssr")]
type ItemSummaryRow = (
    Option<String>,
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminOrderRow {
    pub id: String,
    pub order_number: String,
    pub status: String,
    pub payment_status: String,
    pub order_type: String,
    pub address_line: Option<String>,
    pub contact_name: String,
    pub contact_phone: String,
    pub pickup_label: String,
    pub created_at: String,
    pub total_cents: i64,
    pub items_summary: String, // "2× Pizza Margherita (22 cm), 1× Coca Cola"
    /// "live" | "sandbox". Anything ≠ "live" → render as TEST order
    /// in the kitchen board so staff sees at a glance that this
    /// isn't a real customer.
    pub stripe_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AdminOrders {
    pub received: Vec<AdminOrderRow>,
    pub preparing: Vec<AdminOrderRow>,
    pub ready: Vec<AdminOrderRow>,
}

#[server(
    name = ListAdminOrders,
    prefix = "/api",
    endpoint = "list_admin_orders"
)]
pub async fn list_admin_orders() -> Result<AdminOrders, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            i64,
            String,
            String,
            Option<String>,
            String,
        ),
    >(
        "SELECT id, order_number, status, contact_name, contact_phone, scheduled_for, created_at,
                total_cents, payment_status, order_type, delivery_address_json, stripe_mode
         FROM orders
         WHERE status IN ('received', 'preparing', 'ready_for_pickup', 'pending_payment')
         ORDER BY created_at ASC",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load orders: {e}")))?;

    let mut received = Vec::new();
    let mut preparing = Vec::new();
    let mut ready = Vec::new();

    for (
        id,
        num,
        status,
        name,
        phone,
        scheduled,
        created,
        total,
        payment_status,
        order_type,
        addr_json,
        stripe_mode,
    ) in rows
    {
        // Items summary in one query per order — fine for a kitchen with <100 active orders.
        let items: Vec<ItemSummaryRow> = sqlx::query_as(
            "SELECT menu_number_snapshot, name_snapshot, quantity, options_json, extras_json,
                    removals_json
             FROM order_items WHERE order_id = ?1 ORDER BY id",
        )
        .bind(&id)
        .fetch_all(&db)
        .await
        .unwrap_or_default();

        // Show extras (on the pizza) AND removals (off it) on each line so
        // the admin board mirrors what kitchen + driver see.
        let items_summary = items
            .into_iter()
            .map(|(num, n, q, opts, extras_json, removals_json)| {
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
                let removals_label = removals_json
                    .as_deref()
                    .and_then(|j| {
                        serde_json::from_str::<Vec<rusterando_shared::models::CartRemoval>>(j).ok()
                    })
                    .filter(|v| !v.is_empty())
                    .map(|v| {
                        let labels = v
                            .into_iter()
                            .map(|r| r.label)
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!(" ohne {labels}")
                    })
                    .unwrap_or_default();
                if variant.is_empty() {
                    format!("{q}× {prefix}{n}{extras_label}{removals_label}")
                } else {
                    format!("{q}× {prefix}{n} ({variant}){extras_label}{removals_label}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");

        let pickup_label = match scheduled {
            Some(s) => format!("Heute {}", s.split_whitespace().nth(1).unwrap_or(&s)),
            None => {
                if order_type == "delivery" {
                    "Schnellstmöglich".to_string()
                } else {
                    "ASAP".to_string()
                }
            }
        };

        let address_line = if order_type == "delivery" {
            addr_json
                .as_deref()
                .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                .map(|v| {
                    let street = v.get("street").and_then(|x| x.as_str()).unwrap_or("");
                    let house = v.get("house_number").and_then(|x| x.as_str()).unwrap_or("");
                    let plz = v.get("postcode").and_then(|x| x.as_str()).unwrap_or("");
                    let city = v.get("city").and_then(|x| x.as_str()).unwrap_or("");
                    format!("{street} {house}, {plz} {city}")
                })
        } else {
            None
        };

        let row = AdminOrderRow {
            id,
            order_number: num,
            status: status.clone(),
            payment_status,
            order_type,
            address_line,
            contact_name: name,
            contact_phone: phone,
            pickup_label,
            created_at: created,
            total_cents: total,
            items_summary,
            stripe_mode,
        };

        match status.as_str() {
            "received" | "pending_payment" => received.push(row),
            "preparing" => preparing.push(row),
            "ready_for_pickup" => ready.push(row),
            _ => {}
        }
    }

    Ok(AdminOrders {
        received,
        preparing,
        ready,
    })
}

#[server(
    name = GetAdminOrder,
    prefix = "/api",
    endpoint = "get_admin_order"
)]
pub async fn get_admin_order(
    id: String,
) -> Result<crate::pages::order::OrderDetail, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    crate::pages::order::get_order(id).await
}

#[server(
    name = UpdateOrderStatus,
    prefix = "/api",
    endpoint = "update_order_status"
)]
pub async fn update_order_status(id: String, status: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    if !matches!(
        status.as_str(),
        "received" | "preparing" | "ready_for_pickup" | "picked_up" | "cancelled"
    ) {
        return Err(ServerFnError::new("Ungültiger Status"));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    sqlx::query("UPDATE orders SET status = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2")
        .bind(&status)
        .bind(&id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("update status: {e}")))?;

    // Push the new status live (pill flip) + log it to the message list.
    if let Some(hub) = use_context::<crate::live::LiveHub>() {
        hub.send(rusterando_shared::models::LiveEvent {
            order_id: id.clone(),
            kind: rusterando_shared::models::LiveKind::Status(status.clone()),
        });
    }
    crate::pages::order::ssr::log_status_change(&db, &id, &status).await;

    Ok(())
}

/// Reprint an order's kitchen receipt. Fires a `KitchenEvent::Reprint`
/// into the outbox so the Pi prints the same receipt as the original
/// with a "REPRINT" banner on top. Useful after paper jams, dropped
/// receipts, or to hand a duplicate to a delivery driver who lost theirs.
///
/// No-op (Ok(())) on printerless deploys — KitchenSinkHandle is None
/// in context. The button stays visible because the admin UI doesn't
/// know about deploy config, but the server-side guard keeps it safe.
#[server(
    name = ReprintOrder,
    prefix = "/api",
    endpoint = "reprint_order"
)]
pub async fn reprint_order(id: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    // Admin AND kitchen can reprint — kitchen sees the order list on
    // /kitchen and needs to recover from a jammed/lost ticket without
    // pulling the admin in. require_any() lets admins through too.
    crate::pages::session::ssr::require_any(&[
        crate::pages::session::Role::Admin,
        crate::pages::session::Role::Kitchen,
    ])
    .await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let Some(kitchen) = use_context::<crate::pages::push::KitchenSinkHandle>().flatten() else {
        // Printerless deploy. Don't error — the admin doesn't need to
        // know the kitchen subsystem is disabled; the button is
        // harmless and we just nothing-happens.
        return Ok(());
    };

    kitchen
        .broadcast_reprint(&db, &id)
        .await
        .map_err(|e| ServerFnError::new(format!("kitchen reprint failed: {e}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/// `created_at` kommt als UTC-String `'YYYY-MM-DD HH:MM:SS'` aus SQLite
/// (CURRENT_TIMESTAMP). Für die Karte in lokale Zeit umrechnen und kompakt
/// als `TT.MM. HH:MM` anzeigen. chrono läuft mit `wasmbind` auch im Browser,
/// also funktioniert die Local-Konvertierung bei SSR und nach der Hydration.
/// Fällt auf den Rohstring zurück, falls das Parsen scheitert (defensiv —
/// historische Zeilen in abweichendem Format sollen nicht verschwinden).
fn fmt_created_local(utc: &str) -> String {
    use chrono::{Local, NaiveDateTime, TimeZone, Utc};
    match NaiveDateTime::parse_from_str(utc, "%Y-%m-%d %H:%M:%S") {
        Ok(naive) => {
            let local = Utc.from_utc_datetime(&naive).with_timezone(&Local);
            local.format("%d.%m. %H:%M").to_string()
        }
        Err(_) => utc.to_string(),
    }
}

#[component]
pub fn AdminOrdersPage() -> impl IntoView {
    let updater = ServerAction::<UpdateOrderStatus>::new();
    let orders = Resource::new(
        move || updater.version().get(),
        |_| async move { list_admin_orders().await },
    );

    view! {
        <AdminShell>
            <section class="admin-orders">
                <div class="page-bar">
                    <h1>"Bestellungen"</h1>
                    <button class="btn ghost" on:click=move |_| orders.refetch()>
                        "Aktualisieren"
                    </button>
                </div>

                <Suspense fallback=|| view! { <p>"Lädt…"</p> }>
                    {move || orders.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(data) => view! { <Board data updater/> }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn Board(data: AdminOrders, updater: ServerAction<UpdateOrderStatus>) -> impl IntoView {
    view! {
        <div class="orders-board">
            <Column title="Eingegangen" rows=data.received.clone() updater
                next_status="preparing" next_label="Zubereitung starten"/>
            <Column title="In Zubereitung" rows=data.preparing.clone() updater
                next_status="ready_for_pickup" next_label="Abholbereit melden"/>
            <Column title="Abholbereit" rows=data.ready.clone() updater
                next_status="picked_up" next_label="Als abgeholt markieren"/>
        </div>
    }
}

#[component]
fn Column(
    title: &'static str,
    rows: Vec<AdminOrderRow>,
    updater: ServerAction<UpdateOrderStatus>,
    next_status: &'static str,
    next_label: &'static str,
) -> impl IntoView {
    let count = rows.len();

    view! {
        <div class="orders-column">
            <h2>{title} " " <span class="count">"(" {count} ")"</span></h2>
            {if rows.is_empty() {
                view! { <p class="empty">"Keine Bestellungen"</p> }.into_any()
            } else {
                view! {
                    <ul>
                        {rows.into_iter().map(|r| view! {
                            <OrderCard r updater next_status next_label/>
                        }).collect_view()}
                    </ul>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn OrderCard(
    r: AdminOrderRow,
    updater: ServerAction<UpdateOrderStatus>,
    next_status: &'static str,
    next_label: &'static str,
) -> impl IntoView {
    let id_for_advance = r.id.clone();
    let id_for_cancel = r.id.clone();
    let id_for_reprint = r.id.clone();
    let detail_href = format!("/admin/orders/{}", r.id);
    let next = next_status.to_string();
    let on_advance = move |_| {
        updater.dispatch(UpdateOrderStatus {
            id: id_for_advance.clone(),
            status: next.clone(),
        });
    };
    let on_cancel = move |_| {
        updater.dispatch(UpdateOrderStatus {
            id: id_for_cancel.clone(),
            status: "cancelled".into(),
        });
    };
    // Reprint has its own ServerAction so its `pending()` doesn't
    // interfere with status updates' versioning. No need to refetch
    // the orders list after — the row's data is unchanged.
    let reprinter = ServerAction::<ReprintOrder>::new();
    let on_reprint = move |_| {
        reprinter.dispatch(ReprintOrder {
            id: id_for_reprint.clone(),
        });
    };

    let payment_label = crate::pages::order::payment_status_label_de(&r.payment_status);
    let payment_attr = r.payment_status.clone();
    let is_delivery = r.order_type == "delivery";
    let order_type_chip = if is_delivery {
        "🛵 LIEFERUNG"
    } else {
        "🏪 ABHOLUNG"
    };
    let is_test = r.stripe_mode != "live";
    let card_cls = if is_test {
        "order-card test-order"
    } else {
        "order-card"
    };
    let created_label = fmt_created_local(&r.created_at);
    view! {
        <li class=card_cls>
            <div class="order-head">
                <a class="order-num" href=detail_href>{r.order_number}</a>
                <span class="order-time" title="Bestelleingang">{created_label}</span>
                {is_test.then(|| view! {
                    <span class="test-pill" title="Sandbox/Test-Bestellung">"TEST"</span>
                })}
                <span class="status-pill" data-order-type=r.order_type.clone()>{order_type_chip}</span>
                <span class="status-pill" data-status=r.status.clone()>
                    {status_label_de(&r.status)}
                </span>
                <span class="status-pill" data-payment=payment_attr>{payment_label}</span>
            </div>
            <p class="contact">{r.contact_name} " · " {r.contact_phone}</p>
            <p class="pickup">{if is_delivery { "Lieferung: " } else { "Abholung: " }}<strong>{r.pickup_label}</strong></p>
            {r.address_line.map(|a| view! { <p class="address">"📍 " {a}</p> })}
            <p class="items">{r.items_summary}</p>
            <p class="total">{format_eur(r.total_cents)}</p>
            <div class="row">
                <button class="btn primary" on:click=on_advance>{next_label}</button>
                <button class="btn ghost" on:click=on_reprint
                    disabled=move || reprinter.pending().get()>
                    "🖨 Erneut drucken"
                </button>
                <button class="btn ghost" on:click=on_cancel>"Stornieren"</button>
            </div>
        </li>
    }
}

// ---------------------------------------------------------------------------
// /admin/orders/:id  — full detail + printable kitchen ticket
// ---------------------------------------------------------------------------

#[component]
pub fn AdminOrderDetailPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let id = move || params.read().get("id").unwrap_or_default();

    let updater = ServerAction::<UpdateOrderStatus>::new();
    let order = Resource::new(
        move || (id(), updater.version().get()),
        |(id, _)| async move { get_admin_order(id).await },
    );

    view! {
        <AdminShell>
            <section class="admin-order-detail">
                <Suspense fallback=|| view! { <p>"Lädt…"</p> }>
                    {move || order.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(o) => view! { <DetailCard o updater/> }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn DetailCard(
    o: crate::pages::order::OrderDetail,
    updater: ServerAction<UpdateOrderStatus>,
) -> impl IntoView {
    use crate::pages::order::OrderDetail;

    let OrderDetail {
        id,
        order_number,
        status,
        pickup_time_label,
        contact_name,
        contact_phone,
        contact_email,
        items,
        subtotal_cents,
        total_cents,
        created_at,
        payment_status,
        payment_method_detail,
        order_type,
        delivery_fee_cents,
        voucher_code,
        voucher_discount_cents,
        delivery_address,
        messages,
        // Staff (admin/kitchen/driver) can always reply, so the VIP flag
        // isn't needed to gate the box here — bind it out explicitly.
        customer_is_vip: _,
    } = o;

    let status_label = status_label_de(&status).to_string();
    let status_attr = status.clone();
    let payment_label = crate::pages::order::payment_status_label_de(&payment_status);
    let payment_attr = payment_status.clone();
    let is_delivery = order_type == "delivery";

    // Shop name for the printed ticket header.
    #[cfg(feature = "ssr")]
    let shop_name = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get().display_name())
        .unwrap_or_else(|| "Mein Restaurant".to_string());
    #[cfg(not(feature = "ssr"))]
    let shop_name = "Mein Restaurant".to_string();
    let order_type_label = if is_delivery {
        "🛵 LIEFERUNG"
    } else {
        "🏪 ABHOLUNG"
    };
    let next = next_status_for(&status);
    let id_advance = id.clone();
    let id_cancel = id.clone();
    let id_received = id.clone();

    // Personal messages to the customer (append-only log, live via SSE +
    // persisted). The textarea always adds a NEW message; existing ones
    // show above it with per-message delivery state ("✓ Zugestellt" once
    // the customer's browser acks, else "gesendet"). The admin subscribes
    // to the SAME order stream (ack_on_receive=false) so new sends appear
    // AND acks flip live.
    let messenger = ServerAction::<crate::pages::order::SetOrderMessage>::new();
    let msg_text = RwSignal::new(String::new());
    let msg_history = RwSignal::new(messages.clone());
    let (_admin_status, set_admin_status) = signal(None::<String>);
    // Admin side: no customer notifications (notify=false).
    crate::utils::subscribe_order_live(id.clone(), msg_history, set_admin_status, false, false);
    let id_msg = id.clone();
    let on_send_message = move |_| {
        messenger.dispatch(crate::pages::order::SetOrderMessage {
            order_id: id_msg.clone(),
            message: msg_text.get(),
        });
        msg_text.set(String::new());
    };

    let on_advance = move |_| {
        if let Some((next_status, _)) = next {
            updater.dispatch(UpdateOrderStatus {
                id: id_advance.clone(),
                status: next_status.into(),
            });
        }
    };
    let on_cancel = move |_| {
        updater.dispatch(UpdateOrderStatus {
            id: id_cancel.clone(),
            status: "cancelled".into(),
        });
    };
    let on_back_to_received = move |_| {
        updater.dispatch(UpdateOrderStatus {
            id: id_received.clone(),
            status: "received".into(),
        });
    };

    let print = move |_| {
        #[cfg(feature = "hydrate")]
        {
            if let Some(win) = web_sys::window() {
                let _ = win.print();
            }
        }
    };

    view! {
        <header class="detail-bar no-print">
            <a href="/admin/orders" class="link">"← zurück zur Übersicht"</a>
            <h1>"Bestellung " {order_number.clone()}</h1>
            <span class="status-pill" data-status=status_attr.clone()>{status_label.clone()}</span>
            <button class="btn primary" on:click=print>"🖨 Drucken"</button>
        </header>

        <article class="ticket">
            <div class="ticket-head">
                <h2>{shop_name}</h2>
                <p class="ticket-num">{order_number.clone()}</p>
                <p class="ticket-meta">{created_at.clone()}</p>
                <p class="ticket-ribbon" data-order-type=order_type.clone()>{order_type_label}</p>
            </div>

            <div class="ticket-row">
                <span>{if is_delivery { "Lieferung bis" } else { "Abholung" }}</span>
                <strong>{pickup_time_label.clone()}</strong>
            </div>
            <div class="ticket-row">
                <span>"Kunde"</span>
                <strong>{contact_name.clone()}</strong>
            </div>
            <div class="ticket-row">
                <span>"Telefon"</span>
                <strong>{contact_phone.clone()}</strong>
            </div>
            <div class="ticket-row">
                <span>"E-Mail"</span>
                <span class="muted">{contact_email.clone()}</span>
            </div>
            {delivery_address.as_ref().map(|a| {
                let line = format!("{}, {} {}", a.street_with_number(), a.postcode, a.city);
                let zone = a.zone_name.clone();
                let notes = a.notes.clone();
                view! {
                    <div class="ticket-row">
                        <span>"Liefergebiet"</span>
                        <strong>{zone}</strong>
                    </div>
                    <div class="ticket-row">
                        <span>"Adresse"</span>
                        <strong>{line}</strong>
                    </div>
                    {notes.map(|n| view! {
                        <div class="ticket-row">
                            <span>"Hinweis"</span>
                            <span class="muted">{n}</span>
                        </div>
                    })}
                }
            })}
            <div class="ticket-row">
                <span>"Zahlung"</span>
                <span>
                    <span class="status-pill" data-payment=payment_attr.clone()>{payment_label.to_string()}</span>
                    {payment_method_detail.clone().filter(|s| !s.is_empty()).map(|m| view! {
                        <span class="muted">" · " {m}</span>
                    })}
                </span>
            </div>

            <h3 class="ticket-h">"Bestellung"</h3>
            <table class="ticket-items">
                <tbody>
                    {items.into_iter().map(|it| {
                        let extras = it.extras.clone();
                        let removals = it.removals.clone();
                        view! {
                            <tr>
                                <td class="qty">{it.quantity} "×"</td>
                                <td class="num">
                                    {it.menu_number.clone().unwrap_or_default()}
                                </td>
                                <td class="name">
                                    <strong>{it.name}</strong>
                                    {it.variant.map(|v| view! { <span class="variant">" — " {v}</span> })}
                                    {(!extras.is_empty()).then(|| view! {
                                        <ul class="extras">
                                            {extras.into_iter().map(|e| {
                                                let p = if e.price_cents == 0 {
                                                    "gratis".to_string()
                                                } else {
                                                    format!("+{}", format_eur(e.price_cents))
                                                };
                                                view! { <li>"+ " {e.label} " " <span class="muted">{p}</span></li> }
                                            }).collect_view()}
                                        </ul>
                                    })}
                                    {(!removals.is_empty()).then(|| view! {
                                        <ul class="extras removals">
                                            {removals.into_iter().map(|r| {
                                                view! { <li>"ohne " {r.label}</li> }
                                            }).collect_view()}
                                        </ul>
                                    })}
                                </td>
                                <td class="amt">{format_eur(it.line_total_cents)}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
                <tfoot>
                    {(delivery_fee_cents > 0 || voucher_discount_cents > 0).then(|| view! {
                        <tr>
                            <td></td>
                            <td class="muted">"Zwischensumme"</td>
                            <td class="amt">{format_eur(subtotal_cents)}</td>
                        </tr>
                    })}
                    {(delivery_fee_cents > 0).then(|| view! {
                        <tr>
                            <td></td>
                            <td class="muted">"Lieferzuschlag"</td>
                            <td class="amt">{format_eur(delivery_fee_cents)}</td>
                        </tr>
                    })}
                    {(voucher_discount_cents > 0).then(|| {
                        let label = if voucher_code.is_empty() {
                            "Gutschein".to_string()
                        } else {
                            format!("Gutschein {voucher_code}")
                        };
                        view! {
                            <tr>
                                <td></td>
                                <td class="muted">{label}</td>
                                <td class="amt">"−" {format_eur(voucher_discount_cents)}</td>
                            </tr>
                        }
                    })}
                    <tr>
                        <td></td>
                        <td class="grand">"Gesamt"</td>
                        <td class="grand amt">{format_eur(total_cents)}</td>
                    </tr>
                </tfoot>
            </table>

            <p class="ticket-foot">{
                match (payment_status.as_str(), is_delivery) {
                    ("paid", _)              => "Online bezahlt — keine Kasse nötig.",
                    ("pending", _)           => "Zahlung ausstehend — bitte erst nach Bestätigung ausgeben.",
                    ("failed", _)            => "Zahlung fehlgeschlagen.",
                    ("refunded", _)          => "Erstattet.",
                    ("cash_on_pickup", true) => "Zahlung: Bar bei Lieferung an den Fahrer.",
                    ("cash_on_pickup", false) => "Zahlung: Bar bei Abholung",
                    _                        => "Zahlung: ?",
                }
            }</p>
        </article>

        <footer class="detail-actions no-print">
            {match next {
                Some((next_status, label)) => view! {
                    <button class="btn primary" on:click=on_advance
                            data-next=next_status>{label}</button>
                }.into_any(),
                None => view! { <p class="muted">"Keine weitere Aktion möglich."</p> }.into_any(),
            }}
            <button class="btn ghost" on:click=on_back_to_received>"Status zurücksetzen"</button>
            <button class="btn ghost danger" on:click=on_cancel>"Stornieren"</button>
        </footer>

        <div class="customer-message no-print">
            <h3>"Nachrichten an Kund:in"</h3>
            {move || {
                let msgs = msg_history.get();
                (!msgs.is_empty()).then(|| view! {
                    <ul class="message-log">
                        {msgs.into_iter().map(|m| {
                            let from_customer = m.is_customer();
                            let li_cls = if from_customer { "from-customer" } else { "from-staff" };
                            let who = m.sender_label_de();
                            // Delivery ack only applies to staff→customer
                            // messages; an inbound customer reply has no ack.
                            let ack_line = (!from_customer).then(|| {
                                let ack = if m.delivered { "✓ Zugestellt" } else { "gesendet" };
                                let ack_cls = if m.delivered { "ack delivered" } else { "ack pending" };
                                view! { <span class=ack_cls>" · " {ack}</span> }
                            });
                            view! {
                                <li class=li_cls>
                                    <span class="ts">{who} " · 🕒 " {m.created_at} " — "</span>
                                    {m.body}
                                    {ack_line}
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                })
            }}
            <p class="hint">"Erscheint sofort live auf der Bestellseite der Kund:in (und bleibt nach dem Neuladen). \"✓ Zugestellt\" = im Browser der Kund:in angezeigt. VIP-Kund:innen können hier zurückschreiben (🙋 Kunde)."</p>
            <textarea rows="2" maxlength="500" placeholder="z. B. Deine Bestellung braucht 10 Min länger — danke für die Geduld!"
                prop:value=move || msg_text.get()
                on:input=move |ev| msg_text.set(event_target_value(&ev))></textarea>
            <div class="row">
                <button class="btn primary" on:click=on_send_message>"Nachricht senden"</button>
                {move || messenger.value().get().map(|res| match res {
                    Ok(_) => view! { <span class="ok">"✓ Gespeichert"</span> }.into_any(),
                    Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                })}
            </div>
        </div>
    }
}

/// What the next status transition is, given the current one. Mirrors the
/// kitchen board's column flow.
fn next_status_for(current: &str) -> Option<(&'static str, &'static str)> {
    match current {
        "received" => Some(("preparing", "Zubereitung starten")),
        "preparing" => Some(("ready_for_pickup", "Abholbereit melden")),
        "ready_for_pickup" => Some(("picked_up", "Als abgeholt markieren")),
        _ => None,
    }
}
