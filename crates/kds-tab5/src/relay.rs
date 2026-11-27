//! LoRa relay: a Tab5 without network keeps its kitchen board, over LoRa
//! from a Tab5 that has one, and its taps (Starten, Fertig, …) go back the
//! same way to the server. Receipts are not relayed.
//!
//! The role follows the feed (feed.rs `is_online`):
//!
//! - **Offline**: says so every [`HELLO_EVERY`] (`Hello`), takes the board
//!   the other device sends — the screen shows "LoRa" in amber, not a red
//!   "Offline" ("Über LoRa verbunden – die Bestellungen kommen von …") — and
//!   sends its board taps to it (`Action`, repeated until
//!   `Done`).
//! - **Online**: relays to a device it hears saying it is offline. `relay
//!   auto` (the default) does so only when the server confirms that device
//!   isn't connected (`/api/kitchen/devices`; a server without it is taken
//!   at the device's word), `relay on` always, `relay off` never. It sends
//!   what changed: a new or changed order in full (`Upsert`), a status change
//!   or a removal as a few bytes; every [`DIGEST_EVERY`] a summary
//!   (`Digest`) from which the other side asks for what it misses (`Need`),
//!   so a lost packet heals itself.
//!
//! One relay per device, the best one: two relays sending to it on the same
//! channel collide. A relay's rank is its own link — a LAN cable 2, Wi-Fi 1
//! — plus 2 for a device named in `RELAY_PREFER` (the shop's own Tab5: it
//! wins on Wi-Fi too); the Cardputer 0, it never relays (little RAM, a small
//! battery), it only takes the board. The device without network keeps the relay it has and
//! names it, with its rank, after each `Hello` (`Via`); another relay joins
//! in only when it ranks higher, and the device then moves to it on the
//! `Rank` that comes before each `Digest`. Should the relay go quiet, any
//! relay may start again. `Via` and `Rank` are new at the end of [`Msg`]:
//! older firmware skips them and relays as before.
//!
//! On air: rak.rs frames starting with `T5R`, a [`HEADER`] and up to
//! [`CHUNK`] bytes of a message split into pieces. A message is postcard,
//! deflated when that is shorter, sealed with ChaCha20-Poly1305 — orders
//! carry customers' names, phones and addresses: the key comes from the
//! shared `LAB_KEY`, the nonce is random, the header is associated data.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use esp_idf_svc::sys;
use hmac::{Hmac, Mac};
use kitchen_protocol::{BoardOrder, BoardSnapshot, BoardStatus, ClientMessage};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::rak::{Cmd as RakCmd, Rak, RelayFrame, RELAY_MAGIC};
use crate::terminal::{Terminal, Tone};
use crate::ui::UiMsg;
use crate::{say, Out};

const VERSION: u8 = 1;
/// Deflate what goes out. Not on the Cardputer: miniz_oxide's compressor
/// wants ~160 KB, all its RAM. Its frames go out plain (flag 0), which every
/// device reads; it still inflates what the others send.
const DEFLATE: bool = !cfg!(feature = "cardputer");
const BROADCAST: [u8; 3] = [0xff; 3];
/// `T5R`, version, from, to, message id, piece, pieces.
const HEADER: usize = 3 + 1 + 3 + 3 + 2 + 1 + 1;
/// Message bytes per frame (RUI3 takes up to 255 per packet).
const CHUNK: usize = 200;
/// The radio's spreading factor (rak.rs `P2P_DEFAULT`): the slower ones
/// reach further and take longer on air, so the relay talks less often.
const SF: u8 = crate::rak::P2P_DEFAULT.1;
const SLOW: bool = SF > 7;
const HELLO_SECS: u64 = if SLOW { 40 } else { 20 };
const DIGEST_SECS: u64 = if SLOW { 90 } else { 30 };
const HELLO_EVERY: Duration = Duration::from_secs(HELLO_SECS);
const DIGEST_EVERY: Duration = Duration::from_secs(DIGEST_SECS);
/// An offline device not heard for this long is no longer relayed to…
const PEER_GONE: Duration = Duration::from_secs(HELLO_SECS * 4 + 10);
/// …and a relaying device not heard for this long has stopped.
const SOURCE_GONE: Duration = Duration::from_secs(DIGEST_SECS * 3);
/// A device's `Via` counts this long (it repeats it with every `Hello`).
const VIA_FOR: Duration = Duration::from_secs(HELLO_SECS * 2 + 10);
const SERVER_CHECK_EVERY: Duration = Duration::from_secs(30);
const ACTION_RETRY: Duration = Duration::from_secs(5);
const ACTION_TRIES: u32 = 5;
/// Pieces of a message that never completed are dropped after this.
const PIECES_TIMEOUT: Duration = Duration::from_secs(20);
/// Our share of the airtime; the 869.4–869.65 MHz band allows 10 %.
const AIRTIME_SHARE: f64 = 0.05;
const HOUR: Duration = Duration::from_secs(3600);

/// `relay auto` / `on` / `off`.
static MODE: AtomicU8 = AtomicU8::new(MODE_AUTO);
const MODE_AUTO: u8 = 0;
const MODE_ON: u8 = 1;
const MODE_OFF: u8 = 2;

