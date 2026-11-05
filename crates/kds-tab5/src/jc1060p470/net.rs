//! The JC1060P470's network, chosen at start by the LAN cable:
//!
//! - **LAN** (cable in, an address from DHCP): the cable is the uplink (the
//!   P4's own Ethernet MAC with an IP101 PHY, set up by the board package),
//!   and the board runs the shop's Wi-Fi hotspot through its ESP32-C6 — the
//!   same network as the STM32 board's (systemd/kds/50-kds-wifi-ap.network):
//!   its SSID and password, 192.168.72.1/24, DHCP, NAT out through the
//!   cable. A Tab5 joins it with what it already knows ("rusterando").
//! - **Wi-Fi** (no cable, or no DHCP on it): a Wi-Fi client like the Tab5
//!   (wifi.rs, the same compiled-in networks), no hotspot. A cable plugged
//!   in later restarts the board into LAN mode. A cable pulled in LAN mode
//!   doesn't switch back (a restart would): the hotspot stays up for the
//!   devices on it, and the status line says the LAN is gone.
//!
//! Hotspot name and password: `HOTSPOT_SSID` / `HOTSPOT_PASS` from
//! tmp/tab5-wifi.env, compiled in; without them there is no hotspot. Anyone
//! with the password reaches the shop network too, as on the STM board.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use embedded_svc::ipv4::{self, Mask, RouterConfiguration, Subnet};
use embedded_svc::wifi::{AccessPointConfiguration, AuthMethod, Configuration};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::netif::{EspNetif, NetifConfiguration, NetifStack};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sntp::EspSntp;
use esp_idf_svc::sys::{self, bsp, esp};
use esp_idf_svc::wifi::{EspWifi, WifiDriver};

use crate::wifi;

/// The hotspot's own address and subnet, as on the STM board.
const HOTSPOT_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 72, 1);
const HOTSPOT_MASK: u8 = 24;
/// A fixed channel: the hotspot is alone in AP mode, nothing to follow.
const HOTSPOT_CHANNEL: u8 = 6;
const HOTSPOT_MAX_CLIENTS: u16 = 8;
/// Without DNS from the LAN (static setups), clients get this one.
const FALLBACK_DNS: Ipv4Addr = Ipv4Addr::new(9, 9, 9, 9);
/// No link within this time after the driver started: no cable, Wi-Fi.
/// (A plugged-in cable negotiates in about 1–3 s.)
const LINK_WAIT: Duration = Duration::from_secs(5);
/// Link but no address from DHCP within this time: Wi-Fi.
const DHCP_WAIT: Duration = Duration::from_secs(20);
/// In Wi-Fi mode, a LAN address held this long restarts into LAN mode.
const CABLE_SETTLE: Duration = Duration::from_secs(20);

fn set(status: &Mutex<String>, text: String) {
    if let Ok(mut s) = status.lock() {
        *s = text;
    }
}

/// Get online (LAN + hotspot, or Wi-Fi), start the clock and the kitchen
/// feed (`online`), then watch the link.
pub fn run(
    modem: Modem<'static>,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
    status: Arc<Mutex<String>>,
    online: impl FnOnce(),
) {
    set(&status, "LAN: prüfe Kabel …".into());
    let eth = Ethernet::start().map_err(|e| log::error!("LAN: {e:#}")).ok();
    match eth.as_ref().and_then(Ethernet::wait_for_lan) {
        Some(ip) => {
            log::info!("LAN: up, IP {ip} — LAN mode, with hotspot");
            lan_mode(eth.expect("LAN up"), modem, sysloop, nvs, &status, online)
        }
        None => {
            log::info!("LAN: none — Wi-Fi mode");
            wifi_mode(eth, modem, sysloop, nvs, &status, online)
        }
    }
}

/// Clock and kitchen feed, once online.
fn start_services(online: impl FnOnce()) -> Option<EspSntp<'static>> {
    // Kept alive by the caller; it re-syncs periodically. The feed doesn't
    // wait for it: until it answers, the first board snapshot sets the
    // clock from the server's (feed.rs).
    let sntp = EspSntp::new_default().map_err(|e| log::warn!("SNTP: {e}")).ok();
    // The feed's login is the server check.
    online();
    sntp
}

fn alive(beat: &mut u32) {
    // SAFETY: read-only heap statistics.
    let free_kb = unsafe { sys::esp_get_free_heap_size() } / 1024;
    log::info!("alive #{beat}, free heap {free_kb} KB");
    *beat += 1;
}

