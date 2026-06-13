//! /admin/hours — opening-hours editor, date overrides ("holidays"),
//! and a prominent quick open/close switch.
//!
//! Three things live here:
//!   1. A quick "Jetzt schließen / Wieder öffnen" toggle that flips the
//!      `orders_paused` setting (the manual override that beats the
//!      schedule). One obvious click for "we're closing now".
//!   2. The weekly schedule editor — the rows of `opening_hours`
//!      (weekday 0=Sun..6=Sat, possibly two shifts/day). Edits the
//!      times + closed flag of the existing seeded rows in place.
//!   3. Date overrides — `special_hours` rows that win over the weekday
//!      schedule for a single calendar date, so the shop can open on a
//!      normally-closed holiday (good business) or close for one date.
//!
//! Ordering enforcement reads all of this via `today_slots`
//! (pages/order.rs) + the `orders_paused` gate in `place_order`.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;

/// One editable weekday shift row (mirrors an `opening_hours` row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HourRow {
    pub id: String,
    pub weekday: i64, // 0=Sun..6=Sat
    pub open_time: String,
    pub close_time: String,
    pub is_closed: bool,
}

/// One date override (mirrors a `special_hours` row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecialRow {
    pub date: String, // YYYY-MM-DD
    pub is_closed: bool,
    pub open_time: String,  // "" when closed
    pub close_time: String, // ""
    pub note: String,
}

/// Snapshot for the page: weekly rows + overrides + the current pause flag.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HoursAdmin {
    pub weekly: Vec<HourRow>,
    pub special: Vec<SpecialRow>,
    /// Indefinite manual pause switch.
    pub orders_paused: bool,
    /// When a timed snooze is active, the local "HH:MM" it resumes;
    /// empty when there's no active snooze. (Indefinite pause leaves
    /// this empty — `orders_paused` covers that.)
    pub snooze_until_label: String,
    /// When an ad-hoc force-open is active, the local "HH:MM" it lapses
    /// (end of today); empty otherwise.
    pub force_open_label: String,
    /// Authoritative "are we taking orders right now?" — pause + snooze +
    /// force-open + schedule all resolved. Drives the toggle button label.
    pub effective_open: bool,
    /// THE customer-facing three-level status (green Open / amber OpensLater
    /// / red Closed) — identical to what the customer banner shows. The admin
    /// status line + colour must mirror this so the two never disagree (e.g.
    /// "closed now, opens at 16:00" reads amber here, not a misleading green).
    pub shop_level: rusterando_shared::models::ShopLevel,
    /// The customer-facing reason string for `shop_level` (e.g. "Heute ab
    /// 16:00 Uhr geöffnet"). Empty when plainly open.
    pub shop_reason: String,
    /// Authoritative "can a customer actually place an order RIGHT NOW?" —
    /// the exact result of the `place_order` gate (`shop_closed_state`).
    /// Distinct from `shop_level`: the banner can read green while the
    /// gate is closed near closing time (pre-buffer pushed all slots past
    /// close). Surfaced so the admin status line can warn explicitly
    /// instead of showing a misleading green. `false` = the order button
    /// is disabled for customers right now.
    pub gate_open: bool,
    /// The customer-facing gate-closed reason when `gate_open == false`.
    pub gate_reason: String,
    /// Today's current effective OPEN time as "HH:MM" (earliest window
    /// open). Empty when no window today. Drives the "früher öffnen"
    /// side of the same-day control.
    pub today_open: String,
    /// Today's current effective close time as "HH:MM" (latest window
    /// close — schedule, special-hours override, or force-open). Empty
    /// when the shop has no window today (Ruhetag / closed). Drives the
    /// "Heute länger offen" control's relative bumps + display.
    pub today_close: String,
    /// True when today's hours come from a `special_hours` override with
    /// the "Heute verlängert" note — i.e. the admin already extended
    /// today. Lets the UI show "Heute bis HH:MM (verlängert) — zurück zum
    /// Plan" with an undo.
    pub today_extended: bool,
    /// Today's date "YYYY-MM-DD" (server-computed in shop TZ) — used by
    /// the undo button to delete today's special_hours row without
    /// client-side date math.
    pub today_date: String,
}

// ---------------------------------------------------------------------------
// Server fns
// ---------------------------------------------------------------------------

/// Recompute + broadcast the shop open/closed state to all live `/api/live/shop`
/// subscribers (home + cart). Called after any quick-toggle/snooze change so
/// open customer tabs flip without a reload. No-op if either the DB pool or
/// the LiveHub isn't in context.
#[cfg(feature = "ssr")]
async fn broadcast_shop() {
    use sqlx::SqlitePool;
    if let (Some(db), Some(hub)) = (
        use_context::<SqlitePool>(),
        use_context::<crate::live::LiveHub>(),
    ) {
        crate::live::broadcast_shop_status(&db, &hub).await;
    }
}

