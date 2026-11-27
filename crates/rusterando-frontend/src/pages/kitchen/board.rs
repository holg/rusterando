//! Kitchen board (/kitchen): the kitchen display's board as a web page, for
//! phones and handhelds (the Sunmi V2 in the staff app) — the same orders,
//! built by the same code as the displays and the Bon
//! ([`crate::pages::push::KitchenBoardSource`]), showing what a display's
//! order detail shows, with the same moves: forward, back, handed over /
//! driver gone, Storno. Refunds and revenue stay admin-only.
//!
//! The timing rules (waiting time, pre-order due, late colours) follow
//! crates/kds-ui `timing()`; keep the two in step.

use kitchen_protocol::{BoardStatus, OrderChannel, OrderForKitchen, PaymentStatus};
use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

/// A fresh order (not started yet) stands out this long.
const FRESH_SECS: i64 = 3 * 60;

/// One order on the board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardCard {
    pub id: String,
    pub status: BoardStatus,
    /// The kitchen display's DTO (the Bon's, without brand line and QR).
    pub order: OrderForKitchen,
    /// Pre-order time or a delivery's ETA: "19:30", or "25.05. 19:30" when
    /// it isn't today. `None` = sofort.
    pub when: Option<String>,
    /// That time as unix seconds, for "in 25 min" / "fällig". Worked out on
    /// the server, whose clock runs in the shop's time zone like a
    /// display's.
    pub due_unix: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KitchenBoard {
    /// Server clock when built: every "seit … min" counts from it.
    pub server_now: i64,
    /// Oldest first, all columns mixed (as the displays get them).
    pub orders: Vec<BoardCard>,
}

#[server(name = LoadKitchenBoard, prefix = "/api", endpoint = "kitchen_board")]
pub async fn kitchen_board() -> Result<KitchenBoard, ServerFnError> {
    use crate::pages::session::{ssr::require_any, Role};

    require_any(&[Role::Kitchen]).await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let board = use_context::<crate::pages::push::KitchenBoardHandle>()
        .ok_or_else(|| ServerFnError::new("kitchen board missing from context"))?;
    let snapshot = board
        .snapshot(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("load board: {e}")))?;
    let now = snapshot.server_now_unix;
    let orders = snapshot
        .orders
        .into_iter()
        .map(|o| {
            let (when, due_unix) = ssr::due(&o.order, now);
            BoardCard {
                id: o.id,
                status: o.status,
                order: o.order,
                when,
                due_unix,
            }
        })
        .collect();
    Ok(KitchenBoard {
        server_now: now,
        orders,
    })
}

/// Move an order as a kitchen display does. `Ok(false)`: it had moved on
/// elsewhere already (the refetch shows where).
#[server(name = KitchenMove, prefix = "/api", endpoint = "kitchen_move")]
pub async fn kitchen_move(id: String, from: String, to: String) -> Result<bool, ServerFnError> {
    use crate::pages::session::{ssr::require_any, Role};

    require_any(&[Role::Kitchen]).await?;
    let (Some(from), Some(to)) = (BoardStatus::from_db(&from), BoardStatus::from_db(&to)) else {
        return Err(ServerFnError::new("unbekannter Status"));
    };
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let board = use_context::<crate::pages::push::KitchenBoardHandle>()
        .ok_or_else(|| ServerFnError::new("kitchen board missing from context"))?;
    let live = use_context::<crate::live::LiveHub>();
    board
        .advance(&db, live.as_ref(), &id, from, to)
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))
}

