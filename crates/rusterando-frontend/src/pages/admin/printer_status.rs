//! /admin/drucker — kitchen device status (RPi4, STM32MP135F-DK, Tab5) + OTA control.
//!
//! Surfaces what `kitchen_clients` records on every Hello (running
//! version, arch, last seen) next to the server's target version, with a
//! compat badge — so a version skew like the one that silently stopped
//! printing is **visible**, not discovered by SSHing in. The "<device>
//! aktualisieren" button arms the in-band self-update for the next Hello.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;

/// One row of Pi status for the admin panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiStatus {
    pub shop_slug: String,
    pub version: String,
    pub arch: String,
    /// Unix seconds of last Hello/heartbeat (0 = never).
    pub last_seen_at: i64,
    /// Seconds since last_seen (i64::MAX-ish sentinel if never).
    pub seconds_since_seen: i64,
    pub update_armed: bool,
    /// This device keeps the colour e-ink frame up to date (switched here).
    #[serde(default)]
    pub eink: bool,
    /// Server's target version for this Pi's arch, if an artifact is staged.
    pub target_version: Option<String>,
    /// True when the staged target isn't an update for this device: the same
    /// version, an older build, or nothing staged ([`update_wanted`]).
    pub up_to_date: bool,
    pub last_update_to: String,
    pub last_update_ok: bool,
    pub last_update_error: String,
    pub last_update_at: i64,
    /// Kitchen displays (ESP32-P4) report whether a USB printer is plugged
    /// in; `None` = a tunnel Pi, which always prints.
    pub has_printer: Option<bool>,
    /// Seconds since the last receipt was confirmed (`None` = none yet).
    /// A client confirms a receipt only once it is printed, so this is the
    /// printer's sign of life for the tunnel clients, which can't report a
    /// plugged-in printer. Shop-wide: one printing client per shop.
    pub last_print_secs: Option<i64>,
    /// Receipts not confirmed yet, and the age of the oldest one.
    pub waiting: i64,
    pub oldest_waiting_secs: i64,
    /// Heard over LoRa by a connected device (its relay reports it): which
    /// one, how long ago, at what RSSI. `None` = never (since the restart).
    #[serde(default)]
    pub lora_via: Option<String>,
    #[serde(default)]
    pub lora_secs: Option<i64>,
    #[serde(default)]
    pub lora_rssi: Option<i16>,
}

/// LoRa heard this recently: the device is reachable (over the relay).
pub const LORA_FRESH_SECS: i64 = 90;

/// Kitchen devices heard over LoRa by a connected one: its relay reports
/// them with every `/api/kitchen/devices` check (`?via=…&lora=…`). Kept in
/// memory by device name (unique: from the MAC); a restart forgets them
/// until the next report, 30 s later.
#[cfg(feature = "ssr")]
pub mod lora_seen {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    #[derive(Clone, Debug)]
    pub struct Seen {
        pub via: String,
        /// Unix seconds it was last heard.
        pub at: i64,
        pub rssi: i16,
    }

    fn map() -> &'static Mutex<HashMap<String, Seen>> {
        static MAP: OnceLock<Mutex<HashMap<String, Seen>>> = OnceLock::new();
        MAP.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn valid(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 40
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }

