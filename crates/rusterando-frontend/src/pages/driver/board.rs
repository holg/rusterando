//! Driver board: only delivery orders that are ready or already on the road.
//!
//! Phase 3 adds optimised tours:
//!   - "Abholbereit" column has a checkbox per card; the driver picks which
//!     orders to bundle and clicks "Tour starten" → server calls ORS, returns
//!     an optimised sequence, orders flip to `out_for_delivery`.
//!   - "Unterwegs" splits per-tour. Each tour shows its stops in route order
//!     with sequence numbers, ETAs (when ORS is available), Maps deep-link
//!     to the next stop, cash totals, and a "Tour beenden" button when all
//!     stops are delivered.
//!
//! Orders that ended up in `out_for_delivery` without a tour (legacy state)
//! still appear in their own free-floating column with the old per-card
//! "Geliefert" flow — no data is left behind.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

use crate::pages::order::{
    status_label_de, CancelTour, FinishTour, RemoveTourStop, StartTour, TourStopDelivered,
    TourStopView, TourSummary,
};

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

/// Build map deep-links for one stop. Returns (apple_maps_url, google_maps_url).
///
/// We use **app-only** URL schemes so the driver's phone opens the
/// navigation app directly instead of a browser tab:
///   - `maps://` — Apple Maps app on iPhone (the OS reserves this scheme)
///   - `google.navigation:` — Google Maps Navigation intent on Android
///
/// Both ignore `target="_blank"` and are unsafe on desktop, but for the
/// driver's phone (the only realistic context for /driver) they're exactly
/// what we want: tap → app opens with directions queued.
///
/// Coordinates are preferred when we have them — no on-device geocode is
/// needed and the pin lands on the actual house. Falls back to the address
/// string otherwise.
fn map_links(address_line: &str, lat: Option<f64>, lon: Option<f64>) -> (String, String) {
    match (lat, lon) {
        (Some(lat), Some(lon)) => (
            // Apple Maps: maps://?daddr=<lat>,<lon>&dirflg=d (driving directions)
            format!("maps://?daddr={lat},{lon}&dirflg=d"),
            // Google Maps Navigation intent. `mode=d` = driving.
            format!("google.navigation:q={lat},{lon}&mode=d"),
        ),
        _ => {
            let q = urlencoding::encode(address_line).to_string();
            (
                format!("maps://?daddr={q}&dirflg=d"),
                format!("google.navigation:q={q}&mode=d"),
            )
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverOrderRow {
    pub id: String,
    pub order_number: String,
    pub status: String,
    pub contact_name: String,
    pub contact_phone: String,
    pub pickup_label: String,
    pub items_summary: String,
    pub address_line: String,
    pub address_notes: Option<String>,
    pub zone: String,
    pub total_cents: i64,
    pub payment_status: String, // 'cash_on_pickup' / 'paid' so the driver knows what to collect
    /// Set when the order is part of an active (not-yet-finished) tour.
    pub tour_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DriverOrders {
    /// `ready_for_pickup`, delivery, no tour yet — candidates for "Tour starten".
    pub ready: Vec<DriverOrderRow>,
    /// `out_for_delivery` orders that aren't on a tour (legacy / single-stop
    /// flow). Each shows the old "Geliefert" button.
    pub on_road_loose: Vec<DriverOrderRow>,
    /// Active tours with their stops in delivery order.
    pub active_tours: Vec<TourSummary>,
}

#[server(
    name = ListDriverOrders,
    prefix = "/api",
    endpoint = "list_driver_orders"
)]
pub async fn list_driver_orders() -> Result<DriverOrders, ServerFnError> {
    use crate::pages::session::{ssr::require_any, Role};
    use sqlx::SqlitePool;

    require_any(&[Role::Driver]).await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows = sqlx::query_as::<
        _,
        (
            String,         // id
            String,         // order_number
            String,         // status
            String,         // contact_name
            String,         // contact_phone
            Option<String>, // scheduled_for
            i64,            // total_cents
            String,         // payment_status
            Option<String>, // delivery_address_json
            Option<String>, // tour_id
        ),
    >(
        "SELECT id, order_number, status, contact_name, contact_phone,
                scheduled_for, total_cents, payment_status, delivery_address_json,
                tour_id
         FROM orders
         WHERE order_type = 'delivery'
           AND status IN ('ready_for_pickup', 'out_for_delivery')
         ORDER BY created_at ASC",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load orders: {e}")))?;

    let mut ready: Vec<DriverOrderRow> = Vec::new();
    let mut on_road_loose: Vec<DriverOrderRow> = Vec::new();

    for (id, num, status, name, phone, scheduled, total, payment_status, addr_json, tour_id) in rows
    {
        let items: Vec<ItemSummaryRow> = sqlx::query_as(
            "SELECT menu_number_snapshot, name_snapshot, quantity, options_json, extras_json,
                    removals_json
             FROM order_items
             WHERE order_id = ?1 ORDER BY id",
        )
        .bind(&id)
        .fetch_all(&db)
        .await
        .unwrap_or_default();

        // The driver needs to read off exactly what's ON the pizza (extras)
        // and what's OFF it (removals) at the door, so both append to the
        // item line: "… + Extra Käse ohne Pilze".
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
            None => "Schnellstmöglich".to_string(),
        };

        let (address_line, address_notes, zone) = addr_json
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
                    .unwrap_or("?")
                    .to_string();
                let notes = v
                    .get("notes")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                (format!("{street} {house}, {plz} {city}"), notes, zone)
            })
            .unwrap_or_else(|| ("(keine Adresse)".to_string(), None, "?".to_string()));

        let row = DriverOrderRow {
            id,
            order_number: num,
            status: status.clone(),
            contact_name: name,
            contact_phone: phone,
            pickup_label,
            items_summary,
            address_line,
            address_notes,
            zone,
            total_cents: total,
            payment_status,
            tour_id: tour_id.clone(),
        };
        match (status.as_str(), tour_id.is_some()) {
            ("ready_for_pickup", _) => ready.push(row),
            ("out_for_delivery", false) => on_road_loose.push(row),
            ("out_for_delivery", true) => { /* will surface via active_tours below */ }
            _ => {}
        }
    }

    // Active tours: anything without finished_at where at least one stop is
    // still pending. We load via the same get_tour SSR-side function so the
    // shapes line up.
    let tour_ids: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM delivery_tours WHERE finished_at IS NULL
         ORDER BY started_at ASC",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("active tours: {e}")))?;

    let mut active_tours = Vec::with_capacity(tour_ids.len());
    for (tid,) in tour_ids {
        if let Ok(t) = crate::pages::order::get_tour(tid).await {
            active_tours.push(t);
        }
    }

    Ok(DriverOrders {
        ready,
        on_road_loose,
        active_tours,
    })
}

