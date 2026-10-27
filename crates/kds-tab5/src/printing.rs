//! The kitchen's print jobs over WebSocket (`/api/kitchen/ws`), what the
//! board gets through its SSH tunnel:
//!
//!   * binary frames, postcard both ways: `ServerMessage` (print job with its
//!     seq) in, `ClientMessage` (Ack / Heartbeat / PrinterPresent) out;
//!   * the USB connection is the switch: with a printer plugged in, every
//!     print job is printed and then acked; without one, nothing is printed
//!     and nothing acked;
//!   * the server learns whether a printer is attached (admin printer page,
//!     printer-offline alarm) on connect and on every change;
//!   * the last seq is kept in NVS: a reconnect replays only what was missed,
//!     the very first connect starts from now (no backlog).
//!
//! ESP-IDF's WebSocket client runs its own task, used through the C API: the
//! esp-idf-svc wrapper doesn't say where a frame ends, and a receipt with
//! logos spans several TLS records. Its events come here through a channel.

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::sync::mpsc::{self, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sys::{self, esp};
use kitchen_protocol::bitmap::Bitmap;
use kitchen_protocol::receipt::{self, ComposedSvg, RasterProvider, RasterSlot, ReceiptConfig, ReceiptProgram};
use kitchen_protocol::{
    ClientMessage, Component, Customer, KitchenEvent, LineItem, OrderChannel, OrderForKitchen, OrderId,
    PaymentStatus, ServerMessage,
};

use crate::board;
use crate::feed::{self, Session};
use crate::ota;
use crate::ui::UiMsg;
use crate::usb_printer::UsbPrinter;

/// Device → server sign of life (the server pings on its own).
const HEARTBEAT: Duration = Duration::from_secs(30);
/// Pause before reconnecting.
const RETRY: Duration = Duration::from_secs(5);
/// After a refused connect (e.g. 403): don't hammer the server.
const RETRY_REFUSED: Duration = Duration::from_secs(60);
/// No connection within this time = give up the attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// A larger frame is dropped (a receipt with logos is ~25 KB).
const MAX_FRAME: usize = 512 * 1024;
/// A failed print is tried this often while the printer stays plugged in.
const PRINT_TRIES: u32 = 3;
/// Rendered logos kept (by content hash): a handful of channel/payment
/// variants, the same on every receipt.
const LOGO_CACHE: usize = 16;
/// The logo renderer's stack (measured: resvg peaks at about 80 KB).
const RENDER_STACK: usize = 128 * 1024;
const KEY_SEQ: &str = "kitchen_seq";

/// What the Start page shows about printing.
#[derive(Clone, Default, PartialEq)]
pub struct PrintStatus {
    /// The plugged-in printer's name.
    pub printer: Option<String>,
    /// The print channel is connected.
    pub online: bool,
    /// The last print problem, until the next good print.
    pub problem: Option<String>,
    /// Receipts printed since boot.
    pub printed: u32,
}

pub type SharedStatus = Arc<Mutex<PrintStatus>>;

fn update(status: &SharedStatus, f: impl FnOnce(&mut PrintStatus)) {
    if let Ok(mut s) = status.lock() {
        f(&mut s);
    }
}

/// Start the print channel thread; it connects once the feed has a Küche
/// session.
pub fn start(
    printer: UsbPrinter,
    session: Session,
    nvs: EspDefaultNvsPartition,
    ui: Sender<UiMsg>,
    status: SharedStatus,
) -> Result<()> {
    std::thread::Builder::new()
        .name("printing".into())
        .stack_size(16 * 1024)
        .spawn(move || run(printer, session, nvs, ui, status))?;
    Ok(())
}

fn run(printer: UsbPrinter, session: Session, nvs: EspDefaultNvsPartition, ui: Sender<UiMsg>, status: SharedStatus) {
    warm_up_logos();
    let store = EspNvs::new(nvs, feed::NVS_NAMESPACE, true)
        .map_err(|e| log::warn!("printing: NVS: {e}"))
        .ok();
    let mut channel = Channel {
        printer,
        ui,
        status,
        last_seen: store.as_ref().and_then(|s| s.get_u64(KEY_SEQ).ok().flatten()),
        store,
        download: None,
    };
    let base = feed::SHOP_URL.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
    loop {
        let Some(cookie) = session.lock().ok().and_then(|s| s.clone()) else {
            // The feed logs in; wait for it.
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        let mut url = format!(
            "{base}/api/kitchen/ws?slug={}&version={}&arch=esp32p4",
            board::device_name(),
            ota::VERSION
        );
        if let Some(seq) = channel.last_seen {
            url.push_str(&format!("&last_seen={seq}"));
        }
        let pause = match channel.round(&url, &cookie) {
            Ok(()) => {
                log::info!("printing: disconnected");
                RETRY
            }
            Err(e) => {
                log::warn!("printing: {e:#}");
                if format!("{e:#}").contains("HTTP") {
                    RETRY_REFUSED
                } else {
                    RETRY
                }
            }
        };
        update(&channel.status, |s| s.online = false);
        std::thread::sleep(pause);
    }
}

struct Channel {
    printer: UsbPrinter,
    ui: Sender<UiMsg>,
    status: SharedStatus,
    /// The newest seq handled (printed, or skipped without a printer).
    last_seen: Option<u64>,
    store: Option<EspNvs<NvsDefault>>,
    /// An update image being received (see ota.rs).
    download: Option<ota::Download>,
}

impl Channel {
    /// One connection, until it ends.
    fn round(&mut self, url: &str, cookie: &str) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let conn = Connection::open(url, cookie, tx)?;
        let started = Instant::now();
        let mut connected = false;
        let mut told: Option<bool> = None;
        let mut beat = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(WsEvent::Connected) => {
                    log::info!("printing: connected as '{}', last seq {:?}", board::device_name(), self.last_seen);
                    // Wi-Fi, TLS, login and server work: a fresh update stays.
                    ota::confirm();
                    connected = true;
                    beat = Instant::now();
                    update(&self.status, |s| s.online = true);
                }
                Ok(WsEvent::Frame(bytes)) => self.handle(&conn, &bytes),
                Ok(WsEvent::Refused(code)) => bail!("server refused the connection (HTTP {code})"),
                Ok(WsEvent::Closed) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Err(RecvTimeoutError::Timeout) => {}
            }
            if !connected {
                if started.elapsed() > CONNECT_TIMEOUT {
                    bail!("no connection after {} s", CONNECT_TIMEOUT.as_secs());
                }
                continue;
            }
            let present = self.printer.present();
            if told != Some(present) {
                conn.send(&ClientMessage::PrinterPresent(present))?;
                told = Some(present);
            }
            if beat.elapsed() >= HEARTBEAT {
                conn.send(&ClientMessage::Heartbeat)?;
                beat = Instant::now();
            }
        }
    }

    fn handle(&mut self, conn: &Connection, bytes: &[u8]) {
        let msg = match postcard::from_bytes::<ServerMessage>(bytes) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("printing: undecodable frame ({} B): {e}", bytes.len());
                return;
            }
        };
        // Update frames are out of band (seq 0), never deduplicated.
        match &msg.event {
            KitchenEvent::UpdateOffer { component, target_version, sha256, size } => {
                self.update_offer(conn, *component, target_version.clone(), *sha256, *size);
                return;
            }
            KitchenEvent::ArtifactChunk { component, offset, bytes, last } => {
                self.update_chunk(conn, *component, *offset, bytes, *last);
                return;
            }
            _ => {}
        }
        let seq = msg.seq_id.0;
        if self.last_seen.is_some_and(|last| seq <= last) {
            log::info!("printing: seq {seq} already handled");
            return;
        }
        let KitchenEvent::Print(program) = &msg.event else {
            self.remember(seq);
            return;
        };
        let number = program.display_number;
        if !self.printer.present() {
            log::info!("printing: seq {seq} (Bon #{number}): no printer attached, not printed");
            self.remember(seq);
            return;
        }
        match print_program(&self.printer, program) {
            Ok(()) => {
                log::info!("printing: seq {seq} (Bon #{number}) printed");
                // Remembered before the ack: a lost ack must not print it twice.
                self.remember(seq);
                if let Err(e) = conn.send(&ClientMessage::Ack(msg.seq_id)) {
                    log::warn!("printing: ack for seq {seq} not sent: {e:#}");
                }
                update(&self.status, |s| {
                    s.printed += 1;
                    s.problem = None;
                });
            }
            Err(e) => {
                log::warn!("printing: seq {seq} (Bon #{number}): {e:#}");
                self.remember(seq);
                update(&self.status, |s| s.problem = Some(format!("{e:#}")));
                let _ = self.ui.send(UiMsg::Notice(format!("Bon #{number} nicht gedruckt: {e:#}")));
            }
        }
    }

    /// The server offers an image (an admin armed the update): start
    /// fetching it. A new offer restarts an unfinished download.
    fn update_offer(&mut self, conn: &Connection, component: Component, target: String, sha256: [u8; 32], size: u64) {
        self.download = None;
        let started = ota::Download::begin(target.clone(), size, sha256);
        match started {
            Ok(download) => {
                let _ = self.ui.send(UiMsg::Info(format!("Update auf {target} wird geladen …")));
                self.download = Some(download);
                if let Err(e) = conn.send(&ClientMessage::FetchArtifact { component, offset: 0 }) {
                    log::warn!("ota: request failed: {e:#}");
                    self.download = None;
                }
            }
            Err(e) => {
                log::warn!("ota: {target} refused: {e:#}");
                self.update_result(conn, component, &target, Err(e));
            }
        }
    }

    /// One chunk of the image: store it, ask for the next, or finish.
    fn update_chunk(&mut self, conn: &Connection, component: Component, offset: u64, bytes: &[u8], last: bool) {
        let Some(download) = self.download.as_mut() else {
            log::warn!("ota: chunk without an update in progress");
            return;
        };
        let target = download.target.clone();
        let outcome = match download.write(offset, bytes) {
            Ok(false) if last => Err(anyhow!("image ended early at {} B", download.offset())),
            Ok(false) => {
                let next = download.offset();
                if let Err(e) = conn.send(&ClientMessage::FetchArtifact { component, offset: next }) {
                    log::warn!("ota: request failed: {e:#}");
                    self.download = None;
                }
                return;
            }
            Ok(true) => self.download.take().map_or_else(|| Err(anyhow!("no download")), ota::Download::finish),
            Err(e) => Err(e),
        };
        self.download = None;
        let installed = outcome.is_ok();
        self.update_result(conn, component, &target, outcome);
        if installed {
            let _ = self.ui.send(UiMsg::Info(format!("Update {target} installiert — Neustart")));
            // Time for the result to go out and the notice to show.
            std::thread::sleep(Duration::from_secs(3));
            // SAFETY: plain restart into the new image.
            unsafe { sys::esp_restart() };
        }
    }

    fn update_result(&mut self, conn: &Connection, component: Component, target: &str, outcome: Result<()>) {
        let (ok, error) = match outcome {
            Ok(()) => (true, String::new()),
            Err(e) => {
                let _ = self.ui.send(UiMsg::Notice(format!("Update fehlgeschlagen: {e:#}")));
                (false, format!("{e:#}"))
            }
        };
        let result = ClientMessage::UpdateResult {
            component,
            from: ota::VERSION.to_string(),
            to: target.to_string(),
            ok,
            error,
        };
        if let Err(e) = conn.send(&result) {
            log::warn!("ota: result not sent: {e:#}");
        }
    }

    fn remember(&mut self, seq: u64) {
        self.last_seen = Some(seq);
        if let Some(store) = &self.store {
            if let Err(e) = store.set_u64(KEY_SEQ, seq) {
                log::warn!("printing: NVS: {e}");
            }
        }
    }
}

