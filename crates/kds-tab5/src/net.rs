//! The Tab5's network: Wi-Fi through the ESP32-C6 coprocessor (wifi.rs),
//! and the LAN module (W5500 on the M5-Bus, lab.rs) when there is one.
//!
//! The cable first: with a LAN module, Wi-Fi waits until the cable has an
//! address (or [`LAN_WAIT`] passed) and then stays off — the kitchen feed
//! and the print channel stay on the interface they opened on, so they
//! must open on the cable. Without the cable for [`LAN_LOST_FOR`] Wi-Fi
//! takes over; once the cable is back Wi-Fi goes off again and the
//! connections open anew over the LAN.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sntp::EspSntp;
use esp_idf_svc::sys;

use crate::wifi;

/// Set by lab.rs once the M5-Bus is probed: 1 no LAN module, 2 a W5500.
pub static LAN_MODULE: AtomicU8 = AtomicU8::new(0);
pub const LAN_NONE: u8 = 1;
pub const LAN_W5500: u8 = 2;
/// How long to wait for the bus probe, then for the cable's address.
const PROBE_WAIT: Duration = Duration::from_secs(8);
const LAN_WAIT: Duration = Duration::from_secs(15);
/// No cable link by then: no LAN to wait for.
const CABLE_WAIT: Duration = Duration::from_secs(4);
/// The cable without an address this long: Wi-Fi takes over.
const LAN_LOST_FOR: Duration = Duration::from_secs(10);

/// Online over the LAN cable: the W5500's interface up with an address.
pub fn wired() -> bool {
    lan_ip().is_some()
}

/// The cable is in (the W5500's interface up, address or not).
fn lan_link() -> bool {
    // SAFETY: a lookup by key and a state read; the handle isn't kept.
    unsafe {
        let netif = sys::esp_netif_get_handle_from_ifkey(c"ETH_DEF".as_ptr());
        !netif.is_null() && sys::esp_netif_is_netif_up(netif)
    }
}

/// The LAN's IPv4 address, when it has one.
fn lan_ip() -> Option<std::net::Ipv4Addr> {
    // SAFETY: a lookup by key and reads into a local; the handle isn't kept.
    unsafe {
        let netif = sys::esp_netif_get_handle_from_ifkey(c"ETH_DEF".as_ptr());
        if netif.is_null() || !sys::esp_netif_is_netif_up(netif) {
            return None;
        }
        let mut info = sys::esp_netif_ip_info_t::default();
        if sys::esp_netif_get_ip_info(netif, &mut info) != sys::ESP_OK || info.ip.addr == 0 {
            return None;
        }
        // lwIP keeps it in network order.
        Some(std::net::Ipv4Addr::from(info.ip.addr.to_le_bytes()))
    }
}