#[server(name = LoadHoursAdmin, prefix = "/api", endpoint = "load_hours_admin")]
pub async fn load_hours_admin() -> Result<HoursAdmin, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let weekly = sqlx::query_as::<_, (String, i64, String, String, i64)>(
        "SELECT id, weekday, open_time, close_time, is_closed
         FROM opening_hours ORDER BY weekday, open_time",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load opening_hours: {e}")))?
    .into_iter()
    .map(|(id, weekday, open_time, close_time, is_closed)| HourRow {
        id,
        weekday,
        open_time,
        close_time,
        is_closed: is_closed != 0,
    })
    .collect();

    let special =
        sqlx::query_as::<_, (String, i64, Option<String>, Option<String>, Option<String>)>(
            "SELECT date, is_closed, open_time, close_time, note
         FROM special_hours ORDER BY date",
        )
        .fetch_all(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("load special_hours: {e}")))?
        .into_iter()
        .map(
            |(date, is_closed, open_time, close_time, note)| SpecialRow {
                date,
                is_closed: is_closed != 0,
                open_time: open_time.unwrap_or_default(),
                close_time: close_time.unwrap_or_default(),
                note: note.unwrap_or_default(),
            },
        )
        .collect();

    let orders_paused = crate::pages::settings::ssr::orders_paused(&db).await;
    // Active timed snooze → local resume label for the status line.
    let snooze_until_label = crate::pages::settings::ssr::orders_paused_until(&db)
        .await
        .map(|until| {
            until
                .with_timezone(&chrono::Local)
                .format("%H:%M")
                .to_string()
        })
        .unwrap_or_default();
    // Active ad-hoc force-open → local "bis HH:MM" label for the status line.
    let force_open_label = crate::pages::settings::ssr::force_open_until(&db)
        .await
        .map(|until| {
            until
                .with_timezone(&chrono::Local)
                .format("%H:%M")
                .to_string()
        })
        .unwrap_or_default();
    // The authoritative "are we taking orders right now?" — same logic as
    // place_order's gate: not paused, not in an active timed snooze, AND a
    // slot exists today (which already accounts for force-open + special
    // hours + schedule). Without the snooze check the big toggle would
    // read "Jetzt schließen" during a snooze (because the schedule still
    // has slots) while the customer banner correctly shows closed — and
    // clicking it would force-close instead of clearing the snooze.
    let snoozed = !snooze_until_label.is_empty();
    let effective_open =
        !orders_paused && !snoozed && !crate::pages::order::ssr::today_slots(&db).await.is_empty();

    // The exact customer view (green/amber/red + reason). The admin status
    // line is driven by this so it can never show green while the customer
    // sees amber/red — e.g. "closed now, opens 16:00".
    let (shop_level, shop_reason) = crate::pages::order::ssr::shop_open_state(&db).await;

    // The ACTUAL order gate — identical to what place_order enforces. When
    // this is closed but shop_level is green, the admin sees an explicit
    // warning instead of a misleading "läuft".
    let (gate_closed, gate_reason) = crate::pages::order::ssr::shop_closed_state(&db).await;

    // Today's current effective open + close, and whether it's an admin
    // same-day adjustment. `today_windows` resolves special-hours /
    // force-open / schedule.
    let today_windows = crate::pages::order::ssr::today_windows(&db).await;
    let today_open = today_windows
        .iter()
        .map(|(o, _)| o.clone())
        .min()
        .unwrap_or_default();
    let today_close = today_windows
        .iter()
        .map(|(_, c)| c.clone())
        .max()
        .unwrap_or_default();
    // Did the admin adjust today? A special_hours row for today with our
    // marker note is the signal.
    let today_str = crate::pages::order::ssr::now_local()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let today_extended: bool = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM special_hours
         WHERE date = ?1 AND is_closed = 0 AND note = 'Heute angepasst'",
    )
    .bind(&today_str)
    .fetch_one(&db)
    .await
    .map(|n| n > 0)
    .unwrap_or(false);

    Ok(HoursAdmin {
        weekly,
        special,
        orders_paused,
        snooze_until_label,
        force_open_label,
        effective_open,
        shop_level,
        shop_reason,
        gate_open: !gate_closed,
        gate_reason,
        today_open,
        today_close,
        today_extended,
        today_date: today_str,
    })
}

/// Validate a "HH:MM" string (00:00–23:59). Empty is allowed by the
/// caller (closed days carry no time), so this only runs on non-empty.
#[cfg(feature = "ssr")]
fn valid_hhmm(s: &str) -> bool {
    chrono::NaiveTime::parse_from_str(s, "%H:%M").is_ok()
}

/// Update one weekday shift row (by id). Editing the seeded rows in place
/// keeps the schema stable and matches `today_slots`' weekday query.
#[server(name = UpdateHourRow, prefix = "/api", endpoint = "update_hour_row")]
pub async fn update_hour_row(
    id: String,
    open_time: String,
    close_time: String,
    is_closed: bool,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    if !is_closed && (!valid_hhmm(&open_time) || !valid_hhmm(&close_time)) {
        return Err(ServerFnError::new(
            "Öffnungs-/Schließzeit muss im Format HH:MM sein.",
        ));
    }
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let res = sqlx::query(
        "UPDATE opening_hours SET open_time = ?2, close_time = ?3, is_closed = ?4 WHERE id = ?1",
    )
    .bind(&id)
    .bind(open_time.trim())
    .bind(close_time.trim())
    .bind(i64::from(is_closed))
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("update hour: {e}")))?;

    if res.rows_affected() == 0 {
        return Err(ServerFnError::new("Zeile nicht gefunden."));
    }
    // Weekly hours changed → refresh the cached JSON-LD openingHours.
    // (Special-hours overrides are NOT in the JSON-LD, so their fns skip this.)
    crate::pages::seo::rebuild_jsonld_cache().await;
    Ok(())
}

