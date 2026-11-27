//! /admin/lora — the LoRa lab: what the tab5-lab Tabs send and hear over
//! LoRa, as a chat, and messages typed here for a Tab to send.
//!
//! The Tabs post every packet they send, receive or miss an answer to
//! (server: rusterando-server lab_lora.rs, table `lab_lora_events`). One
//! message shows up once per Tab that saw it — `tx` at the sender, `rx` at
//! each receiver, the ACK back at the sender — and [`group`] folds those
//! into one chat row: who sent what, who heard it how well, and how long
//! the answer took. Messages typed here wait in `lab_lora_outbox` until
//! the chosen Tab picks them up (it asks every few seconds).

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;

/// Events loaded for the chat (the newest).
#[cfg(feature = "ssr")]
const LOAD_EVENTS: i64 = 1500;
/// Chat rows shown (the newest).
#[cfg(feature = "ssr")]
const SHOW_ROWS: usize = 300;
/// A receiver's report belongs to a message sent at most this long before.
const MATCH_WINDOW: i64 = 120;
/// A Tab counts as there when it asked within this time (it asks every
/// 3 s; `last_seen` is written every 30 s).
const ONLINE_SECS: i64 = 75;
/// Longest message: the LoRa frame `T5 MSG <seq> <sender> <text>` must
/// stay under 255 bytes.
const MAX_TEXT: usize = 200;

/// One event as one Tab reported it (a row of `lab_lora_events`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ssr", derive(sqlx::FromRow))]
pub struct LoraEvent {
    pub id: i64,
    pub received_at: i64,
    pub device: String,
    pub dir: String,
    pub kind: String,
    pub seq: Option<i64>,
    pub sender: Option<String>,
    pub text: String,
    pub rssi: Option<i64>,
    pub snr: Option<f64>,
    pub peer_rssi: Option<i64>,
    pub peer_snr: Option<f64>,
    pub rtt_ms: Option<i64>,
    pub admin_id: Option<i64>,
}

/// A Tab that heard a message, and how well.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Heard {
    pub device: String,
    pub rssi: Option<i64>,
    pub snr: Option<f64>,
}

/// The ACK/PONG that came back to the sender.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub by: String,
    pub rtt_ms: Option<i64>,
    /// How the sender heard the answer.
    pub rssi: Option<i64>,
    pub snr: Option<f64>,
}

