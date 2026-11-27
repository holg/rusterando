//! Module LoRaWAN-EU868 (M5Stack M148): a RAK3172 (STM32WLE5 with RAK's
//! RUI3 AT firmware) on a UART, 115200 8N1, powered from the bus 5 V. A
//! 12-way DIP switch puts each of its lines on one of several M5-Bus pins
//! (schematic SCH_LoRaWAN868_HW1.0; TX/RX named from the host's side):
//!
//! | DIP | line                  | Core pin | bus | Tab5 GPIO                 |
//! |-----|-----------------------|----------|-----|---------------------------|
//! | 1   | TX (host → module RX) | G1       | 14  | 37 — UART0 TX, the console |
//! | 2   | TX                    | G17      | 16  | 6                         |
//! | 3   | TX                    | G15      | 23  | 47                        |
//! | 4   | TX                    | G12      | 21  | 2                         |
//! | 5   | TX                    | G0       | 24  | 35 (boot strap)           |
//! | 6   | RX (module TX → host) | G3       | 13  | 38 — UART0 RX, the console |
//! | 7   | RX                    | G16      | 15  | 7                         |
//! | 8   | RX                    | G13      | 22  | 48                        |
//! | 9   | RX                    | G34      | 26  | 51                        |
//! | 10  | RX                    | G35      | 2   | 16                        |
//! | 11  | RST                   | G13      | 22  | 48                        |
//! | 12  | RST                   | G25      | 8   | 45                        |
//!
//! The module pulls its own reset up (10 kΩ), so no RST switch is needed.
//! Switches 1 and 6 would put it on the Tab5's console UART: not used here.
//!
//! [`probe`] finds the positions: the module's TX idles high, so the RX
//! line is the candidate that reads high against a pull-down; then `AT` is
//! sent on each TX candidate until one answers `OK`.
//!
//! [`start`] then runs a worker thread that owns the UART, one AT exchange
//! at a time. In P2P mode it sets [`P2P_DEFAULT`] and offers `listen`,
//! `server` (answers MSG with ACK and PING with PONG), `send` and `ping`,
//! with this text on air:
//!
//! ```text
//! T5 MSG  <seq> <from> <text>
//! T5 ACK  <seq> <from> <rssi> <snr>      (as the answering end heard it)
//! T5 PING <seq> <from>
//! T5 PONG <seq> <from> <rssi> <snr>
//! ```
//!
//! Binary frames starting with [`RELAY_MAGIC`] belong to the kitchen
//! firmware's LoRa relay (crates/kds-tab5 relay.rs): they go out with
//! [`Cmd::SendRaw`] and come in to the sink set with [`Cmd::RelaySink`],
//! not to the screen or the admin page.
//!
//! Every transmission: receive off (AT+PRECV=0), AT+PSEND, wait for
//! `+EVT:TXP2P DONE`, receive back on (AT+PRECV=65534) when the mode wants
//! it — so it doesn't depend on which RUI version allows sending while
//! receiving.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::bus::driven_high;
use crate::report::Event;
use crate::terminal::Tone;
use crate::{say, Out};
use esp_idf_svc::sys::{self, esp};

const UART: sys::uart_port_t = sys::uart_port_t_UART_NUM_1;
const BAUD: i32 = 115_200;

/// Set at start in P2P mode (Hz:SF:BW:CR:preamble:dBm; BW 125 kHz, CR
/// 4/5). 869.525 MHz lies in the EU 869.4–869.65 MHz sub-band: 500 mW ERP
/// and 10 % duty cycle allowed, 14 dBm used. Both ends must match.
/// 869.525 MHz, SF9, 125 kHz, 4/5, preamble 8, 22 dBm — 22 dBm is the
/// most the RAK3172 and the SX1262 give (the 869.4–869.65 MHz band allows
/// 500 mW at 10 % airtime). SF9 instead of SF7 for range: SF7 didn't
/// reach the 1.1 km from the shop (2026-11-13); about 5 dB more, each
/// frame about 4× as long on air (relay.rs paces itself by it).
pub const P2P_DEFAULT: (u32, u8, u16, u8, u16, u8) = (869_525_000, 9, 125, 0, 8, 22);
/// How long to wait for an ACK/PONG.
const ANSWER_WAIT: Duration = Duration::from_secs(3);
/// Between pings of a `ping` run.
const PING_EVERY: Duration = Duration::from_secs(2);
/// The answering end waits this long before it transmits: the asking end
/// needs a moment from TX done to receiving again (one AT exchange).
const TURNAROUND: Duration = Duration::from_millis(50);

/// One DIP position: Tab5 GPIO, M5-Bus pin, switch number.
#[derive(Clone, Copy, Debug)]
pub struct Pos {
    pub gpio: i32,
    pub bus: u8,
    pub dip: u8,
}

const fn pos(gpio: i32, bus: u8, dip: u8) -> Pos {
    Pos { gpio, bus, dip }
}

