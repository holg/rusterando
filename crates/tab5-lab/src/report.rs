//! LoRa traffic to the shop server, for the admin page's LoRa tab
//! (/admin/lora), and messages typed there back to the radio.
//!
//! Every LoRa event this Tab5 sees (sent, received, unanswered) is posted
//! over the LAN to `<SHOP_URL>/api/lab/lora` with the lab key:
//!
//! ```text
//! POST /api/lab/lora      Authorization: Bearer <LAB_KEY>
//! {"device": "tab5-aabbcc", "events": [Event, …]}
//! → {"send": [{"id": 7, "text": "hello from the admin page"}]}
//! ```
//!
//! The answer carries the messages queued for this Tab5 on the admin
//! page; they go out over LoRa like `rak send`, tagged with their id. The
//! Tab5 asks every [`POLL`] even without news, sooner when it has some.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use embedded_svc::http::client::Client;
use embedded_svc::http::Method;
// The request body is written through embedded-io.
use embedded_svc::io::Write as _;
use esp_idf_svc::http::client::{Configuration as HttpConfig, EspHttpConnection};
use esp_idf_svc::sys;
use serde::{Deserialize, Serialize};

use crate::rak::{Cmd as RakCmd, Rak};
use crate::terminal::Tone;
use crate::{say, Out};

/// The shop server, without trailing slash. No default: a build without
/// SHOP_URL reports nothing (see [`start`]) instead of posting to a host
/// that isn't this shop's.
const SHOP_URL: &str = match option_env!("SHOP_URL") {
    Some(url) => url,
    None => "",
};
const KEY: Option<&str> = option_env!("LAB_KEY");
/// Ask for queued messages at least this often.
const POLL: Duration = Duration::from_secs(3);
/// While the server fails (down, or not updated yet), ask less and less
/// often, up to this.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Collect events this long before posting them together.
const BATCH: Duration = Duration::from_millis(300);
/// Events kept while the server can't be reached; the oldest go first.
const KEEP: usize = 500;
/// Report a failing server on the screen at most this often.
const COMPLAIN_EVERY: Duration = Duration::from_secs(300);

/// One LoRa event, as this Tab5 saw it.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Event {
    /// "tx" (sent), "rx" (received) or "lost" (no answer in time).
    pub dir: &'static str,
    /// MSG, ACK, PING, PONG — or RAW for anything not ours.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u8>,
    /// The sender named in the packet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The message (MSG), or the packet as text/hex (RAW).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Signal here (received packets).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rssi: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snr: Option<f32>,
    /// Signal at the other end, as its ACK/PONG reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_rssi: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_snr: Option<f32>,
    /// Round trip of our MSG/PING to its ACK/PONG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u32>,
    /// A message from the admin page: its id there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_id: Option<i64>,
    /// This Tab5's uptime (no clock here): orders events within a post.
    pub uptime_ms: u64,
}

impl Event {
    pub fn new(dir: &'static str, kind: impl Into<String>) -> Self {
        // SAFETY: plain timer read.
        let uptime_ms = (unsafe { sys::esp_timer_get_time() } / 1000) as u64;
        Self {
            dir,
            kind: kind.into(),
            uptime_ms,
            ..Self::default()
        }
    }
}

#[derive(Serialize)]
struct Post<'a> {
    device: &'a str,
    events: &'a [Event],
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    send: Vec<Outgoing>,
}

#[derive(Deserialize)]
struct Outgoing {
    id: i64,
    text: String,
}

/// The channel the radio worker logs into ([`start`] takes the other end).
pub fn channel() -> (Sender<Event>, Receiver<Event>) {
    mpsc::channel()
}

/// Report what arrives on `events`. Without a lab key compiled in nothing
/// is reported (the server wouldn't take it): the events are dropped. The
/// same without SHOP_URL (no server to report to); LAB_KEY alone still
/// serves the phone login, so that build is fine, just without the LoRa log.
pub fn start(name: &'static str, rak: Rak, out: Out, events: Receiver<Event>) -> Result<()> {
    let Some(key) = KEY else {
        say(
            &out,
            Tone::Warn,
            "LoRa log: no LAB_KEY compiled in — the admin page won't see this Tab5".into(),
        );
        return Ok(());
    };
    if SHOP_URL.is_empty() {
        say(
            &out,
            Tone::Warn,
            "LoRa log: no SHOP_URL compiled in — the admin page won't see this Tab5".into(),
        );
        return Ok(());
    }
    std::thread::Builder::new()
        .name("lora-report".into())
        // TLS runs in this thread.
        .stack_size(16 * 1024)
        .spawn(move || Reporter::new(name, key, rak, out).run(events))?;
    Ok(())
}

