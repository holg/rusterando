//! Which way up the Tab5 is: the BMI270 accelerometer (I2C 0x68, through
//! Espressif's bmi270 driver from the board package) gives the gravity
//! vector. The kitchen screen is landscape-only, so the screen follows the
//! two landscape positions — normal and upside down — and the cable can
//! leave on either side. Lying flat (gravity through the panel) keeps the
//! current position.
//!
//! A thread samples every 100 ms; a position has to hold for [`STEADY`]
//! before the screen turns.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use esp_idf_svc::sys::{bsp, esp};

/// The accelerometer axis along the panel's short side (gravity runs along
/// it when the screen stands in landscape), and the sign that means "normal"
/// (the UI's default rotation).
const SHORT_AXIS: usize = 0;
const NORMAL_SIGN: f32 = -1.0;
/// Gravity share along that axis that counts as standing in landscape (g).
const UPRIGHT_G: f32 = 0.6;
/// How long a new position must hold before the screen turns.
const STEADY: Duration = Duration::from_millis(1200);
const SAMPLE: Duration = Duration::from_millis(100);

/// Normal landscape (`false`) or upside down (`true`), shared with the UI.
#[derive(Clone, Default)]
pub struct Orientation(Arc<AtomicU8>);

impl Orientation {
    pub fn flipped(&self) -> bool {
        self.0.load(Ordering::Relaxed) == 1
    }

    fn set(&self, flipped: bool) {
        self.0.store(u8::from(flipped), Ordering::Relaxed);
    }
}

/// Start following the device's position.
pub fn start(orientation: Orientation) -> Result<()> {
    let sensor = Sensor::open()?;
    std::thread::Builder::new()
        .name("imu".into())
        .stack_size(4 * 1024)
        .spawn(move || {
            let mut candidate: Option<(bool, Instant)> = None;
            let mut last_log = Instant::now() - Duration::from_secs(60);
            loop {
                if let Ok(g) = sensor.read() {
                    if last_log.elapsed() >= Duration::from_secs(5) {
                        log::info!("imu: gravity x {:+.2} y {:+.2} z {:+.2} g", g[0], g[1], g[2]);
                        last_log = Instant::now();
                    }
                    let along = g[SHORT_AXIS] * NORMAL_SIGN;
                    let position = if along > UPRIGHT_G {
                        Some(false)
                    } else if along < -UPRIGHT_G {
                        Some(true)
                    } else {
                        None // flat or on its side: keep
                    };
                    match position {
                        Some(p) if p != orientation.flipped() => match candidate {
                            Some((c, since)) if c == p && since.elapsed() >= STEADY => {
                                log::info!("imu: screen turned {}", if p { "upside down" } else { "normal" });
                                orientation.set(p);
                                candidate = None;
                            }
                            Some((c, _)) if c == p => {}
                            _ => candidate = Some((p, Instant::now())),
                        },
                        _ => candidate = None,
                    }
                }
                std::thread::sleep(SAMPLE);
            }
        })?;
    Ok(())
}

struct Sensor(*mut bsp::bmi270_handle_t);

// SAFETY: the handle is only used from the imu thread; the I2C master driver
// serialises access to the shared bus.
unsafe impl Send for Sensor {}

impl Sensor {
    fn open() -> Result<Self> {
        // SAFETY: the board's bus is set up by board::init; the driver keeps
        // its own copy of the configs.
        unsafe {
            let bus = bsp::bsp_i2c_get_handle();
            ensure!(!bus.is_null(), "I2C bus not initialised");
            let mut cfg: bsp::bmi270_driver_config_t = core::mem::zeroed();
            cfg.addr = bsp::BMI270_I2C_ADDRESS_L as u8;
            cfg.interface = bsp::bmi270_interface_e_BMI270_USE_I2C;
            cfg.__bindgen_anon_1.i2c_bus = bus;
            let mut handle = core::ptr::null_mut();
            esp!(bsp::bmi270_create(&cfg, &mut handle))?;
            let run = bsp::bmi270_config_t {
                acce_odr: bsp::bmi270_acce_odr_e_BMI270_ACC_ODR_25_HZ,
                acce_range: bsp::bmi270_acce_range_e_BMI270_ACC_RANGE_2_G,
                gyro_odr: bsp::bmi270_gyro_odr_e_BMI270_GYR_ODR_25_HZ,
                gyro_range: bsp::bmi270_gyro_range_e_BMI270_GYR_RANGE_125_DPS,
            };
            esp!(bsp::bmi270_start(handle, &run))?;
            Ok(Self(handle))
        }
    }

    /// Gravity (x, y, z) in g.
    fn read(&self) -> Result<[f32; 3]> {
        let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
        // SAFETY: live handle; plain out-parameters.
        esp!(unsafe { bsp::bmi270_get_acce_data(self.0, &mut x, &mut y, &mut z) })?;
        Ok([x, y, z])
    }
}