/// Host TX positions, the console's (DIP 1) left out.
const TX: [Pos; 4] = [pos(6, 16, 2), pos(47, 23, 3), pos(2, 21, 4), pos(35, 24, 5)];
/// Host RX positions, the console's (DIP 6) left out.
const RX: [Pos; 4] = [
    pos(7, 15, 7),
    pos(48, 22, 8),
    pos(51, 26, 9),
    pos(16, 2, 10),
];

impl Pos {
    fn describe(&self) -> String {
        format!("GPIO{} (bus {}, DIP {})", self.gpio, self.bus, self.dip)
    }
}

/// The module as found by [`probe`].
#[derive(Clone, Debug)]
pub struct Found {
    pub tx: Pos,
    pub rx: Pos,
    /// `AT+VER=?`, e.g. RUI_4.0.6_RAK3172-E.
    pub version: String,
    /// `AT+NWM=?`: LoRaWAN, P2P or FSK.
    pub mode: String,
}

impl Found {
    pub fn describe(&self) -> String {
        format!(
            "LoRaWAN: RAK3172 ({}, {} mode) — TX {}, RX {}",
            self.version,
            self.mode,
            self.tx.describe(),
            self.rx.describe()
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Idle,
    Listen,
    Server,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Idle => "idle",
            Mode::Listen => "listen",
            Mode::Server => "server",
        }
    }
}

pub enum Cmd {
    Status,
    /// A binary frame as it is (the relay's; no ack, nothing shown).
    #[allow(dead_code)]
    SendRaw(Vec<u8>),
    /// Where frames starting with [`RELAY_MAGIC`] go.
    #[allow(dead_code)]
    RelaySink(Sender<RelayFrame>),
    /// A raw AT command line, answer shown as it comes.
    At(String),
    Mode(Mode),
    Send(String),
    /// A message typed on the admin page (/admin/lora), with its id there.
    SendAdmin(i64, String),
    Ping(u32),
}

/// The first bytes of a relay frame.
pub const RELAY_MAGIC: &[u8; 3] = b"T5R";
/// A relay frame as received: the bytes, RSSI (dBm), SNR (dB).
pub type RelayFrame = (Vec<u8>, i16, f32);

/// For the status bar.
#[derive(Clone, Copy, Default)]
pub struct State {
    pub p2p: bool,
    pub mode: Mode,
    pub received: u32,
    pub last_rssi: Option<i16>,
}

impl State {
    pub fn describe(&self) -> String {
        if !self.p2p {
            return "RAK3172 LoRaWAN".into();
        }
        let mut s = format!("RAK {}", self.mode.name());
        if self.received > 0 {
            s.push_str(&format!(" rx {}", self.received));
        }
        if let Some(r) = self.last_rssi {
            s.push_str(&format!(" {r} dBm"));
        }
        s
    }
}

/// The running module: commands go to its worker thread.
#[derive(Clone)]
pub struct Rak {
    pub found: Found,
    tx: Sender<Cmd>,
    state: Arc<Mutex<State>>,
}