struct Reporter {
    name: &'static str,
    auth: String,
    rak: Rak,
    out: Out,
    pending: VecDeque<Event>,
    client: Option<Client<EspHttpConnection>>,
    complained: Option<Instant>,
    failing: bool,
    /// Time to the next post: POLL, or longer while failing.
    wait: Duration,
}

impl Reporter {
    fn new(name: &'static str, key: &str, rak: Rak, out: Out) -> Self {
        Self {
            name,
            auth: format!("Bearer {key}"),
            rak,
            out,
            pending: VecDeque::new(),
            client: None,
            complained: None,
            failing: false,
            wait: POLL,
        }
    }

    fn run(mut self, events: Receiver<Event>) {
        let mut next = Instant::now();
        loop {
            match events.recv_timeout(next.saturating_duration_since(Instant::now())) {
                Ok(event) => {
                    self.keep(event);
                    // Server failing: keep it for the next (backed-off) try.
                    if self.failing {
                        continue;
                    }
                    // A burst (send + ack) goes out in one post.
                    let until = Instant::now() + BATCH;
                    while let Ok(more) =
                        events.recv_timeout(until.saturating_duration_since(Instant::now()))
                    {
                        self.keep(more);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.post();
            next = Instant::now() + self.wait;
        }
    }

    fn keep(&mut self, event: Event) {
        self.pending.push_back(event);
        while self.pending.len() > KEEP {
            self.pending.pop_front();
        }
    }

    /// Post what's pending (maybe nothing) and send what the admin queued.
    fn post(&mut self) {
        let events: Vec<Event> = self.pending.iter().cloned().collect();
        match self.exchange(&events) {
            Ok(answer) => {
                self.pending.drain(..events.len());
                self.wait = POLL;
                if self.failing {
                    self.failing = false;
                    say(
                        &self.out,
                        Tone::Ok,
                        "LoRa log: server reachable again".into(),
                    );
                }
                for msg in answer.send {
                    say(
                        &self.out,
                        Tone::Dim,
                        format!("admin page → LoRa: {}", msg.text),
                    );
                    self.rak.send(RakCmd::SendAdmin(msg.id, msg.text));
                }
            }
            Err(e) => {
                // A broken connection is rebuilt on the next try.
                self.client = None;
                self.failing = true;
                self.wait = (self.wait * 2).min(MAX_BACKOFF);
                log::warn!("lora report: {e:#}");
                if self
                    .complained
                    .is_none_or(|t| t.elapsed() >= COMPLAIN_EVERY)
                {
                    self.complained = Some(Instant::now());
                    say(
                        &self.out,
                        Tone::Warn,
                        format!("LoRa log: server not reachable ({e:#}) — keeping events"),
                    );
                }
            }
        }
    }

    fn exchange(&mut self, events: &[Event]) -> Result<Answer> {
        let body = serde_json::to_vec(&Post {
            device: self.name,
            events,
        })?;
        if self.client.is_none() {
            let conn = EspHttpConnection::new(&HttpConfig {
                timeout: Some(Duration::from_secs(10)),
                crt_bundle_attach: Some(sys::esp_crt_bundle_attach),
                // One TLS session for all the polls.
                keep_alive_enable: true,
                ..Default::default()
            })?;
            self.client = Some(Client::wrap(conn));
        }
        let client = self.client.as_mut().expect("client");
        let url = format!("{SHOP_URL}/api/lab/lora");
        let len = body.len().to_string();
        let headers = [
            ("Authorization", self.auth.as_str()),
            ("Content-Type", "application/json"),
            ("Content-Length", len.as_str()),
        ];
        let mut request = client.request(Method::Post, &url, &headers)?;
        request.write_all(&body).map_err(|e| anyhow!("{e:?}"))?;
        request.flush().map_err(|e| anyhow!("{e:?}"))?;
        let mut response = request.submit()?;
        let status = response.status();
        let mut answer = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            let n = response.read(&mut buf).map_err(|e| anyhow!("{e:?}"))?;
            if n == 0 {
                break;
            }
            answer.extend_from_slice(&buf[..n]);
        }
        if status != 200 {
            // The first line of the server's answer is enough (an error
            // page goes on for many).
            let text = String::from_utf8_lossy(&answer);
            let first = text.lines().next().unwrap_or("").trim();
            bail!("HTTP {status}: {}", first.chars().take(80).collect::<String>());
        }
        Ok(serde_json::from_slice(&answer)?)
    }
}
