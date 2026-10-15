//! `KDS_DEMO=1`: sample orders for trying the UI without a server or
//! printer (desktop development, first boot on a new board). Moves are
//! applied locally; an order leaving the board just disappears.

use chrono::{Duration, Local};
use kitchen_protocol::{
    BoardOrder, BoardSnapshot, BoardStatus, Customer, DeliveryAddress, LineItem, OrderChannel,
    OrderForKitchen, OrderId, PaymentStatus, ShopAction, ShopLight, ShopPanel,
};

pub fn snapshot() -> BoardSnapshot {
    let now = crate::local_now();
    let in_min = |m: i64| (Local::now() + Duration::minutes(m)).format("%H:%M").to_string();
    let orders = vec![
        order(1, 12, BoardStatus::Received, 1, None, true, &[(1, "#22a Pizza Funghi", &["36 cm", "extra Käse"]), (2, "#80 Cola 0,5 l", &[])], Some("Bitte gut durchbacken"), false),
        order(2, 13, BoardStatus::Received, 4, Some(in_min(40)), false, &[(1, "#51 Pizza Inferno", &["ohne Zwiebeln"]), (1, "#12 Pizzabrötchen", &["Sauce: Kräuterbutter"])], None, false),
        order(3, 9, BoardStatus::Preparing, 14, None, true, &[(3, "#24 Pizza Salami", &["30 cm"]), (1, "#61 Salat Hawaii", &["Dressing: Joghurt"])], None, false),
        order(4, 10, BoardStatus::Preparing, 26, None, false, &[(1, "#31 Calzone", &[])], None, true),
        order(5, 11, BoardStatus::Preparing, 8, Some(in_min(10)), true, &[(2, "#22 Pizza Margherita", &["extra Oliven", "extra Peperoni"])], None, false),
        order(6, 7, BoardStatus::Ready, 38, None, true, &[(1, "#44 Pasta Bolognese", &[])], Some("Klingel defekt, bitte anrufen"), false),
        order(7, 8, BoardStatus::Ready, 22, None, false, &[(2, "#12 Pizzabrötchen", &[]), (1, "#80 Fanta 0,5 l", &[])], None, false),
    ];
    BoardSnapshot {
        server_now_unix: now,
        orders: orders
            .into_iter()
            .map(|mut o| {
                o.order.created_at_unix = now - o.order.created_at_unix * 60;
                o
            })
            .collect(),
    }
}

pub fn apply_move(board: &mut Option<BoardSnapshot>, id: &str, to: BoardStatus) {
    let Some(board) = board else { return };
    match to {
        BoardStatus::PickedUp | BoardStatus::OutForDelivery | BoardStatus::Cancelled => {
            board.orders.retain(|o| o.id != id)
        }
        _ => {
            if let Some(o) = board.orders.iter_mut().find(|o| o.id == id) {
                o.status = to;
            }
        }
    }
}

pub fn shop() -> ShopPanel {
    ShopPanel {
        light: ShopLight::OpensLater,
        reason: "Heute ab 17:00 Uhr geöffnet".into(),
        gate_open: false,
        gate_reason: "Heute ab 17:00 Uhr geöffnet".into(),
        orders_paused: false,
        snooze_until: String::new(),
        force_open_until: String::new(),
        delivery_paused: false,
        today_open: "17:00".into(),
        today_close: "22:00".into(),
        today_extended: false,
        notice: String::new(),
    }
}

/// Rough local stand-in for the server's handling, enough to try the page.
pub fn apply_shop(shop: &mut Option<ShopPanel>, action: &ShopAction) {
    let Some(p) = shop else { return };
    let in_min = |m: u16| (Local::now() + Duration::minutes(m as i64)).format("%H:%M").to_string();
    match action {
        ShopAction::SetOpen(true) => {
            p.light = ShopLight::Open;
            p.gate_open = true;
            p.orders_paused = false;
            p.snooze_until.clear();
            p.force_open_until = "23:59".into();
        }
        ShopAction::SetOpen(false) => {
            p.orders_paused = true;
            p.force_open_until.clear();
            p.light = ShopLight::OpensLater;
            p.gate_open = false;
        }
        ShopAction::ResetToSchedule => *p = shop_reset(),
        ShopAction::Snooze { minutes } => {
            p.orders_paused = false;
            p.snooze_until = if *minutes == 0 { String::new() } else { in_min(*minutes) };
        }
        ShopAction::SetDeliveryPaused(v) => p.delivery_paused = *v,
        ShopAction::SetTodayHours { open, close } => {
            if let Some(o) = open {
                p.today_open = o.clone();
            }
            if let Some(c) = close {
                p.today_close = c.clone();
            }
            p.today_extended = true;
        }
    }
}

fn shop_reset() -> ShopPanel {
    shop()
}

/// `created_at_unix` holds minutes-ago here; [`snapshot`] converts it.
#[allow(clippy::too_many_arguments)]
fn order(
    n: u64,
    number: u32,
    status: BoardStatus,
    minutes_ago: i64,
    due: Option<String>,
    delivery: bool,
    items: &[(u32, &str, &[&str])],
    note: Option<&str>,
    test: bool,
) -> BoardOrder {
    let total: u32 = items.iter().map(|(q, _, _)| q * 950).sum();
    let created = (Local::now() - Duration::minutes(minutes_ago)).format("%H:%M").to_string();
    BoardOrder {
        id: format!("demo-{n}"),
        status,
        order: OrderForKitchen {
            order_id: OrderId(n),
            display_number: number,
            display_label: format!("DP-1005-{number:04}"),
            created_at_unix: minutes_ago,
            created_at_label: created,
            channel: if delivery {
                OrderChannel::Delivery {
                    address: DeliveryAddress {
                        street: "Mühlenstraße 12".into(),
                        postal: "59348".into(),
                        city: "Lüdinghausen".into(),
                        bell: Some("Müller".into()),
                    },
                }
            } else {
                OrderChannel::Pickup
            },
            customer: Customer {
                name: "Erika Mustermann".into(),
                phone: Some("02591 12345".into()),
            },
            items: items
                .iter()
                .map(|(qty, name, mods)| LineItem {
                    qty: *qty,
                    name: name.to_string(),
                    modifications: mods.iter().map(|m| m.to_string()).collect(),
                    unit_price_cents: 950,
                    category: String::new(),
                    modification_prices_cents: Vec::new(),
                })
                .collect(),
            subtotal_cents: total,
            delivery_fee_cents: 0,
            total_cents: total,
            payment: if n.is_multiple_of(2) {
                PaymentStatus::Prepaid
            } else {
                PaymentStatus::CollectOnDelivery {
                    amount_cents: total,
                }
            },
            note: note.map(str::to_string),
            shop_name: String::new(),
            accepted_at_unix: None,
            accepted_at_label: None,
            qr_url: None,
            is_test_mode: test,
            voucher_code: String::new(),
            voucher_discount_cents: 0,
            pickup_time_label: due,
            printer_theme: String::new(),
        },
    }
}
