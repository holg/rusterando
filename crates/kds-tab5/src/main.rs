//! Kitchen display on the M5Stack Tab5 (ESP32-P4), or with feature
//! `jc1060p470` on the 7" Guition JC1060P470 (ESP32-P4, Ethernet).
//!
//! Board setup (I2C, expanders, Wi-Fi power, charging), the display, the USB
//! printer port, then the kitchen screen (kds-ui) on the main task. A network
//! thread (net.rs) gets online — Tab5: Wi-Fi through the ESP32-C6
//! coprocessor; JC1060P470: the LAN cable, plus the shop's Wi-Fi hotspot —
//! sets the clock via SNTP and keeps the connection up; once online it starts
//! the live feed (feed.rs) and the print channel (printing.rs), which prints
//! the kitchen's receipts while a USB printer is plugged in.

// Board-specific parts: the Tab5's in src/, the 7" JC1060P470's in
// src/jc1060p470/ (feature `jc1060p470`), with the same interfaces.
#[cfg_attr(feature = "jc1060p470", path = "jc1060p470/battery.rs")]
mod battery;
#[cfg_attr(feature = "jc1060p470", path = "jc1060p470/board.rs")]
mod board;
mod crashlog;
mod display;
mod feed;
mod gong;
#[cfg_attr(feature = "jc1060p470", path = "jc1060p470/imu.rs")]
mod imu;
#[cfg_attr(feature = "jc1060p470", path = "jc1060p470/net.rs")]
mod net;
mod ota;
mod printing;
mod ui;
mod usb_printer;
mod wifi;

// The "Terminal" page (Tab5 only): tab5-lab's modules, shared by path so
// both firmwares run the same code (lab.rs connects them to this screen).
// The 7" has no M5-Bus and keeps without them.
#[cfg(not(feature = "jc1060p470"))]
mod lab;
// The colour e-ink frame with the Tagesangebot, over Bluetooth LE through
// the C6 (the Tab5: the Terminal page's; the 7": started once online).
mod eink;
// The 7" only starts it (for the frame); the phone link's calls are Tab5's.
#[cfg_attr(feature = "jc1060p470", allow(dead_code))]
#[path = "../../tab5-lab/src/ble.rs"]
mod ble;
#[cfg(not(feature = "jc1060p470"))]
#[path = "../../tab5-lab/src/bus.rs"]
mod bus;
#[cfg(not(feature = "jc1060p470"))]
#[path = "../../tab5-lab/src/keyboard.rs"]
mod keyboard;
#[cfg(not(feature = "jc1060p470"))]
#[path = "../../tab5-lab/src/lan.rs"]
mod lan;
#[cfg(not(feature = "jc1060p470"))]
#[path = "../../tab5-lab/src/rak.rs"]
mod rak;
#[cfg(not(feature = "jc1060p470"))]
#[path = "../../tab5-lab/src/remote.rs"]
mod remote;
#[cfg(not(feature = "jc1060p470"))]
mod relay;
#[cfg(not(feature = "jc1060p470"))]
#[path = "../../tab5-lab/src/report.rs"]
mod report;
#[cfg(not(feature = "jc1060p470"))]
#[path = "../../tab5-lab/src/shell.rs"]
mod shell;
#[cfg(not(feature = "jc1060p470"))]
#[path = "../../tab5-lab/src/terminal.rs"]
mod terminal;

/// Output lines for the terminal, from any thread (the lab modules' `say`).
#[cfg(not(feature = "jc1060p470"))]
pub type Out = std::sync::mpsc::Sender<(terminal::Tone, String)>;

/// A line for the terminal; in the serial log too (target "term", which the
/// log tee in lab.rs doesn't copy back).
#[cfg(not(feature = "jc1060p470"))]
pub fn say(out: &Out, tone: terminal::Tone, text: String) {
    log::info!(target: "term", "{text}");
    let _ = out.send((tone, text));
}

use std::ffi::CStr;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;

/// The UI thread's stack (Slint's software renderer goes deep).
pub(crate) const UI_STACK: usize = 64 * 1024;

/// The shop's local time (POSIX TZ for Europe/Berlin).
const TZ: &str = "CET-1CEST,M3.5.0,M10.5.0/3";