#[cfg(feature = "ssr")]
pub(crate) mod ssr {
    use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone};
    use kitchen_protocol::OrderForKitchen;

    /// The order's pre-order time / delivery ETA: the label to show and the
    /// moment it's due. As kds-ui `timing()`: a label for another day shows
    /// its date.
    pub fn due(o: &OrderForKitchen, now: i64) -> (Option<String>, Option<i64>) {
        let Some(label) = o.pickup_time_label.as_deref().filter(|l| !l.is_empty()) else {
            return (None, None);
        };
        let due = due_at(label, o.created_at_unix);
        let today = Local.timestamp_opt(now, 0).single().map(|d| d.date_naive());
        let when = match due {
            Some(due) if Some(due.date_naive()) != today => due.format("%d.%m. %H:%M").to_string(),
            _ => label.to_string(),
        };
        (Some(when), due.map(|d| d.timestamp()))
    }

    /// "19:30" (the order's day) or "25.05. 19:30" (the next such date) in
    /// the shop's time zone — kds-ui `due_at`.
    pub fn due_at(label: &str, created_at_unix: i64) -> Option<DateTime<Local>> {
        let created = Local
            .timestamp_opt(created_at_unix, 0)
            .single()?
            .date_naive();
        let due = match NaiveTime::parse_from_str(label, "%H:%M") {
            Ok(time) => created.and_time(time),
            Err(_) => {
                let (date, time) = label.split_once(' ')?;
                let time = NaiveTime::parse_from_str(time.trim(), "%H:%M").ok()?;
                let on = |year: i32| {
                    NaiveDate::parse_from_str(&format!("{date}{year}"), "%d.%m.%Y").ok()
                };
                let mut day = on(created.year())?;
                if day < created {
                    day = on(created.year() + 1)?;
                }
                day.and_time(time)
            }
        };
        Local.from_local_datetime(&due).earliest()
    }
}

// ---------------------------------------------------------------------------
// What a card shows (as kds-ui's card and order detail)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Timing {
    /// "19:30" or "sofort".
    when: String,
    /// The big figure: "12 min" waiting, "in 25 min" / "fällig" when due.
    age: String,
    /// 0 on time · 1 getting late · 2 late.
    urgency: u8,
    scheduled: bool,
    waited_min: i64,
}

fn timing(c: &BoardCard, now: i64) -> Timing {
    let waited_min = (now - c.order.created_at_unix).max(0) / 60;
    match &c.when {
        Some(when) => {
            let (age, urgency) = match c.due_unix {
                Some(due) => {
                    let mins = (due - now) / 60;
                    if mins <= 0 {
                        ("fällig".to_string(), 2)
                    } else {
                        (format!("in {mins} min"), if mins <= 15 { 1 } else { 0 })
                    }
                }
                None => (String::new(), 0),
            };
            Timing {
                when: when.clone(),
                age,
                urgency,
                scheduled: true,
                waited_min,
            }
        }
        None => Timing {
            when: "sofort".to_string(),
            age: format!("{waited_min} min"),
            urgency: if waited_min >= 35 {
                2
            } else if waited_min >= 20 {
                1
            } else {
                0
            },
            scheduled: false,
            waited_min,
        },
    }
}

/// "für 19:30 · Eingang 10.11. 19:02 · seit 12 min" — the display's detail
/// head line.
fn head_line(o: &OrderForKitchen, t: &Timing) -> String {
    let mut head = vec![if t.scheduled {
        format!("für {}", t.when)
    } else {
        "sofort".to_string()
    }];
    if !o.created_at_label.is_empty() {
        head.push(format!("Eingang {}", o.created_at_label));
    }
    head.push(format!("seit {} min", t.waited_min));
    head.join(" · ")
}

/// The number as the Bon prints it big: "0012" of "DP-1110-0012".
fn short_number(o: &OrderForKitchen) -> String {
    o.display_label
        .rsplit('-')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| o.display_number.to_string())
}

fn is_delivery(o: &OrderForKitchen) -> bool {
    matches!(o.channel, OrderChannel::Delivery { .. })
}

fn address_line(o: &OrderForKitchen) -> Option<String> {
    match &o.channel {
        OrderChannel::Delivery { address } => {
            let mut a = format!("{}, {} {}", address.street, address.postal, address.city);
            if let Some(bell) = address.bell.as_deref().filter(|b| !b.is_empty()) {
                a.push_str(" · Klingel: ");
                a.push_str(bell);
            }
            Some(a)
        }
        OrderChannel::DineIn { table } => Some(format!("Tisch {table}")),
        OrderChannel::Pickup => None,
    }
}

fn eur(cents: u32) -> String {
    format_eur(i64::from(cents))
}

fn pay_line(o: &OrderForKitchen) -> String {
    match o.payment {
        PaymentStatus::CollectOnDelivery { amount_cents } => {
            format!(
                "Bar kassieren: {} · Summe {}",
                eur(amount_cents),
                eur(o.total_cents)
            )
        }
        PaymentStatus::Prepaid => format!("Bezahlt · Summe {}", eur(o.total_cents)),
    }
}

