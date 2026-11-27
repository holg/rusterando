//! The Cap LoRa-1262 on the Cardputer ADV: an SX1262, driven directly, in
//! the guise of the Tab5's RAK3172 module (tab5-lab rak.rs) — the same
//! [`Rak`] handle, [`Cmd`]s and [`Mode`]s, and the same frames on air:
//!
//! ```text
//! T5 MSG  <seq> <from> <text>
//! T5 ACK  <seq> <from> <rssi> <snr>
//! T5 PING <seq> <from>
//! T5 PONG <seq> <from> <rssi> <snr>
//! T5R…    the kitchen relay's binary frames (relay.rs)
//! ```
//!
//! So the shell's `rak …` commands, the relay and the admin page's LoRa log
//! work unchanged, and it talks to the Tab5s' RAK3172s: their P2P settings
//! ([`P2P_DEFAULT`]) and RUI3's packet format — private sync word 0x1424,
//! explicit header, CRC on, IQ normal, LDRO as RUI3 sets it.
//!
//! Wiring (cap schematic U214, M5's demo, Meshtastic's variant): SPI2 with
//! SCK G40, MOSI G14, MISO G39 (shared with the microSD, whose CS G12 main
//! holds high), NSS G5, NRESET G3, BUSY G6, DIO1 G4. The TCXO runs from DIO3
//! at 3.0 V, DIO2 switches the antenna between TX and RX — and the switch
//! itself is powered from P0 of a PI4IOE5V6408 expander at I2C 0x43
//! ([`antenna_power`]).

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use esp_idf_svc::hal::gpio::{Input, InputPin, Output, OutputPin, PinDriver, Pull};
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::hal::spi::{config, SpiAnyPins, SpiDeviceDriver, SpiDriver};
use esp_idf_svc::hal::units::FromValueType;

use crate::report::Event;
use crate::terminal::Tone;
use crate::{say, Out};

/// Hz:SF:BW(kHz):CR:preamble:dBm — the Tab5s' RAK3172 setting (rak.rs).
/// 869.525 MHz, SF9, 125 kHz, 4/5, preamble 8, 22 dBm — 22 dBm is the
/// most the RAK3172 and the SX1262 give (the 869.4–869.65 MHz band allows
/// 500 mW at 10 % airtime). SF9 instead of SF7 for range: SF7 didn't
/// reach the 1.1 km from the shop (2026-11-13); about 5 dB more, each
/// frame about 4× as long on air (relay.rs paces itself by it).
pub const P2P_DEFAULT: (u32, u8, u16, u8, u16, u8) = (869_525_000, 9, 125, 0, 8, 22);
/// RUI3's default P2P sync word (registers 0x0740/0x0741).
const SYNC_WORD: u16 = 0x1424;
const ANSWER_WAIT: Duration = Duration::from_secs(3);
const PING_EVERY: Duration = Duration::from_secs(2);
/// The answering end waits this long before it transmits (as rak.rs).
const TURNAROUND: Duration = Duration::from_millis(50);

/// The first bytes of a relay frame.
pub const RELAY_MAGIC: &[u8; 3] = b"T5R";
/// A relay frame as received: the bytes, RSSI (dBm), SNR (dB).
pub type RelayFrame = (Vec<u8>, i16, f32);

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

#[allow(dead_code)]
pub enum Cmd {
    Status,
    /// A binary frame as it is (the relay's; no ack, nothing shown).
    SendRaw(Vec<u8>),
    /// Where frames starting with [`RELAY_MAGIC`] go.
    RelaySink(Sender<RelayFrame>),
    /// `at+…` typed in the shell: a few of the RAK's, for the settings.
    At(String),
    Mode(Mode),
    Send(String),
    /// A message typed on the admin page (/admin/lora), with its id there.
    SendAdmin(i64, String),
    Ping(u32),
}

/// What was found, for `rak` and `info`.
#[derive(Clone, Debug)]
pub struct Found {
    pub version: String,
    pub mode: String,
}

impl Found {
    pub fn describe(&self) -> String {
        format!("LoRa: SX1262 on the Cap LoRa-1262 ({}), {} mode", self.version, self.mode)
    }
}