impl Rak {
    pub fn describe(&self) -> String {
        self.found.describe()
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn state(&self) -> State {
        self.state.lock().map(|s| *s).unwrap_or_default()
    }
}

/// Find the module. `busy`: pins the SPI modules use (never touched).
/// Notes for the screen go to `notes`, what each position did to `raw`.
pub fn probe(busy: &[i32], notes: &mut Vec<String>, raw: &mut Vec<String>) -> Option<Found> {
    let mut rx_lines = idle_high_rx(busy);
    if rx_lines.is_empty() {
        raw.push("RAK3172: no line idles high on RX bus 15/22/26/2 (DIP 7–10)".into());
        return None;
    }
    if let Err(e) = install() {
        notes.push(format!("RAK3172: UART: {e:#}"));
        return None;
    }
    if let Some(f) = round(&rx_lines, busy, "", raw) {
        return Some(f);
    }

    // Silent. First what holds the TX lines (the module's RX): driven low
    // and high in turn, read back.
    for tx in TX.into_iter().filter(|p| !busy.contains(&p.gpio)) {
        let (idle, low, high) = line_check(tx.gpio);
        raw.push(format!(
            "RAK3172: TX {}: idles {}, driven low reads {low}, driven high reads {high}",
            tx.describe(),
            if idle { "high" } else { "low" }
        ));
    }
    // The STM32WL's ROM bootloader (BOOT0 high at power-up, or no valid
    // firmware) answers no AT, but 0x7F with ACK 0x79 (NACK 0x1F once
    // synced). Its Go command starts the firmware at 0x08000000.
    for rx in &rx_lines {
        for tx in TX.into_iter().filter(|p| !busy.contains(&p.gpio)) {
            if !set_pins(tx, *rx) {
                continue;
            }
            let sync = rom_exchange(&[0x7f], 300);
            raw.push(format!(
                "RAK3172: ROM bootloader sync 0x7F on TX {}: {}",
                tx.describe(),
                show(&sync)
            ));
            if sync.contains(&0x79) || sync.contains(&0x1f) {
                // SAFETY: the driver is installed.
                unsafe { sys::uart_set_parity(UART, sys::uart_parity_t_UART_PARITY_EVEN) };
                let id = rom_exchange(&[0x02, 0xfd], 300);
                raw.push(format!("RAK3172: ROM Get ID: {}", show(&id)));
                let go = rom_exchange(&[0x21, 0xde], 300);
                let addr = rom_exchange(&[0x08, 0x00, 0x00, 0x00, 0x08], 300);
                raw.push(format!(
                    "RAK3172: ROM Go 0x08000000: {} / {}",
                    show(&go),
                    show(&addr)
                ));
                // SAFETY: as above.
                unsafe { sys::uart_set_parity(UART, sys::uart_parity_t_UART_PARITY_DISABLE) };
                notes.push(format!(
                    "RAK3172 sat in the STM32 ROM bootloader (TX {}): BOOT0 high, or its firmware is gone — Go sent",
                    tx.describe()
                ));
                std::thread::sleep(Duration::from_secs(2));
                if let Some(f) = round(&rx_lines, busy, "after ROM Go", raw) {
                    return Some(f);
                }
            }
            // SAFETY: plain GPIO reset of a free bus pin.
            unsafe { sys::gpio_reset_pin(tx.gpio) };
        }
    }
    // RAK's update boot mode answers no plain AT; AT+RUN starts the app.
    for rx in &rx_lines {
        for tx in TX.into_iter().filter(|p| !busy.contains(&p.gpio)) {
            if set_pins(tx, *rx) {
                write("AT+RUN");
                std::thread::sleep(Duration::from_millis(100));
                // SAFETY: plain GPIO reset of a free bus pin.
                unsafe { sys::gpio_reset_pin(tx.gpio) };
            }
        }
    }
    std::thread::sleep(Duration::from_secs(2));
    if let Some(f) = round(&rx_lines, busy, "after AT+RUN", raw) {
        notes.push("RAK3172 was in its boot mode: AT+RUN started it".into());
        return Some(f);
    }
    // AT+BAUD survives restarts: the other rates RUI3 offers.
    for baud in [9600, 19200, 38400, 57600, 230400, 460800, 921600] {
        set_baud(baud);
        if let Some(f) = round(&rx_lines, busy, &format!("at {baud} Bd"), raw) {
            notes.push(format!(
                "RAK3172 answers at {baud} baud (at+baud=115200 sets it back)"
            ));
            return Some(f);
        }
    }
    set_baud(BAUD as u32);
    // Last: the bus 5 V off and on (the keyboard restarts with it).
    if let Err(e) = crate::bus::ext5v_off() {
        raw.push(format!("RAK3172: bus 5 V off: {e:#}"));
    }
    std::thread::sleep(Duration::from_secs(2));
    if let Err(e) = crate::bus::ext5v_on() {
        raw.push(format!("RAK3172: bus 5 V on: {e:#}"));
    }
    std::thread::sleep(Duration::from_secs(3));
    let again = idle_high_rx(busy);
    if !again.is_empty() {
        rx_lines = again;
    }
    if let Some(f) = round(&rx_lines, busy, "after a bus 5 V power cycle", raw) {
        notes.push("RAK3172 answered after a bus 5 V power cycle".into());
        return Some(f);
    }

    let rx = rx_lines
        .iter()
        .map(|p| format!("bus {} (DIP {})", p.bus, p.dip))
        .collect::<Vec<_>>()
        .join(", ");
    notes.push(format!(
        "RAK3172? Something idles high on {rx}, but AT got no OK on TX bus 16/23/21/24 (DIP 2–5), nor after AT+RUN, other baud rates or a power cycle — see the log (found:)"
    ));
    // SAFETY: the driver was installed above; RX pin back to a plain input.
    unsafe {
        sys::uart_driver_delete(UART);
        for p in &rx_lines {
            sys::gpio_reset_pin(p.gpio);
        }
    }
    None
}

/// RX candidates something drives high: the module's TX idles high once
/// its firmware runs (a few hundred ms after the bus 5 V comes on).
fn idle_high_rx(busy: &[i32]) -> Vec<Pos> {
    let mut lines = Vec::new();
    for attempt in 0..3 {
        lines = RX
            .into_iter()
            .filter(|p| !busy.contains(&p.gpio) && driven_high(p.gpio))
            .collect();
        if !lines.is_empty() || attempt == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    lines
}

/// `AT` on every TX position with each RX line, twice, each after a bare
/// line end (wakes a sleeping module, ends a half line). Every answer goes
/// to `raw` (in later rounds only the non-empty ones).
fn round(rx_lines: &[Pos], busy: &[i32], label: &str, raw: &mut Vec<String>) -> Option<Found> {
    for rx in rx_lines {
        for tx in TX.into_iter().filter(|p| !busy.contains(&p.gpio)) {
            if !set_pins(tx, *rx) {
                continue;
            }
            for _ in 0..2 {
                write("");
                std::thread::sleep(Duration::from_millis(50));
                let reply = ask_bytes("AT", Duration::from_millis(400));
                if label.is_empty() || !reply.is_empty() {
                    raw.push(format!(
                        "RAK3172{}{label}: AT on TX {} / RX {}: {}",
                        if label.is_empty() { "" } else { " " },
                        tx.describe(),
                        rx.describe(),
                        show(&reply)
                    ));
                }
                if String::from_utf8_lossy(&reply).contains("OK") {
                    return Some(identify(tx, *rx));
                }
            }
            // Let the TX pin go before the next one is tried: with several
            // TX switches ON, only one pin may drive the module's RX.
            // SAFETY: plain GPIO reset of a free bus pin.
            unsafe { sys::gpio_reset_pin(tx.gpio) };
        }
    }
    None
}

fn identify(tx: Pos, rx: Pos) -> Found {
    let version = value(&ask("AT+VER=?", Duration::from_millis(500)), "AT+VER=");
    let mode = match value(&ask("AT+NWM=?", Duration::from_millis(500)), "AT+NWM=").as_str() {
        "0" => "P2P".to_string(),
        "1" => "LoRaWAN".to_string(),
        "2" => "FSK".to_string(),
        other => format!("NWM {other}"),
    };
    Found {
        tx,
        rx,
        version,
        mode,
    }
}

fn set_pins(tx: Pos, rx: Pos) -> bool {
    // SAFETY: the driver is installed; both pins are free bus pins.
    unsafe { esp!(sys::uart_set_pin(UART, tx.gpio, rx.gpio, -1, -1)) }.is_ok()
}

fn set_baud(baud: u32) {
    // SAFETY: the driver is installed.
    unsafe { sys::uart_set_baudrate(UART, baud) };
}

/// Is something else holding `pin`? Idle level against a pull-down, then
/// the level read back while driving it low and high (50 µs each).
fn line_check(pin: i32) -> (bool, i32, i32) {
    let idle = driven_high(pin);
    // SAFETY: plain GPIO configuration of a free bus pin, reset afterwards.
    unsafe {
        sys::gpio_reset_pin(pin);
        sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT);
        sys::gpio_set_level(pin, 0);
        sys::esp_rom_delay_us(50);
        let low = sys::gpio_get_level(pin);
        sys::gpio_set_level(pin, 1);
        sys::esp_rom_delay_us(50);
        let high = sys::gpio_get_level(pin);
        sys::gpio_reset_pin(pin);
        (idle, low, high)
    }
}

/// Raw bytes out, whatever comes back within `wait_ms`.
fn rom_exchange(bytes: &[u8], wait_ms: u64) -> Vec<u8> {
    // SAFETY: the driver is installed; `bytes` outlives the call.
    unsafe {
        sys::uart_flush_input(UART);
        sys::uart_write_bytes(UART, bytes.as_ptr().cast(), bytes.len());
    }
    let end = Instant::now() + Duration::from_millis(wait_ms);
    let mut got = Vec::new();
    let mut buf = [0u8; 64];
    while Instant::now() < end {
        let n = read(&mut buf, 20);
        got.extend_from_slice(&buf[..n]);
    }
    got
}

/// An answer for the log: text and the first bytes in hex.
fn show(reply: &[u8]) -> String {
    if reply.is_empty() {
        return "nothing".into();
    }
    let hex: Vec<String> = reply.iter().take(24).map(|b| format!("{b:02x}")).collect();
    format!(
        "{:?} [{} bytes: {}]",
        String::from_utf8_lossy(reply).trim(),
        reply.len(),
        hex.join(" ")
    )
}

/// Start the worker thread for the module found.
/// `log`: every LoRa event, for the admin page (report.rs).
pub fn start(
    found: Found,
    name: &'static str,
    out: Out,
    log: Sender<Event>,
) -> anyhow::Result<Rak> {
    let (tx, rx) = std::sync::mpsc::channel();
    let p2p = found.mode == "P2P";
    let state = Arc::new(Mutex::new(State {
        p2p,
        ..State::default()
    }));
    let worker = Worker {
        name,
        out,
        rx,
        state: state.clone(),
        p2p,
        mode: Mode::Idle,
        rx_on: false,
        seq: 0,
        waiting: None,
        ping: None,
        partial: Vec::new(),
        events: VecDeque::new(),
        log,
        admin_id: None,
        relay: None,
    };
    std::thread::Builder::new()
        .name("rak".into())
        .stack_size(8 * 1024)
        .spawn(move || worker.run())?;
    Ok(Rak { found, tx, state })
}

struct Waiting {
    seq: u8,
    ping: bool,
    sent: Instant,
}

struct PingRun {
    left: u32,
    next: Instant,
    sent: u32,
    answered: u32,
    rtt_total: Duration,
}

struct Worker {
    name: &'static str,
    out: Out,
    rx: Receiver<Cmd>,
    state: Arc<Mutex<State>>,
    /// The module is in P2P mode (AT+NWM=0).
    p2p: bool,
    mode: Mode,
    /// The module is in continuous receive (AT+PRECV=65534).
    rx_on: bool,
    seq: u8,
    waiting: Option<Waiting>,
    ping: Option<PingRun>,
    /// Bytes of a line not complete yet.
    partial: Vec<u8>,
    /// Lines the module sent on its own during an AT exchange.
    events: VecDeque<String>,
    /// LoRa events for the admin page.
    log: Sender<Event>,
    /// The admin page's id of the message being sent.
    admin_id: Option<i64>,
    /// The relay's frames go here.
    relay: Option<Sender<RelayFrame>>,
}

impl Worker {
    fn run(mut self) {
        if self.p2p {
            self.configure();
        } else {
            self.say(
                Tone::Warn,
                "RAK3172 in LoRaWAN mode: at+nwm=0 switches it to P2P (it restarts)".into(),
            );
        }
        loop {
            match self.rx.recv_timeout(Duration::from_millis(5)) {
                Ok(cmd) => self.command(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            while let Some(line) = self.events.pop_front() {
                self.event(line);
            }
            for line in self.lines(5) {
                self.event(line);
            }
            self.timeouts();
            self.next_ping();
        }
    }

    /// P2P_DEFAULT on the module. RUI versions differ in the BW field (kHz
    /// or an index, 0 = 125 kHz): the kHz form first, then the index.
    fn configure(&mut self) {
        let (f, sf, bw, cr, pre, dbm) = P2P_DEFAULT;
        let khz = format!("AT+P2P={f}:{sf}:{bw}:{cr}:{pre}:{dbm}");
        let index = format!("AT+P2P={f}:{sf}:0:{cr}:{pre}:{dbm}");
        if self.exec(&khz, 1000).is_err() {
            if let Err(e) = self.exec(&index, 1000) {
                self.say(Tone::Err, format!("RAK3172 P2P settings: {e}"));
            }
        }
        let now = self
            .exec("AT+P2P=?", 1000)
            .ok()
            .and_then(|r| {
                r.iter()
                    .find_map(|l| l.strip_prefix("AT+P2P=").map(String::from))
            })
            .unwrap_or_else(|| "?".into());
        self.say(
            Tone::Dim,
            format!("RAK3172 P2P ready: {now} (Hz:SF:BW:CR:preamble:dBm)"),
        );
    }

    fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Status => {
                let st = self.state.lock().map(|s| *s).unwrap_or_default();
                let settings = self
                    .exec("AT+P2P=?", 1000)
                    .ok()
                    .and_then(|r| {
                        r.iter()
                            .find_map(|l| l.strip_prefix("AT+P2P=").map(String::from))
                    })
                    .unwrap_or_else(|| "?".into());
                self.say(
                    Tone::Normal,
                    format!(
                        "RAK3172 {} — P2P {settings} (Hz:SF:BW:CR:preamble:dBm), rx {}",
                        if self.p2p {
                            self.mode.name()
                        } else {
                            "LoRaWAN mode"
                        },
                        st.received
                    ),
                );
            }
            Cmd::At(line) => {
                match self.exec(&line, 3000) {
                    Ok(reply) => {
                        for l in reply {
                            self.say(Tone::Normal, format!("rak: {l}"));
                        }
                        self.say(Tone::Ok, "rak: OK".into());
                    }
                    Err(e) => self.say(Tone::Err, format!("rak: {e}")),
                }
                // The command may have changed the radio's state.
                let upper = line.to_ascii_uppercase();
                if upper.starts_with("AT+PRECV") || upper.starts_with("AT+NWM") {
                    self.rx_on = false;
                }
                if let Some(v) = upper.strip_prefix("AT+NWM=") {
                    if v != "?" {
                        self.p2p = v == "0";
                        if let Ok(mut st) = self.state.lock() {
                            st.p2p = self.p2p;
                        }
                    }
                }
            }
            Cmd::Mode(mode) => {
                if !self.p2p {
                    return self.not_p2p();
                }
                self.mode = mode;
                if let Ok(mut st) = self.state.lock() {
                    st.mode = mode;
                }
                if mode == Mode::Idle {
                    self.ping = None;
                }
                self.settle();
                let what = match mode {
                    Mode::Idle => "RAK idle (receive off)",
                    Mode::Listen => "RAK listening (receive only)",
                    Mode::Server => "RAK server: answering messages and pings",
                };
                self.say(Tone::Ok, what.into());
            }
            Cmd::SendRaw(bytes) => {
                if !self.p2p {
                    return self.not_p2p();
                }
                if self.air(&bytes).is_none() {
                    self.say(Tone::Warn, format!("rak: relay frame of {} B not sent", bytes.len()));
                }
                self.settle();
            }
            Cmd::RelaySink(sink) => self.relay = Some(sink),
            Cmd::SendAdmin(id, text) => {
                self.admin_id = Some(id);
                self.command(Cmd::Send(text));
                self.admin_id = None;
            }
            Cmd::Send(text) => {
                if !self.p2p {
                    return self.not_p2p();
                }
                self.seq = self.seq.wrapping_add(1);
                let frame = format!("T5 MSG {} {} {}", self.seq, self.name, text);
                if self.transmit(&frame) {
                    self.waiting = Some(Waiting {
                        seq: self.seq,
                        ping: false,
                        sent: Instant::now(),
                    });
                }
                self.settle();
            }
            Cmd::Ping(count) => {
                if !self.p2p {
                    return self.not_p2p();
                }
                self.ping = Some(PingRun {
                    left: count.max(1),
                    next: Instant::now(),
                    sent: 0,
                    answered: 0,
                    rtt_total: Duration::ZERO,
                });
                self.say(
                    Tone::Dim,
                    format!(
                        "RAK ping: {} × every {} s",
                        count.max(1),
                        PING_EVERY.as_secs()
                    ),
                );
            }
        }
    }