/// One message (or ping) with everything the Tabs reported about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRow {
    /// Unix seconds of the first report.
    pub at: i64,
    /// MSG, PING or RAW.
    pub kind: String,
    pub sender: String,
    pub seq: Option<i64>,
    pub text: String,
    /// Typed on this page.
    pub via_admin: bool,
    pub heard: Vec<Heard>,
    pub answer: Option<Answer>,
    /// The sender waited for an answer in vain.
    pub lost: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoraDevice {
    pub name: String,
    pub secs_ago: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Queued {
    pub device: String,
    pub text: String,
    pub secs_ago: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoraView {
    pub devices: Vec<LoraDevice>,
    /// Oldest first.
    pub chat: Vec<ChatRow>,
    pub waiting: Vec<Queued>,
}

/// Fold the Tabs' reports (oldest first) into chat rows (oldest first).
pub fn group(events: &[LoraEvent]) -> Vec<ChatRow> {
    let mut rows: Vec<ChatRow> = Vec::new();
    // The open row for (sender, seq, MSG|PING) sent within MATCH_WINDOW.
    fn find<'a>(
        rows: &'a mut [ChatRow],
        sender: &str,
        seq: Option<i64>,
        kind: &str,
        at: i64,
    ) -> Option<&'a mut ChatRow> {
        rows.iter_mut().rev().find(|r| {
            r.sender == sender && r.seq == seq && r.kind == kind && at - r.at <= MATCH_WINDOW
        })
    }
    for e in events {
        let sender = e.sender.clone().unwrap_or_else(|| e.device.clone());
        match (e.kind.as_str(), e.dir.as_str()) {
            (kind @ ("MSG" | "PING"), "tx") => rows.push(ChatRow {
                at: e.received_at,
                kind: kind.to_string(),
                sender,
                seq: e.seq,
                text: e.text.clone(),
                via_admin: e.admin_id.is_some(),
                heard: Vec::new(),
                answer: None,
                lost: false,
            }),
            (kind @ ("MSG" | "PING"), "rx") => {
                let heard = Heard {
                    device: e.device.clone(),
                    rssi: e.rssi,
                    snr: e.snr,
                };
                match find(&mut rows, &sender, e.seq, kind, e.received_at) {
                    Some(row) => row.heard.push(heard),
                    // The sender doesn't report (another LoRa device).
                    None => rows.push(ChatRow {
                        at: e.received_at,
                        kind: kind.to_string(),
                        sender,
                        seq: e.seq,
                        text: e.text.clone(),
                        via_admin: false,
                        heard: vec![heard],
                        answer: None,
                        lost: false,
                    }),
                }
            }
            // At the original sender (the reporting Tab): the answer from
            // `sender`, with how the answering end heard the message.
            (kind @ ("ACK" | "PONG"), "rx") => {
                let family = if kind == "ACK" { "MSG" } else { "PING" };
                if let Some(row) = find(&mut rows, &e.device, e.seq, family, e.received_at) {
                    row.answer = Some(Answer {
                        by: sender.clone(),
                        rtt_ms: e.rtt_ms,
                        rssi: e.rssi,
                        snr: e.snr,
                    });
                    // The answering end may not report itself.
                    if !row.heard.iter().any(|h| h.device == sender) {
                        row.heard.push(Heard {
                            device: sender,
                            rssi: e.peer_rssi,
                            snr: e.peer_snr,
                        });
                    }
                }
            }
            (kind @ ("ACK" | "PONG"), "lost") => {
                let family = if kind == "ACK" { "MSG" } else { "PING" };
                if let Some(row) = find(&mut rows, &e.device, e.seq, family, e.received_at) {
                    row.lost = true;
                }
            }
            // The answering end's own ACK/PONG: its rx report has it all.
            ("ACK" | "PONG", _) => {}
            _ => rows.push(ChatRow {
                at: e.received_at,
                kind: "RAW".into(),
                sender: e.sender.clone().unwrap_or_else(|| "?".into()),
                seq: e.seq,
                text: e.text.clone(),
                via_admin: false,
                heard: vec![Heard {
                    device: e.device.clone(),
                    rssi: e.rssi,
                    snr: e.snr,
                }],
                answer: None,
                lost: false,
            }),
        }
    }
    rows
}

/// Whether this deployment takes the Tabs' LoRa reports (`LAB_KEY` set, as
/// lab_lora.rs checks it): the admin menu shows "LoRa" only then, so the
/// shops on a shared server don't get an empty lab tab.
#[server(name = LabLoraEnabled, prefix = "/api", endpoint = "lab_lora_enabled")]
pub async fn lab_lora_enabled() -> Result<bool, ServerFnError> {
    Ok(std::env::var("LAB_KEY").is_ok_and(|k| !k.trim().is_empty()))
}

#[server(name = ListLora, prefix = "/api", endpoint = "list_lora")]
pub async fn list_lora() -> Result<LoraView, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let now = chrono::Utc::now().timestamp();

    let mut events: Vec<LoraEvent> = sqlx::query_as(
        "SELECT id, received_at, device, dir, kind, seq, sender, text, rssi, snr, \
                peer_rssi, peer_snr, rtt_ms, admin_id \
         FROM lab_lora_events ORDER BY id DESC LIMIT ?",
    )
    .bind(LOAD_EVENTS)
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load lab_lora_events: {e}")))?;
    events.reverse();
    let mut chat = group(&events);
    if chat.len() > SHOW_ROWS {
        chat.drain(..chat.len() - SHOW_ROWS);
    }

    let devices: Vec<(String, i64)> =
        sqlx::query_as("SELECT device, last_seen FROM lab_lora_devices ORDER BY device")
            .fetch_all(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("load lab_lora_devices: {e}")))?;
    let waiting: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT device, text, created_at FROM lab_lora_outbox \
         WHERE handed_out_at IS NULL ORDER BY id",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load lab_lora_outbox: {e}")))?;

    Ok(LoraView {
        devices: devices
            .into_iter()
            .map(|(name, seen)| LoraDevice {
                name,
                secs_ago: (now - seen).max(0),
            })
            .collect(),
        chat,
        waiting: waiting
            .into_iter()
            .map(|(device, text, at)| Queued {
                device,
                text,
                secs_ago: (now - at).max(0),
            })
            .collect(),
    })
}

