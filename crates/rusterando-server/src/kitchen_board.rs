//! Kitchen display board over the kitchen channel (kitchen-protocol 2.1+).
//!
//! A display terminal (the STM32MP135 kitchen screen — `rusterando-printer`
//! built with a `kds-*` feature) sends `BoardSubscribe` after Hello. From
//! then on [`run_feed`] pushes a full [`BoardSnapshot`] whenever the set of
//! active orders changes, and [`advance`] moves orders between columns with
//! the same side effects as the web kitchen board's `kitchen_advance`:
//! status UPDATE, live `Status` push, and the "Status: …" line in the
//! customer's message log.
//!
//! Change detection is one cheap signature query — `(id, status,
//! updated_at)` of the active orders — re-run on any trigger:
//!   * a print broadcast on the kitchen channel (cash order placed, card
//!     order paid: exactly the moments an order appears),
//!   * a LiveHub `Status` event (changes from the web boards),
//!   * a poke after a `BoardAdvance` from this terminal,
//!   * a slow safety tick for writers that emit nothing live (tours,
//!     Stripe failure → cancelled).
//!
//! Order DTOs are rebuilt only for orders whose signature changed, so an
//! idle board costs one indexed SELECT per tick.
//!
//! Next to the board the feed keeps the terminal's "Öffnungszeiten" page
//! current: a [`ShopPanel`] (the /admin/hours status + quick controls,
//! computed by the same `admin::hours::ssr` code as the web page), resent
//! when it changes — LiveHub `ShopStatus` events, the tick for schedule
//! edges — and after every [`shop_action`] with its result.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kitchen_protocol::{
    board_transition_allowed, BoardOrder, BoardSnapshot, BoardStatus, KitchenEvent, ServerMessage,
    ShopAction, ShopLight, ShopPanel, BOARD_SEQ,
};
use rusterando_frontend::live::LiveHub;
use rusterando_frontend::pages::admin::hours::ssr::{shop_status, ShopControl};
use rusterando_frontend::pages::settings::OrdersPausedHandle;
use rusterando_shared::models::{LiveEvent, LiveKind, ShopLevel};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, mpsc, Notify};
use tracing::{debug, warn};

/// Re-check even without a trigger, for status writers that don't publish
/// on the LiveHub (start/cancel tour, Stripe payment failure).
const SAFETY_TICK: Duration = Duration::from_secs(10);
/// Let a burst of triggers (UPDATE + Status + Message for one tap) settle
/// into a single snapshot.
const COALESCE: Duration = Duration::from_millis(150);

/// The board's statuses. `pending_payment` stays off, like the web kitchen
/// board: a card order reaches the kitchen once Stripe confirms it.
const ACTIVE_SQL: &str = "SELECT id, status, \
         COALESCE(CAST(strftime('%s', updated_at) AS INTEGER), 0) \
     FROM orders \
     WHERE status IN ('received', 'preparing', 'ready_for_pickup') \
     ORDER BY created_at ASC, id ASC";

/// `(id, status, updated_at_unix)` per active order, in board order.
type Signature = Vec<(String, String, i64)>;

/// Result of the terminal's last shop action, handed from the session's
/// reader to its feed: `Some("")` = done, `Some(msg)` = refused with `msg`.
pub(crate) type ShopNotice = Arc<Mutex<Option<String>>>;