#[server(
    name = DriverAdvance,
    prefix = "/api",
    endpoint = "driver_advance"
)]
pub async fn driver_advance(id: String, status: String) -> Result<(), ServerFnError> {
    use crate::pages::session::{ssr::require_any, Role};
    use sqlx::SqlitePool;

    require_any(&[Role::Driver]).await?;

    if !matches!(status.as_str(), "out_for_delivery" | "delivered") {
        return Err(ServerFnError::new("Status nicht erlaubt für Fahrer"));
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
pub fn DriverBoardPage() -> impl IntoView {
    // Gate: if no session, server emits a redirect to /driver/login during
    // SSR so the browser lands there instead of seeing an "nicht angemeldet"
    // error. Admin sessions pass through (they see everything).
    let _gate = Resource::new(
        || (),
        |_| async move {
            crate::pages::session::require_role_or_redirect("driver".into(), "/driver/login".into())
                .await
        },
    );

    let advance = ServerAction::<DriverAdvance>::new();
    let starter = ServerAction::<StartTour>::new();
    let stop_delivered = ServerAction::<TourStopDelivered>::new();
    let tour_finisher = ServerAction::<FinishTour>::new();
    let stop_remover = ServerAction::<RemoveTourStop>::new();
    let tour_canceller = ServerAction::<CancelTour>::new();

    // Selection set for "Tour starten" — order_ids the driver has ticked.
    let selected: RwSignal<Vec<String>> = RwSignal::new(Vec::new());
    let toggle_selected = move |id: String, on: bool| {
        selected.update(|v| {
            v.retain(|x| x != &id);
            if on {
                v.push(id);
            }
        });
    };

    let orders = Resource::new(
        move || {
            (
                advance.version().get(),
                starter.version().get(),
                stop_delivered.version().get(),
                tour_finisher.version().get(),
                stop_remover.version().get(),
                tour_canceller.version().get(),
            )
        },
        |_| async move { list_driver_orders().await },
    );
    let logout = ServerAction::<crate::pages::session::SessionLogout>::new();

    let on_start_tour = move |_| {
        let ids = selected.get_untracked();
        if ids.is_empty() {
            return;
        }
        starter.dispatch(StartTour {
            order_ids: ids,
            driver_name: None,
        });
        selected.set(Vec::new());
    };

    view! {
        <div class="driver-shell">
            <header class="driver-bar">
                <span class="brand">"🛵 Fahrer"</span>
                <crate::pages::session::RoleSwitcher current=crate::pages::session::Role::Driver/>
                <button class="btn ghost" on:click=move |_| orders.refetch()>"Aktualisieren"</button>
                <button class="logout" on:click=move |_| { logout.dispatch(crate::pages::session::SessionLogout {}); }>"Abmelden"</button>
            </header>
            <main class="orders-page">
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || orders.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(d)  => view! {
                            <Stacks
                                d
                                advance
                                starter
                                stop_delivered
                                tour_finisher
                                stop_remover
                                tour_canceller
                                selected
                                on_start_tour=Callback::new(on_start_tour)
                                toggle_selected=Callback::new(move |(id, on): (String, bool)| toggle_selected(id, on))
                            />
                        }.into_any(),
                    })}
                </Suspense>
                {move || match starter.value().get() {
                    Some(Err(e)) => Some(view! { <p class="error">{format!("Tour-Fehler: {e}")}</p> }.into_any()),
                    _ => None,
                }}
            </main>
        </div>
    }
}