type Move = Option<(&'static str, BoardStatus)>;

/// The display's buttons: (forward, back).
fn actions(status: BoardStatus, delivery: bool) -> (Move, Move) {
    use BoardStatus::*;
    match status {
        Received => (Some(("Starten", Preparing)), None),
        Preparing => (Some(("Fertig", Ready)), Some(("Zurück zu Neu", Received))),
        Ready if delivery => (
            Some(("Fahrer unterwegs", OutForDelivery)),
            Some(("Zurück in Arbeit", Preparing)),
        ),
        Ready => (
            Some(("Abgeholt", PickedUp)),
            Some(("Zurück in Arbeit", Preparing)),
        ),
        PickedUp | OutForDelivery | Cancelled => (None, None),
    }
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

    let mover = ServerAction::<KitchenMove>::new();
    let orders = Resource::new(
        move || mover.version().get(),
        |_| async move { kitchen_board().await },
    );
    // Live: follow changes made elsewhere (kitchen display, admin, new
    // orders). The poll keeps "seit … min" moving and is the fallback.
    crate::utils::on_board_change(move || orders.refetch());
    #[cfg(feature = "hydrate")]
    leptos::leptos_dom::helpers::set_interval(
        move || orders.refetch(),
        std::time::Duration::from_secs(30),
    );

    view! {
        <div class="kitchen-shell">
            <header class="kitchen-bar">
                <span class="brand">"🍳 Küche"</span>
                <crate::pages::session::RoleSwitcher current=crate::pages::session::Role::Kitchen/>
                <button class="btn ghost refresh" title="Aktualisieren" aria-label="Aktualisieren"
                    on:click=move |_| orders.refetch()>"Aktualisieren"</button>
                // Live auth-status chip + [Abmelden] (heartbeat current_role,
                // shows "Sitzung abgelaufen" if the server dropped the cookie).
                <crate::pages::session::LogoutButton role=crate::pages::session::Role::Kitchen/>
            </header>
            <main class="orders-page">
                {move || mover.value().get().and_then(|r| r.err()).map(|e| view! {
                    <p class="error">{format!("Nicht möglich: {e}")}</p>
                })}
                <Transition fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || orders.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(data) => view! { <Board data mover/> }.into_any(),
                    })}
                </Transition>
            </main>
        </div>
    }
}

#[component]
fn Board(data: KitchenBoard, mover: ServerAction<KitchenMove>) -> impl IntoView {
    let now = data.server_now;
    let column = |status: BoardStatus| -> Vec<BoardCard> {
        data.orders
            .iter()
            .filter(|c| c.status == status)
            .cloned()
            .collect()
    };
    view! {
        <div class="orders-board kitchen-board">
            <Column title="Neu" cards=column(BoardStatus::Received) now mover/>
            <Column title="In Arbeit" cards=column(BoardStatus::Preparing) now mover/>
            <Column title="Fertig" cards=column(BoardStatus::Ready) now mover/>
        </div>
    }
}

