use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use kitchen_protocol::{ClientMessage, KitchenEvent, ServerMessage, FRAME_MAX_BYTES};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::idempotency::IdempotencyCache;
use crate::printer::Printer;

pub async fn run(cfg: Config) -> Result<()> {
    let printer = Printer::open(&cfg.printer_path)
        .with_context(|| format!("open printer {}", cfg.printer_path.display()))?;
    info!("printer device opened");

    let mut idempotency = IdempotencyCache::load(&cfg.state_dir)
        .context("load idempotency cache")?;

    let mut backoff = cfg.reconnect_min_ms;
    loop {
        match connect_and_run(&cfg, &printer, &mut idempotency).await {
            Ok(()) => {
                info!("session ended cleanly, reconnecting soon");
                backoff = cfg.reconnect_min_ms;
            }
            Err(e) => {
                warn!(error = %e, "session error, backing off {backoff}ms");
                tokio::time::sleep(Duration::from_millis(backoff)).await;
                backoff = (backoff * 2).min(cfg.reconnect_max_ms);
            }
        }
    }
}

async fn connect_and_run(
    cfg: &Config,
    printer: &Printer,
    idempotency: &mut IdempotencyCache,
) -> Result<()> {
    let stream = TcpStream::connect(&cfg.server_addr)
        .await
        .with_context(|| format!("connect {}", cfg.server_addr))?;
    stream.set_nodelay(true).ok();
    info!(addr = %cfg.server_addr, "connected to kitchen channel");

    let codec = LengthDelimitedCodec::builder()
        .max_frame_length(FRAME_MAX_BYTES)
        .new_codec();
    let mut framed = Framed::new(stream, codec);

    // Hello — server uses last_seen_seq to replay our gap from the outbox.
    let hello = ClientMessage::Hello {
        shop_slug: cfg.shop_slug.clone(),
        version: cfg.version.clone(),
        last_seen_seq: idempotency.last_acked_seq(),
    };
    // Wrap with the kitchen-protocol version envelope so the server
    // can major-mismatch-skip cleanly. Crate version on both sides
    // must match major for this to round-trip; minor differences log
    // a warning but proceed.
    let hello_body = postcard::to_allocvec(&hello).context("encode Hello")?;
    let hello_bytes = kitchen_protocol::encode_versioned(&hello_body);
    framed
        .send(Bytes::from(hello_bytes))
        .await
        .context("send Hello")?;

    // mpsc for outgoing client messages (acks, heartbeats). The select! loop
    // below pulls from this and forwards to the framed sink, so all writes
    // happen from one place.
    let (out_tx, mut out_rx) = mpsc::channel::<ClientMessage>(32);

    // Heartbeat task
    let heartbeat_tx = out_tx.clone();
    let hb_task = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if heartbeat_tx.send(ClientMessage::Heartbeat).await.is_err() {
                break;
            }
        }
    });

    let result = main_loop(&mut framed, &out_tx, &mut out_rx, printer, idempotency).await;
    hb_task.abort();
    result
}

async fn main_loop(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    out_tx: &mpsc::Sender<ClientMessage>,
    out_rx: &mut mpsc::Receiver<ClientMessage>,
    printer: &Printer,
    idempotency: &mut IdempotencyCache,
) -> Result<()> {
    loop {
        tokio::select! {
            // Inbound: server → us
            frame = framed.next() => {
                let bytes = match frame {
                    Some(Ok(b)) => b,
                    Some(Err(e)) => return Err(e).context("read frame from server"),
                    None => anyhow::bail!("server closed connection"),
                };
                // Strip the 2-byte version header before postcard.
                // On major mismatch, bail this session and reconnect
                // with backoff — the server's matching skip-on-decode
                // means the same poison frame won't be replayed.
                let (_minor, postcard_bytes) =
                    kitchen_protocol::decode_versioned(&bytes)
                        .context("decode ServerMessage envelope")?;
                let msg: ServerMessage = postcard::from_bytes(postcard_bytes)
                    .context("decode ServerMessage body")?;

                if idempotency.contains(msg.seq_id) {
                    info!(seq_id = %msg.seq_id, "duplicate (replay) — acking without printing");
                    out_tx.send(ClientMessage::Ack(msg.seq_id)).await.ok();
                    continue;
                }

                let (order, is_reprint) = match &msg.event {
                    KitchenEvent::NewOrder(o) => (o, false),
                    KitchenEvent::Reprint(o) => (o, true),
                };

                if let Err(e) = printer.print_order(order, is_reprint) {
                    // Don't ack. The server's outbox keeps this pending; on
                    // reconnect we'll be replayed. Bail this session so we
                    // reconnect with backoff and don't tight-loop a broken
                    // printer.
                    error!(seq_id = %msg.seq_id, error = %e, "print failed");
                    anyhow::bail!("print failed for seq {}: {e}", msg.seq_id);
                }

                idempotency.record(msg.seq_id).context("persist idempotency")?;
                out_tx.send(ClientMessage::Ack(msg.seq_id)).await.ok();
                info!(seq_id = %msg.seq_id, order = order.display_number, "printed + acked");
            }

            // Outbound: us → server
            outbound = out_rx.recv() => {
                let Some(msg) = outbound else { continue; };
                let body = postcard::to_allocvec(&msg).context("encode outbound")?;
                let bytes = kitchen_protocol::encode_versioned(&body);
                if let Err(e) = framed.send(Bytes::from(bytes)).await {
                    return Err(e).context("send outbound");
                }
                debug!(?msg, "sent");
            }
        }
    }
}
