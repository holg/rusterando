//! Kitchen display on the M5Stack Tab5 (ESP32-P4).
//!
//! Board setup (I2C, expanders, Wi-Fi power, charging), the display, the USB
//! printer port, then the kitchen screen (kds-ui) on the main task. A network
//! thread joins Wi-Fi through the ESP32-C6 coprocessor, sets the clock via
//! SNTP and keeps the connection up; once online it starts the live feed
//! (feed.rs) and the print channel (printing.rs), which prints the kitchen's
//! receipts while a USB printer is plugged in.

mod battery;
mod board;
mod crashlog;
mod display;
mod feed;
mod gong;
mod imu;
mod ota;
mod printing;
mod ui;
mod usb_printer;
mod wifi;

use std::ffi::CStr;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sntp::EspSntp;
use esp_idf_svc::sys;

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
    esp_idf_svc::log::EspLogger::initialize_default();
    log_chip_info();
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
    if let Err(e) = battery::start(battery.clone(), ui_tx.clone()) {
        log::warn!("battery: {e:#}");
    }
    let (tap_tx, tap_rx) = mpsc::channel();
    let demo = feed::PASSWORD.is_none();
    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
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
            network(peripherals.modem, sysloop, nvs, net_status, move || {
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
    ui::run(display, status, print_status, printer, battery, ui_rx, ui_tx, tap_tx, demo, crash, gong, orientation)
}

/// Join Wi-Fi, set the clock, start the kitchen feed (`online`), then keep
/// the link up.
fn network(
    modem: Modem<'static>,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
    status: Arc<Mutex<String>>,
    online: impl FnOnce(),
) {
    let set = |text: String| {
        if let Ok(mut s) = status.lock() {
            *s = text;
        }
    };
    let mut wifi = match wifi::connect(modem, sysloop, nvs) {
        Ok(w) => w,
        Err(e) => {
            log::error!("Wi-Fi: {e:#}");
            set(format!("Wi-Fi: {e:#}"));
            return;
        }
    };
    let link = wifi::describe(&wifi);
    set(link.clone());

    // Kept alive for the life of the thread; it re-syncs periodically. The
    // feed doesn't wait for it: until it answers, the first board snapshot
    // sets the clock from the server's (feed.rs).
    let _sntp = EspSntp::new_default().map_err(|e| log::warn!("SNTP: {e}")).ok();
    // The feed's login is the server check.
    online();

    let mut beat: u32 = 0;
    loop {
        std::thread::sleep(Duration::from_secs(10));
        if !wifi.is_connected().unwrap_or(false) {
            log::warn!("Wi-Fi: connection lost");
            set("Wi-Fi: verbinde neu …".into());
            wifi::rejoin(&mut wifi);
            set(wifi::describe(&wifi));
        }
        // SAFETY: read-only heap statistics.
        let free_kb = unsafe { sys::esp_get_free_heap_size() } / 1024;
        log::info!("alive #{beat}, free heap {free_kb} KB");
        beat += 1;
    }
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