#[component]
fn Column(
    title: &'static str,
    cards: Vec<BoardCard>,
    now: i64,
    mover: ServerAction<KitchenMove>,
) -> impl IntoView {
    let count = cards.len();
    view! {
        <div class="orders-column">
            <h2>{title} " " <span class="count">"(" {count} ")"</span></h2>
            {if cards.is_empty() {
                view! { <p class="empty">"Keine Bestellungen"</p> }.into_any()
            } else {
                view! {
                    <ul>
                        {cards.into_iter().map(|c| view! { <Card c now mover/> }).collect_view()}
                    </ul>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn Card(c: BoardCard, now: i64, mover: ServerAction<KitchenMove>) -> impl IntoView {
    let o = &c.order;
    let t = timing(&c, now);
    let delivery = is_delivery(o);
    let paid = matches!(o.payment, PaymentStatus::Prepaid);
    let fresh = c.status == BoardStatus::Received && now - o.created_at_unix < FRESH_SECS;
    let mut cls = String::from("order-card kitchen-card");
    if fresh {
        cls.push_str(" fresh");
    }
    if o.is_test_mode {
        cls.push_str(" test-order");
    }

    let short = short_number(o);
    let full = if o.display_label.is_empty() {
        o.display_number.to_string()
    } else {
        o.display_label.clone()
    };
    let channel = if delivery {
        "🛵 LIEFERUNG"
    } else {
        "🏪 ABHOLUNG"
    };
    let pay_short = match o.payment {
        PaymentStatus::CollectOnDelivery { amount_cents } => format!("BAR {}", eur(amount_cents)),
        PaymentStatus::Prepaid => "BEZAHLT".to_string(),
    };
    let head = head_line(o, &t);
    let phone = o.customer.phone.clone().filter(|p| !p.is_empty());
    let address = address_line(o);
    let note = o.note.clone().filter(|n| !n.trim().is_empty());
    let pay = pay_line(o);
    let lines: Vec<(String, String)> = o
        .items
        .iter()
        .map(|i| {
            (
                format!("{}× {}", i.qty, i.name),
                i.modifications.join(" · "),
            )
        })
        .collect();

    let (forward, back) = actions(c.status, delivery);
    let from = c.status.as_db();
    let move_to = {
        let id = c.id.clone();
        move |to: BoardStatus| {
            mover.dispatch(KitchenMove {
                id: id.clone(),
                from: from.to_string(),
                to: to.as_db().to_string(),
            });
        }
    };
    let button = |m: Move, class: &'static str| {
        let move_to = move_to.clone();
        m.map(|(label, to)| {
            view! {
                <button class=class on:click=move |_| move_to(to)
                    disabled=move || mover.pending().get()>
                    {label}
                </button>
            }
        })
    };
    let forward_button = button(forward, "btn primary");
    let back_button = button(back, "btn ghost");

    // Storno as on the display: the first tap arms, a second within a few
    // seconds confirms.
    let armed = RwSignal::new(false);
    let on_storno = {
        let move_to = move_to.clone();
        move |_| {
            if armed.get_untracked() {
                armed.set(false);
                move_to(BoardStatus::Cancelled);
            } else {
                armed.set(true);
                #[cfg(feature = "hydrate")]
                leptos::leptos_dom::helpers::set_timeout(
                    move || armed.set(false),
                    std::time::Duration::from_secs(5),
                );
            }
        }
    };
    let storno_text = move || {
        if !armed.get() {
            "Stornieren"
        } else if paid {
            "Wirklich stornieren? Nochmal tippen (online bezahlt: Erstattung im Admin)"
        } else {
            "Wirklich stornieren? Nochmal tippen"
        }
    };

    // Chat is collapsed by default and only mounts its message-loading
    // Resource when opened, so a board of 20 cards doesn't fire 20 queries.
    let chat_open = RwSignal::new(false);
    let chat_id = c.id.clone();

    view! {
        <li class=cls>
            <div class="order-head">
                <strong class="order-num">{short}</strong>
                <span class="pills">
                    <span class="status-pill" data-order-type=if delivery { "delivery" } else { "pickup" }>{channel}</span>
                    {o.is_test_mode.then(|| view! {
                        <span class="test-pill" title="Sandbox/Test-Bestellung">"TEST"</span>
                    })}
                    <span class="status-pill" data-payment=if paid { "paid" } else { "cash_on_pickup" }>{pay_short}</span>
                </span>
                <span class="age" data-urgency=t.urgency.to_string()>{t.age.clone()}</span>
            </div>
            <p class="when">{head}</p>
            <p class="contact">
                <span class="label">{full}</span>
                " · " {o.customer.name.clone()}
                {phone.map(|p| view! { " · " <a href=format!("tel:{p}")>{p.clone()}</a> })}
            </p>
            {address.map(|a| view! { <p class="address">"📍 " {a}</p> })}
            <ul class="lines">
                {lines.into_iter().map(|(text, sub)| view! {
                    <li>
                        <span class="line">{text}</span>
                        {(!sub.is_empty()).then(|| view! { <span class="mods">{sub}</span> })}
                    </li>
                }).collect_view()}
            </ul>
            {note.map(|n| view! { <p class="note">"📝 " {n}</p> })}
            <p class="pay" data-paid=paid>{pay}</p>
            <div class="row moves">
                {forward_button}
                {back_button}
            </div>
            <div class="row">
                <crate::components::device_print::ReprintButtons id=c.id.clone()/>
                <button class="btn ghost" on:click=move |_| chat_open.update(|o| *o = !*o)>
                    "💬 Chat"
                </button>
                <button class="btn ghost storno" class:armed=move || armed.get()
                    on:click=on_storno disabled=move || mover.pending().get()>
                    {storno_text}
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

#[cfg(test)]
mod tests {
    use super::*;
    use kitchen_protocol::{Customer, DeliveryAddress, LineItem, OrderId};

    fn order(created: i64, label: Option<&str>) -> OrderForKitchen {
        OrderForKitchen {
            order_id: OrderId(1),
            display_number: 12,
            display_label: "DP-1110-0012".into(),
            created_at_unix: created,
            created_at_label: "10.11. 19:02".into(),
            channel: OrderChannel::Delivery {
                address: DeliveryAddress {
                    street: "Hauptstraße 12".into(),
                    postal: "99999".into(),
                    city: "Musterstadt".into(),
                    bell: Some("Muster".into()),
                },
            },
            customer: Customer {
                name: "Max".into(),
                phone: Some("0151".into()),
            },
            items: vec![LineItem {
                qty: 2,
                name: "#22a Pizza Sucuk".into(),
                modifications: vec!["groß".into(), "ohne Zwiebeln".into()],
                unit_price_cents: 900,
                category: String::new(),
                modification_prices_cents: vec![],
            }],
            subtotal_cents: 1800,
            delivery_fee_cents: 150,
            total_cents: 1950,
            payment: PaymentStatus::CollectOnDelivery { amount_cents: 1950 },
            note: Some("Hund bellt".into()),
            shop_name: String::new(),
            accepted_at_unix: None,
            accepted_at_label: None,
            qr_url: None,
            is_test_mode: false,
            voucher_code: String::new(),
            voucher_discount_cents: 0,
            pickup_time_label: label.map(str::to_string),
            printer_theme: String::new(),
        }
    }

    fn card(o: OrderForKitchen, when: Option<&str>, due: Option<i64>) -> BoardCard {
        BoardCard {
            id: "x".into(),
            status: BoardStatus::Received,
            order: o,
            when: when.map(str::to_string),
            due_unix: due,
        }
    }

    #[test]
    fn asap_counts_up_and_gets_late() {
        let c = card(order(1_000, None), None, None);
        let t = timing(&c, 1_000 + 12 * 60);
        assert_eq!(
            (t.when.as_str(), t.age.as_str(), t.urgency),
            ("sofort", "12 min", 0)
        );
        assert_eq!(timing(&c, 1_000 + 20 * 60).urgency, 1);
        assert_eq!(timing(&c, 1_000 + 35 * 60).urgency, 2);
        assert_eq!(
            head_line(&c.order, &t),
            "sofort · Eingang 10.11. 19:02 · seit 12 min"
        );
    }

    #[test]
    fn pre_order_counts_down_to_due() {
        let due = 10_000;
        let c = card(order(1_000, Some("19:30")), Some("19:30"), Some(due));
        let t = timing(&c, due - 25 * 60);
        assert_eq!((t.age.as_str(), t.urgency), ("in 25 min", 0));
        assert_eq!(timing(&c, due - 10 * 60).urgency, 1);
        let t = timing(&c, due);
        assert_eq!((t.age.as_str(), t.urgency), ("fällig", 2));
        assert!(head_line(&c.order, &t).starts_with("für 19:30 · Eingang"));
    }

    #[test]
    fn card_texts_match_the_display() {
        let o = order(0, None);
        assert_eq!(short_number(&o), "0012");
        assert_eq!(
            address_line(&o).as_deref(),
            Some("Hauptstraße 12, 99999 Musterstadt · Klingel: Muster")
        );
        assert!(pay_line(&o).starts_with("Bar kassieren: "));
        assert_eq!(
            actions(BoardStatus::Ready, true).0.map(|(l, s)| (l, s)),
            Some(("Fahrer unterwegs", BoardStatus::OutForDelivery))
        );
        assert_eq!(
            actions(BoardStatus::Ready, false).0.map(|(l, _)| l),
            Some("Abgeholt")
        );
    }
}
