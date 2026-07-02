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
        arch,
        cached_rasters,
    } = hello
    else {
        anyhow::bail!("first message was not Hello");
    };
    info!(%peer, %shop_slug, %version, %arch, ?last_seen_seq,
        cached = cached_rasters.len(), "kitchen client hello");
    // Logo hashes the Pi already has cached → we send refs (empty svg) for
    // these and full SVG bytes otherwise. Set for O(1) lookup at send time.
    let cached: std::collections::HashSet<[u8; 32]> = cached_rasters.into_iter().collect();

    // Record this Pi's running version + arch so the admin panel can show
    // it (and so a version skew is never invisible again). Best-effort.
    if let Err(e) = upsert_client(&channel.db, &shop_slug, &version, &arch).await {
        warn!(error = %e, "kitchen_clients upsert failed");
    }

    // ===== Self-update handshake (admin-gated) =====
    // If an admin armed an update for this shop and we hold a newer target
    // binary for the Pi's arch, offer + stream it BEFORE the print loop.
    // The Pi drives the transfer synchronously and won't print during it;
    // on success it installs + exits + reconnects on the new binary. We do
    // this on the un-split `framed` so request/response stays simple.
    if let Err(e) = maybe_serve_update(&channel.db, &mut framed, &shop_slug, &version, &arch).await
    {
        // A failed update must not kill the session — fall through to normal
        // printing so the Pi keeps working on its current binary.
        warn!(error = %e, "update handshake error (continuing to serve prints)");
    }

    // ===== Step 2: replay unacked from outbox =====
    let replay_from = last_seen_seq.unwrap_or(SeqId(0));
    let backlog = channel.unacked_since(replay_from).await?;
    info!(%peer, count = backlog.len(), from = %replay_from, "replaying unacked events");

    // ===== Step 3: subscribe to live broadcast THEN replay =====
    // Subscribe before sending the backlog so we don't miss anything inserted
    // between the outbox query and the live-stream join.
    let mut rx = channel.subscribe();

    for msg in &backlog {
        let msg = ref_cached_rasters(msg.clone(), &cached);
        let body = postcard::to_allocvec(&msg).context("encode replay")?;
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
    let shop_for_hb = shop_slug.clone();
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
                ClientMessage::Heartbeat => {
                    // Keep `last_seen_at` fresh so the admin Pi panel shows
                    // a connected idle Pi as "online", not stale-since-Hello.
                    let _ = sqlx::query(
                        "UPDATE kitchen_clients SET last_seen_at = strftime('%s','now') \
                         WHERE shop_slug = ?1",
                    )
                    .bind(&shop_for_hb)
                    .execute(&chan_for_acks.db)
                    .await;
                    debug!("heartbeat");
                }
                ClientMessage::Hello { .. } => {
                    warn!("unexpected mid-session Hello; ignoring");
                }
                // These belong to the post-Hello update handshake, which
                // runs synchronously before this reader starts. Seeing one
                // here means an out-of-band/late frame — ignore it.
                ClientMessage::FetchArtifact { .. } | ClientMessage::UpdateResult { .. } => {
                    debug!("update-handshake message outside handshake; ignoring");
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
                let msg = ref_cached_rasters(msg, &cached);
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
// Pi version tracking + in-band self-update (admin-gated)
// ---------------------------------------------------------------------------

/// Upsert the per-shop client row on Hello: running version, arch, last
/// seen, and the wire-protocol major. Visible in the admin Pi panel so a
/// version skew is never silent, and read by the deploy guard.
///
/// `protocol_major` is the SERVER's `SCHEMA_VERSION_MAJOR`: a Hello only
/// reaches this point if the Pi's envelope major matched ours
/// (`decode_versioned` rejects a mismatch before we get here), so the
/// stored value is the major this Pi successfully spoke — i.e. the
/// last-known-good protocol major for the shop.
async fn upsert_client(
    db: &SqlitePool,
    shop_slug: &str,
    version: &str,
    arch: &str,
) -> anyhow::Result<()> {
    let protocol_major = kitchen_protocol::SCHEMA_VERSION_MAJOR as i64;
    sqlx::query(
        "INSERT INTO kitchen_clients (shop_slug, version, arch, last_seen_at, protocol_major) \
         VALUES (?1, ?2, ?3, strftime('%s','now'), ?4) \
         ON CONFLICT(shop_slug) DO UPDATE SET \
           version = excluded.version, \
           arch = excluded.arch, \
           last_seen_at = excluded.last_seen_at, \
           protocol_major = excluded.protocol_major",
    )
    .bind(shop_slug)
    .bind(version)
    .bind(arch)
    .bind(protocol_major)
    .execute(db)
    .await
    .context("upsert kitchen_clients")?;
    Ok(())
}

/// If an admin armed an update for this shop AND we hold a newer target
/// binary for the Pi's `arch`, run the full offer→stream handshake on
/// `framed`. The Pi verifies + installs + exits on success and reports an
/// `UpdateResult` either way; we clear the armed flag + record the outcome.
///
/// Returns `Ok(())` whether or not an update happened. Errors only on a
/// stream failure the caller should treat as "continue serving".
async fn maybe_serve_update(
    db: &SqlitePool,
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    shop_slug: &str,
    running_version: &str,
    arch: &str,
) -> anyhow::Result<()> {
    // Armed?
    let armed: Option<(i64,)> =
        sqlx::query_as("SELECT update_armed FROM kitchen_clients WHERE shop_slug = ?1")
            .bind(shop_slug)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    if armed.map(|(a,)| a).unwrap_or(0) == 0 {
        return Ok(());
    }

    // Do we hold a newer target for this arch?
    let store = crate::printer_artifacts::store();
    let Some(art) = store.artifact(arch) else {
        info!(%arch, "update armed but no artifact for arch — skipping");
        return Ok(());
    };
    if art.version == running_version {
        info!(version = %running_version, "update armed but Pi already on target — disarming");
        disarm(db, shop_slug).await;
        return Ok(());
    }

    info!(%shop_slug, from = %running_version, to = %art.version, size = art.size(), "serving Pi self-update");

    // ===== Offer =====
    send_event(
        framed,
        KitchenEvent::UpdateOffer {
            component: kitchen_protocol::Component::PrinterBinary,
            target_version: art.version.clone(),
            sha256: art.sha256,
            size: art.size(),
        },
    )
    .await
    .context("send UpdateOffer")?;

    // ===== Serve the Pi's FetchArtifact requests until it has the whole
    // file, then read its UpdateResult. The Pi may also send an Ack for
    // the offer's outbox row — we skip non-update frames here. =====
    let chunk_sz = crate::printer_artifacts::CHUNK_BYTES;
    loop {
        let frame = match framed.next().await {
            Some(Ok(b)) => b,
            Some(Err(e)) => return Err(e).context("read during update"),
            None => anyhow::bail!("Pi closed during update"),
        };
        let inner = match kitchen_protocol::decode_versioned(&frame) {
            Ok((_m, i)) => i,
            Err(_) => continue,
        };
        let msg: ClientMessage = match postcard::from_bytes(inner) {
            Ok(m) => m,
            Err(_) => continue,
        };
        match msg {
            ClientMessage::FetchArtifact { offset, .. } => {
                let start = offset as usize;
                if start > art.bytes.len() {
                    anyhow::bail!("Pi requested offset past artifact end");
                }
                let end = (start + chunk_sz).min(art.bytes.len());
                let last = end >= art.bytes.len();
                send_event(
                    framed,
                    KitchenEvent::ArtifactChunk {
                        component: kitchen_protocol::Component::PrinterBinary,
                        offset,
                        bytes: art.bytes[start..end].to_vec(),
                        last,
                    },
                )
                .await
                .context("send ArtifactChunk")?;
            }
            ClientMessage::UpdateResult { to, ok, error, .. } => {
                record_update_result(db, shop_slug, &to, ok, &error).await;
                if ok {
                    info!(%shop_slug, %to, "Pi self-update succeeded — it will reconnect on the new binary");
                } else {
                    warn!(%shop_slug, %to, %error, "Pi self-update failed — kept current binary");
                }
                // Disarm regardless: a failed update shouldn't re-offer in a
                // tight reconnect loop. The admin re-arms after fixing.
                disarm(db, shop_slug).await;
                return Ok(());
            }
            // Ack for the offer's outbox row, heartbeat, etc. — ignore and
            // keep serving chunks.
            _ => continue,
        }
    }
}

async fn send_event(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    event: KitchenEvent,
) -> anyhow::Result<()> {
    // seq_id 0: update frames are out-of-band w.r.t. the print outbox.
    let msg = ServerMessage {
        seq_id: SeqId(0),
        event,
    };
    let body = postcard::to_allocvec(&msg).context("encode update event")?;
    let frame = kitchen_protocol::encode_versioned(&body);
    framed
        .send(Bytes::from(frame))
        .await
        .context("send update frame")?;
    Ok(())
}

async fn disarm(db: &SqlitePool, shop_slug: &str) {
    let _ = sqlx::query("UPDATE kitchen_clients SET update_armed = 0 WHERE shop_slug = ?1")
        .bind(shop_slug)
        .execute(db)
        .await;
}

/// Rewrite a `ServerMessage` so any `RasterSvg` whose hash the Pi already
/// has cached becomes a *reference* (empty `svg`) — the Pi uses its cache,
/// and we don't re-send the bytes. Hashes the Pi hasn't advertised keep
/// their full SVG. Leaves non-Print events untouched.
fn ref_cached_rasters(
    mut msg: ServerMessage,
    cached: &std::collections::HashSet<[u8; 32]>,
) -> ServerMessage {
    use kitchen_protocol::receipt::ReceiptLine;
    if cached.is_empty() {
        return msg;
    }
    if let KitchenEvent::Print(prog) = &mut msg.event {
        for line in &mut prog.lines {
            if let ReceiptLine::RasterSvg { hash, svg, .. } = line {
                if !svg.is_empty() && cached.contains(hash) {
                    svg.clear(); // → reference; Pi resolves from its cache
                }
            }
        }
    }
    msg
}

async fn record_update_result(db: &SqlitePool, shop_slug: &str, to: &str, ok: bool, error: &str) {
    let _ = sqlx::query(
        "UPDATE kitchen_clients SET \
           last_update_to = ?2, last_update_ok = ?3, \
           last_update_error = ?4, last_update_at = strftime('%s','now') \
         WHERE shop_slug = ?1",
    )
    .bind(shop_slug)
    .bind(to)
    .bind(if ok { 1 } else { 0 })
    .bind(error)
    .execute(db)
    .await;
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
        let program = self.compose_program(db, order_id, false).await?;
        self.broadcast(KitchenEvent::Print(program)).await?;
        Ok(())
    }

    async fn broadcast_reprint(&self, db: &SqlitePool, order_id: &str) -> anyhow::Result<()> {
        let program = self.compose_program(db, order_id, true).await?;
        self.broadcast(KitchenEvent::Print(program)).await?;
        Ok(())
    }

    async fn broadcast_test_print(
        &self,
        db: &SqlitePool,
        delivery: bool,
        prepaid: bool,
        theme: String,
    ) -> anyhow::Result<()> {
        use rusterando_frontend::pages::settings::ssr as settings;

        let (shop_name, public_url_base) = resolve_brand_and_url(db).await;
        // A representative fake order so the layout is realistic. Marked as
        // test_mode so the *** TEST *** banner makes clear this isn't a real
        // ticket. QR points at the shop's configured target (or homepage).
        let mut dto = sample_order_for_test(&shop_name, &theme, delivery, prepaid);
        dto.qr_url = settings::resolve_qr_url(db, &public_url_base, "TESTDRUCK").await;
        let config = settings::receipt_config(db).await;

        let site_root =
            std::env::var("LEPTOS_SITE_ROOT").unwrap_or_else(|_| "target/site".to_string());
        let provider = crate::bon_designer::provider(&site_root);
        let program = kitchen_protocol::receipt::compose(&dto, false, &config, provider);
        self.broadcast(KitchenEvent::Print(program)).await?;
        Ok(())
    }
}

/// A representative sample `OrderForKitchen` for the Bon-Editor test print.
/// Mirrors the admin preview's sample so the printed paper matches the
/// on-screen preview. `is_test_mode = true` stamps the TEST banner.
fn sample_order_for_test(
    shop_name: &str,
    theme: &str,
    delivery: bool,
    prepaid: bool,
) -> OrderForKitchen {
    let channel = if delivery {
        OrderChannel::Delivery {
            address: DeliveryAddress {
                street: "Hauptstraße 12".into(),
                postal: "63739".into(),
                city: "Aschaffenburg".into(),
                bell: Some("Mustermann".into()),
            },
        }
    } else {
        OrderChannel::Pickup
    };
    let items = vec![
        LineItem {
            qty: 1,
            name: "Pizza Margherita".into(),
            modifications: vec!["extra Käse".into()],
            unit_price_cents: 850,
            category: "Pizza".into(),
            modification_prices_cents: vec![100],
        },
        LineItem {
            qty: 2,
            name: "Pizza Funghi e Prosciutto".into(),
            modifications: vec![],
            unit_price_cents: 1150,
            category: "Pizza".into(),
            modification_prices_cents: vec![],
        },
    ];
    let subtotal_cents = 850 + 100 + 2 * 1150;
    let delivery_fee_cents = if delivery { 150 } else { 0 };
    let voucher_discount_cents = 300;
    let total_cents = subtotal_cents + delivery_fee_cents - voucher_discount_cents;
    let payment = if prepaid {
        PaymentStatus::Prepaid
    } else {
        PaymentStatus::CollectOnDelivery {
            amount_cents: total_cents,
        }
    };
    OrderForKitchen {
        order_id: OrderId(0),
        display_number: 0,
        display_label: "TESTDRUCK".into(),
        created_at_unix: 0,
        created_at_label: "Probedruck".into(),
        channel,
        customer: Customer {
            name: "Maria Mustermann".into(),
            phone: Some("06021 123456".into()),
        },
        items,
        subtotal_cents,
        delivery_fee_cents,
        total_cents,
        payment,
        note: Some("Probedruck aus dem Bon-Editor.".into()),
        shop_name: shop_name.to_string(),
        accepted_at_unix: None,
        accepted_at_label: None,
        qr_url: None,
        is_test_mode: true,
        voucher_code: "WILLKOMMEN".into(),
        voucher_discount_cents,
        pickup_time_label: Some("18:30".into()),
        printer_theme: theme.to_string(),
    }
}

impl KitchenChannel {
    /// The server-side Bon-Designer step: build the order DTO, run the
    /// shared `receipt::compose` with the file-backed raster provider to
    /// resolve the theme into inline 1-bit bytes, and return a complete
    /// `ReceiptProgram` for the Pi to execute. The Pi holds no themes or
    /// assets — all artwork is resolved here.
    async fn compose_program(
        &self,
        db: &SqlitePool,
        order_id: &str,
        is_reprint: bool,
    ) -> anyhow::Result<kitchen_protocol::receipt::ReceiptProgram> {
        use rusterando_frontend::pages::settings::ssr as settings;

        let (shop_name, public_url_base) = resolve_brand_and_url(db).await;
        let mut dto = build_order_for_kitchen(db, order_id, &shop_name, &public_url_base).await?;

        // Apply the admin-editable Bon-Editor config: resolve the QR target
        // per receipt_qr_mode (order-url vs static), and pass the rest of
        // the config (header override, footer, block toggles) into compose.
        dto.qr_url = settings::resolve_qr_url(db, &public_url_base, order_id).await;
        let config = settings::receipt_config(db).await;

        // leptos sets LEPTOS_SITE_ROOT at runtime; fall back to the dev path.
        let site_root =
            std::env::var("LEPTOS_SITE_ROOT").unwrap_or_else(|_| "target/site".to_string());
        let provider = crate::bon_designer::provider(&site_root);
        Ok(kitchen_protocol::receipt::compose(
            &dto, is_reprint, &config, provider,
        ))
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
                scheduled_for AS scheduled_for_raw
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
    // Scheduled fulfillment time. NULL (= ASAP) → None. Stored as a naive
    // LOCAL wall-clock string ("YYYY-MM-DD HH:MM:SS", Europe/Berlin) at
    // place_order time — NOT UTC like created_at. So we must read it raw and
    // NEVER round-trip it through strftime('%s')→Local: that treats the
    // string as UTC and re-localizes, adding the +2h offset (the bug that
    // printed "18:30" pre-orders as "20:30"). Every other reader (confirm
    // page, admin history/orders, kitchen/driver boards) also splits this
    // string directly; we match them.
    let scheduled_for_raw: Option<String> = row
        .try_get::<Option<String>, _>("scheduled_for_raw")
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty());

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
                oi.options_json, oi.extras_json, oi.removals_json, oi.unit_price_cents,
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
        let removals_json: Option<String> = r.get("removals_json");
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
        // Removed ingredients ("ohne Käse") — always free, printed on their
        // own line like extras. `label` is already the bare ingredient name.
        if let Some(j) = removals_json.as_deref() {
            if let Ok(removals) =
                serde_json::from_str::<Vec<rusterando_shared::models::CartRemoval>>(j)
            {
                for rem in removals {
                    modifications.push(format!("ohne {}", rem.label));
                    modification_prices.push(0);
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

    // Scheduled-time line for the receipt:
    //   * scheduled_for set (pre-order)  → use the stored LOCAL wall-clock
    //     directly. It's already Europe/Berlin, so we format from the naive
    //     string WITHOUT any epoch/timezone round-trip.
    //   * ASAP DELIVERY                  → created_at + the zone's ETA, so
    //     the kitchen still sees a target ("Lieferung 20:15") instead of
    //     a blank. The zone id is snapshotted in delivery_address_json;
    //     its eta_minutes lives on delivery_zones. This IS a real epoch
    //     (created_at is UTC), so epoch→Local is correct here.
    //   * ASAP pickup                    → None (no meaningful ETA; the
    //     Eingang line implies "now").
    //
    // Same day → "HH:MM"; another day → "DD.MM. HH:MM" — for both branches.
    let created_naive = {
        use chrono::{Local, TimeZone};
        Local.timestamp_opt(created_at_unix, 0).single()
    };
    let pickup_time_label: Option<String> = if let Some(raw) = scheduled_for_raw {
        // Parse the stored naive local string "YYYY-MM-DD HH:MM:SS".
        chrono::NaiveDateTime::parse_from_str(raw.trim(), "%Y-%m-%d %H:%M:%S")
            .ok()
            .map(|due| {
                let same_day = created_naive
                    .map(|c| c.date_naive() == due.date())
                    .unwrap_or(false);
                if same_day {
                    due.format("%H:%M").to_string()
                } else {
                    due.format("%d.%m. %H:%M").to_string()
                }
            })
    } else if order_type == "delivery" {
        // Zone id from the snapshot JSON → eta_minutes → an ASAP-delivery ETA.
        let zone_id = delivery_address_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| {
                v.get("zone_id")
                    .and_then(|z| z.as_str())
                    .map(|s| s.to_string())
            })
            .filter(|s| !s.is_empty());
        let eta_minutes: Option<i64> = if let Some(zid) = zone_id {
            sqlx::query_scalar::<_, i64>("SELECT eta_minutes FROM delivery_zones WHERE id = ?1")
                .bind(zid)
                .fetch_optional(db)
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        eta_minutes.and_then(|eta| {
            use chrono::{Local, TimeZone};
            let due = Local.timestamp_opt(created_at_unix + eta * 60, 0).single()?;
            let same_day = created_naive
                .map(|c| c.date_naive() == due.date_naive())
                .unwrap_or(false);
            Some(if same_day {
                due.format("%H:%M").to_string()
            } else {
                due.format("%d.%m. %H:%M").to_string()
            })
        })
    } else {
        None
    };

    // Receipt layout theme (app_settings.printer_theme). Empty / missing
    // → the Pi renders the default layout. Read live so an admin change
    // applies to the next order without a server restart.
    let printer_theme: String =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'printer_theme'")
            .fetch_optional(db)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();

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
        printer_theme,
    })
}