    fn not_p2p(&self) {
        self.say(
            Tone::Warn,
            "RAK3172 is in LoRaWAN mode: at+nwm=0 first (P2P, it restarts)".into(),
        );
    }

    fn listening(&self) -> bool {
        self.mode != Mode::Idle || self.waiting.is_some() || self.ping.is_some()
    }

    /// Receive on or off, as the mode wants it.
    fn settle(&mut self) {
        let want = self.p2p && self.listening();
        if want == self.rx_on {
            return;
        }
        let cmd = if want { "AT+PRECV=65534" } else { "AT+PRECV=0" };
        match self.exec(cmd, 1000) {
            Ok(_) => self.rx_on = want,
            Err(e) => self.say(Tone::Err, format!("rak: {cmd}: {e}")),
        }
    }

    /// One packet out: receive off, PSEND, wait for TX done; how long it
    /// took, `None` when it didn't go out. The caller settles afterwards.
    fn air(&mut self, bytes: &[u8]) -> Option<Duration> {
        if self.rx_on {
            // Not settle(): the mode may still want receiving.
            if let Err(e) = self.exec("AT+PRECV=0", 1000) {
                self.say(Tone::Err, format!("rak: AT+PRECV=0: {e}"));
            }
            self.rx_on = false;
        }
        let hex: String = bytes.iter().take(256).map(|b| format!("{b:02x}")).collect();
        let started = Instant::now();
        if let Err(e) = self.exec(&format!("AT+PSEND={hex}"), 1000) {
            self.say(Tone::Err, format!("rak tx: {e}"));
            return None;
        }
        if !self.wait_event("+EVT:TXP2P", Duration::from_secs(3)) {
            self.say(Tone::Warn, format!("rak tx: no TX done ({} B)", bytes.len()));
            return None;
        }
        Some(started.elapsed())
    }