#[derive(Clone, Copy, Default)]
pub struct State {
    pub mode: Mode,
    pub received: u32,
    pub last_rssi: Option<i16>,
}

impl State {
    #[allow(dead_code)]
    pub fn describe(&self) -> String {
        let mut s = format!("LoRa {}", self.mode.name());
        if self.received > 0 {
            s.push_str(&format!(" rx {}", self.received));
        }
        if let Some(r) = self.last_rssi {
            s.push_str(&format!(" {r} dBm"));
        }
        s
    }
}

/// The running radio: commands go to its worker thread.
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

    #[allow(dead_code)]
    pub fn state(&self) -> State {
        self.state.lock().map(|s| *s).unwrap_or_default()
    }
}

/// Power for the cap's antenna switch: P0 of its PI4IOE5V6408 (0x43) as a
/// driven-high output. Before the keyboard takes the I2C bus.
pub fn antenna_power(i2c: &mut I2cDriver<'_>) -> Result<()> {
    const ADDR: u8 = 0x43;
    // Direction: P0 output; high-impedance off for P0; P0 high.
    for (reg, val) in [(0x03u8, 0x01u8), (0x07, 0x00), (0x05, 0x01)] {
        i2c.write(ADDR, &[reg, val], 20).map_err(|e| anyhow!("antenna switch expander (0x43): {e}"))?;
    }
    Ok(())
}

/// The radio's settings (the RAK's P2P fields plus the sync word).
#[derive(Clone, Copy)]
struct Params {
    freq: u32,
    sf: u8,
    bw_khz: u16,
    /// 0 = 4/5 … 3 = 4/8 (RUI3's codes).
    cr: u8,
    preamble: u16,
    dbm: i8,
    sync: u16,
}

impl Params {
    fn describe(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{} sync {:04x}",
            self.freq, self.sf, self.bw_khz, self.cr, self.preamble, self.dbm, self.sync
        )
    }

    /// The SX126x bandwidth code.
    fn bw_code(&self) -> Option<u8> {
        Some(match self.bw_khz {
            125 => 0x04,
            250 => 0x05,
            500 => 0x06,
            62 => 0x03,
            41 => 0x0A,
            31 => 0x02,
            _ => return None,
        })
    }

    /// Low data rate optimisation, as RUI3 decides it (not by symbol time).
    fn ldro(&self) -> bool {
        (self.bw_khz == 125 && self.sf >= 11) || (self.bw_khz == 250 && self.sf == 12)
    }

    /// LoRa time on air for `len` bytes (SX126x datasheet 6.1.4).
    fn airtime(&self, len: usize) -> Duration {
        let sf = self.sf as f32;
        let t_sym = (1u32 << self.sf) as f32 / (self.bw_khz as f32 * 1000.0);
        let de = if self.ldro() { 1.0 } else { 0.0 };
        let payload = 8.0 * len as f32 - 4.0 * sf + 28.0 + 16.0; // CRC on, explicit header
        let n = 8.0 + ((payload / (4.0 * (sf - 2.0 * de))).ceil() * (self.cr as f32 + 5.0)).max(0.0);
        Duration::from_secs_f32((self.preamble as f32 + 4.25 + n) * t_sym)
    }
}

/// The SX1262 itself.
struct Radio {
    spi: SpiDeviceDriver<'static, SpiDriver<'static>>,
    busy: PinDriver<'static, Input>,
    dio1: PinDriver<'static, Input>,
    rst: PinDriver<'static, Output>,
    params: Params,
}

const IRQ_TX_DONE: u16 = 0x0001;
const IRQ_RX_DONE: u16 = 0x0002;
const IRQ_HEADER_ERR: u16 = 0x0020;
const IRQ_CRC_ERR: u16 = 0x0040;
const IRQ_TIMEOUT: u16 = 0x0200;

impl Radio {
    fn wait_busy(&self) -> Result<()> {
        let end = Instant::now() + Duration::from_millis(200);
        while self.busy.is_high() {
            if Instant::now() > end {
                bail!("SX1262 stays busy");
            }
            std::thread::yield_now();
        }
        Ok(())
    }

