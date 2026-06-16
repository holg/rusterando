//! Kitchen channel — server side.
//!
//! Wire up:
//!
//! ```rust,ignore
//! // In your AppState
//! pub struct AppState {
//!     /* ... existing fields ... */
//!     pub kitchen: KitchenChannel,
//! }
//!
//! // In main()
//! let kitchen = KitchenChannel::new(db.clone());
//! let listener_chan = kitchen.clone();
//! tokio::spawn(async move {
//!     let addr: std::net::SocketAddr = "127.0.0.1:9001".parse().unwrap();
//!     if let Err(e) = kitchen::run_listener(listener_chan, addr).await {
//!         tracing::error!(error = %e, "kitchen listener exited");
//!     }
//! });
//!
//! // From your Stripe webhook handler, after marking order paid:
//! let order_for_kitchen = build_order_for_kitchen(&db, order_id).await?;
//! state.kitchen.broadcast(KitchenEvent::NewOrder(order_for_kitchen)).await?;
//! ```

use std::net::SocketAddr;

use anyhow::Context;
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use kitchen_protocol::{ClientMessage, KitchenEvent, SeqId, ServerMessage, FRAME_MAX_BYTES};
use sqlx::{Row, SqlitePool};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{debug, error, info, warn};

const BROADCAST_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct KitchenChannel {
    tx: broadcast::Sender<ServerMessage>,
    db: SqlitePool,
}

impl KitchenChannel {
    pub fn new(db: SqlitePool) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self { tx, db }
    }

    /// Persist and broadcast a kitchen event. The returned SeqId is
    /// the outbox row id, monotonically increasing across the lifetime
    /// of the DB.
    ///
    /// The stored blob is `[major, minor, postcard...]` — see
    /// `kitchen_protocol::encode_versioned`. Any decoder that doesn't
    /// recognise the major bumps over to the skip+log path rather
    /// than poisoning the channel.
    pub async fn broadcast(&self, event: KitchenEvent) -> anyhow::Result<SeqId> {
        let postcard_bytes = postcard::to_allocvec(&event).context("postcard encode event")?;
        let payload = kitchen_protocol::encode_versioned(&postcard_bytes);
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO kitchen_outbox (payload, created_at) \
             VALUES (?, strftime('%s', 'now')) RETURNING id",
        )
        .bind(&payload)
        .fetch_one(&self.db)
        .await
        .context("insert kitchen_outbox")?;

        let seq_id = SeqId(id as u64);
        let msg = ServerMessage { seq_id, event };

        // Best-effort send: if no client is currently subscribed, the event
        // sits in the outbox until reconnect.
        let _ = self.tx.send(msg);
        Ok(seq_id)
    }

    fn subscribe(&self) -> broadcast::Receiver<ServerMessage> {
        self.tx.subscribe()
    }

    async fn unacked_since(&self, after: SeqId) -> anyhow::Result<Vec<ServerMessage>> {
        let rows = sqlx::query(
            "SELECT id, payload FROM kitchen_outbox \
             WHERE acked_at IS NULL AND id > ? \
             ORDER BY id ASC \
             LIMIT 500",
        )
        .bind(after.0 as i64)
        .fetch_all(&self.db)
        .await
        .context("query unacked outbox")?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: i64 = row.get("id");
            let payload: Vec<u8> = row.get("payload");

            // Strip the 2-byte version header. Major mismatch =
            // skip+log (leaves the row unacked for forensics — clean
            // it up by hand with a SQL UPDATE when convenient).
            // Postcard decode errors get the same treatment so a
            // single bad row can't lock up the entire replay batch.
            let postcard_bytes = match kitchen_protocol::decode_versioned(&payload) {
                Ok((_minor, inner)) => inner,
                Err(e) => {
                    warn!(
                        outbox_id = id,
                        error = %e,
                        "skipping incompatible outbox row (left unacked for forensics)",
                    );
                    continue;
                }
            };
            let event: KitchenEvent = match postcard::from_bytes(postcard_bytes) {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        outbox_id = id,
                        error = %e,
                        "skipping un-decodeable outbox row (left unacked for forensics)",
                    );
                    continue;
                }
            };
            out.push(ServerMessage {
                seq_id: SeqId(id as u64),
                event,
            });
        }
        Ok(out)
    }

    async fn ack(&self, seq_id: SeqId) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE kitchen_outbox \
             SET acked_at = strftime('%s', 'now') \
             WHERE id = ? AND acked_at IS NULL",
        )
        .bind(seq_id.0 as i64)
        .execute(&self.db)
        .await
        .context("ack outbox row")?;
        Ok(())
    }
}