    /// One text frame out, shown and logged for the admin page.
    fn transmit(&mut self, frame: &str) -> bool {
        let Some(took) = self.air(frame.as_bytes()) else {
            return false;
        };
        self.say(Tone::Tx, format!("tx {frame}  ({} ms)", took.as_millis()));
        let mut event = frame_event("tx", frame);
        if event.kind == "ACK" || event.kind == "PONG" {
            // Our answer carries what we heard of their packet.
            event.rssi = event.peer_rssi.take();
            event.snr = event.peer_snr.take();
        }
        event.admin_id = self.admin_id.take();
        let _ = self.log.send(event);
        true
    }

    /// A line the module sent on its own.
    fn event(&mut self, line: String) {
        if let Some(rest) = line.strip_prefix("+EVT:RXP2P:") {
            // <rssi>:<snr>:<hex>
            let mut f = rest.splitn(3, ':');
            let rssi = f.next().and_then(|v| v.trim().parse::<i16>().ok());
            let snr = f.next().and_then(|v| v.trim().parse::<f32>().ok());
            let data = f.next().and_then(unhex);
            match (rssi, snr, data) {
                (Some(rssi), Some(snr), Some(data)) => self.packet(rssi, snr, data),
                _ => self.say(Tone::Rx, format!("rak: {line}")),
            }
        } else if line.starts_with("+EVT:RXP2P") {
            // RECEIVE TIMEOUT / ERROR: the module stopped receiving.
            self.say(Tone::Dim, format!("rak: {line}"));
            self.rx_on = false;
            self.settle();
        } else if line == "OK" || line.starts_with("+EVT:TXP2P") {
            // Late answers to our own commands.
        } else {
            self.say(Tone::Normal, format!("rak: {line}"));
        }
    }

