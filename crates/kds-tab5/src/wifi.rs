//! Wi-Fi through the ESP32-C6 coprocessor (esp_wifi_remote over esp_hosted).
//!
//! Up to five networks are compiled in (`tmp/tab5-wifi.env` → `WIFI_SSID` /
//! `WIFI_PASS`, `WIFI2_SSID` / `WIFI2_PASS` … `WIFI5_*`). The order doesn't
//! matter: a scan finds which are in range and the strongest wins, so one
//! image works at home, on the road and next to the shop board ("rusterando").
//!
//! The network that worked last (name + channel) is kept in NVS and tried
//! first, without a scan — that saves the scan's ~2.5 s at every start.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};

pub type Wifi = BlockingWifi<EspWifi<'static>>;

/// Where the last good network is kept.
static MEMORY: OnceLock<Mutex<EspNvs<NvsDefault>>> = OnceLock::new();
const NVS_NAMESPACE: &str = "kds";
const KEY_SSID: &str = "wifi_ssid";
const KEY_CHANNEL: &str = "wifi_ch";

struct Network {
    ssid: &'static str,
    pass: &'static str,
}

fn known_networks() -> Vec<Network> {
    [
        (option_env!("WIFI_SSID"), option_env!("WIFI_PASS")),
        (option_env!("WIFI2_SSID"), option_env!("WIFI2_PASS")),
        (option_env!("WIFI3_SSID"), option_env!("WIFI3_PASS")),
        (option_env!("WIFI4_SSID"), option_env!("WIFI4_PASS")),
        (option_env!("WIFI5_SSID"), option_env!("WIFI5_PASS")),
    ]
    .into_iter()
    .filter_map(|(ssid, pass)| {
        Some(Network {
            ssid: ssid.filter(|s| !s.is_empty())?,
            pass: pass.unwrap_or(""),
        })
    })
    .collect()
}

/// Start Wi-Fi and join a known network, retrying with backoff until one
/// works. Only fails when no network is compiled in at all.
pub fn connect(
    modem: Modem<'static>,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> Result<Wifi> {
    if known_networks().is_empty() {
        bail!("no Wi-Fi network compiled in (tmp/tab5-wifi.env)");
    }
    match EspNvs::new(nvs.clone(), NVS_NAMESPACE, true) {
        Ok(memory) => {
            let _ = MEMORY.set(Mutex::new(memory));
        }
        Err(e) => log::warn!("Wi-Fi: no NVS for the last network: {e}"),
    }
    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sysloop.clone(), Some(nvs))?, sysloop)?;
    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
    wifi.start().context("start (is the C6 coprocessor answering?)")?;
    rejoin(&mut wifi);
    Ok(wifi)
}

/// Join (or re-join) until connected, with backoff 2 s … 30 s.
pub fn rejoin(wifi: &mut Wifi) {
    let mut attempt: u32 = 0;
    loop {
        match join(wifi) {
            Ok(()) => return,
            Err(e) => {
                attempt += 1;
                let wait = Duration::from_secs((2u64 << attempt.min(4)).min(30));
                log::warn!("Wi-Fi: {e:#} — retry in {} s", wait.as_secs());
                std::thread::sleep(wait);
            }
        }
    }
}

fn join(wifi: &mut Wifi) -> Result<()> {
    let networks = known_networks();

    // The network that worked last time, straight away: no scan.
    if let Some((ssid, channel)) = remembered() {
        if let Some(net) = networks.iter().find(|n| n.ssid == ssid) {
            log::info!("Wi-Fi: joining \"{ssid}\" (last used, channel {channel})");
            match join_one(wifi, net, Some(channel)) {
                Ok(()) => return Ok(()),
                Err(e) => log::warn!("Wi-Fi: last used \"{ssid}\": {e:#} — scanning"),
            }
        }
    }

    let seen = wifi.scan().context("scan")?;
    log::info!("Wi-Fi scan: {} networks in range", seen.len());

    // Known networks in range, strongest first. None visible: try them all
    // blind, in case one hides its name.
    let mut candidates: Vec<(&Network, Option<(u8, i8)>)> = seen
        .iter()
        .filter_map(|ap| {
            let net = networks.iter().find(|n| n.ssid == ap.ssid.as_str())?;
            Some((net, Some((ap.channel, ap.signal_strength))))
        })
        .collect();
    candidates.sort_by_key(|(_, ap)| std::cmp::Reverse(ap.map(|(_, rssi)| rssi)));
    if candidates.is_empty() {
        candidates = networks.iter().map(|n| (n, None)).collect();
    }

    let mut last_err = anyhow!("no candidate network");
    for (net, ap) in candidates {
        match ap {
            Some((channel, rssi)) => {
                log::info!("Wi-Fi: joining \"{}\" (channel {channel}, {rssi} dBm)", net.ssid)
            }
            None => log::info!("Wi-Fi: \"{}\" not seen in the scan, trying anyway", net.ssid),
        }
        match join_one(wifi, net, ap.map(|(channel, _)| channel)) {
            Ok(()) => {
                if let Some((channel, _)) = ap {
                    remember(net.ssid, channel);
                }
                return Ok(());
            }
            Err(e) => {
                log::warn!("Wi-Fi: \"{}\": {e:#}", net.ssid);
                last_err = e;
            }
        }
    }
    Err(last_err)
}

fn join_one(wifi: &mut Wifi, net: &Network, channel: Option<u8>) -> Result<()> {
    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: net.ssid.try_into().map_err(|_| anyhow!("SSID longer than 32 bytes"))?,
        password: net.pass.try_into().map_err(|_| anyhow!("password longer than 64 bytes"))?,
        auth_method: if net.pass.is_empty() { AuthMethod::None } else { AuthMethod::WPA2Personal },
        channel,
        ..Default::default()
    }))?;
    wifi.connect().context("connect")?;
    wifi.wait_netif_up().context("no IP address (DHCP)")?;
    let ip = wifi.wifi().sta_netif().get_ip_info()?;
    log::info!("Wi-Fi: up on \"{}\", IP {} via {}", net.ssid, ip.ip, ip.subnet.gateway);
    Ok(())
}

fn remembered() -> Option<(String, u8)> {
    let memory = MEMORY.get()?.lock().ok()?;
    let mut buf = [0u8; 33];
    let ssid = memory.get_str(KEY_SSID, &mut buf).ok().flatten()?.to_string();
    let channel = memory.get_u8(KEY_CHANNEL).ok().flatten()?;
    Some((ssid, channel))
}

fn remember(ssid: &str, channel: u8) {
    if remembered().is_some_and(|(s, c)| s == ssid && c == channel) {
        return; // unchanged: spare the flash a write
    }
    if let Some(Ok(memory)) = MEMORY.get().map(Mutex::lock) {
        let _ = memory.set_str(KEY_SSID, ssid);
        let _ = memory.set_u8(KEY_CHANNEL, channel);
    }
}

/// "Wi-Fi: <network> · <IP>" for the status line.
pub fn describe(wifi: &Wifi) -> String {
    let ssid = match wifi.get_configuration() {
        Ok(Configuration::Client(c)) => c.ssid.to_string(),
        _ => "?".into(),
    };
    match wifi.wifi().sta_netif().get_ip_info() {
        Ok(ip) => format!("Wi-Fi: {ssid} · {}", ip.ip),
        Err(_) => format!("Wi-Fi: {ssid}"),
    }
}