/// Bind a TCP listener and serve kitchen clients indefinitely. Bind address
/// should always be loopback in production (127.0.0.1:9001) — the SSH tunnel
/// from each pizzeria forwards to it. The protocol does no app-level auth,
/// so binding to a public interface would be a security hole.
pub async fn run_listener(channel: KitchenChannel, bind_addr: SocketAddr) -> anyhow::Result<()> {
    if !bind_addr.ip().is_loopback() {
        warn!(
            %bind_addr,
            "kitchen listener is binding a non-loopback address — only the SSH tunnel \
             should reach this port; double-check your firewall"
        );
    }
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind {bind_addr}"))?;
    info!(%bind_addr, "kitchen listener ready");

    loop {
        let (stream, peer) = listener.accept().await?;
        info!(%peer, "kitchen client connected");
        let chan = channel.clone();
        tokio::spawn(async move {
            match handle_client(chan, stream, peer).await {
                Ok(()) => info!(%peer, "kitchen client session ended"),
                Err(e) => warn!(%peer, error = %e, "kitchen session ended with error"),
            }
        });
    }
}

async fn handle_client(
    channel: KitchenChannel,
    stream: TcpStream,
    peer: SocketAddr,
) -> anyhow::Result<()> {
    stream.set_nodelay(true).ok();
    let codec = LengthDelimitedCodec::builder()
        .max_frame_length(FRAME_MAX_BYTES)
        .new_codec();
    let mut framed = Framed::new(stream, codec);

    // ===== Step 1: read Hello =====
    let hello_bytes = framed
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("client closed before Hello"))?
        .context("read Hello frame")?;
    let (_hello_minor, hello_postcard) =
        kitchen_protocol::decode_versioned(&hello_bytes).context("decode Hello envelope")?;
    let hello: ClientMessage = postcard::from_bytes(hello_postcard).context("decode Hello body")?;
    let ClientMessage::Hello {
        shop_slug,
        version,
        last_seen_seq,
    } = hello
    else {
        anyhow::bail!("first message was not Hello");
    };
    info!(%peer, %shop_slug, %version, ?last_seen_seq, "kitchen client hello");

    // ===== Step 2: replay unacked from outbox =====
    let replay_from = last_seen_seq.unwrap_or(SeqId(0));
    let backlog = channel.unacked_since(replay_from).await?;
    info!(%peer, count = backlog.len(), from = %replay_from, "replaying unacked events");

    // ===== Step 3: subscribe to live broadcast THEN replay =====
    // Subscribe before sending the backlog so we don't miss anything inserted
    // between the outbox query and the live-stream join.
    let mut rx = channel.subscribe();

    for msg in &backlog {
        let body = postcard::to_allocvec(msg).context("encode replay")?;
        let frame = kitchen_protocol::encode_versioned(&body);
        framed
            .send(Bytes::from(frame))
            .await
            .context("send replay frame")?;
    }

    // After replay, drain any live events that arrived during replay and
    // dedupe against what we already sent.
    let max_replayed = backlog
        .iter()
        .map(|m| m.seq_id)
        .max()
        .unwrap_or(replay_from);
    drop(backlog);

    let (mut sink, mut input) = framed.split();

    // Reader: handle client → server messages (acks, heartbeats)
    let chan_for_acks = channel.clone();
    let ack_task = tokio::spawn(async move {
        while let Some(frame) = input.next().await {
            let bytes = match frame {
                Ok(b) => b,
                Err(e) => {
                    warn!(error = %e, "ack-stream read error");
                    break;
                }
            };
            // Strip the 2-byte version header, then postcard-decode.
            // Either step failing = skip the frame and keep listening
            // rather than tearing down the session.
            let postcard_bytes = match kitchen_protocol::decode_versioned(&bytes) {
                Ok((_minor, inner)) => inner,
                Err(e) => {
                    warn!(error = %e, "decode inbound envelope");
                    continue;
                }
            };
            let msg: ClientMessage = match postcard::from_bytes(postcard_bytes) {
                Ok(m) => m,
                Err(e) => {
                    warn!(error = %e, "decode inbound body");
                    continue;
                }
            };
            match msg {
                ClientMessage::Ack(seq_id) => {
                    if let Err(e) = chan_for_acks.ack(seq_id).await {
                        warn!(seq_id = %seq_id, error = %e, "persist ack failed");
                    } else {
                        debug!(seq_id = %seq_id, "ack persisted");
                    }
                }
                ClientMessage::Heartbeat => debug!("heartbeat"),
                ClientMessage::Hello { .. } => {
                    warn!("unexpected mid-session Hello; ignoring");
                }
            }
        }
        debug!("ack reader ended");
    });

    // Writer: live broadcast → client, deduped against already-replayed.
    let mut send_result: anyhow::Result<()> = Ok(());
    loop {
        match rx.recv().await {
            Ok(msg) => {
                if msg.seq_id <= max_replayed {
                    debug!(seq_id = %msg.seq_id, "skipping live event already in replay");
                    continue;
                }
                let body = match postcard::to_allocvec(&msg) {
                    Ok(b) => b,
                    Err(e) => {
                        error!(error = %e, "encode live event");
                        continue;
                    }
                };
                let frame = kitchen_protocol::encode_versioned(&body);
                if let Err(e) = sink.send(Bytes::from(frame)).await {
                    send_result = Err(e).context("send live event");
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!(
                    skipped = n,
                    "broadcast lagged — client will recover via outbox replay on reconnect"
                );
            }
            Err(broadcast::error::RecvError::Closed) => {
                debug!("broadcast closed");
                break;
            }
        }
    }

    ack_task.abort();
    send_result
}

