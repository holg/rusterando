use std::collections::VecDeque;
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use kitchen_protocol::receipt::ReceiptProgram;
use kitchen_protocol::{ClientMessage, KitchenEvent, SeqId, ServerMessage, FRAME_MAX_BYTES};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::idempotency::IdempotencyCache;
use crate::link::{emit, UiEvent, UiLink};
use crate::printer::Printer;

/// How often a failing printer is retried while receipts are waiting.
const PRINT_RETRY: Duration = Duration::from_secs(5);

/// Run the kitchen-channel client forever. `ui` is the display terminal's
/// link (the `kds` build): it subscribes to the board, forwards touch
/// actions, and lets the process start without a printer. `None` = the
/// headless Pi printer, behaving as it always has.
pub async fn run(cfg: Config, mut ui: Option<UiLink>) -> Result<()> {
    let printer = if ui.is_some() {
        // The screen must come up with the printer off; receipts wait in
        // the print queue until it answers.
        let printer = Printer::new(&cfg.printer_path);
        match printer.probe() {
            Ok(()) => {
                info!(printer = %printer.describe(), "printer device opened");
                emit(&ui, UiEvent::Printer(Ok(printer.describe())));
            }
            Err(e) => {
                warn!(printer = %printer.describe(), error = %format!("{e:#}"), "printer not available yet");
                emit(&ui, UiEvent::Printer(Err(format!("{e:#}"))));
            }
        }
        printer
    } else {
        let printer = Printer::open(&cfg.printer_path)
            .with_context(|| format!("open printer {}", cfg.printer_path.display()))?;
        info!("printer device opened");
        printer
    };

    let mut idempotency =
        IdempotencyCache::load(&cfg.state_dir).context("load idempotency cache")?;
    let mut rasters =
        crate::raster_cache::RasterCache::load(&cfg.state_dir).context("load raster cache")?;
    let mut queue = PrintQueue::default();

    let mut backoff = cfg.reconnect_min_ms;
    loop {
        let result = connect_and_run(
            &cfg,
            &printer,
            &mut idempotency,
            &mut rasters,
            &mut queue,
            &mut ui,
        )
        .await;
        emit(&ui, UiEvent::Connected(false));
        match result {
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

/// Receipts received this session but not printed yet, oldest first. A
/// failing print stays at the front and is retried every [`PRINT_RETRY`]
/// without dropping the session — so a display keeps live board updates
/// while the printer is out of paper. Unacked, so a reconnect replays them
/// from the server's outbox anyway; the queue is cleared per session.
#[derive(Default)]
struct PrintQueue {
    pending: VecDeque<(SeqId, ReceiptProgram)>,
    /// Last print error, to log/report state changes instead of every retry.
    failing: Option<String>,
}

async fn connect_and_run(
    cfg: &Config,
    printer: &Printer,
    idempotency: &mut IdempotencyCache,
    rasters: &mut crate::raster_cache::RasterCache,
    queue: &mut PrintQueue,
    ui: &mut Option<UiLink>,
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

    // Hello — server uses last_seen_seq to replay our gap from the outbox,
    // records version + arch for the admin Pi panel / update targeting, and
    // reads our cached logo hashes so it can send references instead of SVG
    // bytes for logos we already have.
    let hello = ClientMessage::Hello {
        shop_slug: cfg.shop_slug.clone(),
        version: cfg.version.clone(),
        last_seen_seq: idempotency.last_acked_seq(),
        arch: cfg.arch.clone(),
        cached_rasters: rasters.known_hashes(),
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

    if let Some(link) = ui.as_mut() {
        // A display wants the board. Old servers can't decode this and just
        // log + skip it, so it is safe to send unconditionally.
        let body = postcard::to_allocvec(&ClientMessage::BoardSubscribe)
            .context("encode BoardSubscribe")?;
        framed
            .send(Bytes::from(kitchen_protocol::encode_versioned(&body)))
            .await
            .context("send BoardSubscribe")?;
        // Taps made while we were offline refer to a stale screen.
        while link.commands.try_recv().is_ok() {}
        link.emit(UiEvent::Connected(true));
    }
    queue.pending.clear();
    emit(ui, UiEvent::PendingPrints(0));

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

    let result = main_loop(
        cfg,
        &mut framed,
        &out_tx,
        &mut out_rx,
        printer,
        idempotency,
        rasters,
        queue,
        ui,
    )
    .await;
    hb_task.abort();
    result
}

#[allow(clippy::too_many_arguments)]
async fn main_loop(
    cfg: &Config,
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    out_tx: &mpsc::Sender<ClientMessage>,
    out_rx: &mut mpsc::Receiver<ClientMessage>,
    printer: &Printer,
    idempotency: &mut IdempotencyCache,
    rasters: &mut crate::raster_cache::RasterCache,
    queue: &mut PrintQueue,
    ui: &mut Option<UiLink>,
) -> Result<()> {
    let mut retry = tokio::time::interval(PRINT_RETRY);
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            // Inbound: server → us
            frame = framed.next() => {
                let bytes = match frame {
                    Some(Ok(b)) => b,
                    Some(Err(e)) => return Err(e).context("read frame from server"),
                    None => anyhow::bail!("server closed connection"),
                };
                // Strip the 2-byte version header. A major mismatch
                // here means the server is on an incompatible schema
                // generation — there is nothing the printer can do
                // beyond logging it and dropping the frame on the
                // floor. We deliberately do NOT bail the session,
                // because the server's outbox keeps the row pending,
                // re-replays it on every reconnect, and the result is
                // an infinite hot-loop. Treat it as "skip + keep going"
                // — the loop just falls through to wait for the next
                // frame.
                let postcard_bytes = match kitchen_protocol::decode_versioned(&bytes) {
                    Ok((_minor, inner)) => inner,
                    Err(e) => {
                        warn!(error = %e, frame_len = bytes.len(),
                            "incompatible envelope, dropping frame");
                        continue;
                    }
                };
                // Decode failure: don't try to be clever. A
                // partial/legacy decode risks printing GARBAGE
                // (mis-aligned bytes silently mapped into wrong
                // fields), which is worse than skipping. Best we
                // can do is peel the seq_id off the front (postcard
                // u64 varint at offset 0) so the server stops
                // replaying this row, log the skip, and move on.
                // The session does NOT die; the next frame may be
                // fine.
                let msg: ServerMessage = match postcard::from_bytes(postcard_bytes) {
                    Ok(m) => m,
                    Err(e) => {
                        if let Some(seq) = peel_seq_id_prefix(postcard_bytes) {
                            warn!(error = %e, seq_id = %seq,
                                "ServerMessage body decode failed (incompatible schema); skipping + acking to drain outbox");
                            out_tx.send(ClientMessage::Ack(seq)).await.ok();
                        } else {
                            warn!(error = %e, frame_len = postcard_bytes.len(),
                                "ServerMessage body decode failed AND seq_id unrecoverable; leaving row pending");
                        }
                        continue;
                    }
                };

                // Board snapshots are ephemeral (BOARD_SEQ, never in the
                // outbox): straight to the screen, before the idempotency
                // cache — whose seq 0 entry from an update offer would
                // otherwise swallow them — and never acked.
                match msg.event {
                    KitchenEvent::Board(snapshot) => {
                        emit(ui, UiEvent::Board(snapshot));
                        continue;
                    }
                    KitchenEvent::Shop(panel) => {
                        emit(ui, UiEvent::Shop(panel));
                        continue;
                    }
                    KitchenEvent::Alert { title, text } => {
                        emit(ui, UiEvent::Alert { title, text });
                        continue;
                    }
                    _ => {}
                }

                if idempotency.contains(msg.seq_id) {
                    info!(seq_id = %msg.seq_id, "duplicate (replay) — acking without printing");
                    out_tx.send(ClientMessage::Ack(msg.seq_id)).await.ok();
                    continue;
                }

                match &msg.event {
                    // The server is the Bon-Designer: the event is a fully
                    // composed ReceiptProgram. We just execute it — no theme,
                    // no assets, no layout logic on the Pi.
                    // Display-only: leave it unacked for the real printer.
                    KitchenEvent::Print(program) if printer.is_display_only() => {
                        debug!(seq_id = %msg.seq_id, order = program.display_number,
                            "display-only: not printing");
                    }
                    KitchenEvent::Print(program) => {
                        // Queue behind anything still waiting so receipts
                        // come out in order. A failure leaves it unacked at
                        // the front: the retry tick below tries again, and
                        // the server's outbox still holds it for a replay.
                        if !queue.pending.iter().any(|(s, _)| *s == msg.seq_id) {
                            queue.pending.push_back((msg.seq_id, program.clone()));
                        }
                        flush_prints(queue, printer, idempotency, rasters, out_tx, ui).await?;
                    }

                    // The server offers a newer binary over this trusted
                    // channel. Ack first (so the outbox row drains and we're
                    // not re-offered on every reconnect), then take over the
                    // stream to pull + apply it. On success this process
                    // exits and systemd relaunches the new binary; on any
                    // failure we keep running the current one.
                    KitchenEvent::UpdateOffer {
                        component,
                        target_version,
                        sha256,
                        size,
                    } => {
                        idempotency.record(msg.seq_id).context("persist idempotency")?;
                        out_tx.send(ClientMessage::Ack(msg.seq_id)).await.ok();
                        crate::update::apply_update(
                            cfg,
                            framed,
                            *component,
                            target_version,
                            *sha256,
                            *size,
                        )
                        .await?;
                        // If apply_update returned Ok without exiting, it
                        // declined the offer (already current / not newer).
                        // Nothing else to do; keep serving.
                    }

                    // A stray chunk outside an active transfer — ignore. The
                    // transfer is driven synchronously inside apply_update,
                    // which reads its own chunks; we only get here if the
                    // server sent one unsolicited.
                    KitchenEvent::ArtifactChunk { .. } => {
                        out_tx.send(ClientMessage::Ack(msg.seq_id)).await.ok();
                        debug!(seq_id = %msg.seq_id, "unexpected ArtifactChunk outside transfer — acked + ignored");
                    }

                    // Handled above, before the idempotency check.
                    KitchenEvent::Board(_) | KitchenEvent::Shop(_) | KitchenEvent::Alert { .. } => {}
                }
            }

            // Retry a failing printer while receipts are waiting.
            _ = retry.tick(), if !queue.pending.is_empty() => {
                flush_prints(queue, printer, idempotency, rasters, out_tx, ui).await?;
            }

            // Touch actions from the display → server.
            cmd = recv_command(ui) => {
                let body = postcard::to_allocvec(&cmd).context("encode UI command")?;
                let bytes = kitchen_protocol::encode_versioned(&body);
                if let Err(e) = framed.send(Bytes::from(bytes)).await {
                    return Err(e).context("send UI command");
                }
                debug!(?cmd, "sent UI command");
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

/// Print queued receipts in order until one fails. Each success is recorded
/// in the idempotency cache BEFORE the ack (so a crash between the two can't
/// double-print) and dequeued.
async fn flush_prints(
    queue: &mut PrintQueue,
    printer: &Printer,
    idempotency: &mut IdempotencyCache,
    rasters: &mut crate::raster_cache::RasterCache,
    out_tx: &mpsc::Sender<ClientMessage>,
    ui: &Option<UiLink>,
) -> Result<()> {
    while let Some((seq_id, program)) = queue.pending.front() {
        let seq_id = *seq_id;
        match printer.print_program(program, rasters) {
            Ok(()) => {
                let display_number = program.display_number;
                idempotency.record(seq_id).context("persist idempotency")?;
                out_tx.send(ClientMessage::Ack(seq_id)).await.ok();
                info!(seq_id = %seq_id, order = display_number, "printed + acked");
                queue.pending.pop_front();
                if queue.failing.take().is_some() {
                    info!("printer recovered");
                    emit(ui, UiEvent::Printer(Ok(printer.describe())));
                }
            }
            Err(e) => {
                let reason = format!("{e:#}");
                if queue.failing.as_deref() != Some(reason.as_str()) {
                    error!(seq_id = %seq_id, error = %reason, "print failed — retrying every {}s", PRINT_RETRY.as_secs());
                    emit(ui, UiEvent::Printer(Err(reason.clone())));
                }
                queue.failing = Some(reason);
                break;
            }
        }
    }
    emit(ui, UiEvent::PendingPrints(queue.pending.len()));
    Ok(())
}

/// Next UI command; pending forever without a UI (or once it's gone).
async fn recv_command(ui: &mut Option<UiLink>) -> ClientMessage {
    if let Some(link) = ui.as_mut() {
        if let Some(cmd) = link.commands.recv().await {
            return cmd;
        }
    }
    std::future::pending().await
}

/// Try to peel just the `seq_id` field off the front of a postcard-encoded
/// `ServerMessage` byte slice — used by the body-decode-failure path so we
/// can ACK an unprintable frame and stop the server from replaying it
/// forever.
///
/// `ServerMessage` is `{ seq_id: SeqId(u64), event: KitchenEvent }`.
/// Postcard encodes `u64` as a LEB128-style varint (max 10 bytes; each
/// byte's MSB = continuation flag, low 7 bits = payload). We only need the
/// first field, so as soon as the varint terminates we stop reading.
/// `None` means the bytes ran out before the varint terminated — the frame
/// is too garbled to recover anything useful from.
fn peel_seq_id_prefix(bytes: &[u8]) -> Option<SeqId> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    for &b in bytes.iter().take(10) {
        value |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(SeqId(value));
        }
        shift += 7;
    }
    None
}