    fn packet(&mut self, rssi: i16, snr: f32, data: Vec<u8>) {
        if let Ok(mut st) = self.state.lock() {
            st.received += 1;
            st.last_rssi = Some(rssi);
        }
        if data.starts_with(RELAY_MAGIC) {
            if let Some(sink) = &self.relay {
                let _ = sink.send((data, rssi, snr));
            }
            return;
        }
        let here = format!("{rssi} dBm {snr:+.0} dB");
        let text = String::from_utf8_lossy(&data).into_owned();
        let mut event = frame_event("rx", &text);
        event.rssi = Some(rssi);
        event.snr = Some(snr);
        if event.kind == "RAW" && text.chars().any(|c| c.is_control()) {
            event.text = data.iter().map(|b| format!("{b:02x}")).collect();
        }
        let mut fields = text.splitn(5, ' ');
        let (magic, kind, seq, from, rest) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next().unwrap_or(""),
        );
        let seq: Option<u8> = seq.and_then(|s| s.parse().ok());
        let (Some("T5"), Some(kind), Some(seq), Some(from)) = (magic, kind, seq, from) else {
            let shown = match std::str::from_utf8(&data) {
                Ok(s) if s.chars().all(|c| !c.is_control()) => format!("\"{s}\""),
                _ => data
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            };
            self.say(
                Tone::Rx,
                format!("rx {} bytes: {shown}  ({here})", data.len()),
            );
            let _ = self.log.send(event);
            return;
        };
        match kind {
            "MSG" | "PING" => {
                let _ = self.log.send(std::mem::take(&mut event));
                let ping = kind == "PING";
                if ping {
                    self.say(Tone::Rx, format!("rx ping #{seq} from {from}  ({here})"));
                } else {
                    self.say(Tone::Rx, format!("rx {from} #{seq}: {rest}  ({here})"));
                }
                if self.mode == Mode::Server {
                    std::thread::sleep(TURNAROUND);
                    let answer = if ping { "PONG" } else { "ACK" };
                    let frame = format!("T5 {answer} {seq} {} {rssi} {snr:.0}", self.name);
                    self.transmit(&frame);
                    self.settle();
                }
            }
            "ACK" | "PONG" => {
                let there = {
                    let mut f = rest.split(' ');
                    match (f.next(), f.next()) {
                        (Some(r), Some(s)) => format!("{r} dBm {s} dB"),
                        _ => "?".into(),
                    }
                };
                match self.waiting.take() {
                    Some(w) if w.seq == seq => {
                        let rtt = w.sent.elapsed();
                        event.rtt_ms = Some(rtt.as_millis() as u32);
                        if w.ping {
                            if let Some(run) = self.ping.as_mut() {
                                run.answered += 1;
                                run.rtt_total += rtt;
                            }
                        }
                        self.say(
                            Tone::Ok,
                            format!(
                                "{} #{seq} from {from} in {} ms — there {there}, here {here}",
                                kind.to_lowercase(),
                                rtt.as_millis()
                            ),
                        );
                    }
                    other => {
                        self.waiting = other;
                        self.say(
                            Tone::Dim,
                            format!(
                                "rx {} #{seq} from {from} (not ours)  ({here})",
                                kind.to_lowercase()
                            ),
                        );
                    }
                }
                self.settle();
            }
            _ => self.say(Tone::Rx, format!("rx \"{text}\"  ({here})")),
        }
        // MSG/PING were logged before their answer went out.
        if !event.dir.is_empty() {
            let _ = self.log.send(event);
        }
    }

