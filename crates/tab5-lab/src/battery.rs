//! Battery voltage for the status bar, and the same protection as the
//! kitchen firmware: switch off below 6.6 V so the pack never runs flat
//! (below 6 V it goes into protection mode). INA226 at 0x41 on the board's
//! I2C bus, bus voltage 1.25 mV per bit.

use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use esp_idf_svc::sys::{bsp, esp};

use crate::terminal::Tone;
use crate::{board, say, Out};

const ADDRESS: u16 = 0x41;
const REG_CONFIG: u8 = 0x00;
const REG_SHUNT: u8 = 0x01;
const REG_BUS: u8 = 0x02;
/// 5 mΩ shunt, wired so that charging reads negative.
const SHUNT_OHM: f32 = 0.005;
/// Below this current the pack counts as neither charging nor draining.
const IDLE_MA: i32 = 30;
/// Averaging 16, 1.1 ms conversions, continuous (as the kitchen firmware).
const CONFIG: u16 = 0x4527;
const EVERY: Duration = Duration::from_secs(10);
const WARN_MV: u32 = 7000;
const OFF_MV: u32 = 6600;
/// Below this the reading is no battery (USB only) or garbage.
const PLAUSIBLE_MV: u32 = 5000;

/// Millivolts (0 = unknown) and milliamps (+ charging, − draining).
#[derive(Clone, Default)]
pub struct Battery(Arc<AtomicU32>, Arc<AtomicI32>);

impl Battery {
    /// "7.84 V +650 mA"; `None` without a usable pack.
    pub fn describe(&self) -> Option<String> {
        let mv = self.0.load(Ordering::Relaxed);
        let ma = self.1.load(Ordering::Relaxed);
        (mv >= PLAUSIBLE_MV).then(|| format!("{}.{:02} V {ma:+} mA", mv / 1000, mv % 1000 / 10))
    }
}

pub fn start(out: Out) -> Result<Battery> {
    let monitor = Monitor::open()?;
    let battery = Battery::default();
    let shared = battery.clone();
    std::thread::Builder::new()
        .name("battery".into())
        .stack_size(4 * 1024)
        .spawn(move || {
            let mut low_reads = 0;
            let mut warned: Option<Instant> = None;
            loop {
                if let Ok((mv, ma)) = monitor.read() {
                    shared.0.store(mv, Ordering::Relaxed);
                    shared.1.store(ma, Ordering::Relaxed);
                    // Only while running on the pack: the Tab5 charges only
                    // while it's on, so never switch off on external power.
                    let draining = ma < -IDLE_MA;
                    if draining && (PLAUSIBLE_MV..OFF_MV).contains(&mv) {
                        low_reads += 1;
                        say(
                            &out,
                            Tone::Err,
                            format!("battery {mv} mV — switching off at the next low reading"),
                        );
                        if low_reads >= 2 {
                            board::power_off();
                        }
                    } else {
                        low_reads = 0;
                        if draining
                            && (OFF_MV..WARN_MV).contains(&mv)
                            && warned.is_none_or(|t| t.elapsed() >= Duration::from_secs(300))
                        {
                            say(
                                &out,
                                Tone::Warn,
                                format!("battery low: {mv} mV — connect USB-C"),
                            );
                            warned = Some(Instant::now());
                        }
                    }
                }
                std::thread::sleep(EVERY);
            }
        })?;
    Ok(battery)
}

struct Monitor(bsp::i2c_master_dev_handle_t);

// SAFETY: only used from the battery thread; the I2C master driver
// serialises access to the shared bus.
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
            let [hi, lo] = CONFIG.to_be_bytes();
            let data = [REG_CONFIG, hi, lo];
            esp!(bsp::i2c_master_transmit(
                dev,
                data.as_ptr(),
                data.len(),
                100
            ))?;
            Ok(Self(dev))
        }
    }

    /// Pack voltage (mV) and current (mA, + charging).
    fn read(&self) -> Result<(u32, i32)> {
        let bus = self.register(REG_BUS)? as u32;
        let shunt = self.register(REG_SHUNT)? as i16;
        let milliamps = -(shunt as f32 * 2.5e-6 / SHUNT_OHM * 1000.0) as i32;
        Ok((bus * 125 / 100, milliamps))
    }

    fn register(&self, reg: u8) -> Result<u16> {
        let mut buf = [0u8; 2];
        // SAFETY: live device handle; buffers outlive the call.
        esp!(unsafe {
            bsp::i2c_master_transmit_receive(self.0, &reg, 1, buf.as_mut_ptr(), 2, 100)
        })?;
        Ok(u16::from_be_bytes(buf))
    }
}