    /// A command without an answer.
    fn cmd(&mut self, bytes: &[u8]) -> Result<()> {
        self.wait_busy()?;
        self.spi.write(bytes)?;
        Ok(())
    }

    /// A command, then `n` answer bytes (after the status byte).
    fn read(&mut self, head: &[u8], n: usize) -> Result<Vec<u8>> {
        self.wait_busy()?;
        let mut out = head.to_vec();
        out.resize(head.len() + 1 + n, 0);
        let mut back = vec![0u8; out.len()];
        self.spi.transfer(&mut back, &out)?;
        Ok(back[head.len() + 1..].to_vec())
    }

    fn write_reg(&mut self, addr: u16, data: &[u8]) -> Result<()> {
        let mut b = vec![0x0D, (addr >> 8) as u8, addr as u8];
        b.extend_from_slice(data);
        self.cmd(&b)
    }

    fn read_reg(&mut self, addr: u16) -> Result<u8> {
        Ok(self.read(&[0x1D, (addr >> 8) as u8, addr as u8], 1)?[0])
    }

    fn reset(&mut self) -> Result<()> {
        self.rst.set_low()?;
        std::thread::sleep(Duration::from_millis(2));
        self.rst.set_high()?;
        std::thread::sleep(Duration::from_millis(10));
        self.wait_busy()
    }

    /// GetStatus: the chip mode bits (6:4); 0x2 = STDBY_RC.
    fn status(&mut self) -> Result<u8> {
        self.wait_busy()?;
        let mut back = [0u8; 2];
        self.spi.transfer(&mut back, &[0xC0, 0x00])?;
        Ok(back[1])
    }

    fn standby(&mut self) -> Result<()> {
        self.cmd(&[0x80, 0x00])
    }

    /// Everything from reset to "ready to receive".
    fn init(&mut self) -> Result<()> {
        self.reset()?;
        self.standby()?;
        let st = self.status()?;
        if st == 0x00 || st == 0xFF || (st >> 4) & 0x7 != 0x2 {
            bail!("no SX1262 answering (status {st:#04x})");
        }
        // TCXO from DIO3 at 3.0 V, 5 ms to start (320 × 15.625 µs).
        self.cmd(&[0x97, 0x06, 0x00, 0x01, 0x40])?;
        // Calibrate everything with the TCXO running.
        self.cmd(&[0x89, 0x7F])?;
        std::thread::sleep(Duration::from_millis(5));
        self.wait_busy()?;
        self.cmd(&[0x96, 0x01])?; // DC-DC regulator
        self.cmd(&[0x9D, 0x01])?; // DIO2 drives the RF switch
        self.cmd(&[0x8A, 0x01])?; // packet type LoRa
        self.cmd(&[0x8F, 0x00, 0x00])?; // TX and RX buffers from 0
        // Over-current protection 140 mA (as M5), boosted RX gain.
        self.write_reg(0x08E7, &[0x38])?;
        self.write_reg(0x08AC, &[0x96])?;
        // TX done, RX done, header/CRC error, timeout on DIO1.
        let mask = IRQ_TX_DONE | IRQ_RX_DONE | IRQ_HEADER_ERR | IRQ_CRC_ERR | IRQ_TIMEOUT;
        self.cmd(&[0x08, (mask >> 8) as u8, mask as u8, (mask >> 8) as u8, mask as u8, 0, 0, 0, 0])?;
        self.apply()
    }