    fn timeouts(&mut self) {
        if let Some(w) = &self.waiting {
            if w.sent.elapsed() > ANSWER_WAIT {
                let what = if w.ping { "pong" } else { "ack" };
                let mut event = Event::new("lost", if w.ping { "PONG" } else { "ACK" });
                event.seq = Some(w.seq);
                event.from = Some(self.name.to_string());
                let _ = self.log.send(event);
                self.say(
                    Tone::Warn,
                    format!(
                        "no {what} for #{} within {} s",
                        w.seq,
                        ANSWER_WAIT.as_secs()
                    ),
                );
                self.waiting = None;
                self.settle();
            }
        }
    }

    fn next_ping(&mut self) {
        let Some(run) = self.ping.as_mut() else {
            return;
        };
        if self.waiting.is_some() || Instant::now() < run.next {
            return;
        }
        if run.left == 0 {
            let run = self.ping.take().expect("ping run");
            let avg = if run.answered > 0 {
                run.rtt_total.as_millis() / run.answered as u128
            } else {
                0
            };
            let lost = run.sent - run.answered;
            self.say(
                Tone::Normal,
                format!(
                    "ping: {} sent, {} answered, {lost} lost, avg {avg} ms",
                    run.sent, run.answered
                ),
            );
            self.settle();
            return;
        }
        run.left -= 1;
        run.sent += 1;
        run.next = Instant::now() + PING_EVERY;
        self.seq = self.seq.wrapping_add(1);
        let frame = format!("T5 PING {} {}", self.seq, self.name);
        if self.transmit(&frame) {
            self.waiting = Some(Waiting {
                seq: self.seq,
                ping: true,
                sent: Instant::now(),
            });
        }
        self.settle();
    }

    /// Send one AT command, collect its answer lines until OK (→ Ok) or an
    /// error status (→ Err). Lines the module sends on its own meanwhile
    /// (`+EVT:…`) are kept for [`Worker::event`].
    fn exec(&mut self, cmd: &str, wait_ms: u64) -> Result<Vec<String>, String> {
        write(cmd);
        let end = Instant::now() + Duration::from_millis(wait_ms);
        let mut reply = Vec::new();
        while Instant::now() < end {
            for line in self.lines(10) {
                if line == "OK" {
                    return Ok(reply);
                } else if line.contains("ERROR") {
                    return Err(line);
                } else if line.starts_with("+EVT:") {
                    self.events.push_back(line);
                } else {
                    reply.push(line);
                }
            }
        }
        Err(format!("no answer to {cmd}"))
    }

