//! Kitchen display on the M5Stack Cardputer ADV (ESP32-S3, 240×135, a
//! 56-key keyboard, the LoRa cap): its own screen and drivers here in
//! src/cardputer/; the live feed, Wi-Fi, updates and (soon) the LoRa relay
//! are the Tab5's, shared by path and unchanged.
//!
//! A network thread joins Wi-Fi (the shop's networks from
//! tmp/tab5-wifi.env), sets the clock, then starts the live feed; the screen
//! runs in its own thread with the keyboard. The LoRa cap (lora.rs, in the
//! guise of the Tab5's RAK3172 module) answers from the start and relays the
//! takes the board from a Tab5 when it has no network (it never relays: a
//! Tab5 does, preferably one on a LAN cable); the Terminal page runs the
//! Tab5's shell.

mod battery;
mod board;
mod display;
mod keyboard;
mod ui;
// The SX1262 under the name the shared code knows the radio by.
#[path = "lora.rs"]
mod rak;
#[path = "lan_stub.rs"]
mod lan;

// Shared with the Tab5 (src/), unchanged.
#[allow(dead_code)]
#[path = "../crashlog.rs"]
mod crashlog;
#[allow(dead_code)]
#[path = "../feed.rs"]
mod feed;
#[allow(dead_code)]
#[path = "../net.rs"]
mod net;
#[allow(dead_code)]
#[path = "../ota.rs"]
mod ota;
#[allow(dead_code)]
#[path = "../wifi.rs"]
mod wifi;
#[allow(dead_code)]
#[path = "../relay.rs"]
mod relay;
// The Tab5's terminal and shell (tab5-lab), and the LoRa log for /admin/lora.
#[allow(dead_code)]
#[path = "../../../tab5-lab/src/terminal.rs"]
mod terminal;
#[allow(dead_code)]
#[path = "../../../tab5-lab/src/shell.rs"]
mod shell;
#[allow(dead_code)]
#[path = "../../../tab5-lab/src/report.rs"]
mod report;

/// Output lines for the terminal, from any thread.
pub type Out = std::sync::mpsc::Sender<(terminal::Tone, String)>;

/// A line for the terminal; in the serial log too (target "term", which the
/// log tee doesn't copy back).
pub fn say(out: &Out, tone: terminal::Tone, text: String) {
    log::info!(target: "term", "{text}");
    let _ = out.send((tone, text));
}

/// Shell commands the kitchen firmware adds, for `help`.
const HOST_HELP: &str = "\
  bat                       battery: voltage and charge
  wifi                      Wi-Fi status
  wifi off / wifi on        switch Wi-Fi off (until restart) / on again
  relay                     LoRa relay: where the board comes from when
                            there's no network (the Cardputer only takes it,
                            it never relays)";

use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::units::FromValueType;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use terminal::{Terminal, Tone};
use slint::platform::{Platform, WindowAdapter};
use slint::PhysicalSize;

slint::include_modules!();

/// The UI thread's stack (Slint's renderer goes deep).
const UI_STACK: usize = 40 * 1024;
/// The shop's local time (POSIX TZ for Europe/Berlin).
const TZ: &str = "CET-1CEST,M3.5.0,M10.5.0/3";

struct CardPlatform {
    window: Rc<MinimalSoftwareWindow>,
    start: Instant,
}

impl Platform for CardPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}