    /// The settings in `params` onto the chip (from standby).
    fn apply(&mut self) -> Result<()> {
        let p = self.params;
        let bw = p.bw_code().ok_or_else(|| anyhow!("bandwidth {} kHz not supported", p.bw_khz))?;
        // Image calibration for the band, then the frequency.
        let band: [u8; 2] = match p.freq {
            f if f >= 902_000_000 => [0xE1, 0xE9],
            f if f >= 863_000_000 => [0xD7, 0xDB],
            f if f >= 779_000_000 => [0xC1, 0xC5],
            f if f >= 470_000_000 => [0x75, 0x81],
            _ => [0x6B, 0x6F],
        };
        self.cmd(&[0x98, band[0], band[1]])?;
        let reg = ((p.freq as u64) << 25) / 32_000_000;
        self.cmd(&[0x86, (reg >> 24) as u8, (reg >> 16) as u8, (reg >> 8) as u8, reg as u8])?;
        // PA for up to +22 dBm on the SX1262, then the power, 200 µs ramp.
        self.cmd(&[0x95, 0x04, 0x07, 0x00, 0x01])?;
        self.cmd(&[0x8E, p.dbm.clamp(-9, 22) as u8, 0x04])?;
        self.cmd(&[0x8B, p.sf, bw, p.cr + 1, u8::from(p.ldro())])?;
        self.packet_params(0xFF)?;
        self.write_reg(0x0740, &[(p.sync >> 8) as u8, p.sync as u8])?;
        // Errata 15.4: standard IQ wants bit 2 of 0x0736 set.
        let iq = self.read_reg(0x0736)?;
        self.write_reg(0x0736, &[iq | 0x04])?;
        Ok(())
    }

    fn packet_params(&mut self, len: u8) -> Result<()> {
        let pre = self.params.preamble;
        // Explicit header, CRC on, standard IQ.
        self.cmd(&[0x8C, (pre >> 8) as u8, pre as u8, 0x00, len, 0x01, 0x00])
    }

    fn irq(&mut self) -> Result<u16> {
        let b = self.read(&[0x12], 2)?;
        Ok(u16::from(b[0]) << 8 | u16::from(b[1]))
    }

    fn clear_irq(&mut self) -> Result<()> {
        self.cmd(&[0x02, 0xFF, 0xFF])
    }

    /// Continuous receive.
    fn receive(&mut self) -> Result<()> {
        self.packet_params(0xFF)?;
        self.clear_irq()?;
        self.cmd(&[0x82, 0xFF, 0xFF, 0xFF])
    }