/// Print a receipt; a failure is tried again while the printer stays
/// plugged in.
fn print_program(printer: &UsbPrinter, program: &ReceiptProgram) -> Result<()> {
    let bytes = receipt_escpos::render(program, &mut logo)?;
    let mut tries = 1;
    loop {
        match printer.print(bytes.clone()) {
            Ok(()) => return Ok(()),
            Err(e) if tries < PRINT_TRIES && printer.present() => {
                log::warn!("printing: try {tries}: {e:#}");
                tries += 1;
                std::thread::sleep(Duration::from_secs(3));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Render a small logo with brand text once at start: loads the font for
/// the first real receipt and shows in the log that the renderer works.
fn warm_up_logos() {
    const SAMPLE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="40"><rect width="200" height="40" fill="white"/><text x="10" y="30" font-family="Poppins" font-size="28">Testdruck</text></svg>"#;
    let started = Instant::now();
    match rasterize(SAMPLE, 336) {
        Ok(bitmap) => {
            let ink = bitmap.bytes.iter().filter(|&&b| b != 0).count();
            log::info!(
                "printing: logo renderer ready ({}×{}, {} ink bytes, {} ms)",
                bitmap.width,
                bitmap.height,
                ink,
                started.elapsed().as_millis()
            );
        }
        Err(e) => log::warn!("printing: logo renderer: {e:#}"),
    }
}

/// A logo render job for the renderer thread.
struct RenderJob {
    svg: Vec<u8>,
    width: u16,
    reply: SyncSender<Result<Bitmap>>,
}

/// Render an SVG logo on the renderer thread. resvg needs about 80 KB of
/// stack; that must not come out of the internal RAM (TLS's SHA engine
/// needs it — a big internal stack broke every handshake), so the renderer
/// runs on its own thread with its stack in PSRAM. That thread never touches
/// flash: a task with a PSRAM stack must not run while the cache is off.
fn rasterize(svg: &[u8], width: u16) -> Result<Bitmap> {
    static RENDERER: OnceLock<Option<Mutex<Sender<RenderJob>>>> = OnceLock::new();
    let renderer = RENDERER
        .get_or_init(|| match spawn_renderer() {
            Ok(jobs) => Some(Mutex::new(jobs)),
            Err(e) => {
                log::warn!("printing: no logo renderer: {e:#}");
                None
            }
        })
        .as_ref()
        .ok_or_else(|| anyhow!("no logo renderer"))?;
    let (reply, result) = mpsc::sync_channel(1);
    renderer
        .lock()
        .map_err(|_| anyhow!("logo renderer lock"))?
        .send(RenderJob { svg: svg.to_vec(), width, reply })
        .map_err(|_| anyhow!("logo renderer gone"))?;
    result.recv().map_err(|_| anyhow!("logo renderer gone"))?
}

fn spawn_renderer() -> Result<Sender<RenderJob>> {
    let (jobs, rx) = mpsc::channel::<RenderJob>();
    // Threads created by this thread get their stack from PSRAM — only this
    // one: the default is restored right after.
    // SAFETY: plain config structs; the pthread layer copies them.
    unsafe {
        let mut cfg = sys::esp_pthread_get_default_config();
        cfg.stack_size = RENDER_STACK;
        cfg.stack_alloc_caps = sys::MALLOC_CAP_SPIRAM | sys::MALLOC_CAP_8BIT;
        cfg.inherit_cfg = false;
        esp!(sys::esp_pthread_set_cfg(&cfg))?;
    }
    let spawned = std::thread::Builder::new()
        .name("logos".into())
        .stack_size(RENDER_STACK)
        .spawn(move || {
            let mut first = true;
            for job in rx {
                let _ = job.reply.send(receipt_escpos::svg::rasterize(&job.svg, job.width));
                if std::mem::take(&mut first) {
                    // SAFETY: plain query about this task.
                    let free = unsafe { sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut()) };
                    log::info!("printing: logo renderer stack (PSRAM): {free} of {RENDER_STACK} bytes never used");
                }
            }
        });
    // SAFETY: as above.
    unsafe {
        let default = sys::esp_pthread_get_default_config();
        let _ = sys::esp_pthread_set_cfg(&default);
    }
    spawned?;
    Ok(jobs)
}

/// A receipt logo as a 1-bit raster: rendered from the server's SVG once,
/// then from the cache. `None` leaves it out (the text banners still carry
/// channel and payment).
fn logo(hash: &[u8; 32], target_width: u16, svg: &[u8]) -> Option<Bitmap> {
    static CACHE: Mutex<Option<HashMap<[u8; 32], Bitmap>>> = Mutex::new(None);
    let mut cache = CACHE.lock().ok()?;
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(bitmap) = cache.get(hash) {
        return Some(bitmap.clone());
    }
    if svg.is_empty() {
        // A reference to a logo we were never sent.
        return None;
    }
    let started = Instant::now();
    match rasterize(svg, target_width) {
        Ok(bitmap) => {
            log::info!(
                "printing: logo {}×{} rendered in {} ms",
                bitmap.width,
                bitmap.height,
                started.elapsed().as_millis()
            );
            if cache.len() >= LOGO_CACHE {
                cache.clear();
            }
            cache.insert(*hash, bitmap.clone());
            Some(bitmap)
        }
        Err(e) => {
            log::warn!("printing: logo left out: {e:#}");
            None
        }
    }
}

/// Start page "Testdruck": a sample receipt, composed here with the server's
/// layout code (umlauts, €, QR code, cut).
pub fn test_print(printer: &UsbPrinter) -> Result<()> {
    struct NoLogos;
    impl RasterProvider for NoLogos {
        fn resolve(&self, _theme: &str, _slot: RasterSlot) -> Option<ComposedSvg> {
            None
        }
    }
    let program = receipt::compose(&sample_order(), false, &ReceiptConfig::default(), &NoLogos);
    print_program(printer, &program)
}

fn sample_order() -> OrderForKitchen {
    // SAFETY: time() and localtime_r() fill the given buffers.
    let (now, label) = unsafe {
        let now = sys::time(core::ptr::null_mut());
        let mut tm: sys::tm = core::mem::zeroed();
        sys::localtime_r(&now, &mut tm);
        (now, format!("{:02}.{:02}. {:02}:{:02}", tm.tm_mday, tm.tm_mon + 1, tm.tm_hour, tm.tm_min))
    };
    let item = |qty: u32, name: &str, mods: &[&str], cents: u32| LineItem {
        qty,
        name: name.to_string(),
        modifications: mods.iter().map(|m| m.to_string()).collect(),
        unit_price_cents: cents,
        category: "Testdruck".to_string(),
        modification_prices_cents: Vec::new(),
    };
    OrderForKitchen {
        order_id: OrderId(0),
        display_number: 0,
        display_label: "TEST-0000".to_string(),
        created_at_unix: now as i64,
        created_at_label: label,
        channel: OrderChannel::Pickup,
        customer: Customer { name: "Tab5 Testdruck".to_string(), phone: None },
        items: vec![
            item(1, "#22a Pizza Funghi - Größe XL", &["extra Käse", "ohne Zwiebeln"], 1150),
            item(2, "#12 Pizzabrötchen", &["Sauce: Kräuterbutter"], 450),
        ],
        subtotal_cents: 2050,
        delivery_fee_cents: 0,
        total_cents: 2050,
        payment: PaymentStatus::CollectOnDelivery { amount_cents: 2050 },
        note: Some("Testdruck vom Tab5 - äöüß € - bitte wegwerfen".to_string()),
        shop_name: "Testdruck".to_string(),
        accepted_at_unix: None,
        accepted_at_label: None,
        qr_url: Some(feed::SHOP_URL.to_string()),
        is_test_mode: true,
        voucher_code: String::new(),
        voucher_discount_cents: 0,
        pickup_time_label: None,
        printer_theme: String::new(),
    }
}

/// What the WebSocket client's task reports.
enum WsEvent {
    Connected,
    Closed,
    /// One whole binary message.
    Frame(Vec<u8>),
    /// The server answered the upgrade with this HTTP status.
    Refused(i32),
}

/// State of the event handler, which runs on the client's task.
struct Handler {
    tx: Sender<WsEvent>,
    frame: Vec<u8>,
    too_big: bool,
}

/// One WebSocket connection (ESP-IDF's client, its own task).
struct Connection {
    client: sys::esp_websocket_client_handle_t,
    handler: *mut Handler,
    // The client keeps pointers into these until it's destroyed.
    _uri: CString,
    _headers: CString,
}

impl Connection {
    fn open(url: &str, cookie: &str, tx: Sender<WsEvent>) -> Result<Self> {
        let uri = CString::new(url)?;
        let headers = CString::new(format!("Cookie: {cookie}\r\n"))?;
        // SAFETY: a zeroed config is the client's "all defaults".
        let mut cfg: sys::esp_websocket_client_config_t = unsafe { core::mem::zeroed() };
        cfg.uri = uri.as_ptr();
        cfg.headers = headers.as_ptr();
        // We reconnect ourselves, with a fresh last_seen in the URL.
        cfg.disable_auto_reconnect = true;
        cfg.task_stack = 8 * 1024;
        cfg.buffer_size = 4096;
        cfg.network_timeout_ms = 10_000;
        cfg.crt_bundle_attach = Some(sys::esp_crt_bundle_attach);
        // SAFETY: the config and its strings outlive the call (the strings
        // live as long as the connection).
        let client = unsafe { sys::esp_websocket_client_init(&cfg) };
        if client.is_null() {
            bail!("WebSocket client: init failed");
        }
        let handler = Box::into_raw(Box::new(Handler { tx, frame: Vec::new(), too_big: false }));
        let conn = Self { client, handler, _uri: uri, _headers: headers };
        // SAFETY: live client; the handler state lives until Drop, after the
        // client (and its task) is gone.
        unsafe {
            esp!(sys::esp_websocket_register_events(
                client,
                sys::esp_websocket_event_id_t_WEBSOCKET_EVENT_ANY,
                Some(on_event),
                handler.cast(),
            ))?;
            esp!(sys::esp_websocket_client_start(client))?;
        }
        Ok(conn)
    }

    fn send(&self, msg: &ClientMessage) -> Result<()> {
        let bytes = postcard::to_allocvec(msg)?;
        // SAFETY: live client; the data is copied before the call returns.
        let sent = unsafe {
            sys::esp_websocket_client_send_bin(self.client, bytes.as_ptr().cast(), bytes.len() as i32, 5000)
        };
        if sent < 0 {
            return Err(anyhow!("WebSocket: send failed"));
        }
        Ok(())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // SAFETY: destroy stops the client's task first, so the handler
        // state is no longer used when it's freed.
        unsafe {
            sys::esp_websocket_client_destroy(self.client);
            drop(Box::from_raw(self.handler));
        }
    }
}

/// The client's event handler (on its task): connection state, and binary
/// messages put back together from the chunks the client delivers.
unsafe extern "C" fn on_event(arg: *mut c_void, _base: sys::esp_event_base_t, id: i32, data: *mut c_void) {
    let h = &mut *(arg as *mut Handler);
    let d = &*(data as *const sys::esp_websocket_event_data_t);
    let id = id as sys::esp_websocket_event_id_t;
    #[allow(non_upper_case_globals)]
    match id {
        sys::esp_websocket_event_id_t_WEBSOCKET_EVENT_CONNECTED => {
            let _ = h.tx.send(WsEvent::Connected);
        }
        sys::esp_websocket_event_id_t_WEBSOCKET_EVENT_DISCONNECTED
        | sys::esp_websocket_event_id_t_WEBSOCKET_EVENT_CLOSED => {
            let _ = h.tx.send(WsEvent::Closed);
        }
        sys::esp_websocket_event_id_t_WEBSOCKET_EVENT_ERROR => {
            let code = d.error_handle.esp_ws_handshake_status_code;
            if code != 0 && code != 101 {
                let _ = h.tx.send(WsEvent::Refused(code));
            }
        }
        sys::esp_websocket_event_id_t_WEBSOCKET_EVENT_DATA => {
            match d.op_code {
                // Binary: a new message starts with the frame's first chunk.
                0x2 => {
                    if d.payload_offset == 0 {
                        h.frame.clear();
                        h.too_big = false;
                    }
                }
                // Continuation of a fragmented message.
                0x0 => {}
                // Text, close, ping, pong: nothing for us.
                _ => return,
            }
            let len = d.data_len.max(0) as usize;
            if len > 0 && !d.data_ptr.is_null() {
                if h.frame.len() + len > MAX_FRAME {
                    h.too_big = true;
                    h.frame = Vec::new();
                }
                if !h.too_big {
                    h.frame.extend_from_slice(core::slice::from_raw_parts(d.data_ptr.cast::<u8>(), len));
                }
            }
            let frame_done = d.payload_offset + d.data_len >= d.payload_len;
            if d.fin && frame_done {
                if h.too_big {
                    log::warn!("printing: message over {MAX_FRAME} B dropped");
                } else {
                    let _ = h.tx.send(WsEvent::Frame(core::mem::take(&mut h.frame)));
                }
            }
        }
        _ => {}
    }
}