#[server(name = SendLora, prefix = "/api", endpoint = "send_lora")]
pub async fn send_lora(device: String, text: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(ServerFnError::new("Nachricht ist leer"));
    }
    if text.len() > MAX_TEXT {
        return Err(ServerFnError::new(format!("höchstens {MAX_TEXT} Zeichen")));
    }
    let known: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM lab_lora_devices WHERE device = ?")
        .bind(&device)
        .fetch_optional(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("lab_lora_devices: {e}")))?;
    if known.is_none() {
        return Err(ServerFnError::new(format!("unbekanntes Gerät {device}")));
    }
    sqlx::query("INSERT INTO lab_lora_outbox (created_at, device, text) VALUES (?, ?, ?)")
        .bind(chrono::Utc::now().timestamp())
        .bind(&device)
        .bind(&text)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("queue: {e}")))?;
    Ok(())
}

#[component]
pub fn LoraPage() -> impl IntoView {
    let sender = ServerAction::<SendLora>::new();
    // Bumped by the live channel and a slow timer (online status).
    let news = RwSignal::new(0u64);
    let view_data = Resource::new(
        move || (sender.version().get(), news.get()),
        |_| async move { list_lora().await },
    );
    let device = RwSignal::new(String::new());
    let text = RwSignal::new(String::new());
    live_refresh(news);

    // Why Send did nothing (no Tab known yet).
    let note = RwSignal::new(None::<&'static str>);
    let send = move || {
        let msg = text.get_untracked();
        if msg.trim().is_empty() {
            return;
        }
        if device.get_untracked().is_empty() {
            note.set(Some(
                "Noch hat sich kein Tab5 gemeldet — erst dann kann eins senden.",
            ));
            return;
        }
        note.set(None);
        sender.dispatch(SendLora {
            device: device.get_untracked(),
            text: msg,
        });
        text.set(String::new());
    };

    view! {
        <AdminShell>
            <section class="lora-lab">
                <header class="admin-bar"><h1>"LoRa"</h1></header>
                <p class="hint">
                    "Was die Tab5-Labor-Geräte über LoRa senden und empfangen, live. "
                    "Jede Nachricht zeigt, wer sie gehört hat (Signal in dBm / Rauschabstand in dB) "
                    "und wie schnell die Bestätigung zurückkam. Hier getippte Nachrichten sendet das "
                    "gewählte Gerät, sobald es das nächste Mal fragt (alle paar Sekunden)."
                </p>
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || view_data.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(v) => {
                            // Preselect a Tab that is there, else any: an
                            // offline Tab picks its messages up on return.
                            if device.get_untracked().is_empty() {
                                if let Some(d) = v
                                    .devices
                                    .iter()
                                    .find(|d| d.secs_ago <= ONLINE_SECS)
                                    .or_else(|| v.devices.first())
                                {
                                    device.set(d.name.clone());
                                }
                            }
                            view! { <LoraBody v/> }.into_any()
                        }
                    })}
                </Suspense>
                <div class="lora-send">
                    <Transition fallback=|| ()>
                        {move || view_data.get().and_then(|r| r.ok()).map(|v| view! {
                            <select prop:value=move || device.get()
                                on:change=move |ev| device.set(event_target_value(&ev))>
                                {v.devices.into_iter().map(|d| {
                                    let online = d.secs_ago <= ONLINE_SECS;
                                    let label = if online {
                                        d.name.clone()
                                    } else {
                                        format!("{} (offline — sendet, wenn es zurück ist)", d.name)
                                    };
                                    view! { <option value=d.name.clone()>{label}</option> }
                                }).collect_view()}
                            </select>
                        })}
                    </Transition>
                    <input type="text" maxlength=MAX_TEXT placeholder="Nachricht über LoRa senden"
                        prop:value=move || text.get()
                        on:input=move |ev| text.set(event_target_value(&ev))
                        on:keydown=move |ev| if ev.key() == "Enter" { send(); }/>
                    <button class="btn primary" on:click=move |_| send()>"Senden"</button>
                    {move || sender.value().get().and_then(|r| r.err()).map(|e| view! {
                        <span class="error">{format!(" {e}")}</span>
                    })}
                    {move || note.get().map(|n| view! { <span class="error">{n}</span> })}
                </div>
            </section>
        </AdminShell>
    }
}