/// Insert or replace a date override.
#[server(name = UpsertSpecialHours, prefix = "/api", endpoint = "upsert_special_hours")]
pub async fn upsert_special_hours(
    date: String,
    is_closed: bool,
    open_time: String,
    close_time: String,
    note: String,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    // Basic date shape; SQLite compares these lexicographically.
    if chrono::NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").is_err() {
        return Err(ServerFnError::new("Datum im Format JJJJ-MM-TT erwartet."));
    }
    if !is_closed && (!valid_hhmm(&open_time) || !valid_hhmm(&close_time)) {
        return Err(ServerFnError::new(
            "Für einen offenen Sondertag bitte gültige Öffnungs-/Schließzeit (HH:MM) angeben.",
        ));
    }
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Closed overrides carry no window; store NULLs so reads are clean.
    let (open_opt, close_opt): (Option<&str>, Option<&str>) = if is_closed {
        (None, None)
    } else {
        (Some(open_time.trim()), Some(close_time.trim()))
    };
    let note_trim = note.trim();
    let note_opt = (!note_trim.is_empty()).then_some(note_trim);

    sqlx::query(
        "INSERT INTO special_hours (date, is_closed, open_time, close_time, note)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(date) DO UPDATE SET
             is_closed = excluded.is_closed,
             open_time = excluded.open_time,
             close_time = excluded.close_time,
             note = excluded.note",
    )
    .bind(date.trim())
    .bind(i64::from(is_closed))
    .bind(open_opt)
    .bind(close_opt)
    .bind(note_opt)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("upsert special hours: {e}")))?;
    Ok(())
}

/// "Heute länger / früher offen" — adjust today's window edges via a
/// `special_hours` row for today. The deliberate "we're open differently
/// today" lever, distinct from the 24h-ish Notbetrieb force-open.
/// special_hours wins over the weekday schedule in `today_windows`, so
/// the banner / slots / order gate all follow automatically, and the
/// override auto-expires at midnight when the date rolls over.
///
/// Both edges are optional and independent:
///   * `new_open`  — pull today's OPEN earlier ("we open at 15:00 today").
///                   Only EARLIER allowed (refuse later than current open;
///                   to open later, use the schedule/Sondertag editor).
///   * `new_close` — push today's CLOSE later ("we stay open until 23:00").
///                   Only LATER allowed; refuse a close in the past.
/// The unspecified edge keeps today's current effective value (or `now`
/// for a closed day's open). Result must be a valid open < close window.
#[server(name = SetTodayHours, prefix = "/api", endpoint = "set_today_hours")]
pub async fn set_today_hours(
    #[server(default)] new_open: Option<String>,
    #[server(default)] new_close: Option<String>,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    let new_open = new_open
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let new_close = new_close
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if new_open.is_none() && new_close.is_none() {
        return Err(ServerFnError::new("Keine Zeit angegeben."));
    }
    if let Some(o) = &new_open {
        if !valid_hhmm(o) {
            return Err(ServerFnError::new("Öffnungszeit ungültig (HH:MM)."));
        }
    }
    if let Some(c) = &new_close {
        if !valid_hhmm(c) {
            return Err(ServerFnError::new("Schließzeit ungültig (HH:MM)."));
        }
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let now_hhmm = crate::pages::order::ssr::now_local()
        .format("%H:%M")
        .to_string();
    let windows = crate::pages::order::ssr::today_windows(&db).await;
    let cur_open = windows.iter().map(|(o, _)| o.clone()).min();
    let cur_close = windows.iter().map(|(_, c)| c.clone()).max();

    // Resolve each edge: use the requested value, else keep current.
    // Closed day has no current open → default the open edge to now.
    let open_time = new_open
        .clone()
        .or_else(|| cur_open.clone())
        .unwrap_or_else(|| now_hhmm.clone());
    let close_time = new_close
        .clone()
        .or_else(|| cur_close.clone())
        .ok_or_else(|| {
            ServerFnError::new(
                "Heute ist kein regulärer Betrieb — bitte auch eine Schließzeit angeben.",
            )
        })?;

    // Earlier-only on open: refuse moving the open LATER than it is now.
    if let (Some(req), Some(cur)) = (&new_open, &cur_open) {
        if req > cur {
            return Err(ServerFnError::new(format!(
                "Heute ist ab {cur} Uhr geöffnet. Dieser Schalter macht nur FRÜHER auf — für später den Wochenplan/Sondertag nutzen."
            )));
        }
    }
    // Later-only on close: refuse moving the close EARLIER than it is now.
    if let (Some(req), Some(cur)) = (&new_close, &cur_close) {
        if req < cur {
            return Err(ServerFnError::new(format!(
                "Heute ist bis {cur} Uhr geöffnet. Dieser Schalter macht nur LÄNGER auf — zum Verkürzen den Wochenplan/Sondertag nutzen."
            )));
        }
        if req <= &now_hhmm {
            return Err(ServerFnError::new(
                "Die neue Schließzeit liegt in der Vergangenheit.",
            ));
        }
    }
    if close_time <= open_time {
        return Err(ServerFnError::new(
            "Die Schließzeit muss nach der Öffnungszeit liegen.",
        ));
    }

    let today = crate::pages::order::ssr::now_local()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    sqlx::query(
        "INSERT INTO special_hours (date, is_closed, open_time, close_time, note)
         VALUES (?1, 0, ?2, ?3, 'Heute angepasst')
         ON CONFLICT(date) DO UPDATE SET
             is_closed = 0,
             open_time = excluded.open_time,
             close_time = excluded.close_time,
             note = excluded.note",
    )
    .bind(&today)
    .bind(&open_time)
    .bind(&close_time)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("set today hours: {e}")))?;

    // Customer banners + cart should flip live (today's hours changed),
    // same as the quick-toggle path.
    broadcast_shop().await;
    Ok(())
}

