//! The "Terminal" page (Tab5 only): the lab firmware's features inside the
//! kitchen firmware. The modules are tab5-lab's own (crates/tab5-lab/src,
//! included by path in main.rs); this file starts them and connects them
//! to the kitchen screen.
//!
//! - A text terminal with a shell: the Tab5 Keyboard (EXT header) types
//!   into it, and so does the phone (Tab5 Terminal app over Bluetooth LE,
//!   with the shared `LAB_KEY`). The device log shows there too.
//! - Modules on the M5-Bus, found at start: Module13.2 LAN (W5500, an
//!   extra network: `lan`, `ping`, `get`, `tcp`) and Module LoRaWAN-EU868
//!   (RAK3172: LoRa chat/ping, `rak …`), whose traffic also goes to
//!   /admin/lora.
//!
//! A Tab5 without modules has the terminal and the phone link only. All
//! of it starts in a background thread, so the kitchen screen comes up as
//! fast as without it; the UI loop takes the result ([`Lab`]) when ready
//! and only then shows the tab.

use std::sync::mpsc::{self, Receiver};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::{Gpio18, Gpio19, Gpio5};
use esp_idf_svc::hal::spi::SPI2;
use esp_idf_svc::log::{EspIdfLogFilter, EspLogger};
use kds_ui::{KitchenWindow, TermLine};
use slint::{ModelRc, VecModel};

use crate::keyboard::Keyboard;
use crate::terminal::{Terminal, Tone};
use crate::{bus, lan, rak, remote, report, say, shell, Out};

/// Bluetooth waits at most this long for Wi-Fi to bring the C6 link up.
const C6_WAIT: Duration = Duration::from_secs(30);
/// Lines handed to the page: more than fit, the page clips at the top.
const SHOWN_LINES: usize = 40;
/// The screen faces the keyboard: it attaches at the edge the kitchen
/// screen has at the top in its normal position.
pub const KEYBOARD_SIDE: bool = true;

pub struct Lab {
    term: Terminal,
    shell: shell::Shell,
    keyboard: Option<Keyboard>,
    remote: Option<remote::Remote>,
    lines: Receiver<(Tone, String)>,
    lan: Option<lan::Lan>,
    rak: Option<rak::Rak>,
    last_status: Instant,
    /// "the phone finds this Tab5 as …" said once.
    ble_announced: bool,
    name: &'static str,
}

/// The hardware for the modules: the M5-Bus SPI.
pub struct BusParts {
    pub spi2: SPI2<'static>,
    pub sck: Gpio5<'static>,
    pub mosi: Gpio18<'static>,
    pub miso: Gpio19<'static>,
}

/// Start everything in a background thread; the [`Lab`] arrives on the
/// returned channel. `out`/`lines`: the terminal's output channel (the log
/// tee writes into it too, see [`init_logging`]).
pub fn spawn(
    name: &'static str,
    bus_parts: BusParts,
    sysloop: EspSystemEventLoop,
    out: Out,
    lines: Receiver<(Tone, String)>,
    ui: std::sync::mpsc::Sender<crate::ui::UiMsg>,
) -> Result<Receiver<Lab>> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("lab-start".into())
        .stack_size(16 * 1024)
        .spawn(move || {
            // The threads this one starts (LAN, LoRa, LoRa reports, the
            // phone's sender) get their stacks from PSRAM: internal RAM is
            // for TLS and the drivers. None of them writes flash, which a
            // PSRAM stack would not survive (cache off during the write).
            // SAFETY: plain config struct; the pthread layer copies it.
            unsafe {
                let mut cfg = esp_idf_svc::sys::esp_pthread_get_default_config();
                cfg.stack_alloc_caps =
                    esp_idf_svc::sys::MALLOC_CAP_SPIRAM | esp_idf_svc::sys::MALLOC_CAP_8BIT;
                cfg.inherit_cfg = true;
                let _ = esp_idf_svc::sys::esp_pthread_set_cfg(&cfg);
            }
            match start(name, bus_parts, sysloop, out, lines, ui) {
                Ok(lab) => {
                    let _ = tx.send(lab);
                }
                Err(e) => {
                    log::error!("lab: {e:#}");
                    // No bus, no LAN: net.rs needn't wait for it.
                    crate::net::LAN_MODULE
                        .compare_exchange(0, crate::net::LAN_NONE, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed)
                        .ok();
                }
            }
        })?;
    Ok(rx)
}

/// `help` lines for the kitchen firmware's own commands (wifi.rs,
/// relay.rs).
const HOST_HELP: &str = "\
  wifi                      Wi-Fi status
  wifi off / wifi on        switch Wi-Fi off (until restart) / on again
  relay                     LoRa relay: state, devices heard
  relay auto / on / off     pass the board to a device without network:
                            when the server confirms it (default) / always /
                            never
  eink                      colour e-ink frame: address, last picture sent
  eink push                 fetch the Tagesangebot and send it to the frame now";