/// `esp_chip_info_t` (esp_chip_info.h, ESP-IDF 5.4). Declared here because
/// esp-idf-sys doesn't generate bindings for that header; the function is
/// linked into every ESP-IDF image.
#[repr(C)]
#[derive(Default)]
struct ChipInfo {
    model: u32,
    features: u32,
    /// major * 100 + minor
    revision: u16,
    cores: u8,
}

extern "C" {
    fn esp_chip_info(out: *mut ChipInfo);
}

fn main() -> Result<()> {
    // Required once so the ESP-IDF runtime patches are linked in.
    sys::link_patches();
    // Tab5: the log also goes into the Terminal page.
    #[cfg(not(feature = "jc1060p470"))]
    let (term_out, term_lines) = mpsc::channel::<(terminal::Tone, String)>();
    #[cfg(not(feature = "jc1060p470"))]
    lab::init_logging(term_out.clone());
    #[cfg(feature = "jc1060p470")]
    esp_idf_svc::log::EspLogger::initialize_default();
    log_chip_info();
    log_internal_ram("at start");
    // A fresh update has to confirm itself (print channel), see ota.rs.
    ota::init();
    std::env::set_var("TZ", TZ);
    // SAFETY: re-reads TZ for localtime().
    unsafe { sys::tzset() };

    // Before Wi-Fi: the board package resets the expander that powers the C6.
    board::init()?;
    let display = display::Display::new()?;

    let nvs = EspDefaultNvsPartition::take()?;
    // Shown on the Start page until the next restart.
    let crash = crashlog::init(nvs.clone());
    if let Some(note) = &crash {
        log::warn!("{note}");
    }

    // The USB-A port: a receipt printer plugged in there is detected (and
    // shown) right away; the print channel uses it once online.
    let print_status = printing::SharedStatus::default();
    let printer = {
        let print_status = print_status.clone();
        usb_printer::UsbPrinter::start(move |name| {
            if let Ok(mut s) = print_status.lock() {
                s.printer = name;
            }
        })
        .map_err(|e| log::error!("usb printer: {e:#}"))
        .ok()
    };

    let status = Arc::new(Mutex::new(String::from("Wi-Fi: verbinde …")));
    let (ui_tx, ui_rx) = mpsc::channel();

    // Battery level and charging every 30 s (Start page, log); warns when
    // low and switches the Tab5 off before the pack runs flat.
    let battery = battery::SharedBattery::default();
    // "Akku schonen" (Start page): kept on the device, on by default.
    let care = battery::Care::load(nvs.clone());
    if let Err(e) = battery::start(battery.clone(), care.clone(), ui_tx.clone()) {
        log::warn!("battery: {e:#}");
    }
    let (tap_tx, tap_rx) = mpsc::channel();
    let demo = feed::PASSWORD.is_none();
    let peripherals = Peripherals::take()?;
    // The M5-Bus SPI for the Terminal page's modules (Tab5).
    #[cfg(not(feature = "jc1060p470"))]
    let bus_parts = lab::BusParts {
        spi2: peripherals.spi2,
        sck: peripherals.pins.gpio5,
        mosi: peripherals.pins.gpio18,
        miso: peripherals.pins.gpio19,
    };
    let sysloop = EspSystemEventLoop::take()?;
    #[cfg(not(feature = "jc1060p470"))]
    let lab_sysloop = sysloop.clone();
    let net_status = status.clone();
    let feed_nvs = nvs.clone();
    let net_ui = ui_tx.clone();
    let net_printer = printer.clone();
    let net_print_status = print_status.clone();
    std::thread::Builder::new()
        .name("net".into())
        .stack_size(16 * 1024)
        .spawn(move || {
            let feed_status = net_status.clone();
            net::run(peripherals.modem, sysloop, nvs, net_status, move || {
                // The 7": Bluetooth for the e-ink frame, now that the C6 is up.
                #[cfg(feature = "jc1060p470")]
                match ble::start(board::device_name()) {
                    Ok(()) => {
                        if let Err(e) = eink::start() {
                            log::error!("e-ink: {e:#}");
                        }
                    }
                    Err(e) => log::warn!("bluetooth: {e:#}"),
                }
                if demo {
                    return;
                }
                let session = match feed::start(net_ui.clone(), tap_rx, feed_status, feed_nvs.clone()) {
                    Ok(s) => s,
                    Err(e) => {
                        log::error!("feed: {e:#}");
                        return;
                    }
                };
                if let Some(printer) = net_printer {
                    if let Err(e) = printing::start(printer, session, feed_nvs, net_ui, net_print_status) {
                        log::error!("printing: {e:#}");
                    }
                }
            })
        })?;

    // Landscape either way up: the screen follows the accelerometer.
    let orientation = imu::Orientation::default();
    if let Err(e) = imu::start(orientation.clone()) {
        log::warn!("imu: {e:#}");
    }

    let gong = gong::Gong::start()?;
    // The Terminal page's parts start in the background; the screen shows
    // the tab once they are up.
    #[cfg(not(feature = "jc1060p470"))]
    let lab = lab::spawn(board::device_name(), bus_parts, lab_sysloop, term_out, term_lines, ui_tx.clone())
        .map_err(|e| log::error!("lab: {e:#}"))
        .ok();

    // The UI in its own thread, as the main task had it: 64 KB stack from
    // internal RAM, priority 1, core 0 — created now, after the C6 link's
    // start-up allocations (see sdkconfig.defaults).
    // SAFETY: plain config struct; the pthread layer copies it.
    unsafe {
        let mut cfg = sys::esp_pthread_get_default_config();
        cfg.stack_size = UI_STACK;
        cfg.prio = 1;
        cfg.pin_to_core = 0;
        cfg.inherit_cfg = false;
        sys::esp!(sys::esp_pthread_set_cfg(&cfg))?;
    }
    let ui = std::thread::Builder::new().name("ui".into()).stack_size(UI_STACK).spawn(move || {
        let result = ui::run(
            display,
            status,
            print_status,
            printer,
            battery,
            care,
            ui_rx,
            ui_tx,
            tap_tx,
            demo,
            crash,
            gong,
            orientation,
            #[cfg(not(feature = "jc1060p470"))]
            lab,
        );
        // Never returns unless the UI can't run: start again.
        if let Err(e) = result {
            log::error!("ui: {e:#}");
        }
        // SAFETY: plain restart.
        unsafe { sys::esp_restart() };
    });
    // SAFETY: as above.
    unsafe {
        let default = sys::esp_pthread_get_default_config();
        let _ = sys::esp_pthread_set_cfg(&default);
    }
    ui?;
    // Done: returning ends the main task and frees its stack (internal RAM,
    // which TLS needs); the threads run on.
    log::info!("main: started, the UI runs in its own thread");
    Ok(())
}

