//! The Tab5's battery: a 2-cell Li-ion pack (NP-F550 type, 6.6–8.4 V),
//! measured by an INA226 (I2C 0x41, 5 mΩ shunt) on the board's I2C bus,
//! charged by a charger switched through the I/O expander at 0x44 (board.rs).
//! Register use and scaling as M5Stack's M5Unified library does it.
//!
//! A thread reads it every 30 s into a shared [`BatteryStatus`] for the
//! Start page and the log, and guards the pack against deep discharge (its
//! protection then cuts it off, and the Tab5's charger can't wake it): on
//! battery, a warning below [`WARN_MV`], and below [`OFF_MV`] the Tab5
//! announces it and switches itself off.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::ui::UiMsg;

use anyhow::{ensure, Result};
use esp_idf_svc::sys::{bsp, esp};

const ADDRESS: u16 = 0x41;
const REG_CONFIG: u8 = 0x00;
const REG_SHUNT: u8 = 0x01;
const REG_BUS: u8 = 0x02;
/// Averaging 16, 1.1 ms conversions, shunt and bus continuous.
const CONFIG: u16 = 0x4527;
const SHUNT_OHM: f32 = 0.005;
const EVERY: Duration = Duration::from_secs(30);
/// Read more often once the pack is low.
const EVERY_LOW: Duration = Duration::from_secs(10);
/// Below this the battery counts as neither charging nor draining.
const IDLE_MA: i32 = 30;
/// On battery below this (3.5 V per cell, roughly 15 % left): warn.
const WARN_MV: u32 = 7000;
const WARN_AGAIN: Duration = Duration::from_secs(5 * 60);
/// On battery below this (3.3 V per cell), on two readings in a row: switch
/// off, well before the pack's own protection trips.
const OFF_MV: u32 = 6600;
/// Between the announcement and switching off; power plugged in meanwhile
/// cancels it.
const OFF_GRACE: Duration = Duration::from_secs(20);
/// Readings below this aren't a connected pack (missing, or cut off by its
/// protection): the Tab5 runs on USB power then, nothing to guard.
const PLAUSIBLE_MV: u32 = 5000;

#[derive(Clone, Copy, Default, PartialEq)]
pub struct BatteryStatus {
    /// Pack voltage in mV.
    pub millivolts: u32,
    /// + = charging, − = draining (mA).
    pub milliamps: i32,
    /// Rough charge level from the cell voltage (higher while charging).
    pub percent: u8,
}

impl BatteryStatus {
    pub fn charging(&self) -> bool {
        self.milliamps > IDLE_MA
    }

    /// Running on the battery (no or too little external power).
    fn draining(&self) -> bool {
        self.milliamps < -IDLE_MA && self.millivolts >= PLAUSIBLE_MV
    }

    /// For the Start page: "Akku 72 % · 7.83 V · lädt (+740 mA)"; `None`
    /// without a pack (the Tab5 without battery, or a pack cut off by its
    /// protection), so the line doesn't claim an empty battery.
    pub fn describe(&self) -> Option<String> {
        if self.millivolts < PLAUSIBLE_MV {
            return None;
        }
        let state = if self.charging() {
            format!(" · lädt (+{} mA)", self.milliamps)
        } else if self.milliamps < -IDLE_MA {
            format!(" · {} mA", self.milliamps)
        } else {
            String::new()
        };
        let volts = self.millivolts as f32 / 1000.0;
        Some(format!("Akku {} % · {volts:.2} V{state}", self.percent))
    }
}

pub type SharedBattery = Arc<Mutex<Option<BatteryStatus>>>;

/// Start reading and guarding the battery; the status stays `None` if the
/// monitor doesn't answer. Warnings go to the screen through `ui`.
pub fn start(status: SharedBattery, ui: Sender<UiMsg>) -> Result<()> {
    let monitor = Monitor::open()?;
    std::thread::Builder::new()
        .name("battery".into())
        .stack_size(6 * 1024)
        .spawn(move || {
            let mut guard = Guard::default();
            loop {
                let pause = match monitor.read() {
                    Ok(b) => {
                        log::info!(
                            "battery: {:.2} V, {:+} mA, {} %, charger status line {}",
                            b.millivolts as f32 / 1000.0,
                            b.milliamps,
                            b.percent,
                            crate::board::charge_stat()
                        );
                        if let Ok(mut s) = status.lock() {
                            *s = Some(b);
                        }
                        guard.check(&b, &monitor, &ui)
                    }
                    Err(e) => {
                        log::warn!("battery: {e:#}");
                        EVERY
                    }
                };
                std::thread::sleep(pause);
            }
        })?;
    Ok(())
}