#[component]
fn LoraBody(v: LoraView) -> impl IntoView {
    let devices = if v.devices.is_empty() {
        view! { <p class="empty">"Noch kein Gerät hat sich gemeldet."</p> }.into_any()
    } else {
        view! {
            <ul class="lora-devices">
                {v.devices.into_iter().map(|d| {
                    let online = d.secs_ago <= ONLINE_SECS;
                    view! {
                        <li class=if online { "chip chip-info" } else { "chip" }>
                            {d.name}
                            {if online { " · da".to_string() } else { format!(" · vor {}", ago(d.secs_ago)) }}
                        </li>
                    }
                }).collect_view()}
            </ul>
        }
        .into_any()
    };
    let waiting = v
        .waiting
        .into_iter()
        .map(|q| {
            view! {
                <li class="lora-waiting">
                    {format!("wartet auf {} (seit {}): {}", q.device, ago(q.secs_ago), q.text)}
                </li>
            }
        })
        .collect_view();
    let chat = if v.chat.is_empty() {
        view! { <p class="empty">"Noch keine LoRa-Nachrichten."</p> }.into_any()
    } else {
        view! {
            <ol class="lora-chat">
                {v.chat.into_iter().rev().map(|row| view! { <ChatLine row/> }).collect_view()}
            </ol>
        }
        .into_any()
    };
    view! {
        {devices}
        <ul class="lora-queue">{waiting}</ul>
        {chat}
    }
}

#[component]
fn ChatLine(row: ChatRow) -> impl IntoView {
    let class = match row.kind.as_str() {
        "MSG" => "lora-msg",
        "PING" => "lora-ping",
        _ => "lora-raw",
    };
    let headline = match row.kind.as_str() {
        "MSG" => format!("{} → „{}“", row.sender, row.text),
        "PING" => format!("Ping #{} von {}", row.seq.unwrap_or(0), row.sender),
        _ => format!("unbekanntes Paket: {}", row.text),
    };
    let heard = row
        .heard
        .iter()
        .map(|h| format!("{}: {}", h.device, signal(h.rssi, h.snr)))
        .collect::<Vec<_>>()
        .join(" · ");
    let answer = match (&row.answer, row.lost) {
        (Some(a), _) => format!(
            "{} in {} ms zurück ({})",
            if row.kind == "PING" {
                "Pong"
            } else {
                "Bestätigung"
            },
            a.rtt_ms.unwrap_or(0),
            signal(a.rssi, a.snr)
        ),
        (None, true) => "keine Antwort".to_string(),
        (None, false) if row.kind == "RAW" => String::new(),
        (None, false) => "…".to_string(),
    };
    let lost = row.lost && row.answer.is_none();
    view! {
        <li class=class>
            <span class="ts">{clock(row.at)}</span>
            <span class="what">{headline}</span>
            {row.via_admin.then(|| view! { <span class="chip chip-info">"von hier"</span> })}
            <div class="detail">
                {if heard.is_empty() { "von niemandem gehört".to_string() } else { format!("gehört von {heard}") }}
                {(!answer.is_empty()).then(|| view! {
                    <span class=if lost { "lora-lost" } else { "lora-answer" }>{format!(" · {answer}")}</span>
                })}
            </div>
        </li>
    }
}

fn signal(rssi: Option<i64>, snr: Option<f64>) -> String {
    match (rssi, snr) {
        (Some(r), Some(s)) => format!("{r} dBm / {s:+.0} dB"),
        (Some(r), None) => format!("{r} dBm"),
        _ => "?".to_string(),
    }
}

/// Local wall-clock time of a unix timestamp, as the other admin pages do.
fn clock(unix: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(unix, 0)
        .single()
        .map(|t| t.format("%d.%m. %H:%M:%S").to_string())
        .unwrap_or_default()
}

fn ago(secs: i64) -> String {
    match secs {
        s if s < 90 => format!("{s} s"),
        s if s < 90 * 60 => format!("{} min", s / 60),
        s if s < 48 * 3600 => format!("{} h", s / 3600),
        s => format!("{} Tagen", s / 86_400),
    }
}

