//! Kitchen board: a focused, reduced-permission view of the active orders.
//! Cooks can advance orders received → preparing → ready_for_pickup but cannot
//! cancel, refund, or see revenue.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

use crate::pages::order::status_label_de;

/// (menu_number_snapshot, name_snapshot, quantity, options_json, extras_json)
#[cfg(feature = "ssr")]
type ItemSummaryRow = (Option<String>, String, i64, String, Option<String>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KitchenOrderRow {
    pub id: String,
    pub order_number: String,
    pub status: String,
    pub order_type: String, // "pickup" | "delivery"
    pub contact_name: String,
    pub contact_phone: String,
    pub pickup_label: String,
    pub created_at: String,
    pub items_summary: String,
    pub address_line: Option<String>, // pre-formatted "Street 12, 59348 Lüdinghausen" for delivery
    pub address_notes: Option<String>,
    pub delivery_zone: Option<String>,
    /// Total — the kitchen sees this only because the printable ticket carries
    /// it; cash/card distinction is muted. Revenue/aggregates are admin-only.
    pub total_cents: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KitchenOrders {
    pub received: Vec<KitchenOrderRow>,
    pub preparing: Vec<KitchenOrderRow>,
    pub ready: Vec<KitchenOrderRow>,
}

#[server(
    name = ListKitchenOrders,
    prefix = "/api",
    endpoint = "list_kitchen_orders"
)]
pub async fn list_kitchen_orders() -> Result<KitchenOrders, ServerFnError> {
    use crate::pages::session::{ssr::require_any, Role};
    use sqlx::SqlitePool;

    require_any(&[Role::Kitchen]).await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows = sqlx::query_as::<
        _,
        (
            String,         // id
            String,         // order_number
            String,         // status
            String,         // order_type
            String,         // contact_name
            String,         // contact_phone
            Option<String>, // scheduled_for
            String,         // created_at
            i64,            // total_cents
            Option<String>, // delivery_address_json
        ),
    >(
        "SELECT id, order_number, status, order_type, contact_name, contact_phone,
                scheduled_for, created_at, total_cents, delivery_address_json
         FROM orders
         WHERE status IN ('received', 'preparing', 'ready_for_pickup')
         ORDER BY created_at ASC",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load orders: {e}")))?;

    let mut received = Vec::new();
    let mut preparing = Vec::new();
    let mut ready = Vec::new();

    for (id, num, status, order_type, name, phone, scheduled, created, total, addr_json) in rows {
        let items: Vec<ItemSummaryRow> = sqlx::query_as(
            "SELECT menu_number_snapshot, name_snapshot, quantity, options_json, extras_json
             FROM order_items
             WHERE order_id = ?1 ORDER BY id",
        )
        .bind(&id)
        .fetch_all(&db)
        .await
        .unwrap_or_default();

        // Kitchen reads the menu number first ("22a Pizza Sucuk" / "51 Pizza
        // Inferno") because that's how the paper menu is keyed and how staff
        // call orders out to each other. Extras append after the item ("…
        // + Extra Käse, Tabasco") so the kitchen knows which toppings to add.
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

        let pickup_label = match scheduled {
            Some(s) => format!("Heute {}", s.split_whitespace().nth(1).unwrap_or(&s)),
            None => "ASAP".to_string(),
        };

        let (address_line, address_notes, zone) = if order_type == "delivery" {
            addr_json
                .as_deref()
                .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                .map(|v| {
                    let street = v.get("street").and_then(|x| x.as_str()).unwrap_or("");
                    let house = v.get("house_number").and_then(|x| x.as_str()).unwrap_or("");
                    let plz = v.get("postcode").and_then(|x| x.as_str()).unwrap_or("");
                    let city = v.get("city").and_then(|x| x.as_str()).unwrap_or("");
                    let zone = v
                        .get("zone_name")
                        .and_then(|x| x.as_str())
                        .map(String::from);
                    let notes = v
                        .get("notes")
                        .and_then(|x| x.as_str())
                        .filter(|s| !s.is_empty())
                        .map(String::from);
                    (Some(format!("{street} {house}, {plz} {city}")), notes, zone)
                })
                .unwrap_or((None, None, None))
        } else {
            (None, None, None)
        };

        let row = KitchenOrderRow {
            id,
            order_number: num,
            status: status.clone(),
            order_type,
            contact_name: name,
            contact_phone: phone,
            pickup_label,
            created_at: created,
            items_summary,
            address_line,
            address_notes,
            delivery_zone: zone,
            total_cents: total,
        };
        match status.as_str() {
            "received" => received.push(row),
            "preparing" => preparing.push(row),
            "ready_for_pickup" => ready.push(row),
            _ => {}
        }
    }

    Ok(KitchenOrders {
        received,
        preparing,
        ready,
    })
}