// ---------------------------------------------------------------------------
// build_order_for_kitchen + KitchenSink impl
// ---------------------------------------------------------------------------

use kitchen_protocol::{
    Customer, DeliveryAddress, LineItem, OrderChannel, OrderForKitchen, OrderId, PaymentStatus,
};

/// `KitchenSink` is defined in the frontend crate so server fns
/// there can broadcast without depending on this crate. We implement
/// it here for our concrete `KitchenChannel`.
///
/// The trait method takes only `db` + `order_id`, so we resolve
/// `shop_name` from the BrandingHandle and `public_url_base` from
/// the PUBLIC_URL env var inside the impl. Both are cheap reads;
/// doing it here keeps the call sites trivial.
#[async_trait::async_trait]
impl rusterando_frontend::pages::push::KitchenSink for KitchenChannel {
    async fn broadcast_new_order(&self, db: &SqlitePool, order_id: &str) -> anyhow::Result<()> {
        let (shop_name, public_url_base) = resolve_brand_and_url(db).await;
        let dto = build_order_for_kitchen(db, order_id, &shop_name, &public_url_base).await?;
        self.broadcast(KitchenEvent::NewOrder(dto)).await?;
        Ok(())
    }

    async fn broadcast_reprint(&self, db: &SqlitePool, order_id: &str) -> anyhow::Result<()> {
        let (shop_name, public_url_base) = resolve_brand_and_url(db).await;
        let dto = build_order_for_kitchen(db, order_id, &shop_name, &public_url_base).await?;
        self.broadcast(KitchenEvent::Reprint(dto)).await?;
        Ok(())
    }
}

