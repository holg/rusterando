//! Printer (Pi) connection monitor — alerts when the kitchen printer drops off.
//!
//! The Pi is a persistent client that refreshes `kitchen_clients.last_seen_at`
//! on every Hello + keepalive (see kitchen.rs). If that timestamp goes stale
//! the shop stops receiving printed tickets — a silent, business-critical
//! failure the old build only surfaced by SSHing in. This background task
//! watches the freshness and, on the online→offline **edge**, fires:
//!   * an APNs push to the shop's admins, and
//!   * an alert email (reusing the SMTP_* env the order mailer uses).
//! On recovery it sends a single "wieder online" push + mail. Each edge fires
//! exactly once — no repeat spam while it stays down.
//!
//! The admin order board computes its offline banner directly from
//! `last_seen_at` (so it's correct between ticks); this task owns only the
//! push/email side effects.

use std::collections::HashMap;
use std::time::Duration;

use sqlx::SqlitePool;

use crate::apns::ApnsHandle;
use rusterando_frontend::branding::BrandingHandle;

/// A printer counts as offline once its last heartbeat is older than this.
/// Matches the "offline" band the /admin/drucker panel already uses (600s is
/// its red threshold; 300s is a touch tighter so we alert before a whole order
/// backs up, while still tolerating a Pi reboot / brief network blip).
pub const OFFLINE_AFTER_SECS: i64 = 300;

/// How often we sample. A minute is plenty — the threshold is 5 minutes, so
/// we can't miss an edge, and it's one cheap indexed query per tick.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Per-printer connection state, so we alert only on transitions.
#[derive(Clone, Copy, PartialEq)]
enum ConnState {
    Online,
    Offline,
}

/// Spawn the monitor. Watches every printer in `db`'s `kitchen_clients`
/// (Davids' global pool). `apns` is `None` on a build without APNs configured
/// — then we still email + log. `branding` supplies the shop name for the
/// alert text.
pub fn spawn(db: SqlitePool, apns: Option<ApnsHandle>, branding: BrandingHandle) {
    tokio::spawn(async move {
        // shop_slug → last known state. Seeded lazily on first sight; a printer
        // that's already offline at boot alerts on the first tick (intended —
        // if it's down when we start, staff should know).
        let mut states: HashMap<String, ConnState> = HashMap::new();
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        // Skip the immediate first tick so a restart doesn't double-fire with
        // the board's own reload; wait one interval, then poll steadily.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(e) = poll_once(&db, apns.as_ref(), &branding, &mut states).await {
                tracing::warn!(target: "printer_monitor", error = %e, "poll failed");
            }
        }
    });
}

/// One sampling pass. Reads each printer's staleness and fires edge alerts.
async fn poll_once(
    db: &SqlitePool,
    apns: Option<&ApnsHandle>,
    branding: &BrandingHandle,
    states: &mut HashMap<String, ConnState>,
) -> anyhow::Result<()> {
    // (shop_slug, seconds since last heartbeat). A row with last_seen_at = 0
    // (Hello never recorded a time) is treated as offline.
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT shop_slug,
                CASE WHEN last_seen_at = 0 THEN 999999
                     ELSE MAX(strftime('%s','now') - last_seen_at, 0) END AS stale
         FROM kitchen_clients",
    )
    .fetch_all(db)
    .await?;

    for (slug, stale_secs) in rows {
        let now_state = if stale_secs > OFFLINE_AFTER_SECS {
            ConnState::Offline
        } else {
            ConnState::Online
        };
        let prev = states.get(&slug).copied();

        match (prev, now_state) {
            // First time we see this printer already down → alert.
            (None, ConnState::Offline) => {
                alert_offline(apns, branding, &slug, stale_secs).await;
            }
            // Online → offline edge.
            (Some(ConnState::Online), ConnState::Offline) => {
                alert_offline(apns, branding, &slug, stale_secs).await;
            }
            // Offline → online edge (recovery).
            (Some(ConnState::Offline), ConnState::Online) => {
                alert_recovered(apns, branding, &slug).await;
            }
            // No change (or first sight while online) → nothing.
            _ => {}
        }
        states.insert(slug, now_state);
    }
    Ok(())
}

/// Fire the offline alert: push to admins + email + a warn log.
async fn alert_offline(
    apns: Option<&ApnsHandle>,
    branding: &BrandingHandle,
    slug: &str,
    stale_secs: i64,
) {
    let shop = branding.get().display_name();
    let mins = (stale_secs / 60).max(OFFLINE_AFTER_SECS / 60);
    tracing::warn!(
        target: "printer_monitor",
        %slug, stale_secs,
        "printer OFFLINE — alerting admins"
    );

    let title = "🖨️ Drucker offline".to_string();
    let body = format!(
        "Der Küchendrucker ({shop}) meldet sich seit über {mins} Min. nicht mehr. \
         Bestellungen werden evtl. nicht gedruckt!"
    );
    if let Some(apns) = apns {
        apns.notify_roles(&["admin"], &title, &body, Some("/admin/drucker"));
    } else {
        tracing::warn!(target: "printer_monitor", "no APNs — offline push skipped");
    }

    let subject = format!("⚠️ Drucker offline — {shop}");
    let mail_body = format!(
        "Achtung,\n\n\
         der Küchendrucker von {shop} meldet sich seit über {mins} Minuten nicht mehr \
         am Server.\n\n\
         Solange der Drucker offline ist, werden eingehende Bestellungen NICHT \
         automatisch gedruckt. Bitte prüfen:\n\
           • Strom / Netzwerk des Druckers (Raspberry Pi)\n\
           • Status unter /admin/drucker\n\n\
         Diese Meldung wird einmalig beim Ausfall gesendet.\n"
    );
    send_alert_email(&subject, &mail_body).await;
}

/// Fire the recovery notice: a single push + email so staff know it's back.
async fn alert_recovered(apns: Option<&ApnsHandle>, branding: &BrandingHandle, slug: &str) {
    let shop = branding.get().display_name();
    tracing::info!(target: "printer_monitor", %slug, "printer back ONLINE");

    let title = "🖨️ Drucker wieder online".to_string();
    let body = format!("Der Küchendrucker ({shop}) ist wieder verbunden.");
    if let Some(apns) = apns {
        apns.notify_roles(&["admin"], &title, &body, Some("/admin/drucker"));
    }
    let subject = format!("✅ Drucker wieder online — {shop}");
    let mail_body = format!(
        "Entwarnung,\n\nder Küchendrucker von {shop} ist wieder mit dem Server \
         verbunden. Bestellungen werden wieder gedruckt.\n"
    );
    send_alert_email(&subject, &mail_body).await;
}

/// Send a plain-text alert email via the frontend's shared SMTP helper (which
/// owns the `lettre` dependency + all `SMTP_*` env handling). Best-effort — a
/// send failure is logged, never fatal.
async fn send_alert_email(subject: &str, body: &str) {
    match rusterando_frontend::pages::order::notify::send_alert_email(subject, body).await {
        Ok(()) => {}
        Err(e) => tracing::warn!(target: "printer_monitor", error = %e, "alert email failed"),
    }
}