/// Internal RAM as the app starts: what constructors (esp_hosted) left.
fn log_internal_ram(when: &str) {
    // SAFETY: read-only heap statistics.
    let (free, largest) = unsafe {
        (
            sys::heap_caps_get_free_size(sys::MALLOC_CAP_INTERNAL),
            sys::heap_caps_get_largest_free_block(sys::MALLOC_CAP_INTERNAL),
        )
    };
    log::info!("internal RAM {when}: free {} KB, largest block {} KB", free / 1024, largest / 1024);
}

fn log_chip_info() {
    let mut chip = ChipInfo::default();
    let mut flash_bytes: u32 = 0;
    // SAFETY: plain out-parameter queries; null selects the default flash chip.
    let (idf, psram, internal) = unsafe {
        esp_chip_info(&mut chip);
        sys::esp_flash_get_size(core::ptr::null_mut(), &mut flash_bytes);
        (
            CStr::from_ptr(sys::esp_get_idf_version()).to_string_lossy(),
            sys::heap_caps_get_total_size(sys::MALLOC_CAP_SPIRAM),
            sys::heap_caps_get_total_size(sys::MALLOC_CAP_INTERNAL),
        )
    };
    log::info!(
        "kds-tab5 {}: ESP32-P4 rev v{}.{}, {} cores, flash {} MB, PSRAM {} MB, internal RAM {} KB, ESP-IDF {}",
        ota::VERSION,
        chip.revision / 100,
        chip.revision % 100,
        chip.cores,
        flash_bytes / (1024 * 1024),
        psram / (1024 * 1024),
        internal / 1024,
        idf,
    );
}