#[server(name = DeleteSpecialHours, prefix = "/api", endpoint = "delete_special_hours")]
pub async fn delete_special_hours(date: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query("DELETE FROM special_hours WHERE date = ?1")
        .bind(date.trim())
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("delete special hours: {e}")))?;
    Ok(())
}

/// Quick open/close — flips the `orders_paused` setting. `paused=true`
/// closes online ordering immediately regardless of the schedule.
/// Reuses the same validated write path + write-through handle as
/// /admin/settings.
/// Quick open/close toggle, schedule-aware in BOTH directions.
///
/// `open = false` → force CLOSED now (sets `orders_paused=1`; clearing the
///   timer/force-open happens via update_setting's write-through).
/// `open = true`  → force OPEN until end of today, even on a Ruhetag /
///   outside opening hours. Clears the pause, then sets `force_open_until`
///   to local end-of-day (auto-reverts tomorrow). On a normally-open day
///   this is harmless (slots already exist). This fixes the bug where
///   "Jetzt öffnen" did nothing when the closure came from the schedule
///   rather than the pause flag.
#[server(name = SetOrdersOpen, prefix = "/api", endpoint = "set_orders_open")]
pub async fn set_orders_open(open: bool) -> Result<(), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    if !open {
        // Force closed. update_setting clears force_open_until + snooze
        // when orders_paused flips on? It clears them on OFF; on ON we
        // clear force-open explicitly so a stale force-open can't linger.
        crate::pages::settings::update_setting("force_open_until".to_string(), String::new())
            .await?;
        crate::pages::settings::update_setting("orders_paused".to_string(), "1".to_string())
            .await?;
        log::info!("[orders] CLOSED by admin (manual pause)");
        broadcast_shop().await;
        return Ok(());
    }
    // Force open until end of the local day, expressed in UTC.
    use chrono::{Local, TimeZone, Utc};
    let now_local = Local::now();
    let end_local = now_local
        .date_naive()
        .and_hms_opt(23, 59, 59)
        .and_then(|naive| Local.from_local_datetime(&naive).single())
        .unwrap_or(now_local);
    let until_utc = end_local.with_timezone(&Utc);
    let stamp = until_utc.format("%Y-%m-%d %H:%M:%S").to_string();
    // Clear pause + snooze (update_setting wipes orders_paused_until on
    // orders_paused→0), then set the force-open deadline.
    crate::pages::settings::update_setting("orders_paused".to_string(), "0".to_string()).await?;
    crate::pages::settings::update_setting("force_open_until".to_string(), stamp).await?;
    log::info!(
        "[orders] FORCE-OPENED by admin until {} (end of day)",
        end_local.format("%H:%M")
    );
    broadcast_shop().await;
    Ok(())
}

/// Reset to the weekly plan ("Automatik"): clear every manual override —
/// indefinite pause, timed snooze, and ad-hoc force-open — so the shop
/// state is driven purely by the `opening_hours` / `special_hours`
/// schedule again. This is how the admin gets back to e.g. "Mittwoch
/// Ruhetag" after having manually toggled open/closed on a rest day:
/// neither the open nor the close button returns to schedule-driven
/// state (one force-opens, the other indefinitely pauses), so we need an
/// explicit "hand control back to the schedule" action.
#[server(name = SetOrdersSchedule, prefix = "/api", endpoint = "set_orders_schedule")]
pub async fn set_orders_schedule() -> Result<(), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    crate::pages::settings::update_setting("force_open_until".to_string(), String::new()).await?;
    crate::pages::settings::update_setting("orders_paused_until".to_string(), String::new())
        .await?;
    // Set indefinite pause OFF last; update_setting also wipes
    // orders_paused_until on the OFF transition (belt-and-suspenders).
    crate::pages::settings::update_setting("orders_paused".to_string(), "0".to_string()).await?;
    log::info!("[orders] reset to schedule (Automatik) by admin");
    broadcast_shop().await;
    Ok(())
}