/// Read the brand name from `app_settings` (same source BrandingHandle
/// uses, but without needing leptos context) and the public URL base
/// from the PUBLIC_URL env var. Both feed the receipt header / QR code.
async fn resolve_brand_and_url(db: &SqlitePool) -> (String, String) {
    let shop_name: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'shop_name'")
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    let shop_name = shop_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    let public_url_base = std::env::var("PUBLIC_URL").unwrap_or_default();
    (shop_name, public_url_base)
}

/// Translate a persisted `orders` row + its `order_items` into the
/// protocol's `OrderForKitchen` DTO. Called from `place_order` (cash
/// path), the Stripe webhook (paid card path), and the admin reprint
/// endpoint. Reading from the DB rather than constructing in-flight
/// gives us one source of truth: the receipt always reflects what
/// was actually persisted, never a stale in-memory copy.
///
/// `order_id` is the `orders.id` (TEXT primary key). The hash of
/// that id is used as the protocol's u64 `OrderId` — the Pi just
/// echoes it in logs, no need for an integer counter on the orders
/// table.
///
/// `shop_name` is the BrandingHandle display name; rendered as the
/// brand line at the top of the receipt. Pass empty string to omit.
///
/// `public_url_base` is the scheme+host (no trailing slash) used to
/// build the QR-code URL: `<base>/orders/<id>`. Pass empty string
/// to skip the QR code entirely.
pub async fn build_order_for_kitchen(
    db: &SqlitePool,
    order_id: &str,
    shop_name: &str,
    public_url_base: &str,
) -> anyhow::Result<OrderForKitchen> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // ---- Order header ----
    let row = sqlx::query(
        "SELECT order_number, order_type, contact_name, contact_phone,
                delivery_address_json, subtotal_cents, delivery_fee_cents,
                total_cents, payment_status, status, notes, stripe_mode,
                voucher_code, voucher_discount_cents,
                strftime('%s', created_at) AS created_at_unix,
                strftime('%s', updated_at) AS updated_at_unix,
                strftime('%s', scheduled_for) AS scheduled_for_unix
         FROM orders
         WHERE id = ?1",
    )
    .bind(order_id)
    .fetch_one(db)
    .await
    .context("load order")?;

    let order_number: String = row.get("order_number");
    let order_type: String = row.get("order_type");
    let contact_name: String = row.get("contact_name");
    let contact_phone: String = row.get("contact_phone");
    let delivery_address_json: Option<String> = row.get("delivery_address_json");
    let subtotal_cents: i64 = row.get("subtotal_cents");
    let delivery_fee_cents: i64 = row.get("delivery_fee_cents");
    let total_cents: i64 = row.get("total_cents");
    let payment_status: String = row.get("payment_status");
    let status: String = row.get("status");
    let notes: Option<String> = row.get("notes");
    // Stripe mode snapshot from place_order time. Anything but
    // 'live' counts as test (sandbox, or legacy NULL/empty rows
    // backfilled by the migration). Receipts in test mode get a
    // big ⚠ banner so the kitchen can't fulfill them by accident.
    let stripe_mode: String = row.get::<String, _>("stripe_mode");
    let is_test_mode = stripe_mode.trim().to_ascii_lowercase() != "live";
    let voucher_code: String = row
        .try_get::<Option<String>, _>("voucher_code")
        .unwrap_or(None)
        .unwrap_or_default();
    let voucher_discount_cents: i64 = row.try_get::<i64, _>("voucher_discount_cents").unwrap_or(0);
    // SQLite returns strftime('%s', ...) as text. Parse to i64.
    let created_at_unix: i64 = row.get::<String, _>("created_at_unix").parse().unwrap_or(0);
    let updated_at_unix: i64 = row.get::<String, _>("updated_at_unix").parse().unwrap_or(0);
    // Scheduled fulfillment time. NULL (= ASAP) → None. strftime returns
    // NULL as a SQL NULL, so try_get gives Option<String>.
    let scheduled_for_unix: Option<i64> = row
        .try_get::<Option<String>, _>("scheduled_for_unix")
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok());

    // Order is "accepted" once it advances past the initial state.
    // pending_payment → waiting for Stripe; received → kitchen has it.
    // Any later status (preparing, ready_for_pickup, …) implies it
    // was previously received, so updated_at is at-or-after acceptance.
    let accepted_at_unix: Option<i64> = match status.as_str() {
        "pending_payment" | "" => None,
        _ => Some(updated_at_unix),
    };

    // ---- Channel ----
    let channel = if order_type == "delivery" {
        let addr = delivery_address_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        let street_raw = addr
            .get("street")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let house = addr
            .get("house_number")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let postal = addr
            .get("postcode")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let city = addr
            .get("city")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let bell = addr
            .get("notes")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);
        let street = if house.is_empty() {
            street_raw
        } else {
            format!("{street_raw} {house}")
        };
        OrderChannel::Delivery {
            address: DeliveryAddress {
                street,
                postal,
                city,
                bell,
            },
        }
    } else {
        OrderChannel::Pickup
    };

    // ---- Line items ----
    //
    // LEFT JOIN so the category is best-effort: if the menu item was
    // later deleted/renamed/recategorised, we still render the order
    // without a heading (better than panicking on a missing FK).
    let item_rows = sqlx::query(
        "SELECT oi.menu_number_snapshot, oi.name_snapshot, oi.quantity,
                oi.options_json, oi.extras_json, oi.unit_price_cents,
                mc.name AS category_name
         FROM order_items oi
         LEFT JOIN menu_items mi ON mi.id = oi.menu_item_id
         LEFT JOIN menu_categories mc ON mc.id = mi.category_id
         WHERE oi.order_id = ?1
         ORDER BY oi.id",
    )
    .bind(order_id)
    .fetch_all(db)
    .await
    .context("load order_items")?;

    let mut items = Vec::with_capacity(item_rows.len());
    for r in item_rows {
        let menu_number: Option<String> = r.get("menu_number_snapshot");
        let name_snapshot: String = r.get("name_snapshot");
        let quantity: i64 = r.get("quantity");
        let options_json: String = r.get("options_json");
        let extras_json: Option<String> = r.get("extras_json");
        let unit_price_cents: i64 = r.get("unit_price_cents");
        let category_name: Option<String> = r.get("category_name");

        // Prefix with the menu number so kitchen + customer share one
        // vocabulary. "#22a Pizza Funghi" reads cleaner than "Pizza
        // Funghi (Nr. 22a)" and matches the Lieferando style.
        let display_name = match menu_number {
            Some(n) if !n.trim().is_empty() => format!("#{n} {name_snapshot}"),
            _ => name_snapshot,
        };

        // Modifications come in two flavours:
        //   1. Size variant from options_json ("36 cm", "60 × 40 cm").
        //      No price — it's already baked into unit_price_cents.
        //   2. Picked extras from extras_json, each with its own price.
        //      Prices may be 0 (e.g. the "first 3 free" Pizzablech).
        // We keep them in the same `modifications` array but track
        // prices in a parallel vec so the Pi can right-align the price.
        let mut modifications: Vec<String> = Vec::new();
        let mut modification_prices: Vec<u32> = Vec::new();
        if let Ok(opts) = serde_json::from_str::<serde_json::Value>(&options_json) {
            if let Some(label) = opts.get("size_label").and_then(|v| v.as_str()) {
                if !label.is_empty() {
                    modifications.push(label.to_string());
                    modification_prices.push(0);
                }
            }
        }
        if let Some(j) = extras_json.as_deref() {
            if let Ok(extras) = serde_json::from_str::<Vec<rusterando_shared::models::CartExtra>>(j)
            {
                for e in extras {
                    modifications.push(e.label);
                    modification_prices.push(e.price_cents.max(0) as u32);
                }
            }
        }

        items.push(LineItem {
            qty: quantity.max(0) as u32,
            name: display_name,
            modifications,
            unit_price_cents: unit_price_cents.max(0) as u32,
            category: category_name.unwrap_or_default(),
            modification_prices_cents: modification_prices,
        });
    }

    // ---- Payment ----
    let payment = match payment_status.as_str() {
        "paid" => PaymentStatus::Prepaid,
        _ => PaymentStatus::CollectOnDelivery {
            amount_cents: total_cents.max(0) as u32,
        },
    };

    // ---- Display number ----
    // The protocol DTO wants a u32. Our order_number is "DP-1042" or
    // similar; pull trailing digits if any, otherwise fall back to 0.
    // The Pi shows it as "BESTELLUNG #<n>" — staff-facing only.
    let display_number: u32 = order_number
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>()
        .parse()
        .unwrap_or(0);

    // ---- u64 OrderId from the TEXT primary key ----
    let mut hasher = DefaultHasher::new();
    order_id.hash(&mut hasher);

    // QR code target — same URL the customer's confirmation email
    // links to. Empty base disables the QR block entirely.
    let qr_url = if public_url_base.is_empty() {
        None
    } else {
        Some(format!(
            "{}/orders/{}",
            public_url_base.trim_end_matches('/'),
            order_id
        ))
    };

    // Server-formatted timestamp labels in the SHOP's local timezone
    // (TZ env var, see systemd unit). Sent to the Pi as ready-to-print
    // strings so the Pi doesn't have to know about timezones — its
    // chrono::Local is unreliable (the device may run UTC).
    let fmt_local = |unix: i64| -> String {
        use chrono::{Local, TimeZone};
        Local
            .timestamp_opt(unix, 0)
            .single()
            .map(|t| t.format("%d.%m. %H:%M").to_string())
            .unwrap_or_default()
    };
    let created_at_label = fmt_local(created_at_unix);
    let accepted_at_label = accepted_at_unix.map(fmt_local);
    // Pickup/delivery time label. Same-day pre-orders show just "HH:MM"
    // (compact, the common case); a different calendar day (rare far-out
    // pre-order) shows "DD.MM. HH:MM" so the date isn't lost. Compared in
    // the shop's local TZ, same as the receipt timestamps.
    let pickup_time_label: Option<String> = scheduled_for_unix.map(|sched_unix| {
        use chrono::{Local, TimeZone};
        let sched = Local.timestamp_opt(sched_unix, 0).single();
        let created = Local.timestamp_opt(created_at_unix, 0).single();
        match (sched, created) {
            (Some(s), Some(c)) if s.date_naive() == c.date_naive() => s.format("%H:%M").to_string(),
            (Some(s), _) => s.format("%d.%m. %H:%M").to_string(),
            (None, _) => String::new(),
        }
    });

    Ok(OrderForKitchen {
        order_id: OrderId(hasher.finish()),
        display_number,
        // Full server-assigned identifier, same string the admin UI
        // shows in /admin/orders and the customer sees in their
        // confirmation email. Receipts and admin grep the same value.
        display_label: order_number,
        created_at_unix,
        created_at_label,
        channel,
        customer: Customer {
            name: contact_name,
            phone: Some(contact_phone),
        },
        items,
        subtotal_cents: subtotal_cents.max(0) as u32,
        delivery_fee_cents: delivery_fee_cents.max(0) as u32,
        total_cents: total_cents.max(0) as u32,
        payment,
        note: notes.filter(|s| !s.trim().is_empty()),
        shop_name: shop_name.to_string(),
        accepted_at_unix,
        accepted_at_label,
        qr_url,
        is_test_mode,
        voucher_code,
        voucher_discount_cents: voucher_discount_cents.max(0) as u32,
        pickup_time_label,
    })
}