    /// `lora` as the relay sends it: `name:rssi:secs,…`.
    pub fn record(via: &str, lora: &str, now: i64) {
        if !valid(via) {
            return;
        }
        let Ok(mut map) = map().lock() else { return };
        for entry in lora.split(',').take(16) {
            let mut f = entry.rsplitn(3, ':');
            let (Some(secs), Some(rssi), Some(name)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            let (Ok(secs), Ok(rssi)) = (secs.parse::<i64>(), rssi.parse::<i16>()) else {
                continue;
            };
            if !valid(name) || map.len() >= 256 && !map.contains_key(name) {
                continue;
            }
            let at = now - secs.clamp(0, 3600);
            // The freshest word wins (two devices may hear it).
            if map.get(name).is_some_and(|s| s.at > at) {
                continue;
            }
            map.insert(
                name.to_string(),
                Seen {
                    via: via.to_string(),
                    at,
                    rssi,
                },
            );
        }
    }

    pub fn get(name: &str) -> Option<Seen> {
        map().lock().ok()?.get(name).cloned()
    }
}

/// SSR-only DB row shape for `kitchen_clients`. Kept separate from the
/// wire `PiStatus` so the query stays a simple `query_as` (no 9-tuple).
#[cfg(feature = "ssr")]
#[derive(sqlx::FromRow)]
struct ClientRow {
    shop_slug: String,
    version: String,
    arch: String,
    last_seen_at: i64,
    update_armed: i64,
    eink: i64,
    last_update_to: String,
    last_update_ok: i64,
    last_update_error: String,
    last_update_at: i64,
    has_printer: Option<i64>,
}

#[server(name = ListPiStatus, prefix = "/api", endpoint = "list_pi_status")]
pub async fn list_pi_status() -> Result<Vec<PiStatus>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows: Vec<ClientRow> = sqlx::query_as(
        "SELECT shop_slug, version, arch, last_seen_at, update_armed, eink, \
                last_update_to, last_update_ok, last_update_error, last_update_at, has_printer \
         FROM kitchen_clients ORDER BY shop_slug",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load kitchen_clients: {e}")))?;

    let now = chrono::Utc::now().timestamp();

    let (last_print_at, waiting, oldest_waiting_at): (Option<i64>, i64, Option<i64>) =
        sqlx::query_as(
            "SELECT MAX(acked_at), COUNT(*) - COUNT(acked_at), \
                    MIN(CASE WHEN acked_at IS NULL THEN created_at END) \
             FROM kitchen_outbox",
        )
        .fetch_one(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("load kitchen_outbox: {e}")))?;
    let last_print_secs = last_print_at.map(|t| (now - t).max(0));
    let oldest_waiting_secs = oldest_waiting_at.map_or(0, |t| (now - t).max(0));

    // The artifact store lives in the server crate; the frontend (SSR) can't
    // depend on it directly. Read the target version straight from the same
    // artifacts dir the server uses, by arch.
    let artifact_dir =
        std::env::var("PRINTER_ARTIFACT_DIR").unwrap_or_else(|_| "artifacts/printer".to_string());

    Ok(rows
        .into_iter()
        .map(|r| {
            let target_version = std::fs::read_to_string(
                std::path::Path::new(&artifact_dir)
                    .join(&r.arch)
                    .join("version"),
            )
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

            let up_to_date = match &target_version {
                Some(t) => !update_wanted(&r.version, t),
                None => true, // nothing staged → nothing to push
            };
            let seconds_since_seen = if r.last_seen_at == 0 {
                i64::MAX
            } else {
                (now - r.last_seen_at).max(0)
            };

            let lora = lora_seen::get(&r.shop_slug);
            PiStatus {
                lora_via: lora.as_ref().map(|l| l.via.clone()),
                lora_secs: lora.as_ref().map(|l| (now - l.at).max(0)),
                lora_rssi: lora.as_ref().map(|l| l.rssi),
                shop_slug: r.shop_slug,
                version: r.version,
                arch: r.arch,
                last_seen_at: r.last_seen_at,
                seconds_since_seen,
                update_armed: r.update_armed != 0,
                eink: r.eink != 0,
                target_version,
                up_to_date,
                last_update_to: r.last_update_to,
                last_update_ok: r.last_update_ok != 0,
                last_update_error: r.last_update_error,
                last_update_at: r.last_update_at,
                has_printer: r.has_printer.map(|v| v != 0),
                last_print_secs,
                waiting,
                oldest_waiting_secs,
            }
            // (last_update_at is surfaced in PiStatus for future use.)
        })
        .collect())
}