/// The relay thread's inbox, for feed.rs.
static INBOX: OnceLock<Mutex<Sender<Input>>> = OnceLock::new();
/// Offline with a relaying device in range: taps can go over LoRa.
static RECEIVING: AtomicBool = AtomicBool::new(false);
/// What `relay` shows.
static STATUS: Mutex<String> = Mutex::new(String::new());

enum Input {
    /// The server's board (online: to pass on).
    Board(BoardSnapshot),
    /// A tap of this screen the server didn't take (offline: over LoRa).
    Action(ClientMessage),
}

#[derive(Serialize, Deserialize, Debug)]
enum Msg {
    /// A device's network state: offline = wants the board.
    Hello { name: String, online: bool },
    Upsert(BoardOrder),
    Status { id: u32, status: BoardStatus },
    Remove { id: u32 },
    /// Every order on the board: (id, status, content hash).
    Digest { now: i64, orders: Vec<(u32, BoardStatus, u32)> },
    Need { ids: Vec<u32> },
    Action { seq: u16, msg: ClientMessage },
    Done { seq: u16, refused: Option<String> },
    // New ones only below: postcard numbers the variants in order.
    /// After a `Hello` without network: the relay it takes the board from
    /// ([`BROADCAST`]: none yet — any may start).
    Via { relay: [u8; 3], rank: u8 },
    /// Before a `Digest`: the sender's rank as a relay.
    Rank(u8),
}

/// The server's board from the feed (online: passed on if someone needs it).
pub fn board(board: BoardSnapshot) {
    send_input(Input::Board(board));
}

/// A tap the server couldn't take: `true` = it goes over LoRa instead.
pub fn offer_action(msg: ClientMessage) -> bool {
    if !RECEIVING.load(Ordering::Relaxed) || !matches!(msg, ClientMessage::BoardAdvance { .. }) {
        return false;
    }
    send_input(Input::Action(msg))
}

fn send_input(input: Input) -> bool {
    INBOX
        .get()
        .and_then(|inbox| inbox.lock().ok()?.send(input).ok())
        .is_some()
}

/// The terminal's `relay` commands. `false`: not one of them.
pub fn command(words: &[&str], term: &mut Terminal) -> bool {
    let set = |mode: u8, what: &str, term: &mut Terminal| {
        MODE.store(mode, Ordering::Relaxed);
        term.push(Tone::Ok, what);
    };
    match words {
        ["relay", "auto" | "on" | "off"] if cfg!(feature = "cardputer") => {
            term.push(Tone::Warn, "relay: this device only takes the board over LoRa, it never relays");
        }
        ["relay"] => {
            let status = STATUS.lock().map(|s| s.clone()).unwrap_or_default();
            term.push(
                Tone::Normal,
                if status.is_empty() { "relay: not running (no LoRa module or no LAB_KEY)" } else { &status },
            );
        }
        ["relay", "auto"] => set(MODE_AUTO, "relay auto: to devices the server doesn't see", term),
        ["relay", "on"] => set(MODE_ON, "relay on: to every device that says it's offline", term),
        ["relay", "off"] => set(MODE_OFF, "relay off: nothing is passed on", term),
        _ => return false,
    }
    true
}

/// Start the relay over this LoRa module. `name`: this device ("tab5-ddeeff").
pub fn start(name: &'static str, rak: Rak, ui: Sender<UiMsg>, out: Out) {
    let lab_key = option_env!("LAB_KEY").unwrap_or("");
    let Some(me) = device_id(name) else {
        say(&out, Tone::Warn, format!("relay: no device id in {name:?}"));
        return;
    };
    if lab_key.is_empty() {
        say(&out, Tone::Warn, "relay: no LAB_KEY compiled in".into());
        return;
    }
    let (frames_tx, frames) = mpsc::channel();
    rak.send(RakCmd::RelaySink(frames_tx));
    let (inbox_tx, inbox) = mpsc::channel();
    let _ = INBOX.set(Mutex::new(inbox_tx));
    let relay = Relay::new(name, me, lab_key, rak, ui, out.clone(), frames, inbox);
    if let Err(e) = std::thread::Builder::new()
        .name("relay".into())
        // Deflate keeps ~64 KB of work on the stack (miniz_oxide's LZ
        // buffer); the lab's threads get theirs from PSRAM (lab.rs). The
        // Cardputer (no PSRAM) doesn't deflate: a small stack does.
        .stack_size(if DEFLATE { 192 * 1024 } else { 24 * 1024 })
        .spawn(move || relay.run())
    {
        say(&out, Tone::Err, format!("relay: {e}"));
    }
}

/// "tab5-ddeeff" → [0xdd, 0xee, 0xff].
fn device_id(name: &str) -> Option<[u8; 3]> {
    let hex = name.get(name.len().checked_sub(6)?..)?;
    let n = u32::from_str_radix(hex, 16).ok()?;
    let [_, a, b, c] = n.to_be_bytes();
    Some([a, b, c])
}

fn device_name(id: [u8; 3]) -> String {
    format!("tab5-{:02x}{:02x}{:02x}", id[0], id[1], id[2])
}