/// Snooze online ordering for `minutes` from now — kitchen-overwhelmed
/// button. Sets `orders_paused_until` to now+minutes (UTC) and ensures
/// the indefinite switch is off so the timer governs. Ordering resumes
/// automatically when the deadline passes (no background job — the next
/// order/page check sees the expired timestamp). `minutes <= 0` cancels.
#[server(name = SnoozeOrders, prefix = "/api", endpoint = "snooze_orders")]
pub async fn snooze_orders(minutes: i64) -> Result<(), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    if minutes <= 0 {
        // Cancel: clear the timer (and make sure indefinite is off too).
        crate::pages::settings::update_setting("orders_paused_until".to_string(), String::new())
            .await?;
        crate::pages::settings::update_setting("orders_paused".to_string(), "0".to_string())
            .await?;
        log::info!("[orders] snooze cancelled by admin");
        broadcast_shop().await;
        return Ok(());
    }
    if minutes > 24 * 60 {
        return Err(ServerFnError::new("Pause darf höchstens 24 Stunden sein."));
    }
    let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
    let stamp = until.format("%Y-%m-%d %H:%M:%S").to_string();
    // Timer governs → keep the indefinite switch off so it auto-resumes.
    crate::pages::settings::update_setting("orders_paused".to_string(), "0".to_string()).await?;
    crate::pages::settings::update_setting("orders_paused_until".to_string(), stamp.clone())
        .await?;
    log::info!(
        "[orders] snoozed {minutes} min by admin (until {} local)",
        until.with_timezone(&chrono::Local).format("%H:%M")
    );
    broadcast_shop().await;

    // Auto-expiry push: wake when the snooze lapses and re-broadcast the
    // (recomputed) shop status so home/cart re-open live without a reload.
    // Superseded-safe: only fire if `orders_paused_until` is STILL this
    // exact deadline when we wake (a changed/cancelled/re-snooze writes a
    // different value, so the stale task no-ops). One short-lived task per
    // snooze; no recurring scheduler.
    #[cfg(feature = "ssr")]
    if let (Some(db), Some(hub)) = (
        use_context::<sqlx::SqlitePool>(),
        use_context::<crate::live::LiveHub>(),
    ) {
        let wait = (minutes as u64) * 60 + 2; // +2s cushion past the deadline
        let deadline = stamp.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            let current: Option<(String,)> =
                sqlx::query_as("SELECT value FROM app_settings WHERE key = 'orders_paused_until'")
                    .fetch_optional(&db)
                    .await
                    .ok()
                    .flatten();
            // Still the same deadline we scheduled for? Then it just lapsed
            // (no later snooze/open superseded it) → broadcast the reopen.
            if current.map(|(v,)| v).as_deref() == Some(deadline.as_str()) {
                crate::live::broadcast_shop_status(&db, &hub).await;
                log::info!("[orders] snooze expired — broadcast shop reopen");
            }
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

fn weekday_label(weekday: i64) -> &'static str {
    match weekday {
        0 => "Sonntag",
        1 => "Montag",
        2 => "Dienstag",
        3 => "Mittwoch",
        4 => "Donnerstag",
        5 => "Freitag",
        6 => "Samstag",
        _ => "?",
    }
}

