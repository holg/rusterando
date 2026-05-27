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
}

// ---------------------------------------------------------------------------
// Server fns
// ---------------------------------------------------------------------------

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

    let special = sqlx::query_as::<_, (String, i64, Option<String>, Option<String>, Option<String>)>(
        "SELECT date, is_closed, open_time, close_time, note
         FROM special_hours ORDER BY date",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load special_hours: {e}")))?
    .into_iter()
    .map(|(date, is_closed, open_time, close_time, note)| SpecialRow {
        date,
        is_closed: is_closed != 0,
        open_time: open_time.unwrap_or_default(),
        close_time: close_time.unwrap_or_default(),
        note: note.unwrap_or_default(),
    })
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

    Ok(HoursAdmin {
        weekly,
        special,
        orders_paused,
        snooze_until_label,
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
#[server(name = SetOrdersPaused, prefix = "/api", endpoint = "set_orders_paused")]
pub async fn set_orders_paused(paused: bool) -> Result<(), ServerFnError> {
    // update_setting already enforces admin auth + write-through.
    // Turning OFF also clears any active timed snooze (handled inside
    // update_setting), so "Wieder öffnen" fully reopens.
    crate::pages::settings::update_setting(
        "orders_paused".to_string(),
        if paused { "1" } else { "0" }.to_string(),
    )
    .await
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
        return crate::pages::settings::update_setting("orders_paused".to_string(), "0".to_string())
            .await;
    }
    if minutes > 24 * 60 {
        return Err(ServerFnError::new("Pause darf höchstens 24 Stunden sein."));
    }
    let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
    let stamp = until.format("%Y-%m-%d %H:%M:%S").to_string();
    // Timer governs → keep the indefinite switch off so it auto-resumes.
    crate::pages::settings::update_setting("orders_paused".to_string(), "0".to_string()).await?;
    crate::pages::settings::update_setting("orders_paused_until".to_string(), stamp).await
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
    let pause_setter = ServerAction::<SetOrdersPaused>::new();
    let snoozer = ServerAction::<SnoozeOrders>::new();

    let data = Resource::new(
        move || {
            (
                row_updater.version().get(),
                special_upsert.version().get(),
                special_delete.version().get(),
                pause_setter.version().get(),
                snoozer.version().get(),
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
                        Ok(d) => view! { <HoursBody d row_updater special_upsert special_delete pause_setter snoozer/> }.into_any(),
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
    pause_setter: ServerAction<SetOrdersPaused>,
    snoozer: ServerAction<SnoozeOrders>,
) -> impl IntoView {
    let paused = d.orders_paused;
    let snooze_label = d.snooze_until_label.clone();
    let snoozing = !snooze_label.is_empty();
    // "Closed right now" = indefinite switch OR an active timer.
    let closed = paused || snoozing;

    let on_toggle = move |_| {
        // The big button: when closed (either reason) → reopen; else
        // start an indefinite pause.
        pause_setter.dispatch(SetOrdersPaused { paused: !closed });
    };
    let snooze = move |mins: i64| {
        move |_| {
            snoozer.dispatch(SnoozeOrders { minutes: mins });
        }
    };

    // Group weekday rows for the table (Mon-first display order).
    let weekly = d.weekly.clone();
    let week_order = [1_i64, 2, 3, 4, 5, 6, 0];

    view! {
        // ----- Quick open/close + snooze -----
        <div class=move || if closed { "quick-toggle paused" } else { "quick-toggle open" }>
            <div class="state">
                <span class="dot"></span>
                <strong>
                    {if snoozing {
                        format!("Pausiert — automatisch wieder offen ab {snooze_label} Uhr")
                    } else if paused {
                        "Online-Bestellungen sind PAUSIERT".to_string()
                    } else {
                        "Online-Bestellungen laufen (nach Öffnungszeiten)".to_string()
                    }}
                </strong>
            </div>
            <button class=move || if closed { "btn primary big" } else { "btn danger big" }
                    on:click=on_toggle>
                {if closed { "Wieder öffnen" } else { "Jetzt schließen" }}
            </button>
        </div>

        // Snooze buttons: pause for a fixed time, then auto-resume.
        <div class="snooze-row">
            <span class="snooze-label">"Bei Überlastung kurz pausieren:"</span>
            <button class="btn ghost" on:click=snooze(30)>"30 Min"</button>
            <button class="btn ghost" on:click=snooze(60)>"1 Std"</button>
            <button class="btn ghost" on:click=snooze(120)>"2 Std"</button>
        </div>
        <p class="hint">
            "Schnellpause stoppt eingehende Bestellungen für die gewählte Dauer und gibt sie \
             danach automatisch wieder frei. \"Jetzt schließen\" pausiert ohne Zeitlimit, \
             \"Wieder öffnen\" hebt beides sofort auf."
        </p>

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