#[server(
    name = KitchenAdvance,
    prefix = "/api",
    endpoint = "kitchen_advance"
)]
pub async fn kitchen_advance(id: String, status: String) -> Result<(), ServerFnError> {
    use crate::pages::session::{ssr::require_any, Role};
    use sqlx::SqlitePool;

    require_any(&[Role::Kitchen]).await?;

    // Only the three forward-going states are allowed from the kitchen role.
    // `cancelled` and arbitrary jumps are admin/driver territory.
    if !matches!(
        status.as_str(),
        "preparing" | "ready_for_pickup" | "received"
    ) {
        return Err(ServerFnError::new("Status nicht erlaubt für Küche"));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query("UPDATE orders SET status = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2")
        .bind(&status)
        .bind(&id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("update status: {e}")))?;
    // Live pill flip + timestamped entry in the customer's message log.
    if let Some(hub) = use_context::<crate::live::LiveHub>() {
        hub.send(rusterando_shared::models::LiveEvent {
            order_id: id.clone(),
            kind: rusterando_shared::models::LiveKind::Status(status.clone()),
        });
    }
    crate::pages::order::ssr::log_status_change(&db, &id, &status).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

#[component]
pub fn KitchenBoardPage() -> impl IntoView {
    // Gate: redirect to /kitchen/login if no session. Admin passes through.
    let _gate = Resource::new(
        || (),
        |_| async move {
            crate::pages::session::require_role_or_redirect(
                "kitchen".into(),
                "/kitchen/login".into(),
            )
            .await
        },
    );

    let advance = ServerAction::<KitchenAdvance>::new();
    // Reuse the same server fn admin uses for reprint — its auth was
    // relaxed to accept the kitchen role too, so this call lands in
    // the kitchen-printer outbox identically.
    let reprinter = ServerAction::<crate::pages::admin::orders::ReprintOrder>::new();
    let orders = Resource::new(
        move || advance.version().get(),
        |_| async move { list_kitchen_orders().await },
    );
    let logout = ServerAction::<crate::pages::session::SessionLogout>::new();

    view! {
        <div class="kitchen-shell">
            <header class="kitchen-bar">
                <span class="brand">"🍳 Küche"</span>
                <crate::pages::session::RoleSwitcher current=crate::pages::session::Role::Kitchen/>
                <button class="btn ghost" on:click=move |_| orders.refetch()>"Aktualisieren"</button>
                <button class="logout" on:click=move |_| { logout.dispatch(crate::pages::session::SessionLogout {}); }>"Abmelden"</button>
            </header>
            <main class="orders-page">
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || orders.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(data) => view! { <Board data advance reprinter/> }.into_any(),
                    })}
                </Suspense>
            </main>
        </div>
    }
}

#[component]
fn Board(
    data: KitchenOrders,
    advance: ServerAction<KitchenAdvance>,
    reprinter: ServerAction<crate::pages::admin::orders::ReprintOrder>,
) -> impl IntoView {
    view! {
        <div class="orders-board">
            <Column title="Eingegangen" rows=data.received.clone() advance reprinter
                next_status="preparing" next_label="Zubereitung starten"/>
            <Column title="In Zubereitung" rows=data.preparing.clone() advance reprinter
                next_status="ready_for_pickup" next_label="Abholbereit / Lieferbereit"/>
            <Column title="Fertig" rows=data.ready.clone() advance reprinter
                next_status="received" next_label=""/>
        </div>
    }
}

#[component]
fn Column(
    title: &'static str,
    rows: Vec<KitchenOrderRow>,
    advance: ServerAction<KitchenAdvance>,
    reprinter: ServerAction<crate::pages::admin::orders::ReprintOrder>,
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
                            <Card r advance reprinter next_status next_label/>
                        }).collect_view()}
                    </ul>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn Card(
    r: KitchenOrderRow,
    advance: ServerAction<KitchenAdvance>,
    reprinter: ServerAction<crate::pages::admin::orders::ReprintOrder>,
    next_status: &'static str,
    next_label: &'static str,
) -> impl IntoView {
    let id_for_advance = r.id.clone();
    let next = next_status.to_string();
    let on_advance = move |_| {
        advance.dispatch(KitchenAdvance {
            id: id_for_advance.clone(),
            status: next.clone(),
        });
    };
    let id_for_reprint = r.id.clone();
    let on_reprint = move |_| {
        reprinter.dispatch(crate::pages::admin::orders::ReprintOrder {
            id: id_for_reprint.clone(),
        });
    };
    let is_delivery = r.order_type == "delivery";
    let order_type_chip = if is_delivery {
        "🛵 LIEFERUNG"
    } else {
        "🏪 ABHOLUNG"
    };

    view! {
        <li class="order-card">
            <div class="order-head">
                <strong class="order-num">{r.order_number}</strong>
                <span class="status-pill" data-order-type=r.order_type.clone()>{order_type_chip}</span>
                <span class="status-pill" data-status=r.status.clone()>
                    {status_label_de(&r.status)}
                </span>
            </div>
            <p class="contact">{r.contact_name} " · " {r.contact_phone}</p>
            <p class="pickup">{if is_delivery { "Lieferung: " } else { "Abholung: " }}<strong>{r.pickup_label}</strong></p>
            {r.address_line.map(|line| view! {
                <p class="address">"📍 " {line}</p>
            })}
            {r.delivery_zone.map(|z| view! {
                <p class="muted small">{format!("Liefergebiet: {z}")}</p>
            })}
            {r.address_notes.map(|n| view! {
                <p class="muted small">{format!("Hinweis: {n}")}</p>
            })}
            <p class="items">{r.items_summary}</p>
            <p class="total">{format_eur(r.total_cents)}</p>
            <div class="row">
                {if next_label.is_empty() {
                    view! { <span class="muted">"Wartet auf Abholung / Fahrer."</span> }.into_any()
                } else {
                    view! { <button class="btn primary" on:click=on_advance>{next_label}</button> }.into_any()
                }}
                <button class="btn ghost" on:click=on_reprint
                    disabled=move || reprinter.pending().get()>
                    "🖨 Erneut drucken"
                </button>
            </div>
        </li>
    }
}