/// Arm (or disarm) the in-band self-update for a shop's Pi. When armed,
/// the server offers the current target binary on the Pi's next Hello.
/// "Alarm auslösen": the kitchen display's new-order alert (3× white flash +
/// gong) right now, with an optional message on the flash — and the same
/// message as an urgent push (gong) to the devices logged in as Küche, so a
/// kitchen iPad gets it even with the display closed. Admin-only. Returns
/// the status line for the page.
#[server(name = KitchenDisplayAlert, prefix = "/api", endpoint = "kitchen_display_alert")]
pub async fn kitchen_display_alert(message: String) -> Result<String, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    #[cfg(feature = "ssr")]
    {
        let kitchen = use_context::<crate::pages::push::KitchenSinkHandle>().flatten();
        let push = use_context::<crate::pages::push::BroadcastSinkHandle>().flatten();
        if kitchen.is_none() && push.is_none() {
            return Err(ServerFnError::new(
                "Weder Küchenkanal (KITCHEN_LISTEN_ADDR) noch Push konfiguriert.",
            ));
        }
        let text: String = message.trim().chars().take(60).collect();
        let (title, text) = if text.is_empty() {
            ("TESTALARM".to_string(), "Gruß aus dem Admin".to_string())
        } else {
            ("NACHRICHT".to_string(), text)
        };
        if let Some(push) = &push {
            push.notify_roles_urgent(
                &["kitchen"],
                &format!("Küche: {title}"),
                &text,
                Some("/kitchen"),
            );
        }
        let reached = match &kitchen {
            Some(k) => k.display_alert(title, text).await,
            None => 0,
        };
        let displays = if reached == 0 {
            "Kein Küchen-Display verbunden".to_string()
        } else {
            format!("Gesendet an {reached} Display(s)")
        };
        Ok(if push.is_some() {
            format!("{displays} · Push an die Küchen-Geräte")
        } else {
            displays
        })
    }
    #[cfg(not(feature = "ssr"))]
    {
        let _ = message;
        Ok(String::new())
    }
}

/// Switch which device keeps the colour e-ink frame (the Tagesangebot) up to
/// date: stored here, survives restarts and power-off. The board feeds look
/// again at once (kitchen_board.rs); a device switched off gets an empty
/// tag and leaves the frame alone.
#[server(name = SetDeviceEink, prefix = "/api", endpoint = "set_device_eink")]
pub async fn set_device_eink(shop_slug: String, on: bool) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query("UPDATE kitchen_clients SET eink = ?2 WHERE shop_slug = ?1")
        .bind(&shop_slug)
        .bind(if on { 1 } else { 0 })
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("e-ink switch: {e}")))?;
    crate::pages::daily_offer::ssr::picture_changed();
    Ok(())
}

#[server(name = SetPiUpdateArmed, prefix = "/api", endpoint = "set_pi_update_armed")]
pub async fn set_pi_update_armed(shop_slug: String, armed: bool) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    sqlx::query("UPDATE kitchen_clients SET update_armed = ?2 WHERE shop_slug = ?1")
        .bind(&shop_slug)
        .bind(if armed { 1 } else { 0 })
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("arm update: {e}")))?;
    Ok(())
}

/// A device counts as gone (and may be removed from the list) after this
/// long without a Hello or heartbeat — the card shows it red from here on.
pub const FORGET_AFTER_SECS: i64 = 600;

/// "Entfernen": drop a kitchen device that is gone for good (replaced, or
/// switched off) from the list — and from the printer-offline monitor,
/// which otherwise alerts for it again after every server restart. Only
/// while it is offline; should it ever connect again, its Hello adds it
/// back. Admin-only.
#[server(name = ForgetKitchenClient, prefix = "/api", endpoint = "forget_kitchen_client")]
pub async fn forget_kitchen_client(shop_slug: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let done = sqlx::query(
        "DELETE FROM kitchen_clients WHERE shop_slug = ?1 \
         AND (last_seen_at = 0 OR strftime('%s','now') - last_seen_at >= ?2)",
    )
    .bind(&shop_slug)
    .bind(FORGET_AFTER_SECS)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("forget device: {e}")))?;
    if done.rows_affected() == 0 {
        return Err(ServerFnError::new(format!(
            "{shop_slug} ist gerade verbunden (oder schon entfernt)."
        )));
    }
    Ok(())
}