/// The relays to prefer (`RELAY_PREFER`, tmp/tab5-wifi.env): device names,
/// comma-separated — the shop's Tab5.
const PREFERRED: &str = match option_env!("RELAY_PREFER") {
    Some(names) => names,
    None => "",
};

fn preferred(name: &str) -> bool {
    PREFERRED.split(',').any(|n| n.trim() == name)
}

/// What this device is worth as a relay: 2 on a LAN cable, 1 on Wi-Fi, 2
/// more when preferred; 0 never (the Cardputer).
fn my_rank(name: &str) -> u8 {
    if cfg!(feature = "cardputer") {
        return 0;
    }
    let link = if crate::net::wired() { 2 } else { 1 };
    if preferred(name) {
        link + 2
    } else {
        link
    }
}

fn rank_name(rank: u8) -> &'static str {
    match rank {
        4 => "preferred, LAN",
        3 => "preferred, Wi-Fi",
        2 => "LAN",
        1 => "Wi-Fi",
        _ => "no relay",
    }
}

fn fnv(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(*b)).wrapping_mul(0x0100_0193))
}

fn short_id(order_id: &str) -> u32 {
    fnv(order_id.as_bytes())
}

/// Changes when anything but the status changes.
fn content_hash(o: &BoardOrder) -> u32 {
    postcard::to_allocvec(&o.order).map(|b| fnv(&b)).unwrap_or(0)
}

/// On-air time of a frame at the radio's SF.
fn airtime(len: usize) -> Duration {
    airtime_at(len, SF)
}

/// On-air time of a frame: 125 kHz, CR 4/5, 8 symbols preamble, explicit
/// header, CRC, low data rate optimisation from SF11 (Semtech's formula).
fn airtime_at(len: usize, sf: u8) -> Duration {
    let sf = u64::from(sf);
    let de = if sf >= 11 { 2 } else { 0 };
    let bits = (8 * len as u64 + 28 + 16).saturating_sub(4 * sf);
    let payload_symbols = 8 + bits.div_ceil(4 * (sf - de)) * 5;
    // One symbol: 2^SF / 125 kHz.
    let symbol_us = (1u64 << sf) as f64 * 8.0;
    Duration::from_micros(((12.25 + payload_symbols as f64) * symbol_us) as u64)
}

struct Pieces {
    got: Vec<Option<Vec<u8>>>,
    to: [u8; 3],
    first: Instant,
}

/// A device that said it's offline (online role).
struct Peer {
    name: String,
    heard: Instant,
    rssi: i16,
    /// What the server says: connected (`Some(true)`), not, or unknown.
    server_online: Option<bool>,
    relaying: bool,
    /// The relay it takes the board from, that one's rank, when it said so
    /// (`Via`; stale after [`VIA_FOR`]).
    source: Option<([u8; 3], u8, Instant)>,
    /// What it has from us: id → (status, content hash).
    sent: HashMap<u32, (BoardStatus, u32)>,
    digest_at: Option<Instant>,
}

struct Pending {
    seq: u16,
    msg: ClientMessage,
    tries: u32,
    next: Instant,
}

struct Relay {
    name: &'static str,
    me: [u8; 3],
    cipher: ChaCha20Poly1305,
    rak: Rak,
    ui: Sender<UiMsg>,
    out: Out,
    frames: Receiver<RelayFrame>,
    inbox: Receiver<Input>,
    msg_id: u16,
    pieces: HashMap<([u8; 3], u16), Pieces>,
    airtime: VecDeque<(Instant, Duration)>,
    // ----- online: passing the board on
    peers: HashMap<[u8; 3], Peer>,
    board: Option<(BoardSnapshot, Instant)>,
    server_check: Option<Instant>,
    server_check_failed: bool,
    /// Taps done for others, so a repeat gets the same answer.
    done: VecDeque<([u8; 3], u16, Option<String>)>,
    // ----- offline: taking it
    source: Option<([u8; 3], Instant)>,
    /// The source's rank (its `Rank`; Wi-Fi until then — older firmware
    /// sends none).
    source_rank: u8,
    relayed: HashMap<u32, BoardOrder>,
    clock: Option<(i64, Instant)>,
    shown_from: Option<String>,
    hello_at: Option<Instant>,
    /// What the radio brings (for `relay` and the log): frames per sender
    /// with the last RSSI, messages opened, messages that failed.
    heard_frames: HashMap<[u8; 3], (u32, i16)>,
    opened: u32,
    failed: u32,
    heard_logged: Instant,
    was_online: bool,
    pending: Vec<Pending>,
    action_seq: u16,
    status_at: Instant,
}

impl Relay {
    #[allow(clippy::too_many_arguments)]
    fn new(
        name: &'static str,
        me: [u8; 3],
        lab_key: &str,
        rak: Rak,
        ui: Sender<UiMsg>,
        out: Out,
        frames: Receiver<RelayFrame>,
        inbox: Receiver<Input>,
    ) -> Self {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(lab_key.as_bytes()).expect("HMAC takes any key");
        mac.update(b"t5 lora relay v1");
        let key: [u8; 32] = mac.finalize().into_bytes().into();
        Relay {
            name,
            me,
            cipher: ChaCha20Poly1305::new(Key::from_slice(&key)),
            rak,
            ui,
            out,
            frames,
            inbox,
            msg_id: 0,
            pieces: HashMap::new(),
            airtime: VecDeque::new(),
            peers: HashMap::new(),
            board: None,
            server_check: None,
            server_check_failed: false,
            done: VecDeque::new(),
            source: None,
            source_rank: 1,
            relayed: HashMap::new(),
            clock: None,
            shown_from: None,
            hello_at: None,
            heard_frames: HashMap::new(),
            opened: 0,
            failed: 0,
            heard_logged: Instant::now(),
            was_online: false,
            pending: Vec::new(),
            action_seq: 0,
            status_at: Instant::now(),
        }
    }