    /// One packet out (from any state); how long it took on air.
    fn transmit(&mut self, data: &[u8]) -> Result<Duration> {
        if data.len() > 255 {
            bail!("{} B: at most 255 per packet", data.len());
        }
        self.standby()?;
        let mut b = vec![0x0E, 0x00];
        b.extend_from_slice(data);
        self.cmd(&b)?;
        self.packet_params(data.len() as u8)?;
        // Errata 15.1: below 500 kHz, bit 2 of 0x0889 set before TX.
        let v = self.read_reg(0x0889)?;
        let v = if self.params.bw_khz == 500 { v & !0x04 } else { v | 0x04 };
        self.write_reg(0x0889, &[v])?;
        self.clear_irq()?;
        let started = Instant::now();
        self.cmd(&[0x83, 0x00, 0x00, 0x00])?;
        let limit = self.params.airtime(data.len()) * 2 + Duration::from_millis(500);
        loop {
            if self.dio1.is_high() {
                let irq = self.irq()?;
                self.clear_irq()?;
                if irq & IRQ_TX_DONE != 0 {
                    return Ok(started.elapsed());
                }
                if irq & IRQ_TIMEOUT != 0 {
                    bail!("TX timeout");
                }
            }
            if started.elapsed() > limit {
                self.standby()?;
                bail!("no TX done after {} ms", limit.as_millis());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A packet if one came in: bytes, RSSI, SNR. Bad CRCs are dropped.
    fn poll(&mut self) -> Result<Option<(Vec<u8>, i16, f32)>> {
        if !self.dio1.is_high() {
            return Ok(None);
        }
        let irq = self.irq()?;
        self.clear_irq()?;
        if irq & IRQ_RX_DONE == 0 || irq & (IRQ_CRC_ERR | IRQ_HEADER_ERR) != 0 {
            return Ok(None);
        }
        let st = self.read(&[0x13], 2)?;
        let (len, start) = (st[0] as usize, st[1]);
        let data = self.read(&[0x1E, start], len)?;
        let pk = self.read(&[0x14], 3)?;
        let rssi = -(i16::from(pk[0]) / 2);
        let snr = f32::from(pk[1] as i8) / 4.0;
        Ok(Some((data, rssi, snr)))
    }
}

/// Bring the cap up and start the worker; `Err` when no SX1262 answers.
#[allow(clippy::too_many_arguments)]
pub fn start<S: SpiAnyPins + 'static>(
    spi: S,
    sck: impl OutputPin + 'static,
    mosi: impl OutputPin + 'static,
    miso: impl InputPin + 'static,
    nss: impl OutputPin + 'static,
    rst: impl OutputPin + 'static,
    busy: impl InputPin + 'static,
    dio1: impl InputPin + 'static,
    name: &'static str,
    out: Out,
    log: Sender<Event>,
) -> Result<Rak> {
    let driver = SpiDriver::new(spi, sck, mosi, Some(miso), &config::DriverConfig::new())?;
    let spi = SpiDeviceDriver::new(
        driver,
        Some(nss),
        &config::Config::new().baudrate(4.MHz().into()).data_mode(config::MODE_0),
    )?;
    let (f, sf, bw, cr, pre, dbm) = P2P_DEFAULT;
    let mut radio = Radio {
        spi,
        busy: PinDriver::input(busy, Pull::Down)?,
        dio1: PinDriver::input(dio1, Pull::Down)?,
        rst: PinDriver::output(rst)?,
        params: Params { freq: f, sf, bw_khz: bw, cr, preamble: pre, dbm: dbm as i8, sync: SYNC_WORD },
    };
    radio.init()?;
    let found = Found { version: "Cap LoRa-1262".into(), mode: "P2P".into() };
    let (tx, rx) = std::sync::mpsc::channel();
    let state = Arc::new(Mutex::new(State::default()));
    let worker = Worker {
        radio,
        name,
        out,
        rx,
        state: state.clone(),
        mode: Mode::Idle,
        rx_on: false,
        seq: 0,
        waiting: None,
        ping: None,
        log,
        admin_id: None,
        relay: None,
    };
    std::thread::Builder::new().name("lora".into()).stack_size(8 * 1024).spawn(move || worker.run())?;
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
    radio: Radio,
    name: &'static str,
    out: Out,
    rx: Receiver<Cmd>,
    state: Arc<Mutex<State>>,
    mode: Mode,
    rx_on: bool,
    seq: u8,
    waiting: Option<Waiting>,
    ping: Option<PingRun>,
    log: Sender<Event>,
    admin_id: Option<i64>,
    relay: Option<Sender<RelayFrame>>,
}

impl Worker {
    fn run(mut self) {
        self.say(Tone::Dim, format!("LoRa SX1262 ready: {} (Hz:SF:BW:CR:preamble:dBm)", self.radio.params.describe()));
        loop {
            match self.rx.recv_timeout(Duration::from_millis(5)) {
                Ok(cmd) => self.command(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if self.rx_on {
                match self.radio.poll() {
                    Ok(Some((data, rssi, snr))) => self.packet(rssi, snr, data),
                    Ok(None) => {}
                    Err(e) => self.say(Tone::Err, format!("lora rx: {e:#}")),
                }
            }
            self.timeouts();
            self.next_ping();
        }
    }

    fn say(&self, tone: Tone, text: String) {
        say(&self.out, tone, text);
    }

    fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Status => {
                let st = self.state.lock().map(|s| *s).unwrap_or_default();
                self.say(
                    Tone::Normal,
                    format!(
                        "LoRa SX1262 {} — P2P {} (Hz:SF:BW:CR:preamble:dBm), rx {}",
                        self.mode.name(),
                        self.radio.params.describe(),
                        st.received
                    ),
                );
            }
            Cmd::At(line) => self.at(&line),
            Cmd::Mode(mode) => {
                self.mode = mode;
                if let Ok(mut st) = self.state.lock() {
                    st.mode = mode;
                }
                if mode == Mode::Idle {
                    self.ping = None;
                }
                self.settle();
                let what = match mode {
                    Mode::Idle => "LoRa idle (receive off unless the relay runs)",
                    Mode::Listen => "LoRa listening (receive only)",
                    Mode::Server => "LoRa server: answering messages and pings",
                };
                self.say(Tone::Ok, what.into());
            }
            Cmd::SendRaw(bytes) => {
                if self.air(&bytes).is_none() {
                    self.say(Tone::Warn, format!("lora: relay frame of {} B not sent", bytes.len()));
                }
                self.settle_after_tx();
            }
            Cmd::RelaySink(sink) => {
                self.relay = Some(sink);
                self.settle();
            }
            Cmd::SendAdmin(id, text) => {
                self.admin_id = Some(id);
                self.command(Cmd::Send(text));
                self.admin_id = None;
            }
            Cmd::Send(text) => {
                self.seq = self.seq.wrapping_add(1);
                let frame = format!("T5 MSG {} {} {}", self.seq, self.name, text);
                if self.transmit(&frame) {
                    self.waiting = Some(Waiting { seq: self.seq, ping: false, sent: Instant::now() });
                }
                self.settle_after_tx();
            }
            Cmd::Ping(count) => {
                self.ping = Some(PingRun {
                    left: count.max(1),
                    next: Instant::now(),
                    sent: 0,
                    answered: 0,
                    rtt_total: Duration::ZERO,
                });
                self.say(Tone::Dim, format!("LoRa ping: {} × every {} s", count.max(1), PING_EVERY.as_secs()));
            }
        }
    }

    /// The RAK's AT commands that make sense here: the P2P settings and the
    /// sync word, to match a module set differently.
    fn at(&mut self, line: &str) {
        let upper = line.to_ascii_uppercase();
        let mut p = self.radio.params;
        let reply = if upper == "AT" {
            Ok(None)
        } else if upper == "AT+P2P=?" {
            Ok(Some(format!("AT+P2P={}:{}:{}:{}:{}:{}", p.freq, p.sf, p.bw_khz, p.cr, p.preamble, p.dbm)))
        } else if upper == "AT+SYNCWORD=?" {
            Ok(Some(format!("AT+SYNCWORD={:04X}", p.sync)))
        } else if upper == "AT+VER=?" {
            Ok(Some("AT+VER=SX1262 on the Cap LoRa-1262 (no RUI3)".into()))
        } else if let Some(v) = upper.strip_prefix("AT+P2P=") {
            let f: Vec<&str> = v.split(':').collect();
            let num = |i: usize| f.get(i).and_then(|s| s.parse::<i64>().ok());
            match (num(0), num(1), num(2), num(3), num(4), num(5)) {
                (Some(freq), Some(sf), Some(bw), Some(cr), Some(pre), Some(dbm)) => {
                    // RUI3 takes the BW as an index (0/1/2) or in kHz.
                    p.freq = freq as u32;
                    p.sf = sf as u8;
                    p.bw_khz = match bw {
                        0 => 125,
                        1 => 250,
                        2 => 500,
                        b => b as u16,
                    };
                    p.cr = cr as u8;
                    p.preamble = pre as u16;
                    p.dbm = dbm as i8;
                    self.set_params(p).map(|_| None)
                }
                _ => Err("AT+P2P=<Hz>:<SF>:<BW>:<CR>:<preamble>:<dBm>".to_string()),
            }
        } else if let Some(v) = upper.strip_prefix("AT+SYNCWORD=") {
            match u16::from_str_radix(v.trim(), 16) {
                Ok(sync) => {
                    p.sync = sync;
                    self.set_params(p).map(|_| None)
                }
                Err(_) => Err("AT+SYNCWORD=<4 hex digits>, e.g. 1424".to_string()),
            }
        } else {
            Err(format!("{line}: not on the SX1262 (only AT+P2P, AT+SYNCWORD, AT+VER)"))
        };
        match reply {
            Ok(text) => {
                if let Some(t) = text {
                    self.say(Tone::Normal, format!("rak: {t}"));
                }
                self.say(Tone::Ok, "rak: OK".into());
            }
            Err(e) => self.say(Tone::Err, format!("rak: {e}")),
        }
    }

    fn set_params(&mut self, p: Params) -> Result<(), String> {
        let old = self.radio.params;
        self.radio.params = p;
        let result = self.radio.standby().and_then(|_| self.radio.apply());
        self.rx_on = false;
        if let Err(e) = result {
            self.radio.params = old;
            let _ = self.radio.apply();
            self.settle();
            return Err(format!("{e:#}"));
        }
        self.settle();
        Ok(())
    }

    fn listening(&self) -> bool {
        self.mode != Mode::Idle || self.waiting.is_some() || self.ping.is_some() || self.relay.is_some()
    }

    /// Receive on or off, as the mode wants it.
    fn settle(&mut self) {
        let want = self.listening();
        if want == self.rx_on {
            return;
        }
        let r = if want { self.radio.receive() } else { self.radio.standby() };
        match r {
            Ok(()) => self.rx_on = want,
            Err(e) => self.say(Tone::Err, format!("lora: {e:#}")),
        }
    }

    /// After a transmission the chip is in standby.
    fn settle_after_tx(&mut self) {
        self.rx_on = false;
        self.settle();
    }

    fn air(&mut self, bytes: &[u8]) -> Option<Duration> {
        match self.radio.transmit(bytes) {
            Ok(took) => Some(took),
            Err(e) => {
                self.say(Tone::Err, format!("lora tx: {e:#}"));
                None
            }
        }
    }

    fn transmit(&mut self, frame: &str) -> bool {
        let Some(took) = self.air(frame.as_bytes()) else {
            return false;
        };
        self.rx_on = false;
        self.say(Tone::Tx, format!("tx {frame}  ({} ms)", took.as_millis()));
        let mut event = frame_event("tx", frame);
        if event.kind == "ACK" || event.kind == "PONG" {
            event.rssi = event.peer_rssi.take();
            event.snr = event.peer_snr.take();
        }
        event.admin_id = self.admin_id.take();
        let _ = self.log.send(event);
        true
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
        let mut fields = text.splitn(5, ' ');
        let (magic, kind, seq, from, rest) =
            (fields.next(), fields.next(), fields.next(), fields.next(), fields.next().unwrap_or(""));
        let seq: Option<u8> = seq.and_then(|s| s.parse().ok());
        let (Some("T5"), Some(kind), Some(seq), Some(from)) = (magic, kind, seq, from) else {
            let shown = match std::str::from_utf8(&data) {
                Ok(s) if s.chars().all(|c| !c.is_control()) => format!("\"{s}\""),
                _ => data.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "),
            };
            self.say(Tone::Rx, format!("rx {} bytes: {shown}  ({here})", data.len()));
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
                    self.settle_after_tx();
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
                        self.say(Tone::Dim, format!("rx {} #{seq} from {from} (not ours)  ({here})", kind.to_lowercase()));
                    }
                }
                self.settle();
            }
            _ => self.say(Tone::Rx, format!("rx \"{text}\"  ({here})")),
        }
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
                self.say(Tone::Warn, format!("no {what} for #{} within {} s", w.seq, ANSWER_WAIT.as_secs()));
                self.waiting = None;
                self.settle();
            }
        }
    }

    fn next_ping(&mut self) {
        let Some(run) = self.ping.as_mut() else { return };
        if self.waiting.is_some() || Instant::now() < run.next {
            return;
        }
        if run.left == 0 {
            let run = self.ping.take().expect("ping run");
            let avg = if run.answered > 0 { run.rtt_total.as_millis() / run.answered as u128 } else { 0 };
            let lost = run.sent - run.answered;
            self.say(
                Tone::Normal,
                format!("ping: {} sent, {} answered, {lost} lost, avg {avg} ms", run.sent, run.answered),
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
            self.waiting = Some(Waiting { seq: self.seq, ping: true, sent: Instant::now() });
        }
        self.settle_after_tx();
    }
}

/// A `T5 …` frame (or anything else) as an event for the admin page.
fn frame_event(dir: &'static str, text: &str) -> Event {
    let mut f = text.splitn(5, ' ');
    let (magic, kind, seq, from, rest) = (f.next(), f.next(), f.next(), f.next(), f.next().unwrap_or(""));
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
        "ACK" | "PONG" => {
            let mut v = rest.split(' ');
            event.peer_rssi = v.next().and_then(|r| r.parse().ok());
            event.peer_snr = v.next().and_then(|r| r.parse().ok());
        }
        _ => event.text = rest.to_string(),
    }
    event
}
