//! The Cardputer ADV's battery: G10 (ADC1 channel 9) sees half the pack
//! voltage (100 kΩ / 100 kΩ divider, schematic; M5Unified uses ×2 too).
//! Read every 10 s, 16 samples averaged, curve-fitting calibration. While
//! USB charges, the reading is the charger's voltage (the pin sits behind
//! the power switch), so it shows near full.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::Result;
use esp_idf_svc::hal::adc::attenuation::DB_12;
use esp_idf_svc::hal::adc::oneshot::config::{AdcChannelConfig, Calibration};
use esp_idf_svc::hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_svc::hal::adc::ADC1;
use esp_idf_svc::hal::gpio::Gpio10;

use crate::terminal::{Terminal, Tone};

/// Pack voltage (mV), 0 = not measured yet.
static MV: AtomicU32 = AtomicU32::new(0);
/// 0 % and 100 % (a 1S Li-ion pack under the device's load).
const EMPTY_MV: u32 = 3300;
const FULL_MV: u32 = 4150;
const EVERY: Duration = Duration::from_secs(10);

pub fn start(adc: ADC1<'static>, pin: Gpio10<'static>) -> Result<()> {
    std::thread::Builder::new().name("battery".into()).stack_size(4 * 1024).spawn(move || {
        let adc = match AdcDriver::new(adc) {
            Ok(a) => a,
            Err(e) => return log::warn!("battery: ADC: {e}"),
        };
        let cfg = AdcChannelConfig { attenuation: DB_12, calibration: Calibration::Curve, ..AdcChannelConfig::new() };
        let mut ch = match AdcChannelDriver::new(&adc, pin, &cfg) {
            Ok(c) => c,
            Err(e) => return log::warn!("battery: ADC channel: {e}"),
        };
        loop {
            let (mut sum, mut n) = (0u32, 0u32);
            for _ in 0..16 {
                if let Ok(mv) = ch.read() {
                    sum += u32::from(mv);
                    n += 1;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            if n > 0 {
                MV.store(sum / n * 2, Ordering::Relaxed);
            }
            std::thread::sleep(EVERY);
        }
    })?;
    Ok(())
}

pub fn mv() -> Option<u32> {
    match MV.load(Ordering::Relaxed) {
        0 => None,
        v => Some(v),
    }
}

pub fn percent() -> Option<u8> {
    mv().map(|v| (v.saturating_sub(EMPTY_MV) * 100 / (FULL_MV - EMPTY_MV)).min(100) as u8)
}

/// "Akku 3,92 V · 74 %".
pub fn describe() -> String {
    match (mv(), percent()) {
        (Some(v), Some(p)) => format!("Akku {},{:02} V · {p} %", v / 1000, v % 1000 / 10),
        _ => "Akku: noch keine Messung".to_string(),
    }
}

/// `bat` in the terminal.
pub fn command(words: &[&str], term: &mut Terminal) -> bool {
    match words {
        ["bat"] | ["battery"] | ["akku"] => {
            let tone = match percent() {
                Some(p) if p < 15 => Tone::Err,
                Some(p) if p < 30 => Tone::Warn,
                Some(_) => Tone::Ok,
                None => Tone::Dim,
            };
            term.push(tone, &describe());
            if mv().is_some_and(|v| v > FULL_MV) {
                term.push(Tone::Dim, "(über 4,15 V: am USB, lädt — der Wert ist die Ladespannung)");
            }
            true
        }
        _ => false,
    }
}