#[component]
pub fn PrinterStatusPage() -> impl IntoView {
    let armer = ServerAction::<SetPiUpdateArmed>::new();
    let einker = ServerAction::<SetDeviceEink>::new();
    let forgetter = ServerAction::<ForgetKitchenClient>::new();
    let alerter = ServerAction::<KitchenDisplayAlert>::new();
    let alert_text = RwSignal::new(String::new());
    let rows = Resource::new(
        move || (armer.version().get(), forgetter.version().get(), einker.version().get()),
        |_| async move { list_pi_status().await },
    );

    view! {
        <AdminShell>
            <section class="printer-status">
                <header class="admin-bar"><h1>"Drucker"</h1></header>
                <p class="hint">
                    "Status der Küchen-Geräte (RPi4, STM32MP135F-DK, Tab5). Zeigt die laufende Version gegen die "
                    "Server-Zielversion — so bleibt ein Versions-Mismatch sichtbar. "
                    "Updates laufen over-the-air über die Druckverbindung."
                </p>

                <div class="kds-alert">
                    <h2>"Küchen-Display"</h2>
                    <p class="hint">
                        "Dasselbe Display im Browser, live und bedienbar: "
                        <a href="/kitchen/display" target="_blank" rel="noopener">"Küchen-Display öffnen"</a>
                    </p>
                    <p class="hint">
                        "Löst den Neue-Bestellung-Alarm auf dem Küchen-Display aus: "
                        "3× weißes Blinken + Gong. Optional mit einer Nachricht auf dem Blinken. "
                        "Geht zusätzlich als Push (Gong) an alle Geräte, die als Küche angemeldet sind."
                    </p>
                    <input type="text" maxlength="60" placeholder="Nachricht (optional)"
                        prop:value=move || alert_text.get()
                        on:input=move |ev| alert_text.set(event_target_value(&ev))/>
                    <button class="btn primary" on:click=move |_| {
                        alerter.dispatch(KitchenDisplayAlert { message: alert_text.get() });
                    }>"Alarm auslösen"</button>
                    {move || alerter.value().get().map(|r| match r {
                        Ok(line) => view! { <span class="ok">{format!(" {line}.")}</span> }.into_any(),
                        Err(e) => view! { <span class="error">{format!(" {e}")}</span> }.into_any(),
                    })}
                </div>

                {move || forgetter.value().get().and_then(|r| r.err()).map(|e| view! {
                    <p class="error">{e.to_string()}</p>
                })}
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || rows.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(list) if list.is_empty() => {
                            view! { <p class="empty">"Noch kein Drucker verbunden."</p> }.into_any()
                        }
                        Ok(list) => view! {
                            <div class="pi-cards">
                                {list.into_iter().map(|p| view! { <PiCard p armer einker forgetter/> }).collect_view()}
                            </div>
                        }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

/// The kitchen device generations, by the arch each one reports:
/// Raspberry Pi 4, then the STM32MP135F-DK (touch display, still Linux),
/// then the all-in-one M5Stack Tab5 and the 7" Guition JC1060P470 (same
/// firmware, its own update image).
fn device_model(arch: &str) -> Option<&'static str> {
    match arch {
        "aarch64-unknown-linux-gnu" => Some("RPi4"),
        "armv7-unknown-linux-gnueabihf" => Some("STM32MP135F-DK"),
        "esp32p4" => Some("Tab5"),
        "esp32p4-jc1060p470" => Some("JC1060P470"),
        _ => None,
    }
}

/// Whether a device running `running` should get the staged image `staged`.
/// Tab5 versions end in their build time ("0.1.0-44db834-20261029-1130");
/// when both carry one, only a newer build is an update. One image is staged
/// for all devices of an arch, so a Tab5 flashed over USB after the last
/// staging would otherwise be "updated" back to the older image. Versions
/// without a build time (the Linux boards: "0.3.0") differ → update.
pub fn update_wanted(running: &str, staged: &str) -> bool {
    if running == staged {
        return false;
    }
    match (build_time(running), build_time(staged)) {
        (Some(running), Some(staged)) => staged >= running,
        _ => true,
    }
}

/// A version's trailing build time, "YYYYMMDD-HHMM", if it has one.
fn build_time(version: &str) -> Option<&str> {
    let start = version.len().checked_sub(14)?;
    let tail = version.get(start..)?;
    let stamp = tail.strip_prefix('-')?;
    let (date, time) = stamp.split_once('-')?;
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    (digits(date, 8) && digits(time, 4)).then_some(stamp)
}

/// The card's printer line. A display without printer just says so; a
/// printing client shows its last printed receipt, or a warning once a
/// receipt has waited a minute without being printed.
fn printer_line(p: &PiStatus) -> (&'static str, String) {
    let lead = match p.has_printer {
        Some(false) => return ("muted", "kein Drucker — nur Anzeige".to_string()),
        Some(true) => "USB-Drucker angeschlossen — ",
        None => "",
    };
    if p.waiting > 0 && p.oldest_waiting_secs >= 60 {
        let n = if p.waiting == 1 {
            "1 Bon wartet".to_string()
        } else {
            format!("{} Bons warten", p.waiting)
        };
        return (
            "error",
            format!(
                "⚠ {n} seit {} — Drucker prüfen (Strom, Kabel, Papier)",
                ago(p.oldest_waiting_secs)
            ),
        );
    }
    let last = match p.last_print_secs {
        Some(s) => format!("letzter Bon gedruckt {}", ago(s)),
        None => "noch kein Bon gedruckt".to_string(),
    };
    ("ok", format!("{lead}{last}"))
}

/// "vor 40 s", "vor 12 min", "vor 19 h", "vor 3 Tagen".
fn ago(secs: i64) -> String {
    match secs {
        s if s < 90 => format!("vor {s} s"),
        s if s < 90 * 60 => format!("vor {} min", s / 60),
        s if s < 48 * 3600 => format!("vor {} h", s / 3600),
        s => format!("vor {} Tagen", s / 86400),
    }
}

#[component]
fn PiCard(
    p: PiStatus,
    armer: ServerAction<SetPiUpdateArmed>,
    einker: ServerAction<SetDeviceEink>,
    forgetter: ServerAction<ForgetKitchenClient>,
) -> impl IntoView {
    // Connection freshness: green <90s, orange "LoRa" when only the relay
    // still hears it, amber <10min, red otherwise.
    let tcp_online = p.seconds_since_seen < 90;
    let lora_fresh = p.lora_secs.is_some_and(|s| s < LORA_FRESH_SECS);
    let (conn_cls, conn_label) = if tcp_online {
        ("pi-conn online", "online".to_string())
    } else if lora_fresh {
        ("pi-conn lora", "LoRa".to_string())
    } else if p.seconds_since_seen == i64::MAX {
        ("pi-conn offline", "nie gesehen".to_string())
    } else if p.seconds_since_seen < 600 {
        ("pi-conn stale", format!("vor {}s", p.seconds_since_seen))
    } else {
        (
            "pi-conn offline",
            format!("vor {}min", p.seconds_since_seen / 60),
        )
    };

    let (ver_cls, ver_label) = match &p.target_version {
        Some(t) if !p.up_to_date => ("pi-ver mismatch", format!("{} → Ziel {}", p.version, t)),
        Some(t) if *t != p.version => (
            "pi-ver ok",
            format!("{} (neuer als bereitgestellt: {})", p.version, t),
        ),
        _ => ("pi-ver ok", format!("{} (aktuell)", p.version)),
    };

    let can_update = p.target_version.is_some() && !p.up_to_date;
    let model = device_model(&p.arch);
    let update_label = match model {
        Some(m) => format!("{m} aktualisieren"),
        None => "Aktualisieren".to_string(),
    };
    let shop_for_arm = p.shop_slug.clone();
    let shop_for_disarm = p.shop_slug.clone();
    let armed = p.update_armed;
    // Not on TCP/IP: when it was, and what LoRa says.
    let link_line = (!tcp_online).then(|| {
        let tcp = if p.seconds_since_seen == i64::MAX {
            "TCP/IP nie".to_string()
        } else {
            format!("TCP/IP {}", ago(p.seconds_since_seen))
        };
        let via = p.lora_via.clone().unwrap_or_default();
        let rssi = p
            .lora_rssi
            .map(|r| format!(", {r} dBm"))
            .unwrap_or_default();
        match p.lora_secs {
            Some(s) if s < LORA_FRESH_SECS => (true, format!("{tcp} · LoRa über {via}{rssi}")),
            Some(s) => (false, format!("{tcp} · LoRa {} (über {via})", ago(s))),
            None => (false, tcp),
        }
    });
    // Gone devices get "Entfernen", with a second tap to confirm — not while
    // LoRa still hears them.
    let gone = p.seconds_since_seen >= FORGET_AFTER_SECS && !lora_fresh;
    let shop_for_forget = p.shop_slug.clone();
    let confirm_forget = RwSignal::new(false);

    view! {
        <div class="pi-card">
            <header class="pi-head">
                <strong class="pi-shop">{p.shop_slug.clone()}</strong>
                <span class=conn_cls>{conn_label}</span>
            </header>
            <dl class="pi-meta">
                <dt>"Gerät"</dt>
                <dd>{model.unwrap_or("unbekannt")}</dd>
                <dt>"Version"</dt>
                <dd class=ver_cls>{ver_label}</dd>
                <dt>"Arch"</dt>
                <dd class="muted">{p.arch.clone()}</dd>
                {link_line.map(|(fresh, text)| {
                    let cls = if fresh { "lora" } else { "muted" };
                    view! { <><dt>"Verbindung"</dt><dd class=cls>{text}</dd></> }
                })}
                {
                    let (cls, txt) = printer_line(&p);
                    view! { <><dt>"Drucker"</dt><dd class=cls>{txt}</dd></> }
                }
                // The Tab5s and the 7" can drive the colour e-ink frame.
                {p.arch.starts_with("esp32p4").then(|| {
                    let shop = p.shop_slug.clone();
                    let on = p.eink;
                    view! {
                        <>
                            <dt>"E-Ink-Rahmen"</dt>
                            <dd>
                                <label class="pi-eink">
                                    <input type="checkbox" prop:checked=on
                                        on:change=move |ev| {
                                            einker.dispatch(SetDeviceEink {
                                                shop_slug: shop.clone(),
                                                on: event_target_checked(&ev),
                                            });
                                        }/>
                                    {if on {
                                        " zeigt das Tagesangebot auf dem Rahmen (Bluetooth)"
                                    } else {
                                        " aus"
                                    }}
                                </label>
                            </dd>
                        </>
                    }
                })}
                {(!p.last_update_to.is_empty()).then(|| {
                    let cls = if p.last_update_ok { "ok" } else { "error" };
                    let txt = if p.last_update_ok {
                        format!("✓ auf {} aktualisiert", p.last_update_to)
                    } else {
                        format!("✗ Update auf {} fehlgeschlagen: {}", p.last_update_to, p.last_update_error)
                    };
                    view! { <><dt>"Letztes Update"</dt><dd class=cls>{txt}</dd></> }
                })}
            </dl>

            <div class="pi-actions">
                {move || if armed {
                    let shop = shop_for_disarm.clone();
                    view! {
                        <span class="armed">"⏳ Update vorgemerkt — wird beim nächsten Verbinden ausgeführt"</span>
                        <button class="btn ghost small"
                            on:click=move |_| { armer.dispatch(SetPiUpdateArmed { shop_slug: shop.clone(), armed: false }); }>
                            "Abbrechen"
                        </button>
                    }.into_any()
                } else if can_update {
                    let shop = shop_for_arm.clone();
                    view! {
                        <button class="btn primary small"
                            on:click=move |_| { armer.dispatch(SetPiUpdateArmed { shop_slug: shop.clone(), armed: true }); }>
                            {update_label.clone()}
                        </button>
                    }.into_any()
                } else {
                    view! { <span class="muted small">"Kein Update verfügbar"</span> }.into_any()
                }}
                {gone.then(|| {
                    let shop = shop_for_forget.clone();
                    view! {
                        <button class="btn ghost small" title="Gerät aus der Liste entfernen. Verbindet es sich wieder, erscheint es von selbst."
                            on:click=move |_| {
                                if confirm_forget.get_untracked() {
                                    forgetter.dispatch(ForgetKitchenClient { shop_slug: shop.clone() });
                                } else {
                                    confirm_forget.set(true);
                                }
                            }>
                            {move || if confirm_forget.get() { "Wirklich entfernen?" } else { "Entfernen" }}
                        </button>
                    }
                })}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::update_wanted;

    #[test]
    fn only_a_newer_build_is_an_update() {
        let staged = "0.1.0-deee4f2-dirty-20261027-1140";
        // Flashed over USB after staging: newer than the staged image.
        assert!(!update_wanted("0.1.0-872de01-dirty-20261027-1745", staged));
        assert!(update_wanted("0.1.0-ba14735-20261025-1706", staged));
        assert!(!update_wanted(staged, staged));
        // A build without a time (before this rule) still gets the update.
        assert!(update_wanted("0.1.0-872de01-dirty", staged));
    }

    #[test]
    fn linux_boards_update_whenever_the_version_differs() {
        assert!(update_wanted("0.3.0", "0.3.1"));
        assert!(!update_wanted("0.3.1", "0.3.1"));
    }
}

#[cfg(all(test, feature = "ssr"))]
mod lora_seen_tests {
    use super::lora_seen::{get, record};

    #[test]
    fn reports_are_parsed_and_the_freshest_wins() {
        record(
            "tab5-aaaa01",
            "tab5-bbbb01:-48:12,cardputer-cccc01:-71:3",
            1_000,
        );
        let b = get("tab5-bbbb01").expect("heard");
        assert_eq!((b.via.as_str(), b.at, b.rssi), ("tab5-aaaa01", 988, -48));
        assert_eq!(get("cardputer-cccc01").map(|c| c.rssi), Some(-71));
        // An older word from another device doesn't replace a fresher one.
        record("tab5-dddd01", "tab5-bbbb01:-60:30", 1_000);
        assert_eq!(
            get("tab5-bbbb01").map(|b| b.via),
            Some("tab5-aaaa01".to_string())
        );
        // A newer one does.
        record("tab5-dddd01", "tab5-bbbb01:-60:0", 1_010);
        assert_eq!(
            get("tab5-bbbb01").map(|b| (b.via, b.at)),
            Some(("tab5-dddd01".to_string(), 1_010))
        );
    }

    #[test]
    fn junk_is_ignored() {
        record(
            "tab5-eeee01",
            "no-colons,x:y:z,bad name!:-1:1,tab5-ffff01:-50",
            1_000,
        );
        record("bad via!", "tab5-gggg01:-50:1", 1_000);
        assert!(get("no-colons").is_none());
        assert!(get("bad name!").is_none());
        assert!(get("tab5-ffff01").is_none());
        assert!(get("tab5-gggg01").is_none());
    }
}