/// Deep-discharge protection; returns when to read next.
#[derive(Default)]
struct Guard {
    warned: Option<Instant>,
    /// Readings in a row below OFF_MV.
    empty: u8,
}

impl Guard {
    fn check(&mut self, b: &BatteryStatus, monitor: &Monitor, ui: &Sender<UiMsg>) -> Duration {
        if !b.draining() {
            // On external power (or no usable pack): nothing to guard.
            self.warned = None;
            self.empty = 0;
            return EVERY;
        }
        if b.millivolts >= WARN_MV {
            self.empty = 0;
            return EVERY;
        }
        if self.warned.is_none_or(|t| t.elapsed() >= WARN_AGAIN) {
            log::warn!("battery: low ({} mV), warning", b.millivolts);
            let _ = ui.send(UiMsg::Notice(format!(
                "Akku schwach ({} %) — bitte Netzteil anschließen",
                b.percent
            )));
            self.warned = Some(Instant::now());
        }
        if b.millivolts >= OFF_MV {
            self.empty = 0;
            return EVERY_LOW;
        }
        self.empty += 1;
        if self.empty < 2 {
            return EVERY_LOW;
        }
        log::warn!("battery: empty ({} mV), switching off in {} s", b.millivolts, OFF_GRACE.as_secs());
        let _ = ui.send(UiMsg::Notice(format!(
            "Akku leer — der Tab5 schaltet sich in {} s aus. Netzteil anschließen, um weiterzuarbeiten.",
            OFF_GRACE.as_secs()
        )));
        std::thread::sleep(OFF_GRACE);
        if monitor.read().is_ok_and(|now| now.draining()) {
            crate::board::power_off();
            // Still running: something keeps it powered after all.
            log::warn!("battery: still powered after the power-off pulse");
        } else {
            log::info!("battery: external power back, staying on");
            let _ = ui.send(UiMsg::Info("Netzteil erkannt — der Tab5 bleibt an".into()));
        }
        self.empty = 0;
        EVERY_LOW
    }
}

struct Monitor(bsp::i2c_master_dev_handle_t);

// SAFETY: the device handle is only used from the battery thread; the I2C
// master driver serialises access to the shared bus.
unsafe impl Send for Monitor {}

impl Monitor {
    fn open() -> Result<Self> {
        // SAFETY: the board's bus is set up by board::init; the device config
        // is plain data.
        unsafe {
            let bus = bsp::bsp_i2c_get_handle();
            ensure!(!bus.is_null(), "I2C bus not initialised");
            let mut cfg: bsp::i2c_device_config_t = core::mem::zeroed();
            cfg.dev_addr_length = bsp::i2c_addr_bit_len_t_I2C_ADDR_BIT_LEN_7;
            cfg.device_address = ADDRESS;
            cfg.scl_speed_hz = 400_000;
            let mut dev = core::ptr::null_mut();
            esp!(bsp::i2c_master_bus_add_device(bus, &cfg, &mut dev))?;
            let monitor = Self(dev);
            monitor.write(REG_CONFIG, CONFIG)?;
            Ok(monitor)
        }
    }

    fn read(&self) -> Result<BatteryStatus> {
        // Bus voltage: 1.25 mV per bit. Shunt voltage: 2.5 µV per bit, signed;
        // the shunt is wired so that charging reads negative.
        let bus = self.register(REG_BUS)? as u32;
        let shunt = self.register(REG_SHUNT)? as i16;
        let millivolts = bus * 125 / 100;
        let milliamps = -(shunt as f32 * 2.5e-6 / SHUNT_OHM * 1000.0) as i32;
        // Per cell, linear between 3.3 V (empty) and 4.15 V (full).
        let cell = millivolts as i32 / 2;
        let percent = ((cell - 3300) * 100 / (4150 - 3300)).clamp(0, 100) as u8;
        Ok(BatteryStatus { millivolts, milliamps, percent })
    }

    fn register(&self, reg: u8) -> Result<u16> {
        let mut buf = [0u8; 2];
        // SAFETY: live device handle; buffers outlive the call.
        esp!(unsafe { bsp::i2c_master_transmit_receive(self.0, &reg, 1, buf.as_mut_ptr(), 2, 100) })?;
        Ok(u16::from_be_bytes(buf))
    }

    fn write(&self, reg: u8, value: u16) -> Result<()> {
        let [hi, lo] = value.to_be_bytes();
        let data = [reg, hi, lo];
        // SAFETY: live device handle; the buffer outlives the call.
        esp!(unsafe { bsp::i2c_master_transmit(self.0, data.as_ptr(), data.len(), 100) })?;
        Ok(())
    }
}