fn lan_mode(
    eth: Ethernet,
    modem: Modem<'static>,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
    status: &Mutex<String>,
    online: impl FnOnce(),
) {
    // After the LAN: the hotspot hands out the LAN's DNS server.
    let hotspot = match Hotspot::start(modem, sysloop, nvs, eth.dns().unwrap_or(FALLBACK_DNS)) {
        Ok(h) => h,
        Err(e) => {
            log::error!("hotspot: {e:#}");
            None
        }
    };
    set(status, describe(&eth, hotspot.as_ref()));
    let _sntp = start_services(online);

    let mut beat: u32 = 0;
    let mut was_up = true;
    loop {
        std::thread::sleep(Duration::from_secs(10));
        let up = eth.ip().is_some();
        if up != was_up {
            if up {
                log::info!("LAN: back, IP {}", eth.ip().map(|ip| ip.to_string()).unwrap_or_default());
            } else {
                log::warn!("LAN: connection lost (cable?)");
            }
            was_up = up;
        }
        set(status, describe(&eth, hotspot.as_ref()));
        alive(&mut beat);
    }
}

fn wifi_mode(
    eth: Option<Ethernet>,
    modem: Modem<'static>,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
    status: &Mutex<String>,
    online: impl FnOnce(),
) {
    set(status, "Wi-Fi: verbinde …".into());
    let mut wifi = match wifi::connect(modem, sysloop, nvs) {
        Ok(w) => w,
        Err(e) => {
            log::error!("Wi-Fi: {e:#}");
            set(status, format!("Wi-Fi: {e:#} · kein LAN-Kabel"));
            return;
        }
    };
    set(status, wifi::describe(&wifi));
    let _sntp = start_services(online);

    let mut beat: u32 = 0;
    let mut lan_since: Option<Instant> = None;
    loop {
        std::thread::sleep(Duration::from_secs(10));
        if !wifi.is_connected().unwrap_or(false) {
            log::warn!("Wi-Fi: connection lost");
            set(status, "Wi-Fi: verbinde neu …".into());
            wifi::rejoin(&mut wifi);
            set(status, wifi::describe(&wifi));
        }
        // A cable plugged in now: LAN mode (and the hotspot) need a restart.
        match eth.as_ref().and_then(Ethernet::ip) {
            Some(ip) => {
                let since = *lan_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= CABLE_SETTLE {
                    log::warn!("LAN: cable plugged in (IP {ip}), restarting into LAN mode");
                    set(status, "LAN-Kabel erkannt: Neustart …".into());
                    std::thread::sleep(Duration::from_secs(1));
                    // SAFETY: plain restart; does not return.
                    unsafe { sys::esp_restart() };
                }
            }
            None => lan_since = None,
        }
        alive(&mut beat);
    }
}

/// "LAN · <IP> · Hotspot rusterando (1 Gerät)" for the status line.
fn describe(eth: &Ethernet, hotspot: Option<&Hotspot>) -> String {
    let lan = match eth.ip() {
        Some(ip) => format!("LAN · {ip}"),
        None => "LAN: kein Netz (Kabel?)".to_string(),
    };
    match hotspot {
        Some(h) => match h.clients() {
            Some(1) => format!("{lan} · Hotspot {} (1 Gerät)", h.ssid),
            Some(n) => format!("{lan} · Hotspot {} ({n} Geräte)", h.ssid),
            None => format!("{lan} · Hotspot {}", h.ssid),
        },
        None => lan,
    }
}

/// The LAN interface; its driver runs for the life of the device.
struct Ethernet(*mut sys::esp_netif_t);

// SAFETY: esp_netif calls are thread-safe; the handle is never freed.
unsafe impl Send for Ethernet {}

impl Ethernet {
    fn start() -> Result<Self> {
        // The board package sets up the netif stack, the driver and its
        // interface; the system event loop exists already (taken in main).
        let mut netif: *mut bsp::esp_netif_t = core::ptr::null_mut();
        // SAFETY: out-parameter receives the started interface on success.
        esp!(unsafe { bsp::bsp_eth_start(&mut netif) }).context("Ethernet start")?;
        if netif.is_null() {
            bail!("Ethernet: no interface");
        }
        log::info!("LAN: Ethernet started, waiting for the cable and DHCP");
        Ok(Self(netif.cast()))
    }