fn start(
    name: &'static str,
    p: BusParts,
    sysloop: EspSystemEventLoop,
    out: Out,
    lines: Receiver<(Tone, String)>,
    ui: std::sync::mpsc::Sender<crate::ui::UiMsg>,
) -> Result<Lab> {
    let mut info = vec![format!("{name}: kitchen firmware {}", crate::ota::VERSION)];
    if let Err(e) = bus::ext5v_on() {
        say(&out, Tone::Err, format!("bus 5 V: {e:#}"));
    }
    let keyboard = Keyboard::open(out.clone())
        .map_err(|e| say(&out, Tone::Err, format!("keyboard bus: {e:#}")))
        .ok();
    info.push("keyboard: Tab5 Keyboard on the EXT header, I2C 0x6D, GPIO0/1, INT GPIO50".into());

    let spi = bus::spi(p.spi2, p.sck, p.mosi, p.miso)?;
    let mut found = bus::probe(spi)?;
    // The UART module after the LAN module, keeping off its pins.
    let busy: Vec<i32> = found
        .w5500
        .iter()
        .flat_map(|c| [Some(c.cs), c.rst])
        .flatten()
        .collect();
    let rak_found = rak::probe(&busy, &mut found.notes, &mut found.raw);
    for note in &found.notes {
        say(&out, Tone::Warn, format!("bus: {note}"));
    }
    for line in &found.raw {
        log::info!("lab: {line}");
    }
    // net.rs waits for this before Wi-Fi: the cable first.
    crate::net::LAN_MODULE.store(
        if found.w5500.is_some() { crate::net::LAN_W5500 } else { crate::net::LAN_NONE },
        std::sync::atomic::Ordering::Relaxed,
    );
    match found.w5500 {
        Some(c) => info.push(format!("LAN: W5500 (v{}) — {}", c.version, c.describe())),
        None => info.push("LAN: no W5500 on the M5-Bus".into()),
    }
    match &rak_found {
        Some(r) => info.push(r.describe()),
        None => info.push("LoRaWAN: no RAK3172 on the M5-Bus".into()),
    }
    for line in &info[1..] {
        say(&out, Tone::Dim, line.clone());
    }

    let lan = found.w5500.and_then(|chip| {
        lan::start(spi, chip, sysloop.clone(), out.clone())
            .map_err(|e| say(&out, Tone::Err, format!("LAN: {e:#}")))
            .ok()
    });
    // LoRa traffic also goes to /admin/lora, which can send through here.
    let (lora_log, lora_events) = report::channel();
    let rak = rak_found.and_then(|found| {
        rak::start(found, name, out.clone(), lora_log)
            .map_err(|e| say(&out, Tone::Err, format!("RAK3172: {e:#}")))
            .ok()
    });
    if let Some(rak) = &rak {
        if let Err(e) = report::start(name, rak.clone(), out.clone(), lora_events) {
            say(&out, Tone::Err, format!("LoRa log: {e:#}"));
        }
    }

    // The phone link through the C6 — only once Wi-Fi has brought the link
    // up: both setting it up at once breaks it (wifi.rs, C6_SETTLED).
    let waited = Instant::now();
    while !crate::wifi::C6_SETTLED.load(std::sync::atomic::Ordering::Relaxed)
        && waited.elapsed() < C6_WAIT
    {
        std::thread::sleep(Duration::from_millis(200));
    }
    let remote = remote::Remote::start(name)
        .map_err(|e| say(&out, Tone::Err, format!("bluetooth: {e:#}")))
        .ok();
    // The e-ink frame, over the same Bluetooth (only the cabled Tab5 sends).
    if let Err(e) = crate::eink::start() {
        say(&out, Tone::Err, format!("e-ink: {e:#}"));
    }

    let mut term = Terminal::default();
    let mut shell = shell::Shell::new(out.clone(), lan.clone(), rak.clone(), info)
        .with_host_commands(
            HOST_HELP,
            Box::new(|words, term| {
                crate::wifi::command(words, term)
                    || crate::relay::command(words, term)
                    || crate::eink::command(words, term)
            }),
        );
    // LoRa answers from the start, as in tab5-lab, and relays the board to
    // (or takes it from) a device without network.
    if let Some(rak) = &rak {
        shell.submit("rak server".into(), &mut term);
        crate::relay::start(name, rak.clone(), ui, out.clone());
    }
    Ok(Lab {
        term,
        shell,
        keyboard,
        remote,
        lines,
        lan,
        rak,
        last_status: Instant::now() - Duration::from_secs(1),
        ble_announced: false,
        name,
    })
}