/// With a LAN module: wait for the cable's address. `true`: the LAN is up.
fn wait_for_lan() -> bool {
    if cfg!(feature = "cardputer") {
        return false;
    }
    let since = Instant::now();
    while LAN_MODULE.load(Ordering::Relaxed) == 0 && since.elapsed() < PROBE_WAIT {
        std::thread::sleep(Duration::from_millis(100));
    }
    if LAN_MODULE.load(Ordering::Relaxed) != LAN_W5500 {
        return false;
    }
    log::info!("LAN: module found, waiting for the cable before Wi-Fi");
    let since = Instant::now();
    while !wired() && since.elapsed() < LAN_WAIT {
        if since.elapsed() >= CABLE_WAIT && !lan_link() {
            log::info!("LAN: no cable — Wi-Fi");
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    wired()
}

fn describe_lan() -> String {
    match lan_ip() {
        Some(ip) => format!("LAN: {ip} · Wi-Fi in Reserve"),
        None => "LAN: kein Kabel".to_string(),
    }
}

/// Join the network (the cable first), set the clock, start the kitchen
/// feed (`online`), then keep the link up.
pub fn run(
    modem: Modem<'static>,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
    status: Arc<Mutex<String>>,
    online: impl FnOnce(),
) {
    let set = |text: String| {
        wifi::set_link(&text);
        if let Ok(mut s) = status.lock() {
            *s = text;
        }
    };
    // The C6 link first, as it always came up: its start-up garbles the
    // frames of a W5500 that already runs (the LAN guard then restarts the
    // Tab5). Not joined yet — that waits for the cable.
    let mut wifi = match wifi::connect(modem, sysloop, nvs, false) {
        Ok(w) => w,
        Err(e) => {
            log::error!("Wi-Fi: {e:#}");
            set(format!("Wi-Fi: {e:#}"));
            return;
        }
    };
    let mut lan_ok = wait_for_lan();
    if lan_ok {
        log::info!("{}", describe_lan());
        set(describe_lan());
    } else if wifi::wanted() {
        set("Wi-Fi: verbinde …".into());
        if let Err(e) = wifi.start() {
            log::warn!("Wi-Fi: start: {e}");
        }
        wifi::rejoin(&mut wifi);
        set(wifi::describe(&wifi));
    }
    let mut lan_gone_at: Option<Instant> = None;

    // Kept alive for the life of the thread; it re-syncs periodically. The
    // feed doesn't wait for it: until it answers, the first board snapshot
    // sets the clock from the server's (feed.rs).
    let _sntp = EspSntp::new_default().map_err(|e| log::warn!("SNTP: {e}")).ok();
    // The feed's login is the server check.
    online();

    let mut beat: u32 = 0;
    let mut tick: u32 = 0;
    loop {
        // Every second for `wifi on/off`; the rest every 10 s.
        std::thread::sleep(Duration::from_secs(1));
        tick += 1;
        // The cable: back = Wi-Fi off; gone for a while = Wi-Fi on.
        if wired() {
            lan_gone_at = None;
            if !lan_ok {
                lan_ok = true;
                log::info!("LAN: up — the kitchen moves to the cable, Wi-Fi on standby");
            }
        } else if lan_ok && lan_gone_at.get_or_insert_with(Instant::now).elapsed() >= LAN_LOST_FOR {
            lan_ok = false;
            lan_gone_at = None;
            log::warn!("LAN: no address for {} s — Wi-Fi takes over", LAN_LOST_FOR.as_secs());
        }
        let started = wifi.is_started().unwrap_or(false);
        if !wifi::wanted() || lan_ok {
            if started {
                // Stopping the station takes its address away: the LAN
                // carries the traffic from here (connections over Wi-Fi
                // break and open again on the cable).
                if lan_ok {
                    log::info!("Wi-Fi: standby, the LAN carries");
                } else {
                    log::warn!("Wi-Fi: switched off (terminal)");
                }
                let _ = wifi.disconnect();
                if let Err(e) = wifi.stop() {
                    log::warn!("Wi-Fi: stop: {e}");
                }
            }
            if tick % 10 == 0 || started {
                set(if lan_ok { describe_lan() } else { "Wi-Fi: aus".into() });
            }
            continue;
        }
        if !started {
            log::info!("Wi-Fi: on (terminal, or no LAN)");
            set("Wi-Fi: verbinde …".into());
            if let Err(e) = wifi.start() {
                log::warn!("Wi-Fi: start: {e}");
                continue;
            }
            wifi::rejoin(&mut wifi);
            if wifi::wanted() {
                set(wifi::describe(&wifi));
            }
            continue;
        }
        if tick % 10 != 0 {
            continue;
        }
        if !wifi.is_connected().unwrap_or(false) {
            log::warn!("Wi-Fi: connection lost");
            set("Wi-Fi: verbinde neu …".into());
            wifi::rejoin(&mut wifi);
            if wifi::wanted() {
                set(wifi::describe(&wifi));
            }
        }
        // SAFETY: read-only heap statistics.
        let (free_kb, internal_kb, internal_min_kb) = unsafe {
        (
            sys::esp_get_free_heap_size() / 1024,
            sys::heap_caps_get_free_size(sys::MALLOC_CAP_INTERNAL) / 1024,
            sys::heap_caps_get_minimum_free_size(sys::MALLOC_CAP_INTERNAL) / 1024,
        )
    };
        log::info!("alive #{beat}, free heap {free_kb} KB, internal {internal_kb} KB (lowest {internal_min_kb} KB)");
        beat += 1;
    }
}