#[component]
fn Stacks(
    d: DriverOrders,
    advance: ServerAction<DriverAdvance>,
    starter: ServerAction<StartTour>,
    stop_delivered: ServerAction<TourStopDelivered>,
    tour_finisher: ServerAction<FinishTour>,
    stop_remover: ServerAction<RemoveTourStop>,
    tour_canceller: ServerAction<CancelTour>,
    selected: RwSignal<Vec<String>>,
    on_start_tour: Callback<()>,
    toggle_selected: Callback<(String, bool)>,
) -> impl IntoView {
    let DriverOrders {
        ready,
        on_road_loose,
        active_tours,
    } = d;
    let ready_sv = StoredValue::new(ready);
    let loose_sv = StoredValue::new(on_road_loose);
    let tours_sv = StoredValue::new(active_tours);

    view! {
        <div class="orders-board driver-board">
            <ReadyColumn
                rows_sv=ready_sv
                selected
                toggle_selected
                on_start_tour
                pending=starter.pending().into()
            />
            <ToursColumn
                tours_sv
                stop_delivered
                tour_finisher
                stop_remover
                tour_canceller
            />
            <LooseColumn rows_sv=loose_sv advance/>
        </div>
    }
}

#[component]
fn ReadyColumn(
    rows_sv: StoredValue<Vec<DriverOrderRow>>,
    selected: RwSignal<Vec<String>>,
    toggle_selected: Callback<(String, bool)>,
    on_start_tour: Callback<()>,
    pending: leptos::prelude::Signal<bool>,
) -> impl IntoView {
    let count = rows_sv.with_value(|v| v.len());
    let any_selected = move || !selected.get().is_empty();
    view! {
        <div class="orders-column ready-column">
            <h2>"Abholbereit " <span class="count">"(" {count} ")"</span></h2>
            <div class="tour-controls">
                <button class="btn primary"
                    disabled=move || pending.get() || !any_selected()
                    on:click=move |_| on_start_tour.run(())>
                    {move || if pending.get() { "Tour wird optimiert…".to_string() }
                             else { format!("Tour starten ({})", selected.get().len()) }}
                </button>
                <span class="hint">"Adressen anhaken, dann auf 'Tour starten'."</span>
            </div>
            {if count == 0 {
                view! { <p class="empty">"Keine Bestellungen"</p> }.into_any()
            } else {
                view! {
                    <ul>
                        {rows_sv.with_value(|rows| rows.clone().into_iter().map(|r| view! {
                            <ReadyCard r selected toggle_selected/>
                        }).collect_view())}
                    </ul>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn ReadyCard(
    r: DriverOrderRow,
    selected: RwSignal<Vec<String>>,
    toggle_selected: Callback<(String, bool)>,
) -> impl IntoView {
    let id_for_check = r.id.clone();
    let id_for_track = r.id.clone();
    let is_checked = Memo::new(move |_| selected.get().contains(&id_for_track));
    let chat_open = RwSignal::new(false);
    let chat_id = r.id.clone();
    let (apple_url, google_url) = map_links(&r.address_line, None, None);
    let tel_url = format!("tel:{}", r.contact_phone);
    let needs_cash = r.payment_status == "cash_on_pickup";
    let cash_label = if needs_cash {
        format!("BAR: {}", format_eur(r.total_cents))
    } else if r.payment_status == "voucher_paid" {
        "Mit Gutschein bezahlt".to_string()
    } else {
        "Online bezahlt".to_string()
    };

    // Full admin-orders-style card. No more <details> collapsing —
    // everything the driver needs to make a routing decision (items,
    // cash, address, notes) is visible at a glance. Tour selection
    // checkbox sits at the right edge of the head row instead of
    // wrapping the order number.
    view! {
        <li class="order-card driver-card" class:selected=is_checked>
            <div class="order-head">
                <strong class="order-num">{r.order_number}</strong>
                <span class="status-pill" data-status=r.status.clone()>
                    {status_label_de(&r.status)}
                </span>
                <label class="select-tour-corner" title="Für Tour auswählen">
                    <input type="checkbox"
                        prop:checked=move || is_checked.get()
                        on:change=move |ev| {
                            toggle_selected.run((id_for_check.clone(), event_target_checked(&ev)));
                        }/>
                </label>
            </div>
            <p class="contact">
                {r.contact_name.clone()} " · "
                <a href=tel_url.clone()>{r.contact_phone.clone()}</a>
            </p>
            <p class="address">"📍 " {r.address_line.clone()}</p>
            <p class="muted small">{format!("Liefergebiet: {}", r.zone)}</p>
            {r.address_notes.clone().map(|n| view! {
                <p class="muted small">{format!("Hinweis: {n}")}</p>
            })}
            <p class="items">{r.items_summary}</p>
            <p class={if needs_cash { "cash" } else { "total" }}>{cash_label}</p>
            <div class="row">
                <a class="btn ghost" href=apple_url>"🗺 Apple"</a>
                <a class="btn ghost" href=google_url>"🗺 Google"</a>
                <button class="btn ghost" on:click=move |_| chat_open.update(|o| *o = !*o)>
                    "💬 Chat"
                </button>
            </div>
            {move || chat_open.get().then({
                let chat_id = chat_id.clone();
                move || view! {
                    <crate::pages::order::StaffOrderChat order_id=chat_id.clone() compact=true/>
                }
            })}
        </li>
    }
}

#[component]
fn ToursColumn(
    tours_sv: StoredValue<Vec<TourSummary>>,
    stop_delivered: ServerAction<TourStopDelivered>,
    tour_finisher: ServerAction<FinishTour>,
    stop_remover: ServerAction<RemoveTourStop>,
    tour_canceller: ServerAction<CancelTour>,
) -> impl IntoView {
    let count = tours_sv.with_value(|v| v.len());
    view! {
        <div class="orders-column tours-column">
            <h2>"Touren " <span class="count">"(" {count} ")"</span></h2>
            {if count == 0 {
                view! { <p class="empty">"Keine aktive Tour"</p> }.into_any()
            } else {
                view! {
                    <ul class="tours-list">
                        {tours_sv.with_value(|tours| tours.clone().into_iter().map(|t| view! {
                            <TourCard t stop_delivered tour_finisher stop_remover tour_canceller/>
                        }).collect_view())}
                    </ul>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn TourCard(
    t: TourSummary,
    stop_delivered: ServerAction<TourStopDelivered>,
    tour_finisher: ServerAction<FinishTour>,
    stop_remover: ServerAction<RemoveTourStop>,
    tour_canceller: ServerAction<CancelTour>,
) -> impl IntoView {
    let id = t.id.clone();
    let id_for_finish = t.id.clone();
    let id_for_cancel = t.id.clone();
    let optimised_label = match t.optimised_by.as_str() {
        "ors" => "Route optimiert",
        _ => "Reihenfolge nach Eingang",
    };
    let total_dist_km = t
        .total_distance_m
        .map(|m| format!("{:.1} km", m as f64 / 1000.0));
    let total_dur_min = t
        .total_duration_s
        .map(|s| format!("{:.0} Min.", s as f64 / 60.0));

    let cash_total: i64 = t
        .stops
        .iter()
        .filter(|s| s.payment_status == "cash_on_pickup" && s.delivered_at.is_none())
        .map(|s| s.total_cents)
        .sum();

    let all_delivered = t.stops.iter().all(|s| s.delivered_at.is_some());
    let any_delivered = t.stops.iter().any(|s| s.delivered_at.is_some());
    let stops_sv = StoredValue::new(t.stops);
    let id_for_card = id.clone();

    view! {
        <li class="tour-card">
            <header class="tour-head">
                <strong>"Tour #" {id_for_card.chars().take(8).collect::<String>()}</strong>
                <span class="muted">{optimised_label}</span>
                {total_dist_km.map(|d| view! { <span class="muted">" · " {d}</span> })}
                {total_dur_min.map(|m| view! { <span class="muted">" · " {m}</span> })}
            </header>
            {(cash_total > 0).then(|| view! {
                <p class="cash big">{format!("Bar einsammeln offen: {}", format_eur(cash_total))}</p>
            })}
            <ol class="tour-stops">
                {stops_sv.with_value(|stops| stops.clone().into_iter().map(|s| {
                    let tour_id = id.clone();
                    view! { <TourStopRow s tour_id stop_delivered stop_remover/> }
                }).collect_view())}
            </ol>
            <div class="row">
                <button class="btn ghost"
                    disabled=move || !all_delivered
                    on:click=move |_| {
                        tour_finisher.dispatch(FinishTour { tour_id: id_for_finish.clone() });
                    }>
                    {if all_delivered { "Tour beenden" } else { "Tour beenden (offene Stops)" }}
                </button>
                <button class="btn ghost danger"
                    on:click=move |_| {
                        let confirm_msg = if any_delivered {
                            "Tour abbrechen? Offene Stops gehen zurück nach 'Abholbereit'. Bereits gelieferte bleiben gespeichert."
                        } else {
                            "Tour abbrechen? Alle Stops gehen zurück nach 'Abholbereit'."
                        };
                        if confirm_dialog(confirm_msg) {
                            tour_canceller.dispatch(CancelTour { tour_id: id_for_cancel.clone() });
                        }
                    }>
                    "Tour abbrechen"
                </button>
            </div>
        </li>
    }
}

/// Confirm prompt that's a no-op on SSR (returns true) and uses
/// `window.confirm` in the browser. Kept tiny so it's used inline above.
fn confirm_dialog(msg: &str) -> bool {
    #[cfg(feature = "hydrate")]
    {
        web_sys::window()
            .and_then(|w| w.confirm_with_message(msg).ok())
            .unwrap_or(true)
    }
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = msg;
        true
    }
}

#[component]
fn TourStopRow(
    s: TourStopView,
    tour_id: String,
    stop_delivered: ServerAction<TourStopDelivered>,
    stop_remover: ServerAction<RemoveTourStop>,
) -> impl IntoView {
    let (apple_url, google_url) = map_links(&s.address_line, s.latitude, s.longitude);
    let tel_url = format!("tel:{}", s.contact_phone);
    let needs_cash = s.payment_status == "cash_on_pickup";
    let delivered = s.delivered_at.is_some();
    let chat_open = RwSignal::new(false);
    let chat_id = s.order_id.clone();
    let seq = s.sequence;
    let tid_for_done = tour_id.clone();
    let tid_for_revert = tour_id.clone();
    let on_delivered = move |_| {
        stop_delivered.dispatch(TourStopDelivered {
            tour_id: tid_for_done.clone(),
            sequence: seq,
        });
    };
    let on_revert = move |_| {
        if confirm_dialog(
            "Diesen Stop aus der Tour nehmen? Bestellung geht zurück nach 'Abholbereit'.",
        ) {
            stop_remover.dispatch(RemoveTourStop {
                tour_id: tid_for_revert.clone(),
                sequence: seq,
            });
        }
    };
    let eta_label = s.eta_seconds.map(|sec| {
        let mins = (sec as f64 / 60.0).round() as i64;
        format!("ETA +{mins} Min.")
    });

    let cash_label = if needs_cash {
        format!("BAR: {}", format_eur(s.total_cents))
    } else if s.payment_status == "voucher_paid" {
        "Mit Gutschein bezahlt".to_string()
    } else {
        "Online bezahlt".to_string()
    };

    // Tour stops use the same .order-card markup as ready cards
    // (admin-orders style). Sequence number is the badge in the head;
    // everything else is inline. No <details> collapse.
    view! {
        <li class=move || if delivered { "order-card tour-stop done" } else { "order-card tour-stop" }>
            <div class="order-head">
                <span class="seq">{seq + 1}</span>
                <strong class="order-num">{s.order_number.clone()}</strong>
                {delivered
                    .then(|| view! {
                        <span class="status-pill" data-status="delivered">"Geliefert"</span>
                    })}
                {eta_label.clone().map(|l| view! {
                    <span class="muted small">{l}</span>
                })}
            </div>
            <p class="contact">
                {s.contact_name.clone()} " · "
                <a href=tel_url.clone()>{s.contact_phone.clone()}</a>
            </p>
            <p class="address">"📍 " {s.address_line.clone()}</p>
            <p class={if needs_cash && !delivered { "cash" } else { "total" }}>{cash_label}</p>
            <div class="row">
                <a class="btn ghost" href=apple_url>"🗺 Apple"</a>
                <a class="btn ghost" href=google_url>"🗺 Google"</a>
                {(!delivered).then(|| view! {
                    <button class="btn primary" on:click=on_delivered>"✓ Geliefert"</button>
                })}
                {(!delivered).then(|| view! {
                    <button class="btn ghost" on:click=on_revert>"↩ Zurück"</button>
                })}
                <button class="btn ghost" on:click=move |_| chat_open.update(|o| *o = !*o)>
                    "💬 Chat"
                </button>
            </div>
            {move || chat_open.get().then({
                let chat_id = chat_id.clone();
                move || view! {
                    <crate::pages::order::StaffOrderChat order_id=chat_id.clone() compact=true/>
                }
            })}
        </li>
    }
}

#[component]
fn LooseColumn(
    rows_sv: StoredValue<Vec<DriverOrderRow>>,
    advance: ServerAction<DriverAdvance>,
) -> impl IntoView {
    let count = rows_sv.with_value(|v| v.len());
    if count == 0 {
        return ().into_any();
    }
    view! {
        <div class="orders-column">
            <h2>"Einzeln unterwegs " <span class="count">"(" {count} ")"</span></h2>
            <ul>
                {rows_sv.with_value(|rows| rows.clone().into_iter().map(|r| view! {
                    <LooseCard r advance/>
                }).collect_view())}
            </ul>
        </div>
    }
    .into_any()
}

#[component]
fn LooseCard(r: DriverOrderRow, advance: ServerAction<DriverAdvance>) -> impl IntoView {
    let id_for_advance = r.id.clone();
    let on_advance = move |_| {
        advance.dispatch(DriverAdvance {
            id: id_for_advance.clone(),
            status: "delivered".to_string(),
        });
    };
    let (apple_url, google_url) = map_links(&r.address_line, None, None);
    let tel_url = format!("tel:{}", r.contact_phone);
    let needs_cash = r.payment_status == "cash_on_pickup";
    let cash_label = if needs_cash {
        format!("BAR: {}", format_eur(r.total_cents))
    } else if r.payment_status == "voucher_paid" {
        "Mit Gutschein bezahlt".to_string()
    } else {
        "Online bezahlt".to_string()
    };

    // Legacy single orders out for delivery without a tour. Same
    // admin-orders card shape as the ready / tour-stop cards — no
    // <details>, all info inline, action row at the bottom.
    view! {
        <li class="order-card driver-card">
            <div class="order-head">
                <strong class="order-num">{r.order_number}</strong>
                <span class="status-pill" data-status=r.status.clone()>
                    {status_label_de(&r.status)}
                </span>
            </div>
            <p class="contact">
                {r.contact_name.clone()} " · "
                <a href=tel_url.clone()>{r.contact_phone.clone()}</a>
            </p>
            <p class="address">"📍 " {r.address_line.clone()}</p>
            <p class="muted small">{format!("Liefergebiet: {}", r.zone)}</p>
            {r.address_notes.clone().map(|n| view! {
                <p class="muted small">{format!("Hinweis: {n}")}</p>
            })}
            <p class={if needs_cash { "cash" } else { "total" }}>{cash_label}</p>
            <div class="row">
                <a class="btn ghost" href=apple_url>"🗺 Apple"</a>
                <a class="btn ghost" href=google_url>"🗺 Google"</a>
                <button class="btn primary" on:click=on_advance>"✓ Geliefert"</button>
            </div>
        </li>
    }
}