    fn run(mut self) {
        loop {
            while let Ok((bytes, rssi, _snr)) = self.frames.try_recv() {
                self.frame(&bytes, rssi);
            }
            match self.inbox.recv_timeout(Duration::from_millis(100)) {
                Ok(Input::Board(board)) => self.new_board(board),
                Ok(Input::Action(msg)) => self.queue_action(msg),
                Err(_) => {}
            }
            self.tick();
        }
    }

    // ---------------------------------------------------------------- air

    fn aad(from: [u8; 3], to: [u8; 3], id: u16) -> [u8; 12] {
        let mut aad = [0u8; 12];
        aad[..3].copy_from_slice(RELAY_MAGIC);
        aad[3] = VERSION;
        aad[4..7].copy_from_slice(&from);
        aad[7..10].copy_from_slice(&to);
        aad[10..].copy_from_slice(&id.to_be_bytes());
        aad
    }

    fn seal(&self, to: [u8; 3], id: u16, msg: &Msg) -> Result<Vec<u8>> {
        let plain = postcard::to_allocvec(msg)?;
        // Small messages (hello, status, taps) don't shrink.
        let packed = if DEFLATE && plain.len() > 96 {
            miniz_oxide::deflate::compress_to_vec(&plain, 8)
        } else {
            plain.clone()
        };
        let mut body = Vec::with_capacity(plain.len() + 1);
        if packed.len() < plain.len() {
            body.push(1);
            body.extend_from_slice(&packed);
        } else {
            body.push(0);
            body.extend_from_slice(&plain);
        }
        let mut nonce = [0u8; 12];
        // SAFETY: fills the 12-byte buffer from the hardware RNG.
        unsafe { sys::esp_fill_random(nonce.as_mut_ptr().cast(), nonce.len()) };
        let aad = Self::aad(self.me, to, id);
        let sealed = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: &body, aad: &aad })
            .map_err(|_| anyhow!("seal"))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    fn open(&self, from: [u8; 3], to: [u8; 3], id: u16, data: &[u8]) -> Result<Msg> {
        if data.len() < 12 + 16 + 1 {
            bail!("too short");
        }
        let (nonce, sealed) = data.split_at(12);
        let aad = Self::aad(from, to, id);
        let body = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), Payload { msg: sealed, aad: &aad })
            .map_err(|_| anyhow!("not sealed with our key (other LAB_KEY?) or damaged"))?;
        let (flag, rest) = body.split_first().ok_or_else(|| anyhow!("empty"))?;
        let plain = match flag {
            0 => rest.to_vec(),
            1 => miniz_oxide::inflate::decompress_to_vec_with_limit(rest, 64 * 1024)
                .map_err(|e| anyhow!("inflate: {e:?}"))?,
            _ => bail!("unknown packing {flag}"),
        };
        Ok(postcard::from_bytes(&plain)?)
    }

    fn send(&mut self, to: [u8; 3], msg: &Msg) {
        self.msg_id = self.msg_id.wrapping_add(1);
        let id = self.msg_id;
        let data = match self.seal(to, id, msg) {
            Ok(d) => d,
            Err(e) => return log::warn!("relay: {e:#}"),
        };
        let count = data.len().div_ceil(CHUNK);
        if count > usize::from(u8::MAX) {
            return log::warn!("relay: message of {} B too large", data.len());
        }
        for (i, chunk) in data.chunks(CHUNK).enumerate() {
            let mut frame = Vec::with_capacity(HEADER + chunk.len());
            frame.extend_from_slice(RELAY_MAGIC);
            frame.push(VERSION);
            frame.extend_from_slice(&self.me);
            frame.extend_from_slice(&to);
            frame.extend_from_slice(&id.to_be_bytes());
            frame.push(i as u8);
            frame.push(count as u8);
            frame.extend_from_slice(chunk);
            self.airtime.push_back((Instant::now(), airtime(frame.len())));
            self.rak.send(RakCmd::SendRaw(frame));
        }
    }

    /// Airtime used in the last hour.
    fn airtime_used(&mut self) -> Duration {
        while self.airtime.front().is_some_and(|(at, _)| at.elapsed() > HOUR) {
            self.airtime.pop_front();
        }
        self.airtime.iter().map(|(_, d)| *d).sum()
    }

    /// Room in our airtime budget for optional sends (summaries, catch-up).
    fn airtime_left(&mut self) -> bool {
        self.airtime_used().as_secs_f64() < HOUR.as_secs_f64() * AIRTIME_SHARE
    }

    fn frame(&mut self, bytes: &[u8], rssi: i16) {
        if bytes.len() <= HEADER || bytes[3] != VERSION {
            return;
        }
        let from = [bytes[4], bytes[5], bytes[6]];
        let to = [bytes[7], bytes[8], bytes[9]];
        let id = u16::from_be_bytes([bytes[10], bytes[11]]);
        let (index, count) = (usize::from(bytes[12]), usize::from(bytes[13]));
        if from != self.me {
            let e = self.heard_frames.entry(from).or_insert((0, rssi));
            e.0 += 1;
            e.1 = rssi;
        }
        if from == self.me || (to != self.me && to != BROADCAST) || index >= count {
            return;
        }
        let pieces = self.pieces.entry((from, id)).or_insert_with(|| Pieces {
            got: vec![None; count],
            to,
            first: Instant::now(),
        });
        if pieces.got.len() != count {
            return;
        }
        pieces.got[index] = Some(bytes[HEADER..].to_vec());
        if pieces.got.iter().any(Option::is_none) {
            return;
        }
        let Some(pieces) = self.pieces.remove(&(from, id)) else { return };
        let data: Vec<u8> = pieces.got.into_iter().flatten().flatten().collect();
        match self.open(from, pieces.to, id, &data) {
            Ok(msg) => {
                self.opened += 1;
                self.handle(from, rssi, msg)
            }
            Err(e) => {
                self.failed += 1;
                log::warn!("relay: from {}: {e:#}", device_name(from))
            }
        }
    }

    fn handle(&mut self, from: [u8; 3], rssi: i16, msg: Msg) {
        let online = crate::feed::is_online();
        match msg {
            Msg::Hello { name, online: false } => self.heard_offline(from, name, rssi),
            Msg::Via { relay, rank } => {
                if let Some(peer) = self.peers.get_mut(&from) {
                    peer.source = (relay != BROADCAST).then(|| (relay, rank, Instant::now()));
                }
            }
            // A relay's rank: ours to move to if it's better.
            Msg::Rank(rank) if !online => {
                self.heard_source(from, Some(rank));
            }
            Msg::Hello { online: true, .. } => {
                if self.peers.remove(&from).is_some_and(|p| p.relaying) {
                    self.note(Tone::Ok, format!("relay: {} has network again, stopped", device_name(from)));
                }
            }
            // The board, while we have no network of our own.
            Msg::Upsert(order) if !online => {
                if !self.heard_source(from, None) {
                    return;
                }
                self.relayed.insert(short_id(&order.id), order);
                self.show();
            }
            Msg::Status { id, status } if !online => {
                if !self.heard_source(from, None) {
                    return;
                }
                if let Some(o) = self.relayed.get_mut(&id) {
                    o.status = status;
                    self.show();
                }
            }
            Msg::Remove { id } if !online => {
                if !self.heard_source(from, None) {
                    return;
                }
                if self.relayed.remove(&id).is_some() {
                    self.show();
                }
            }
            Msg::Digest { now, orders } if !online => {
                if !self.heard_source(from, None) {
                    return;
                }
                self.clock = Some((now, Instant::now()));
                let ids: HashSet<u32> = orders.iter().map(|(id, _, _)| *id).collect();
                self.relayed.retain(|id, _| ids.contains(id));
                let mut need = Vec::new();
                for (id, status, hash) in orders {
                    match self.relayed.get_mut(&id) {
                        Some(o) if content_hash(o) == hash => o.status = status,
                        _ => need.push(id),
                    }
                }
                if !need.is_empty() {
                    need.truncate(16);
                    self.send(from, &Msg::Need { ids: need });
                }
                self.show();
            }
            Msg::Done { seq, refused } => {
                self.pending.retain(|p| p.seq != seq);
                match refused {
                    Some(reason) => {
                        let _ = self.ui.send(UiMsg::Notice(reason));
                    }
                    None => log::info!("relay: tap {seq} done via {}", device_name(from)),
                }
            }
            // Asked of the device with network.
            Msg::Need { ids } if online => self.send_orders(from, &ids),
            Msg::Action { seq, msg } if online => self.relayed_action(from, seq, msg),
            other => log::debug!("relay: ignored {other:?}"),
        }
    }

    // ------------------------------------------------- online: pass it on

    fn heard_offline(&mut self, from: [u8; 3], name: String, rssi: i16) {
        let peer = self.peers.entry(from).or_insert_with(|| Peer {
            name: name.clone(),
            heard: Instant::now(),
            rssi,
            server_online: None,
            relaying: false,
            source: None,
            sent: HashMap::new(),
            digest_at: None,
        });
        peer.name = name;
        peer.heard = Instant::now();
        peer.rssi = rssi;
    }

    fn wants_relay(&self, peer: &Peer) -> bool {
        let rank = my_rank(self.name);
        if rank == 0 || !crate::feed::is_online() || peer.heard.elapsed() > PEER_GONE {
            return false;
        }
        // It has a relay: only that one, or a better one, sends.
        if let Some((relay, its_rank, at)) = peer.source {
            if relay != self.me && rank <= its_rank && at.elapsed() < VIA_FOR {
                return false;
            }
        }
        match MODE.load(Ordering::Relaxed) {
            MODE_ON => true,
            MODE_OFF => false,
            _ => peer.server_online != Some(true),
        }
    }

    fn new_board(&mut self, board: BoardSnapshot) {
        self.board = Some((board, Instant::now()));
        let ids: Vec<[u8; 3]> = self.peers.iter().filter(|(_, p)| p.relaying).map(|(id, _)| *id).collect();
        for id in ids {
            self.push_changes(id);
        }
    }

    /// Send a relayed device what changed since it last heard from us.
    fn push_changes(&mut self, to: [u8; 3]) {
        let Some((board, _)) = &self.board else { return };
        let Some(peer) = self.peers.get(&to) else { return };
        let mut out = Vec::new();
        let mut sent = HashMap::new();
        for o in &board.orders {
            let id = short_id(&o.id);
            let hash = content_hash(o);
            match peer.sent.get(&id) {
                Some((_, h)) if *h != hash => out.push(Msg::Upsert(o.clone())),
                None => out.push(Msg::Upsert(o.clone())),
                Some((status, _)) if *status != o.status => out.push(Msg::Status { id, status: o.status }),
                Some(_) => {}
            }
            sent.insert(id, (o.status, hash));
        }
        for id in peer.sent.keys().filter(|id| !sent.contains_key(id)) {
            out.push(Msg::Remove { id: *id });
        }
        if let Some(peer) = self.peers.get_mut(&to) {
            peer.sent = sent;
        }
        for msg in out {
            self.send(to, &msg);
        }
    }

    fn send_digest(&mut self, to: [u8; 3]) {
        let Some((board, at)) = &self.board else { return };
        let msg = Msg::Digest {
            now: board.server_now_unix + at.elapsed().as_secs() as i64,
            orders: board.orders.iter().map(|o| (short_id(&o.id), o.status, content_hash(o))).collect(),
        };
        self.send(to, &Msg::Rank(my_rank(self.name)));
        self.send(to, &msg);
        if let Some(peer) = self.peers.get_mut(&to) {
            peer.digest_at = Some(Instant::now());
        }
    }

    fn send_orders(&mut self, to: [u8; 3], ids: &[u32]) {
        let Some((board, _)) = &self.board else { return };
        let orders: Vec<BoardOrder> =
            board.orders.iter().filter(|o| ids.contains(&short_id(&o.id))).cloned().collect();
        for o in orders {
            if let Some(peer) = self.peers.get_mut(&to) {
                peer.sent.insert(short_id(&o.id), (o.status, content_hash(&o)));
            }
            self.send(to, &Msg::Upsert(o));
        }
    }

    /// A tap made on the device without network: to the server, answer back.
    fn relayed_action(&mut self, from: [u8; 3], seq: u16, msg: ClientMessage) {
        let repeat = self
            .done
            .iter()
            .find(|(f, s, _)| *f == from && *s == seq)
            .map(|(_, _, refused)| refused.clone());
        if let Some(refused) = repeat {
            return self.send(from, &Msg::Done { seq, refused });
        }
        let refused = match crate::feed::deliver_now(&msg) {
            Ok(outcome) => outcome,
            Err(e) => {
                log::warn!("relay: tap from {}: {e:#}", device_name(from));
                Some("Server nicht erreichbar".to_string())
            }
        };
        self.note(
            Tone::Rx,
            format!(
                "relay: tap from {} → server{}",
                device_name(from),
                refused.as_deref().map(|r| format!(": {r}")).unwrap_or_default()
            ),
        );
        self.done.push_back((from, seq, refused.clone()));
        if self.done.len() > 32 {
            self.done.pop_front();
        }
        self.send(from, &Msg::Done { seq, refused });
    }

    /// Ask the server which kitchen devices are connected — and tell it the
    /// devices heard over LoRa (name, RSSI, seconds since), so /admin/drucker
    /// shows them as reachable over LoRa rather than offline. A server
    /// without that ignores the query.
    fn check_server(&mut self) {
        self.server_check = Some(Instant::now());
        #[derive(Deserialize)]
        struct Device {
            name: String,
            online: bool,
        }
        let heard: Vec<String> = self
            .peers
            .values()
            .filter(|p| p.heard.elapsed() <= PEER_GONE)
            .map(|p| format!("{}:{}:{}", p.name, p.rssi, p.heard.elapsed().as_secs()))
            .collect();
        let path = if heard.is_empty() {
            "/api/kitchen/devices".to_string()
        } else {
            format!("/api/kitchen/devices?via={}&lora={}", self.name, heard.join(","))
        };
        let devices = crate::feed::get(&path, 16 * 1024)
            .and_then(|body| serde_json::from_slice::<Vec<Device>>(&body).map_err(Into::into));
        match devices {
            Ok(devices) => {
                self.server_check_failed = false;
                for peer in self.peers.values_mut() {
                    peer.server_online = devices.iter().find(|d| d.name == peer.name).map(|d| d.online);
                }
            }
            Err(e) => {
                if !self.server_check_failed {
                    log::warn!("relay: server check: {e:#} — taking devices at their word");
                }
                self.server_check_failed = true;
                for peer in self.peers.values_mut() {
                    peer.server_online = None;
                }
            }
        }
    }

    // ------------------------------------------------------ offline: take it

    /// A board message from `from` (`rank`: its `Rank`). `false`: not
    /// our relay — one at a time, so it's dropped — unless it ranks higher,
    /// then it becomes ours. Its board is the same server's, so what we
    /// have stays; its `Digest` fills in the rest.
    fn heard_source(&mut self, from: [u8; 3], rank: Option<u8>) -> bool {
        match self.source {
            Some((current, _)) if current == from => {}
            Some((current, _)) => {
                let better = rank.is_some_and(|r| r > self.source_rank);
                if !better {
                    return false;
                }
                self.note(
                    Tone::Ok,
                    format!(
                        "relay: moving from {} to {} ({})",
                        device_name(current),
                        device_name(from),
                        rank_name(rank.unwrap_or(1))
                    ),
                );
                // Tell the others at once, not at the next Hello.
                self.hello_at = None;
            }
            None => self.hello_at = None,
        }
        if let Some(r) = rank {
            self.source_rank = r;
        } else if self.source.is_none() {
            self.source_rank = 1;
        }
        self.source = Some((from, Instant::now()));
        RECEIVING.store(true, Ordering::Relaxed);
        true
    }

    /// The relayed board on this screen.
    fn show(&mut self) {
        let Some((from, _)) = self.source else { return };
        let now = match self.clock {
            Some((now, at)) => now + at.elapsed().as_secs() as i64,
            // SAFETY: plain time read.
            None => unsafe { sys::time(core::ptr::null_mut()) as i64 },
        };
        let mut orders: Vec<BoardOrder> = self.relayed.values().cloned().collect();
        orders.sort_by(|a, b| {
            (a.order.created_at_unix, &a.id).cmp(&(b.order.created_at_unix, &b.id))
        });
        let from = device_name(from);
        if self.shown_from.as_deref() != Some(&from) {
            self.note(Tone::Ok, format!("relay: board over LoRa from {from}"));
            let _ = self.ui.send(UiMsg::Event(kds_ui::Event::Relay(Some(from.clone()))));
            self.shown_from = Some(from);
        }
        let _ = self
            .ui
            .send(UiMsg::Event(kds_ui::Event::Board(BoardSnapshot { server_now_unix: now, orders })));
    }

    fn queue_action(&mut self, msg: ClientMessage) {
        self.action_seq = self.action_seq.wrapping_add(1);
        self.pending.push(Pending {
            seq: self.action_seq,
            msg,
            tries: 0,
            next: Instant::now(),
        });
    }

    fn stop_receiving(&mut self) {
        self.source = None;
        self.relayed.clear();
        self.clock = None;
        RECEIVING.store(false, Ordering::Relaxed);
        if self.shown_from.take().is_some() {
            let _ = self.ui.send(UiMsg::Event(kds_ui::Event::Relay(None)));
        }
    }

    // ---------------------------------------------------------------- time

    fn tick(&mut self) {
        let online = crate::feed::is_online();
        self.pieces.retain(|_, p| p.first.elapsed() < PIECES_TIMEOUT);

        if online {
            if !self.was_online {
                // Network back: the server's board counts, and whoever
                // relayed to us can stop.
                self.stop_receiving();
                self.pending.clear();
                if self.hello_at.is_some() {
                    self.send(BROADCAST, &Msg::Hello { name: self.name.into(), online: true });
                    self.hello_at = None;
                }
            }
            self.tick_online();
        } else {
            if self.was_online {
                self.peers.clear();
            }
            self.tick_offline();
        }
        self.was_online = online;

        // Without network: what the radio hears, every 30 s in the log.
        if !online && self.heard_logged.elapsed() >= Duration::from_secs(30) {
            self.heard_logged = Instant::now();
            log::info!("relay: {}", self.heard_line());
        }
        if self.status_at.elapsed() >= Duration::from_secs(1) {
            self.status_at = Instant::now();
            let status = self.describe(online);
            if let Ok(mut s) = STATUS.lock() {
                *s = status;
            }
        }
    }

    fn tick_online(&mut self) {
        self.peers.retain(|_, p| p.heard.elapsed() <= PEER_GONE);
        let auto = MODE.load(Ordering::Relaxed) == MODE_AUTO;
        if auto
            && my_rank(self.name) > 0
            && !self.peers.is_empty()
            && self.server_check.is_none_or(|t| t.elapsed() >= SERVER_CHECK_EVERY)
        {
            self.check_server();
        }
        let ids: Vec<[u8; 3]> = self.peers.keys().copied().collect();
        for id in ids {
            let wants = self.peers.get(&id).is_some_and(|p| self.wants_relay(p));
            let Some(peer) = self.peers.get_mut(&id) else { continue };
            if wants != peer.relaying {
                peer.relaying = wants;
                peer.sent.clear();
                peer.digest_at = None;
                let name = peer.name.clone();
                if wants {
                    self.note(Tone::Ok, format!("relay: passing the board to {name} over LoRa"));
                    self.push_changes(id);
                } else {
                    self.note(Tone::Dim, format!("relay: stopped for {name}"));
                }
            }
            let due = self
                .peers
                .get(&id)
                .is_some_and(|p| p.relaying && p.digest_at.is_none_or(|t| t.elapsed() >= DIGEST_EVERY));
            if due && self.airtime_left() {
                self.send_digest(id);
            }
        }
    }

    fn tick_offline(&mut self) {
        if self.hello_at.is_none_or(|t| t.elapsed() >= HELLO_EVERY) {
            self.hello_at = Some(Instant::now());
            self.send(BROADCAST, &Msg::Hello { name: self.name.into(), online: false });
            let (relay, rank) = match self.source {
                Some((relay, _)) => (relay, self.source_rank),
                None => (BROADCAST, 0),
            };
            self.send(BROADCAST, &Msg::Via { relay, rank });
        }
        if self.source.is_some_and(|(_, at)| at.elapsed() > SOURCE_GONE) {
            self.note(Tone::Warn, "relay: no board over LoRa for 90 s".into());
            self.stop_receiving();
            // Say so now: another relay may take over.
            self.hello_at = None;
        }
        // Taps waiting for the relaying device's answer.
        let Some((to, _)) = self.source else {
            if !self.pending.is_empty() {
                self.pending.clear();
                let _ = self.ui.send(UiMsg::Notice("Keine Verbindung — auch nicht über LoRa".into()));
            }
            return;
        };
        let now = Instant::now();
        let mut out = Vec::new();
        let mut given_up = false;
        self.pending.retain_mut(|p| {
            if p.next > now {
                return true;
            }
            if p.tries >= ACTION_TRIES {
                given_up = true;
                return false;
            }
            p.tries += 1;
            p.next = now + ACTION_RETRY;
            out.push(Msg::Action { seq: p.seq, msg: p.msg.clone() });
            true
        });
        for msg in out {
            self.send(to, &msg);
        }
        if given_up {
            let _ = self.ui.send(UiMsg::Notice("Über LoRa nicht bestätigt".into()));
        }
    }

    fn note(&self, tone: Tone, text: String) {
        log::info!("{text}");
        say(&self.out, tone, text);
    }

    /// "heard: tab5-aabbcc 12 frames (-98 dBm) · 3 messages, 0 failed".
    fn heard_line(&self) -> String {
        if self.heard_frames.is_empty() {
            return "heard: no relay frames yet".into();
        }
        let who: Vec<String> = self
            .heard_frames
            .iter()
            .map(|(id, (n, rssi))| format!("{} {n} frames ({rssi} dBm)", device_name(*id)))
            .collect();
        format!("heard: {} · {} messages, {} failed", who.join(", "), self.opened, self.failed)
    }

    fn describe(&mut self, online: bool) -> String {
        let mode = match MODE.load(Ordering::Relaxed) {
            MODE_ON => "on",
            MODE_OFF => "off",
            _ => "auto",
        };
        let used = self.airtime_used();
        let rank = my_rank(self.name);
        let network = match (online, rank) {
            (false, _) => "network: no".to_string(),
            (true, 0) => "network: yes · receives only, never relays".to_string(),
            (true, r) => format!("network: yes ({}, rank {r})", rank_name(r)),
        };
        let mut lines = vec![format!(
            "relay {mode} · {network} · airtime last hour {:.1} s ({:.1} %)",
            used.as_secs_f64(),
            used.as_secs_f64() / HOUR.as_secs_f64() * 100.0
        )];
        if online {
            if self.peers.is_empty() {
                lines.push("  no device without network heard".into());
            }
            for p in self.peers.values() {
                let server = match p.server_online {
                    Some(true) => "server: connected",
                    Some(false) => "server: not connected",
                    None => "server: unknown",
                };
                let via = match p.source {
                    Some((id, r, at)) if id != self.me && at.elapsed() < VIA_FOR => {
                        format!(", takes it from {} ({})", device_name(id), rank_name(r))
                    }
                    _ => String::new(),
                };
                lines.push(format!(
                    "  {}: offline, heard {} s ago ({} dBm), {server}{via}{}",
                    p.name,
                    p.heard.elapsed().as_secs(),
                    p.rssi,
                    if p.relaying { " → relaying" } else { "" }
                ));
            }
        } else {
            match self.source {
                Some((from, at)) => lines.push(format!(
                    "  board over LoRa from {} ({}, {} orders, {} s ago){}",
                    device_name(from),
                    rank_name(self.source_rank),
                    self.relayed.len(),
                    at.elapsed().as_secs(),
                    if self.pending.is_empty() {
                        String::new()
                    } else {
                        format!(", {} taps waiting", self.pending.len())
                    }
                )),
                None => lines.push("  asking for a relay; none heard yet".into()),
            }
            lines.push(format!("  {}", self.heard_line()));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ids_round_trip() {
        assert_eq!(device_id("tab5-ddeeff"), Some([0xdd, 0xee, 0xff]));
        assert_eq!(device_name([0xdd, 0xee, 0xff]), "tab5-ddeeff");
        assert_eq!(device_id("x"), None);
    }

    #[test]
    fn airtime_of_a_full_frame() {
        let t = airtime_at(HEADER + CHUNK, 7);
        assert!(t > Duration::from_millis(300) && t < Duration::from_millis(400), "{t:?}");
        let t = airtime_at(HEADER + CHUNK, 9);
        assert!(t > Duration::from_millis(950) && t < Duration::from_millis(1200), "{t:?}");
    }
}