/// Feed one subscribed terminal until its session ends (`out` closes).
pub(crate) async fn run_feed(
    db: SqlitePool,
    live: Option<LiveHub>,
    mut kitchen_rx: broadcast::Receiver<ServerMessage>,
    mut alert_rx: broadcast::Receiver<(String, String)>,
    poke: Arc<Notify>,
    notice: ShopNotice,
    out: mpsc::Sender<ServerMessage>,
) {
    let mut last_panel: Option<ShopPanel> = None;
    let mut live_rx = live.as_ref().map(LiveHub::subscribe);
    let mut cache: HashMap<String, (String, i64, BoardOrder)> = HashMap::new();
    let mut last: Option<Signature> = None;
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + SAFETY_TICK, SAFETY_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        match signature(&db).await {
            Ok(sig) if last.as_ref() != Some(&sig) => {
                let (snapshot, complete) = build_snapshot(&db, &sig, &mut cache).await;
                let n = snapshot.orders.len();
                let msg = ServerMessage {
                    seq_id: BOARD_SEQ,
                    event: KitchenEvent::Board(snapshot),
                };
                if out.send(msg).await.is_err() {
                    return;
                }
                debug!(orders = n, "board snapshot sent");
                // An order whose DTO failed to build was left out; forget the
                // signature so the next trigger retries it.
                last = complete.then_some(sig);
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, "board signature query failed"),
        }

        // Shop panel: resend when it changed, or to answer an action.
        let pending = notice.lock().ok().and_then(|mut n| n.take());
        let mut panel = shop_panel(&db).await;
        if pending.is_some() || last_panel.as_ref() != Some(&panel) {
            last_panel = Some(panel.clone());
            panel.notice = pending.unwrap_or_default();
            let msg = ServerMessage {
                seq_id: BOARD_SEQ,
                event: KitchenEvent::Shop(panel),
            };
            if out.send(msg).await.is_err() {
                return;
            }
            debug!("shop panel sent");
        }

        // Wait for the next trigger that can change the board.
        loop {
            tokio::select! {
                _ = out.closed() => return,
                _ = tick.tick() => break,
                _ = poke.notified() => break,
                r = kitchen_rx.recv() => match r {
                    Err(broadcast::error::RecvError::Closed) => return,
                    _ => break,
                },
                // Admin "Alarm auslösen": straight through, no refresh needed.
                a = alert_rx.recv() => match a {
                    Ok((title, text)) => {
                        let msg = ServerMessage {
                            seq_id: BOARD_SEQ,
                            event: KitchenEvent::Alert { title, text },
                        };
                        if out.send(msg).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                r = recv_live(&mut live_rx) => match r {
                    Ok(LiveEvent { kind: LiveKind::Status(_) | LiveKind::ShopStatus { .. }, .. }) => break,
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => break,
                    Err(broadcast::error::RecvError::Closed) => live_rx = None,
                },
            }
        }
        tokio::time::sleep(COALESCE).await;
    }
}

/// `recv` on an optional receiver; pending forever when there is none.
async fn recv_live(
    rx: &mut Option<broadcast::Receiver<LiveEvent>>,
) -> Result<LiveEvent, broadcast::error::RecvError> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

async fn signature(db: &SqlitePool) -> sqlx::Result<Signature> {
    sqlx::query_as(ACTIVE_SQL).fetch_all(db).await
}

/// Assemble the snapshot, reusing cached DTOs whose `(status, updated_at)`
/// is unchanged. Returns `false` alongside when some order had to be left
/// out because its DTO failed to build.
async fn build_snapshot(
    db: &SqlitePool,
    sig: &Signature,
    cache: &mut HashMap<String, (String, i64, BoardOrder)>,
) -> (BoardSnapshot, bool) {
    let mut orders = Vec::with_capacity(sig.len());
    let mut seen = HashSet::with_capacity(sig.len());
    let mut complete = true;
    for (id, status, updated) in sig {
        let Some(board_status) = BoardStatus::from_db(status) else {
            continue;
        };
        seen.insert(id.as_str());
        let fresh = matches!(cache.get(id), Some((s, u, _)) if s == status && u == updated);
        if !fresh {
            // No brand line or QR — the screen shows neither.
            match crate::kitchen::build_order_for_kitchen(db, id, "", "").await {
                Ok(order) => {
                    let entry = BoardOrder {
                        id: id.clone(),
                        status: board_status,
                        order,
                    };
                    cache.insert(id.clone(), (status.clone(), *updated, entry));
                }
                Err(e) => {
                    warn!(order_id = %id, error = %e, "board: build order failed, left out");
                    cache.remove(id);
                    complete = false;
                    continue;
                }
            }
        }
        if let Some((_, _, entry)) = cache.get(id) {
            orders.push(entry.clone());
        }
    }
    cache.retain(|id, _| seen.contains(id.as_str()));

    let server_now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    (
        BoardSnapshot {
            server_now_unix,
            orders,
        },
        complete,
    )
}

/// Apply a terminal's `BoardAdvance`. `Ok(false)` = nothing to do: the order
/// is gone or no longer in `from` (someone moved it on the web board first);
/// the snapshot that follows shows the terminal the real state.
pub(crate) async fn advance(
    db: &SqlitePool,
    live: Option<&LiveHub>,
    order_id: &str,
    from: BoardStatus,
    to: BoardStatus,
) -> anyhow::Result<bool> {
    let order_type: Option<String> =
        sqlx::query_scalar("SELECT order_type FROM orders WHERE id = ?1")
            .bind(order_id)
            .fetch_optional(db)
            .await?;
    let Some(order_type) = order_type else {
        return Ok(false);
    };
    if !board_transition_allowed(from, to, order_type == "delivery") {
        anyhow::bail!("board move {from:?} → {to:?} not allowed for a {order_type} order");
    }

    // Conditional on `from`: a tap based on a stale screen must not undo a
    // change made elsewhere in the meantime.
    let res = sqlx::query(
        "UPDATE orders SET status = ?1, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ?2 AND status = ?3",
    )
    .bind(to.as_db())
    .bind(order_id)
    .bind(from.as_db())
    .execute(db)
    .await?;
    if res.rows_affected() == 0 {
        return Ok(false);
    }

    // Same side effects as kitchen_advance / driver_advance.
    if let Some(hub) = live {
        hub.send(LiveEvent {
            order_id: order_id.to_string(),
            kind: LiveKind::Status(to.as_db().to_string()),
        });
    }
    rusterando_frontend::pages::order::ssr::log_status_change_with(db, live, order_id, to.as_db())
        .await;
    Ok(true)
}

/// The terminal's view of /admin/hours (notice left empty).
async fn shop_panel(db: &SqlitePool) -> ShopPanel {
    let st = shop_status(db).await;
    ShopPanel {
        light: match st.shop_level {
            ShopLevel::Open => ShopLight::Open,
            ShopLevel::OpensLater => ShopLight::OpensLater,
            ShopLevel::Closed => ShopLight::Closed,
        },
        reason: st.shop_reason,
        gate_open: st.gate_open,
        gate_reason: st.gate_reason,
        orders_paused: st.orders_paused,
        snooze_until: st.snooze_until_label,
        force_open_until: st.force_open_label,
        delivery_paused: st.delivery_paused,
        today_open: st.today_open,
        today_close: st.today_close,
        today_extended: st.today_extended,
        notice: String::new(),
    }
}

/// Run a terminal's quick control through the same code as the /admin/hours
/// buttons. `Err` carries the user-facing reason.
pub(crate) async fn shop_action(
    db: &SqlitePool,
    live: Option<&LiveHub>,
    paused: Option<&OrdersPausedHandle>,
    action: ShopAction,
) -> Result<(), String> {
    let ctl = ShopControl {
        db,
        live,
        paused,
        by: "kitchen display",
    };
    match action {
        ShopAction::SetOpen(open) => ctl.set_open(open).await,
        ShopAction::ResetToSchedule => ctl.reset_to_schedule().await,
        ShopAction::Snooze { minutes } => ctl.snooze(minutes as i64).await,
        ShopAction::SetDeliveryPaused(p) => ctl.set_delivery_paused(p).await,
        ShopAction::SetTodayHours { open, close } => ctl.set_today_hours(open, close).await,
    }
}