    /// Wait for a line starting with `prefix`; others are kept as events.
    fn wait_event(&mut self, prefix: &str, wait: Duration) -> bool {
        let end = Instant::now() + wait;
        while Instant::now() < end {
            for line in self.lines(10) {
                if line.starts_with(prefix) {
                    return true;
                }
                self.events.push_back(line);
            }
        }
        false
    }

    /// Complete lines received within `wait_ms`.
    fn lines(&mut self, wait_ms: u32) -> Vec<String> {
        let mut buf = [0u8; 256];
        let n = read(&mut buf, wait_ms);
        let mut out = Vec::new();
        for &b in &buf[..n] {
            match b {
                b'\n' | b'\r' => {
                    let line = String::from_utf8_lossy(&self.partial).trim().to_string();
                    self.partial.clear();
                    if !line.is_empty() {
                        out.push(line);
                    }
                }
                _ if self.partial.len() < 600 => self.partial.push(b),
                _ => {}
            }
        }
        out
    }

    fn say(&self, tone: Tone, text: String) {
        say(&self.out, tone, text);
    }
}

/// Hex digits → bytes.
fn unhex(hex: &str) -> Option<Vec<u8>> {
    let hex = hex.trim();
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

/// `AT+X=value` out of an answer (the line that starts with `key`).
fn value(reply: &str, key: &str) -> String {
    reply
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix(key))
        .unwrap_or("?")
        .to_string()
}

/// Send a command and collect the answer until OK/ERROR or `wait`.
fn ask(cmd: &str, wait: Duration) -> String {
    String::from_utf8_lossy(&ask_bytes(cmd, wait)).into_owned()
}

/// As [`ask`], the bytes as they came (for the log when they're garbage).
fn ask_bytes(cmd: &str, wait: Duration) -> Vec<u8> {
    // SAFETY: the driver is installed.
    unsafe { sys::uart_flush_input(UART) };
    write(cmd);
    let end = Instant::now() + wait;
    let mut got = Vec::new();
    let mut buf = [0u8; 128];
    while Instant::now() < end {
        let n = read(&mut buf, 20);
        got.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&got);
        if text.contains("OK\r\n") || text.contains("ERROR") {
            break;
        }
    }
    got
}

fn write(cmd: &str) {
    let line = format!("{cmd}\r\n");
    // SAFETY: the driver is installed; the buffer outlives the call (the
    // driver copies into its TX ring buffer).
    unsafe { sys::uart_write_bytes(UART, line.as_ptr().cast(), line.len()) };
}

fn read(buf: &mut [u8], wait_ms: u32) -> usize {
    // SAFETY: the driver is installed; `buf` is valid for its length.
    let n =
        unsafe { sys::uart_read_bytes(UART, buf.as_mut_ptr().cast(), buf.len() as u32, wait_ms) };
    n.max(0) as usize
}

fn install() -> anyhow::Result<()> {
    // SAFETY: zeroed is a valid uart_config_t (flags off); the fields that
    // matter are set below. UART1 is unused otherwise (the console is
    // UART0 and USB).
    unsafe {
        let mut cfg: sys::uart_config_t = core::mem::zeroed();
        cfg.baud_rate = BAUD;
        cfg.data_bits = sys::uart_word_length_t_UART_DATA_8_BITS;
        cfg.parity = sys::uart_parity_t_UART_PARITY_DISABLE;
        cfg.stop_bits = sys::uart_stop_bits_t_UART_STOP_BITS_1;
        cfg.flow_ctrl = sys::uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_DISABLE;
        cfg.__bindgen_anon_1.source_clk = sys::soc_periph_uart_clk_src_legacy_t_UART_SCLK_DEFAULT;
        esp!(sys::uart_driver_install(
            UART,
            2048,
            1024,
            0,
            core::ptr::null_mut(),
            0
        ))?;
        esp!(sys::uart_param_config(UART, &cfg))?;
    }
    Ok(())
}

/// A `T5 …` frame (or anything else) as an event for the admin page.
fn frame_event(dir: &'static str, text: &str) -> Event {
    let mut f = text.splitn(5, ' ');
    let (magic, kind, seq, from, rest) = (
        f.next(),
        f.next(),
        f.next(),
        f.next(),
        f.next().unwrap_or(""),
    );
    let seq = seq.and_then(|s| s.parse::<u8>().ok());
    let (Some("T5"), Some(kind), Some(seq), Some(from)) = (magic, kind, seq, from) else {
        let mut event = Event::new(dir, "RAW");
        event.text = text.to_string();
        return event;
    };
    let mut event = Event::new(dir, kind);
    event.seq = Some(seq);
    event.from = Some(from.to_string());
    match kind {
        // "<rssi> <snr>": how the answering end heard the MSG/PING.
        "ACK" | "PONG" => {
            let mut v = rest.split(' ');
            event.peer_rssi = v.next().and_then(|r| r.parse().ok());
            event.peer_snr = v.next().and_then(|r| r.parse().ok());
        }
        _ => event.text = rest.to_string(),
    }
    event
}
