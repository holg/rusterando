//! The Tab5's network: Wi-Fi through the ESP32-C6 coprocessor (wifi.rs).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sntp::EspSntp;
use esp_idf_svc::sys;

use crate::wifi;

/// Join Wi-Fi, set the clock, start the kitchen feed (`online`), then keep
/// the link up.
pub fn run(
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