#[component]
pub fn HoursAdminPage() -> impl IntoView {
    let row_updater = ServerAction::<UpdateHourRow>::new();
    let special_upsert = ServerAction::<UpsertSpecialHours>::new();
    let special_delete = ServerAction::<DeleteSpecialHours>::new();
    let open_setter = ServerAction::<SetOrdersOpen>::new();
    let scheduler = ServerAction::<SetOrdersSchedule>::new();
    let snoozer = ServerAction::<SnoozeOrders>::new();
    let extender = ServerAction::<SetTodayHours>::new();

    let data = Resource::new(
        move || {
            (
                row_updater.version().get(),
                special_upsert.version().get(),
                special_delete.version().get(),
                open_setter.version().get(),
                scheduler.version().get(),
                snoozer.version().get(),
                extender.version().get(),
            )
        },
        |_| async move { load_hours_admin().await },
    );

    view! {
        <AdminShell>
            <section class="admin-hours">
                <header class="admin-bar">
                    <h1>"Öffnungszeiten"</h1>
                </header>

                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || data.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(d) => view! { <HoursBody d row_updater special_upsert special_delete open_setter scheduler snoozer extender/> }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn HoursBody(
    d: HoursAdmin,
    row_updater: ServerAction<UpdateHourRow>,
    special_upsert: ServerAction<UpsertSpecialHours>,
    special_delete: ServerAction<DeleteSpecialHours>,
    open_setter: ServerAction<SetOrdersOpen>,
    scheduler: ServerAction<SetOrdersSchedule>,
    snoozer: ServerAction<SnoozeOrders>,
    extender: ServerAction<SetTodayHours>,
) -> impl IntoView {
    use rusterando_shared::models::ShopLevel;

    let paused = d.orders_paused;
    let snooze_label = d.snooze_until_label.clone();
    let snoozing = !snooze_label.is_empty();
    let force_label = d.force_open_label.clone();
    let forced = !force_label.is_empty();
    // The real order gate at load time. When it's closed while the shop
    // otherwise looks open (near closing, prep buffer ate the last slots),
    // we surface an explicit warning + the one-click "Notbetrieb" rescue.
    let gate_open_initial = d.gate_open;
    let gate_reason = d.gate_reason.clone();
    // "Heute länger / früher offen" state.
    let today_open = d.today_open.clone();
    let today_close = d.today_close.clone();
    let today_extended = d.today_extended;

    // The customer-facing 3-level status, seeded from SSR and kept live via
    // the same `/api/live/shop` SSE channel the customer banner + shell chip
    // use — so this status line flips in real time and ALWAYS matches what
    // the customer sees (no more green-while-closed). The toggle button's
    // "open" notion is `level == Open` (open right now), so when the shop is
    // closed-now-but-opens-later the button reads "Jetzt öffnen" (force-open).
    let (shop_live, set_shop_live) =
        signal::<Option<(ShopLevel, String)>>(Some((d.shop_level, d.shop_reason.clone())));
    crate::utils::subscribe_shop_status(set_shop_live);
    let level = move || shop_live.get().map(|(l, _)| l).unwrap_or(ShopLevel::Open);
    let reason = move || shop_live.get().map(|(_, r)| r).unwrap_or_default();
    // "Open right now" — drives the toggle direction. NOT `effective_open`
    // (which counts a future pre-order slot as open).
    let open = move || level() == ShopLevel::Open;

    let on_toggle = move |_| {
        // Schedule-aware in both directions: open→close pauses; closed→open
        // FORCE-opens until end of day (works even on a Ruhetag).
        open_setter.dispatch(SetOrdersOpen { open: !open() });
    };
    // Any manual override active? Then "Auf Plan zurücksetzen" is offered so
    // the admin can hand control back to the weekly schedule (e.g. to get
    // "Mittwoch Ruhetag" back after manually toggling on a rest day).
    let overridden = paused || snoozing || forced;
    let on_reset = move |_| {
        scheduler.dispatch(SetOrdersSchedule {});
    };
    let snooze = move |mins: i64| {
        move |_| {
            snoozer.dispatch(SnoozeOrders { minutes: mins });
        }
    };
    // "Notbetrieb" — accept everything right now, ASAP, no questions.
    // This is the in-the-shop panic button: a customer is standing at the
    // counter wanting to pay by app and the gate is closed for any reason
    // (near closing, Ruhetag, outside hours). Force-open until end of day
    // makes `open_right_now` true → the gate opens and ASAP is allowed.
    let on_notbetrieb = move |_| {
        open_setter.dispatch(SetOrdersOpen { open: true });
    };

    // "Heute länger / früher offen" — adjust today's window edges. Relative
    // bumps shift the current open earlier / close later; the pickers set
    // absolute times. All route through `set_today_hours`, which validates
    // (earlier-only on open, later-only on close) + writes today's
    // special_hours row. HH:MM math here is a UX convenience; the server
    // re-validates everything.
    let open_picker = RwSignal::new(String::new());
    let close_picker = RwSignal::new(String::new());

    // Parse "HH:MM" → minutes-since-midnight, default fallback supplied.
    fn hhmm_to_min(s: &str, default_h: i64) -> i64 {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 2 {
            parts[0].parse::<i64>().unwrap_or(default_h) * 60 + parts[1].parse::<i64>().unwrap_or(0)
        } else {
            default_h * 60
        }
    }
    fn min_to_hhmm(total: i64) -> String {
        let t = total.clamp(0, 23 * 60 + 59);
        format!("{:02}:{:02}", t / 60, t % 60)
    }

    // Push the close LATER by `add_min` from today's current close.
    let close_for_bump = today_close.clone();
    let bump_close = move |add_min: i64| {
        let base = close_for_bump.clone();
        move |_| {
            let new_close = min_to_hhmm(hhmm_to_min(&base, 22) + add_min);
            extender.dispatch(SetTodayHours {
                new_open: None,
                new_close: Some(new_close),
            });
        }
    };
    // Pull the open EARLIER by `sub_min` from today's current open.
    let open_for_bump = today_open.clone();
    let bump_open = move |sub_min: i64| {
        let base = open_for_bump.clone();
        move |_| {
            let new_open = min_to_hhmm(hhmm_to_min(&base, 16) - sub_min);
            extender.dispatch(SetTodayHours {
                new_open: Some(new_open),
                new_close: None,
            });
        }
    };
    let on_open_picker = move |_| {
        let v = open_picker.get();
        if v.trim().is_empty() {
            return;
        }
        extender.dispatch(SetTodayHours {
            new_open: Some(v),
            new_close: None,
        });
        open_picker.set(String::new());
    };
    let on_close_picker = move |_| {
        let v = close_picker.get();
        if v.trim().is_empty() {
            return;
        }
        extender.dispatch(SetTodayHours {
            new_open: None,
            new_close: Some(v),
        });
        close_picker.set(String::new());
    };
    // Undo today's adjustment = delete today's special row → back to plan.
    let today_date_for_undo = d.today_date.clone();
    let on_undo_extend = move |_| {
        special_delete.dispatch(DeleteSpecialHours {
            date: today_date_for_undo.clone(),
        });
    };

    // Group weekday rows for the table (Mon-first display order).
    let weekly = d.weekly.clone();
    let week_order = [1_i64, 2, 3, 4, 5, 6, 0];

    let snooze_label_for_text = snooze_label.clone();
    let force_label_for_text = force_label.clone();
    view! {
        // ----- Quick open/close + snooze -----
        // Colour mirrors the customer banner exactly: green Open, amber
        // OpensLater (closed now / opens later / pause-with-resume), red
        // Closed (Ruhetag / hard-closed). Reuses ShopLevel::css_class so the
        // admin and the customer can't drift apart.
        <div class=move || format!("quick-toggle {}", level().css_class())>
            <div class="state">
                <span class="dot"></span>
                <strong>
                    {move || {
                        // Operational overrides get a precise label; otherwise
                        // show the EXACT customer-facing reason so the admin
                        // sees what the customer sees ("Heute ab 16:00 …").
                        if paused {
                            "Online-Bestellungen sind PAUSIERT".to_string()
                        } else if snoozing {
                            format!("Pausiert — automatisch wieder offen ab {snooze_label_for_text} Uhr")
                        } else if forced {
                            format!("Manuell GEÖFFNET (Sonderöffnung bis {force_label_for_text} Uhr)")
                        } else {
                            match level() {
                                ShopLevel::Open => "Online-Bestellungen laufen (geöffnet)".to_string(),
                                // OpensLater / Closed → the customer's own reason
                                // string ("Heute ab 16:00 Uhr geöffnet" / Ruhetag …).
                                _ => reason(),
                            }
                        }
                    }}
                </strong>
            </div>
            <div class="toggle-actions">
                <button class=move || if open() { "btn danger big" } else { "btn primary big" }
                        on:click=on_toggle>
                    {move || if open() { "Jetzt schließen" } else { "Jetzt öffnen" }}
                </button>
                {overridden.then(|| view! {
                    <button class="btn ghost" on:click=on_reset
                            title="Manuelle Übersteuerung aufheben — wieder nach Wochenplan (inkl. Ruhetag)">
                        "↺ Auf Plan zurücksetzen"
                    </button>
                })}
            </div>
        </div>

        // Snooze buttons: pause for a fixed time, then auto-resume.
        <div class="snooze-row">
            <span class="snooze-label">"Bei Überlastung kurz pausieren:"</span>
            <button class="btn ghost" on:click=snooze(30)>"30 Min"</button>
            <button class="btn ghost" on:click=snooze(60)>"1 Std"</button>
            <button class="btn ghost" on:click=snooze(120)>"2 Std"</button>
        </div>
        <p class="hint">
            "\"Jetzt öffnen\" gibt Bestellungen frei — auch an einem Ruhetag oder außerhalb der \
             Öffnungszeiten (bis Tagesende, danach gilt wieder der Plan). \"Jetzt schließen\" \
             pausiert ohne Zeitlimit. Schnellpause stoppt nur für die gewählte Dauer und gibt \
             danach automatisch wieder frei. \"Auf Plan zurücksetzen\" hebt jede manuelle \
             Übersteuerung auf, sodass wieder der Wochenplan gilt (inkl. Ruhetag)."
        </p>

        // ----- Notbetrieb: explicit gate warning + rescue button -----
        // When the order gate is CLOSED but the shop otherwise looks open
        // (e.g. near closing, prep buffer ate the last slots), the customer
        // sees a disabled order button. This warns the admin explicitly and
        // offers the one-click rescue.
        {(!gate_open_initial).then(|| {
            let reason = if gate_reason.is_empty() {
                "Kund:innen können gerade NICHT bestellen.".to_string()
            } else {
                format!("Kund:innen können gerade NICHT bestellen: {gate_reason}")
            };
            view! {
                <div class="gate-warning">
                    <strong>"⚠ Bestell-Sperre aktiv"</strong>
                    <p>{reason}</p>
                </div>
            }
        })}
        <div class="notbetrieb-row">
            <button class="btn primary big" on:click=on_notbetrieb
                    title="Nimmt sofort alle Bestellungen an (ASAP, bis Tagesende) — egal ob Ruhetag, ausserhalb der Zeiten oder kurz vor Schluss. Für \"Kunde steht vor mir und will per App zahlen\".">
                "🚨 Notbetrieb: alles sofort annehmen"
            </button>
        </div>

        // ----- Heute früher / länger offen -----
        // Deliberate same-day hours adjustment. Writes a special_hours row
        // for today (auto-expires at midnight). Distinct from Notbetrieb
        // (24h-ish panic). "Früher" pulls the open earlier; "Länger" pushes
        // the close later.
        <div class="extend-today">
            <div class="extend-head">
                <strong>"Heute früher / länger offen"</strong>
                {
                    let to = today_open.clone();
                    let tc = today_close.clone();
                    move || if tc.is_empty() {
                        view! { <span class="muted">" — heute kein regulärer Betrieb"</span> }.into_any()
                    } else {
                        view! {
                            <span class="muted">
                                " — aktuell " {to.clone()} "–" {tc.clone()} " Uhr"
                                {today_extended.then(|| view! { <span class="ext-badge">" (angepasst)"</span> })}
                            </span>
                        }.into_any()
                    }
                }
            </div>

            // Früher öffnen
            <div class="extend-actions">
                <span class="extend-sep">"Früher öffnen:"</span>
                <button class="btn ghost" on:click=bump_open(30)>"−30 Min"</button>
                <button class="btn ghost" on:click=bump_open(60)>"−1 Std"</button>
                <span class="extend-sep">"oder ab"</span>
                <input type="time" class="extend-picker"
                    prop:value=move || open_picker.get()
                    on:input=move |ev| open_picker.set(event_target_value(&ev))/>
                <button class="btn primary" on:click=on_open_picker>"Setzen"</button>
            </div>

            // Länger offen
            <div class="extend-actions">
                <span class="extend-sep">"Länger offen:"</span>
                <button class="btn ghost" on:click=bump_close(30)>"+30 Min"</button>
                <button class="btn ghost" on:click=bump_close(60)>"+1 Std"</button>
                <span class="extend-sep">"oder bis"</span>
                <input type="time" class="extend-picker"
                    prop:value=move || close_picker.get()
                    on:input=move |ev| close_picker.set(event_target_value(&ev))/>
                <button class="btn primary" on:click=on_close_picker>"Setzen"</button>
                {today_extended.then(|| view! {
                    <button class="btn ghost danger" on:click=on_undo_extend
                            title="Anpassung aufheben — heute gilt wieder der Wochenplan.">
                        "↺ Anpassung aufheben"
                    </button>
                })}
            </div>

            {move || match extender.value().get() {
                Some(Ok(())) => Some(view! { <p class="ok small">"✓ Öffnungszeit für heute angepasst."</p> }.into_any()),
                Some(Err(e)) => Some(view! { <p class="error small">{format!("{e}")}</p> }.into_any()),
                None => None,
            }}
        </div>

        // ----- Weekly schedule -----
        <h2>"Wöchentliche Öffnungszeiten"</h2>
        <p class="hint">
            "Diese Zeiten steuern, wann Kunden online bestellen können. Mehrere Schichten \
             pro Tag (Mittag/Abend) werden einzeln bearbeitet."
        </p>
        <div class="week-grid">
            {week_order.into_iter().map(|wd| {
                let day_rows: Vec<HourRow> = weekly.iter().filter(|r| r.weekday == wd).cloned().collect();
                view! {
                    <div class="day-block">
                        <h3>{weekday_label(wd)}</h3>
                        {if day_rows.is_empty() {
                            view! { <p class="muted">"—"</p> }.into_any()
                        } else {
                            view! {
                                {day_rows.into_iter().map(|r| view! {
                                    <HourRowEditor r updater=row_updater/>
                                }).collect_view()}
                            }.into_any()
                        }}
                    </div>
                }
            }).collect_view()}
        </div>

        // ----- Date overrides -----
        <h2>"Sondertage (Feiertage / Ausnahmen)"</h2>
        <p class="hint">
            "Ein Sondertag überschreibt die Wochenzeiten für genau dieses Datum — z. B. an einem \
             Feiertag öffnen, obwohl sonst Ruhetag, oder an einem einzelnen Tag schließen."
        </p>
        <SpecialAddCard upserter=special_upsert/>
        {if d.special.is_empty() {
            view! { <p class="empty">"Keine Sondertage angelegt."</p> }.into_any()
        } else {
            view! {
                <table class="special-table">
                    <thead>
                        <tr><th>"Datum"</th><th>"Status"</th><th>"Notiz"</th><th></th></tr>
                    </thead>
                    <tbody>
                        {d.special.into_iter().map(|s| {
                            let date_for_del = s.date.clone();
                            let status = if s.is_closed {
                                "Geschlossen".to_string()
                            } else {
                                format!("{}–{}", s.open_time, s.close_time)
                            };
                            view! {
                                <tr>
                                    <td>{s.date.clone()}</td>
                                    <td>{status}</td>
                                    <td class="muted">{s.note.clone()}</td>
                                    <td>
                                        <button class="btn ghost danger small"
                                            on:click=move |_| {
                                                special_delete.dispatch(DeleteSpecialHours {
                                                    date: date_for_del.clone(),
                                                });
                                            }>"Löschen"</button>
                                    </td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
            }.into_any()
        }}
    }
}

#[component]
fn HourRowEditor(r: HourRow, updater: ServerAction<UpdateHourRow>) -> impl IntoView {
    let id = r.id.clone();
    let open = RwSignal::new(r.open_time.clone());
    let close = RwSignal::new(r.close_time.clone());
    let closed = RwSignal::new(r.is_closed);

    let on_save = move |_| {
        updater.dispatch(UpdateHourRow {
            id: id.clone(),
            open_time: open.get(),
            close_time: close.get(),
            is_closed: closed.get(),
        });
    };

    view! {
        <div class="shift-row">
            <label class="closed-check">
                <input type="checkbox"
                    prop:checked=move || closed.get()
                    on:change=move |ev| closed.set(event_target_checked(&ev))/>
                <span>"geschlossen"</span>
            </label>
            <input type="time" class="t" disabled=move || closed.get()
                prop:value=move || open.get()
                on:input=move |ev| open.set(event_target_value(&ev))/>
            <span>"–"</span>
            <input type="time" class="t" disabled=move || closed.get()
                prop:value=move || close.get()
                on:input=move |ev| close.set(event_target_value(&ev))/>
            <button class="btn primary small" on:click=on_save>"Speichern"</button>
        </div>
    }
}

#[component]
fn SpecialAddCard(upserter: ServerAction<UpsertSpecialHours>) -> impl IntoView {
    let date = RwSignal::new(String::new());
    let closed = RwSignal::new(false);
    let open = RwSignal::new("17:00".to_string());
    let close = RwSignal::new("22:00".to_string());
    let note = RwSignal::new(String::new());

    let on_add = move |_| {
        upserter.dispatch(UpsertSpecialHours {
            date: date.get(),
            is_closed: closed.get(),
            open_time: open.get(),
            close_time: close.get(),
            note: note.get(),
        });
        date.set(String::new());
        note.set(String::new());
    };

    view! {
        <details class="card special-add" open=false>
            <summary><strong>"Sondertag hinzufügen"</strong></summary>
            <div class="grid">
                <label>
                    <span>"Datum"</span>
                    <input type="date"
                        prop:value=move || date.get()
                        on:input=move |ev| date.set(event_target_value(&ev))/>
                </label>
                <label class="closed-check">
                    <input type="checkbox"
                        prop:checked=move || closed.get()
                        on:change=move |ev| closed.set(event_target_checked(&ev))/>
                    <span>"geschlossen (sonst offen mit Zeiten)"</span>
                </label>
                <label>
                    <span>"Öffnet"</span>
                    <input type="time" disabled=move || closed.get()
                        prop:value=move || open.get()
                        on:input=move |ev| open.set(event_target_value(&ev))/>
                </label>
                <label>
                    <span>"Schließt"</span>
                    <input type="time" disabled=move || closed.get()
                        prop:value=move || close.get()
                        on:input=move |ev| close.set(event_target_value(&ev))/>
                </label>
                <label>
                    <span>"Notiz (optional)"</span>
                    <input type="text" placeholder="z. B. Ostermontag"
                        prop:value=move || note.get()
                        on:input=move |ev| note.set(event_target_value(&ev))/>
                </label>
            </div>
            <div class="actions">
                <button class="btn primary" on:click=on_add>"Hinzufügen"</button>
                {move || upserter.value().get().map(|res| match res {
                    Ok(_) => view! { <span class="ok">"✓ Gespeichert"</span> }.into_any(),
                    Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                })}
            </div>
        </details>
    }
}
