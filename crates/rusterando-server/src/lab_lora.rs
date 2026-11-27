//! LoRa lab traffic from the tab5-lab Tabs (crates/tab5-lab/src/report.rs)
//! for the admin page's LoRa tab (/admin/lora, frontend
//! pages/admin/lora.rs).
//!
//! ```text
//! POST /api/lab/lora      Authorization: Bearer <LAB_KEY>
//! {"device": "tab5-a1b2c3", "events": [Event, …]}
//! → 200 {"send": [{"id": 7, "text": "…"}]}
//! ```
//!
//! The Tabs post every few seconds, with or without events; the answer
//! hands out the messages queued for that Tab on the admin page. The key
//! is the process env's `LAB_KEY` (the same as the Tabs' and the phone
//! app's); without it the endpoint refuses everything. Tables:
//! migrations/20261110000001_lab_lora.sql.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::broadcast;

/// At most this many events per post (the Tabs keep 500 while offline).
const MAX_EVENTS: usize = 500;
const MAX_TEXT: usize = 1000;
/// A Tab's `last_seen` is written at most this often.
const SEEN_EVERY: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
pub struct Post {
    pub device: String,
    #[serde(default)]
    pub events: Vec<Event>,
}

/// One LoRa event as one Tab saw it (fields: report.rs `Event`).
#[derive(Debug, Deserialize)]
pub struct Event {
    pub dir: String,
    pub kind: String,
    pub seq: Option<i64>,
    pub from: Option<String>,
    #[serde(default)]
    pub text: String,
    pub rssi: Option<i64>,
    pub snr: Option<f64>,
    pub peer_rssi: Option<i64>,
    pub peer_snr: Option<f64>,
    pub rtt_ms: Option<i64>,
    pub admin_id: Option<i64>,
    pub uptime_ms: Option<i64>,
}

/// A message queued on the admin page, handed to its Tab.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Outgoing {
    pub id: i64,
    pub text: String,
}

/// `Authorization: Bearer <key>` against the process env's `LAB_KEY`
/// (constant time). No key configured: nobody gets in.
pub fn authorized(header: Option<&str>) -> bool {
    let want = std::env::var("LAB_KEY").unwrap_or_default();
    let want = want.trim();
    if want.is_empty() {
        return false;
    }
    let got = header.and_then(|h| h.strip_prefix("Bearer ")).unwrap_or("");
    constant_time_eq(got.trim().as_bytes(), want.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A Tab's name as the firmware makes them (`tab5-a1b2c3`): short, plain.
pub fn valid_device(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Store a post's events and hand out (mark taken) the messages waiting
/// for its Tab. `tenant` keys the live signal and the seen-throttle.
pub async fn ingest(
    db: &SqlitePool,
    tenant: &str,
    post: &Post,
) -> Result<Vec<Outgoing>, sqlx::Error> {
    let now = unix_now();
    let mut tx = db.begin().await?;
    for e in post.events.iter().take(MAX_EVENTS) {
        let dir = match e.dir.as_str() {
            "tx" | "rx" | "lost" => e.dir.as_str(),
            _ => continue,
        };
        sqlx::query(
            "INSERT INTO lab_lora_events \
             (received_at, device, dir, kind, seq, sender, text, rssi, snr, \
              peer_rssi, peer_snr, rtt_ms, admin_id, uptime_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(now)
        .bind(&post.device)
        .bind(dir)
        .bind(clip(&e.kind, 8))
        .bind(e.seq)
        .bind(e.from.as_deref().map(|f| clip(f, 40)))
        .bind(clip(&e.text, MAX_TEXT))
        .bind(e.rssi)
        .bind(e.snr)
        .bind(e.peer_rssi)
        .bind(e.peer_snr)
        .bind(e.rtt_ms)
        .bind(e.admin_id)
        .bind(e.uptime_ms)
        .execute(&mut *tx)
        .await?;
    }
    let waiting: Vec<Outgoing> = sqlx::query_as(
        "SELECT id, text FROM lab_lora_outbox \
         WHERE device = ? AND handed_out_at IS NULL ORDER BY id",
    )
    .bind(&post.device)
    .fetch_all(&mut *tx)
    .await?;
    if !waiting.is_empty() {
        sqlx::query(
            "UPDATE lab_lora_outbox SET handed_out_at = ? \
             WHERE device = ? AND handed_out_at IS NULL",
        )
        .bind(now)
        .bind(&post.device)
        .execute(&mut *tx)
        .await?;
    }
    let first_sight = mark_seen(tenant, &post.device);
    if first_sight {
        sqlx::query(
            "INSERT INTO lab_lora_devices (device, last_seen) VALUES (?, ?) \
             ON CONFLICT(device) DO UPDATE SET last_seen = excluded.last_seen",
        )
        .bind(&post.device)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    if !post.events.is_empty() || !waiting.is_empty() || first_sight {
        notify(tenant);
    }
    Ok(waiting)
}

/// True when the Tab's `last_seen` is due for a write (first post since
/// start, or older than [`SEEN_EVERY`]).
fn mark_seen(tenant: &str, device: &str) -> bool {
    static SEEN: OnceLock<Mutex<HashMap<(String, String), Instant>>> = OnceLock::new();
    let mut seen = SEEN
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let key = (tenant.to_string(), device.to_string());
    match seen.get(&key) {
        Some(t) if t.elapsed() < SEEN_EVERY => false,
        _ => {
            seen.insert(key, Instant::now());
            true
        }
    }
}

/// "Something new for this tenant": the admin page reloads its view.
fn hub() -> &'static broadcast::Sender<String> {
    static HUB: OnceLock<broadcast::Sender<String>> = OnceLock::new();
    HUB.get_or_init(|| broadcast::channel(64).0)
}

pub fn subscribe() -> broadcast::Receiver<String> {
    hub().subscribe()
}

pub fn notify(tenant: &str) {
    let _ = hub().send(tenant.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_check() {
        assert!(constant_time_eq(b"abcd", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abce"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn device_names() {
        assert!(valid_device("tab5-a1b2c3"));
        assert!(!valid_device(""));
        assert!(!valid_device("tab5 a1b2c3"));
        assert!(!valid_device("../etc"));
    }
}