/// Reload on `/api/live/lora` pings (a Tab reported something), and every
/// 15 s for the online status. Reconnects with backoff like the other live
/// channels.
#[cfg(feature = "hydrate")]
fn live_refresh(news: RwSignal<u64>) {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    Effect::new(move |done: Option<()>| {
        if done.is_some() {
            return;
        }
        let backoff = Rc::new(RefCell::new(1_000u32));
        let connect: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let again = connect.clone();
        let open: Rc<dyn Fn()> = Rc::new(move || {
            let Ok(es) = web_sys::EventSource::new("/api/live/lora") else {
                return;
            };
            let b = backoff.clone();
            let bump = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                *b.borrow_mut() = 1_000;
                news.update(|v| *v += 1);
            });
            es.set_onmessage(Some(bump.as_ref().unchecked_ref()));
            es.set_onopen(Some(bump.as_ref().unchecked_ref()));
            bump.forget();
            let b = backoff.clone();
            let again = again.clone();
            let es_err = es.clone();
            let on_err = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                es_err.close();
                let delay = {
                    let mut cur = b.borrow_mut();
                    let d = *cur;
                    *cur = (d * 2).min(30_000);
                    d
                };
                if let Some(f) = again.borrow().clone() {
                    leptos::leptos_dom::helpers::set_timeout(
                        move || f(),
                        std::time::Duration::from_millis(delay as u64),
                    );
                }
            });
            es.set_onerror(Some(on_err.as_ref().unchecked_ref()));
            on_err.forget();
        });
        *connect.borrow_mut() = Some(open.clone());
        open();
        let _ = leptos::leptos_dom::helpers::set_interval_with_handle(
            move || news.update(|v| *v += 1),
            std::time::Duration::from_secs(15),
        );
    });
}

/// SSR: no live channel on the server.
#[cfg(not(feature = "hydrate"))]
fn live_refresh(_news: RwSignal<u64>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: i64, device: &str, dir: &str, kind: &str, seq: i64, sender: &str) -> LoraEvent {
        LoraEvent {
            id,
            received_at: 1000 + id,
            device: device.into(),
            dir: dir.into(),
            kind: kind.into(),
            seq: Some(seq),
            sender: Some(sender.into()),
            ..LoraEvent::default()
        }
    }

    #[test]
    fn message_heard_and_acked() {
        let mut tx = ev(1, "a", "tx", "MSG", 7, "a");
        tx.text = "hello".into();
        let mut rx = ev(2, "b", "rx", "MSG", 7, "a");
        rx.rssi = Some(-45);
        rx.snr = Some(9.0);
        let ack_tx = ev(3, "b", "tx", "ACK", 7, "b");
        let mut ack_rx = ev(4, "a", "rx", "ACK", 7, "b");
        ack_rx.rtt_ms = Some(180);
        ack_rx.rssi = Some(-47);
        ack_rx.peer_rssi = Some(-45);
        let rows = group(&[tx, rx, ack_tx, ack_rx]);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.text, "hello");
        assert_eq!(
            row.heard,
            vec![Heard {
                device: "b".into(),
                rssi: Some(-45),
                snr: Some(9.0)
            }]
        );
        let answer = row.answer.as_ref().expect("answer");
        assert_eq!(
            (answer.by.as_str(), answer.rtt_ms, answer.rssi),
            ("b", Some(180), Some(-47))
        );
        assert!(!row.lost);
    }

    #[test]
    fn ping_lost() {
        let rows = group(&[
            ev(1, "a", "tx", "PING", 3, "a"),
            ev(2, "a", "lost", "PONG", 3, "a"),
        ]);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].lost);
        assert!(rows[0].heard.is_empty());
    }

    #[test]
    fn foreign_sender_and_raw() {
        let mut rx = ev(1, "b", "rx", "MSG", 1, "arduino");
        rx.text = "hi".into();
        let mut raw = ev(2, "b", "rx", "RAW", 0, "");
        raw.sender = None;
        raw.seq = None;
        raw.text = "48656c6c6f".into();
        let rows = group(&[rx, raw]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].sender, "arduino");
        assert_eq!(rows[1].kind, "RAW");
    }

    #[test]
    fn same_seq_later_is_a_new_message() {
        let first = ev(1, "a", "tx", "MSG", 9, "a");
        let mut later = ev(2, "a", "tx", "MSG", 9, "a");
        later.received_at += 600;
        let mut rx = ev(3, "b", "rx", "MSG", 9, "a");
        rx.received_at += 600;
        let rows = group(&[first, later, rx]);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].heard.is_empty());
        assert_eq!(rows[1].heard.len(), 1);
    }
}