fn main() -> Result<()> {
    sys::link_patches();
    // The log, with a copy of the interesting lines in the terminal.
    let (out, term_lines) = mpsc::channel::<(Tone, String)>();
    init_logging(out.clone());
    let name = board::device_name();
    log::info!("{name} {}", ota::VERSION);
    log_memory("at start");
    // A fresh update has to confirm itself (once the feed runs), see ota.rs.
    ota::init();
    std::env::set_var("TZ", TZ);
    // SAFETY: re-reads TZ for localtime().
    unsafe { sys::tzset() };

    let p = Peripherals::take()?;
    let display = display::Display::new(
        p.spi3,
        p.pins.gpio36,
        p.pins.gpio35,
        p.pins.gpio37,
        p.pins.gpio34,
        p.pins.gpio33,
        p.pins.gpio38,
    )?;
    // The microSD card's select line: high, so a card never answers on the
    // SPI lines it shares with the LoRa cap. Kept for the program's life.
    let mut sd_cs = PinDriver::output(p.pins.gpio12)?;
    sd_cs.set_high()?;
    core::mem::forget(sd_cs);
    let mut i2c = I2cDriver::new(p.i2c0, p.pins.gpio8, p.pins.gpio9, &I2cConfig::new().baudrate(400.kHz().into()))?;
    // The LoRa cap's antenna switch needs its power from the cap's expander
    // — set once, before the keyboard takes the bus for itself.
    let antenna = rak::antenna_power(&mut i2c);
    if let Err(e) = &antenna {
        say(&out, Tone::Warn, format!("LoRa: {e:#} (no cap?)"));
    }
    let keyboard = keyboard::Keyboard::new(i2c).map_err(|e| log::error!("keyboard: {e:#}")).ok();

    if let Err(e) = battery::start(p.adc1, p.pins.gpio10) {
        log::warn!("battery: {e:#}");
    }

    let nvs = EspDefaultNvsPartition::take()?;
    if let Some(note) = crashlog::init(nvs.clone()) {
        log::warn!("{note}");
    }

    let status = Arc::new(Mutex::new(String::from("WLAN: verbinde …")));
    let (ui_tx, ui_rx) = mpsc::channel();

    // The LoRa cap; its traffic also goes to /admin/lora.
    let (lora_log, lora_events) = report::channel();
    let rak = if antenna.is_ok() {
        rak::start(
            p.spi2,
            p.pins.gpio40,
            p.pins.gpio14,
            p.pins.gpio39,
            p.pins.gpio5,
            p.pins.gpio3,
            p.pins.gpio6,
            p.pins.gpio4,
            name,
            out.clone(),
            lora_log,
        )
        .map_err(|e| say(&out, Tone::Err, format!("LoRa: {e:#}")))
        .ok()
    } else {
        None
    };
    let mut term = Terminal::default();
    let info = vec![
        format!("{name} {}", ota::VERSION),
        rak.as_ref().map(|r| r.describe()).unwrap_or_else(|| "LoRa: no SX1262 cap found".into()),
    ];
    let mut shell = shell::Shell::new(out.clone(), None, rak.clone(), info).with_host_commands(
        HOST_HELP,
        Box::new(|words, term| {
            battery::command(words, term) || wifi::command(words, term) || relay::command(words, term)
        }),
    );
    // LoRa answers from the start (as the Tab5s), and relays the board.
    // The LoRa log (/admin/lora) and the relay talk to the server: they
    // start once Wi-Fi has set up the network stack — before it, lwIP
    // asserts ("Invalid mbox"). Without any Wi-Fi the relay still runs.
    if let Some(rak) = rak.clone() {
        shell.submit("rak server".into(), &mut term);
        let (out, ui) = (out.clone(), ui_tx.clone());
        std::thread::Builder::new().name("lora-up".into()).stack_size(8 * 1024).spawn(move || {
            let since = Instant::now();
            // SAFETY: read-only count of the network interfaces.
            while unsafe { sys::esp_netif_get_nr_of_ifs() } == 0 && since.elapsed() < Duration::from_secs(60) {
                std::thread::sleep(Duration::from_millis(200));
            }
            if let Err(e) = report::start(name, rak.clone(), out.clone(), lora_events) {
                say(&out, Tone::Err, format!("LoRa log: {e:#}"));
            }
            relay::start(name, rak, ui, out);
        })?;
    }
    let lab = ui::Lab { shell, term, lines: term_lines, rak };
    let (tap_tx, tap_rx) = mpsc::channel();
    let demo = feed::PASSWORD.is_none();
    let sysloop = EspSystemEventLoop::take()?;
    let net_status = status.clone();
    let feed_nvs = nvs.clone();
    std::thread::Builder::new().name("net".into()).stack_size(16 * 1024).spawn(move || {
        let feed_status = net_status.clone();
        net::run(p.modem, sysloop, nvs, net_status, move || {
            if demo {
                return;
            }
            match feed::start(ui_tx, tap_rx, feed_status, feed_nvs) {
                // Online with the new image: keep it (ota.rs).
                Ok(_session) => ota::confirm(),
                Err(e) => log::error!("feed: {e:#}"),
            }
        })
    })?;

    std::thread::Builder::new().name("ui".into()).stack_size(UI_STACK).spawn(move || {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        window.set_size(PhysicalSize::new(display::WIDTH as u32, display::HEIGHT as u32));
        let platform = CardPlatform { window: window.clone(), start: Instant::now() };
        if let Err(e) = slint::platform::set_platform(Box::new(platform)) {
            log::error!("ui: platform: {e:?}");
            return;
        }
        log_memory("ui up");
        if let Err(e) = ui::run(window, display, keyboard, status, ui_rx, tap_tx, demo, lab) {
            log::error!("ui: {e:#}");
        }
        // The UI never returns unless it can't run: start again.
        // SAFETY: plain restart.
        unsafe { sys::esp_restart() };
    })?;
    // The main task ends here; its stack goes back to the heap.
    Ok(())
}

static ESP: esp_idf_svc::log::EspLogger =
    esp_idf_svc::log::EspLogger::new(esp_idf_svc::log::EspIdfLogFilter::new());
static TEE_OUT: std::sync::OnceLock<Out> = std::sync::OnceLock::new();

/// The ESP-IDF logger, plus a copy of the interesting lines into the
/// terminal: warnings and errors from everything, this firmware's own info
/// lines (as the Tab5's lab.rs).
struct Tee;

impl log::Log for Tee {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        log::Log::enabled(&ESP, metadata)
    }

    fn log(&self, record: &log::Record) {
        log::Log::log(&ESP, record);
        let target = record.target();
        if target == "term" {
            return;
        }
        let tone = match record.level() {
            log::Level::Error => Tone::Err,
            log::Level::Warn => Tone::Warn,
            log::Level::Info if target.starts_with("kds_cardputer") && !target.ends_with("::ui") => Tone::Dim,
            _ => return,
        };
        if let Some(out) = TEE_OUT.get() {
            let _ = out.send((tone, format!("{}", record.args())));
        }
    }

    fn flush(&self) {}
}

static TEE: Tee = Tee;

fn init_logging(out: Out) {
    let _ = TEE_OUT.set(out);
    if log::set_logger(&TEE).is_ok() {
        ESP.filter().initialize();
    }
}

fn free_kb() -> usize {
    // SAFETY: read-only heap statistics.
    unsafe { sys::heap_caps_get_free_size(sys::MALLOC_CAP_INTERNAL) / 1024 }
}

fn log_memory(when: &str) {
    // SAFETY: read-only heap statistics.
    let largest = unsafe { sys::heap_caps_get_largest_free_block(sys::MALLOC_CAP_INTERNAL) / 1024 };
    log::info!("RAM {when}: free {} KB, largest block {largest} KB", free_kb());
}