    /// Wait for the cable and an address from DHCP: `None` when there is no
    /// link within [`LINK_WAIT`], or no address within [`DHCP_WAIT`] more.
    fn wait_for_lan(&self) -> Option<Ipv4Addr> {
        let start = Instant::now();
        let mut linked: Option<Instant> = None;
        loop {
            if let Some(ip) = self.ip() {
                return Some(ip);
            }
            if self.link_up() {
                let since = *linked.get_or_insert_with(Instant::now);
                if since.elapsed() >= DHCP_WAIT {
                    log::warn!("LAN: cable in, but no address from DHCP after {} s", DHCP_WAIT.as_secs());
                    return None;
                }
            } else if start.elapsed() >= LINK_WAIT {
                log::info!("LAN: no cable (no link after {} s)", LINK_WAIT.as_secs());
                return None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn link_up(&self) -> bool {
        // SAFETY: reads a flag kept by the board package's event handler.
        unsafe { bsp::bsp_eth_link_up() }
    }

    fn ip(&self) -> Option<Ipv4Addr> {
        // SAFETY: live netif; plain out-parameter.
        unsafe {
            let mut info: sys::esp_netif_ip_info_t = core::mem::zeroed();
            if sys::esp_netif_get_ip_info(self.0, &mut info) != sys::ESP_OK || info.ip.addr == 0 {
                return None;
            }
            Some(Ipv4Addr::from(u32::from_be(info.ip.addr)))
        }
    }

    /// The DNS server the LAN's DHCP handed out.
    fn dns(&self) -> Option<Ipv4Addr> {
        // SAFETY: live netif; plain out-parameter.
        unsafe {
            let mut dns: sys::esp_netif_dns_info_t = core::mem::zeroed();
            if sys::esp_netif_get_dns_info(self.0, sys::esp_netif_dns_type_t_ESP_NETIF_DNS_MAIN, &mut dns)
                != sys::ESP_OK
            {
                return None;
            }
            let addr = dns.ip.u_addr.ip4.addr;
            (addr != 0).then(|| Ipv4Addr::from(u32::from_be(addr)))
        }
    }
}

struct Hotspot {
    _wifi: EspWifi<'static>,
    ssid: &'static str,
}

impl Hotspot {
    /// Start the hotspot, or `None` if no name is compiled in.
    fn start(
        modem: Modem<'static>,
        sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        dns: Ipv4Addr,
    ) -> Result<Option<Self>> {
        let Some(ssid) = option_env!("HOTSPOT_SSID").filter(|s| !s.is_empty()) else {
            log::info!("hotspot: none (no HOTSPOT_SSID compiled in)");
            return Ok(None);
        };
        let pass = option_env!("HOTSPOT_PASS").unwrap_or("");
        if pass.len() < 8 {
            bail!("HOTSPOT_PASS must have 8 to 63 characters (WPA2)");
        }

        let driver = WifiDriver::new(modem, sysloop, Some(nvs))?;
        let sta = EspNetif::new(NetifStack::Sta)?;
        let ap = EspNetif::new_with_conf(&NetifConfiguration {
            ip_configuration: Some(ipv4::Configuration::Router(RouterConfiguration {
                subnet: Subnet { gateway: HOTSPOT_IP, mask: Mask(HOTSPOT_MASK) },
                dhcp_enabled: true,
                dns: Some(dns),
                secondary_dns: None,
            })),
            ..NetifConfiguration::wifi_default_router()
        })?;
        let mut wifi = EspWifi::wrap_all(driver, sta, ap)?;
        wifi.set_configuration(&Configuration::AccessPoint(AccessPointConfiguration {
            ssid: ssid.try_into().map_err(|_| anyhow!("HOTSPOT_SSID longer than 32 bytes"))?,
            password: pass.try_into().map_err(|_| anyhow!("HOTSPOT_PASS longer than 64 bytes"))?,
            auth_method: AuthMethod::WPA2Personal,
            channel: HOTSPOT_CHANNEL,
            max_connections: HOTSPOT_MAX_CLIENTS,
            ..Default::default()
        }))?;
        wifi.start().context("start (is the C6 coprocessor answering?)")?;
        // From here on `wifi` must live: dropping it detaches the AP
        // interface, and the coprocessor's "AP started" event (handled on the
        // event loop, a moment later) would then start a detached interface
        // — ESP-IDF aborts on that.

        // NAT only takes an interface that is up (ESP_FAIL otherwise, without
        // a log line), i.e. after that event was handled.
        let mut waited = Duration::ZERO;
        while !wifi.ap_netif().is_up().unwrap_or(false) && waited < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(100));
            waited += Duration::from_millis(100);
        }
        // Clients' traffic goes out through the LAN under the board's address.
        // SAFETY: live AP interface.
        match esp!(unsafe { sys::esp_netif_napt_enable(wifi.ap_netif().handle()) }) {
            Ok(()) => log::info!(
                "hotspot: \"{ssid}\" up on channel {HOTSPOT_CHANNEL}, {HOTSPOT_IP}/{HOTSPOT_MASK}, DNS {dns} for clients, NAT to the LAN"
            ),
            Err(e) => log::error!(
                "hotspot: \"{ssid}\" up, but no NAT ({e}, interface up: {:?}): clients get no internet",
                wifi.ap_netif().is_up()
            ),
        }
        Ok(Some(Self { _wifi: wifi, ssid }))
    }

    /// Devices on the hotspot right now.
    fn clients(&self) -> Option<usize> {
        // SAFETY: plain out-parameter; the hotspot is running.
        unsafe {
            let mut list: sys::wifi_sta_list_t = core::mem::zeroed();
            (sys::esp_wifi_ap_get_sta_list(&mut list) == sys::ESP_OK).then_some(list.num as usize)
        }
    }
}