/// What a tick brought.
#[derive(Default)]
pub struct Tick {
    /// Keys were typed (the screen wakes and shows the terminal).
    pub typed: bool,
}

impl Lab {
    /// Every UI loop: keys, output lines, the phone, the status line.
    pub fn tick(&mut self) -> Tick {
        let mut tick = Tick::default();
        if let Some(kb) = self.keyboard.as_mut() {
            for key in kb.poll() {
                tick.typed = true;
                self.shell.key(key, &mut self.term);
            }
        }
        while let Ok((tone, text)) = self.lines.try_recv() {
            self.term.push(tone, &text);
        }
        if !self.ble_announced && crate::ble::ready() {
            self.ble_announced = true;
            self.term.push(
                Tone::Dim,
                &format!(
                    "bluetooth: the phone finds this Tab5 as {} ({})",
                    self.name,
                    crate::ble::address()
                ),
            );
        }
        if let Some(r) = self.remote.as_mut() {
            r.poll(&mut self.shell, &mut self.term);
        }
        self.shell.tick(&mut self.term);
        if self.last_status.elapsed() >= Duration::from_secs(1) {
            self.last_status = Instant::now();
            let status = self.status_line();
            if status != self.term.status {
                self.term.status = status;
                self.term.touch();
            }
        }
        tick
    }

    /// A command from the page's buttons, as if typed.
    pub fn command(&mut self, line: &str) {
        self.shell.submit(line.to_string(), &mut self.term);
    }

    /// A Tab5 Keyboard is attached: the screen faces it.
    pub fn keyboard_on(&self) -> bool {
        self.keyboard.as_ref().is_some_and(|k| k.present())
    }

    /// `flip` typed: the screen the other way round than it would be.
    pub fn turned(&self) -> bool {
        self.shell.turned()
    }

    /// Put the terminal on the page (only when something changed).
    pub fn show(&mut self, w: &KitchenWindow) {
        if !self.term.is_dirty() {
            return;
        }
        let lines = self.term.lines();
        let skip = lines.len().saturating_sub(SHOWN_LINES);
        let model: Vec<TermLine> = lines
            .iter()
            .skip(skip)
            .map(|(tone, text)| TermLine {
                text: text.as_str().into(),
                tone: tone.index(),
            })
            .collect();
        w.set_term_lines(ModelRc::new(VecModel::from(model)));
        w.set_term_status(self.term.status.as_str().into());
        w.set_term_prompt(self.term.prompt.as_str().into());
        let (before, after) = self.term.input_split();
        w.set_term_before(before.into());
        w.set_term_after(after.into());
        self.term.drawn();
    }

    fn status_line(&self) -> String {
        let mut parts = Vec::new();
        parts.push(
            self.lan
                .as_ref()
                .map(|l| l.state().short())
                .unwrap_or_else(|| "no LAN module".into()),
        );
        parts.push(
            self.rak
                .as_ref()
                .map(|r| r.state().describe())
                .unwrap_or_else(|| "no LoRa module".into()),
        );
        parts.push(if self.remote.as_ref().is_some_and(|r| r.linked()) {
            "phone".into()
        } else if crate::ble::ready() {
            "BT ready".into()
        } else {
            "BT off".into()
        });
        parts.push(if self.keyboard_on() {
            "keyboard".into()
        } else {
            "no keyboard".into()
        });
        parts.join("  ·  ")
    }
}

// ----- the device log in the terminal -----

static ESP: EspLogger = EspLogger::new(EspIdfLogFilter::new());
static TEE_OUT: OnceLock<Out> = OnceLock::new();

/// The ESP-IDF logger, plus a copy of the interesting lines into the
/// terminal: warnings and errors from everything, the kitchen firmware's
/// own info lines (not the noisy touch and accelerometer ones).
struct Tee;

impl log::Log for Tee {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        log::Log::enabled(&ESP, metadata)
    }

    fn log(&self, record: &log::Record) {
        log::Log::log(&ESP, record);
        let target = record.target();
        // `say` lines are in the terminal already.
        if target == "term" {
            return;
        }
        let tone = match record.level() {
            log::Level::Error => Tone::Err,
            log::Level::Warn => Tone::Warn,
            log::Level::Info
                if target.starts_with("kds_tab5")
                    && !target.ends_with("::imu")
                    && !target.ends_with("::ui") =>
            {
                Tone::Dim
            }
            _ => return,
        };
        if let Some(out) = TEE_OUT.get() {
            let _ = out.send((tone, format!("{}", record.args())));
        }
    }

    fn flush(&self) {}
}

static TEE: Tee = Tee;

/// Install the logger (instead of `EspLogger::initialize_default`).
pub fn init_logging(out: Out) {
    let _ = TEE_OUT.set(out);
    if log::set_logger(&TEE).is_ok() {
        ESP.filter().initialize();
    }
}
