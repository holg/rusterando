//! Tab5 board setup through Espressif's board package (m5stack_tab5_noglib):
//! the internal I2C bus and the two PI4IOE5V6408 I/O expanders.
//!
//! Pin map (M5Stack docs, board package, M5Unified):
//!   0x43: P1 speaker, P4 LCD, P5 touch, P6 camera
//!   0x44: P0 Wi-Fi coprocessor (ESP32-C6) power, P3 USB 5 V,
//!         P4 power-off pulse, P5 nCHG_QC_EN (low = fast charging),
//!         P6 CHG_STAT (input), P7 CHG_EN (battery charging)
//!
//! The board package resets an expander the first time it touches it. On
//! 0x44 that switches the Wi-Fi coprocessor and battery charging off, so this
//! runs before Wi-Fi starts and switches both back on, charging the way
//! M5Stack's own firmware does (CHG_EN high, nCHG_QC_EN low). With
//! nCHG_QC_EN left floating the charger stays in its slow mode, which is
//! about what the running device draws: on power for hours, no charge.
//!
//! The package uses ESP-IDF's new I2C master API; esp-idf-hal still links the
//! legacy driver, hence CONFIG_I2C_SKIP_LEGACY_CONFLICT_CHECK in
//! sdkconfig.defaults. Never use esp-idf-hal's `I2cDriver` on this bus.

use std::time::Duration;

use anyhow::{bail, Result};
use esp_idf_svc::sys::{self, bsp, esp};

/// E2.P7: battery charging enable.
const CHARGE_EN: u32 = bsp::esp_io_expander_pin_num_t_IO_EXPANDER_PIN_NUM_7;
/// E2.P5: fast charging, active low.
const FAST_CHARGE_N: u32 = bsp::esp_io_expander_pin_num_t_IO_EXPANDER_PIN_NUM_5;
/// E2.P6: the charger's status line (input).
const CHARGE_STAT: u32 = bsp::esp_io_expander_pin_num_t_IO_EXPANDER_PIN_NUM_6;
/// E2.P4: pulsing it switches the Tab5 off (M5Unified's powerOff).
const POWER_OFF_PULSE: u32 = bsp::esp_io_expander_pin_num_t_IO_EXPANDER_PIN_NUM_4;

pub fn init() -> Result<()> {
    // SAFETY: plain board-package calls, made once from the main task before
    // anything else uses the bus.
    unsafe {
        esp!(bsp::bsp_i2c_init())?;
        // Create both expanders here, before anything else touches them.
        expander(|| bsp::bsp_io_expander_init(), "0x43")?;
        let e2 = expander(|| bsp::bsp_io_expander1_init(), "0x44")?;
        esp!(bsp::bsp_feature_enable(bsp::bsp_feature_t_BSP_FEATURE_WIFI, true))?;
        esp!(bsp::esp_io_expander_set_level(e2, CHARGE_EN, 1))?;
        esp!(bsp::esp_io_expander_set_level(e2, FAST_CHARGE_N, 0))?;
        esp!(bsp::esp_io_expander_set_dir(
            e2,
            CHARGE_EN | FAST_CHARGE_N,
            bsp::esp_io_expander_dir_t_IO_EXPANDER_OUTPUT
        ))?;
        // The expander comes out of its reset with every output in high
        // impedance (pulled down): without this the lines are never driven
        // and the charger stays off. The board package does the same for its
        // own lines.
        esp!(bsp::esp_io_expander_set_output_mode(
            e2,
            CHARGE_EN | FAST_CHARGE_N,
            bsp::esp_io_expander_output_mode_t_IO_EXPANDER_OUTPUT_MODE_PUSH_PULL
        ))?;
        esp!(bsp::esp_io_expander_set_dir(
            e2,
            CHARGE_STAT,
            bsp::esp_io_expander_dir_t_IO_EXPANDER_INPUT
        ))?;
    }
    log::info!("board: I2C up, Wi-Fi coprocessor powered, battery charging on (fast), charger status line {}", charge_stat());
    Ok(())
}

/// An I/O expander from the board package, created (and reset) on the first
/// call and cached after that. The reset command is sometimes not
/// acknowledged — the chip resets in the middle of the transfer — and the
/// package then has no expander: the display start-up crashed on exactly
/// that. A failed attempt leaves nothing behind, so try again.
fn expander(
    init: impl Fn() -> bsp::esp_io_expander_handle_t,
    name: &str,
) -> Result<bsp::esp_io_expander_handle_t> {
    for attempt in 1..=5 {
        let handle = init();
        if !handle.is_null() {
            if attempt > 1 {
                log::warn!("board: I/O expander {name} up on attempt {attempt}");
            }
            return Ok(handle);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    bail!("I/O expander {name} not found")
}

/// Switch the Tab5 off: ten edges on the power-off line, 50 ms apart, as
/// M5Stack's firmware does. Returns only if the board stays powered.
pub fn power_off() {
    log::warn!("board: switching off");
    // SAFETY: the expander was set up in init(); only the power-off line is
    // touched.
    unsafe {
        let e2 = bsp::bsp_io_expander1_init();
        if e2.is_null() {
            return;
        }
        bsp::esp_io_expander_set_level(e2, POWER_OFF_PULSE, 0);
        bsp::esp_io_expander_set_dir(e2, POWER_OFF_PULSE, bsp::esp_io_expander_dir_t_IO_EXPANDER_OUTPUT);
        bsp::esp_io_expander_set_output_mode(
            e2,
            POWER_OFF_PULSE,
            bsp::esp_io_expander_output_mode_t_IO_EXPANDER_OUTPUT_MODE_PUSH_PULL,
        );
        for i in 0..10u8 {
            bsp::esp_io_expander_set_level(e2, POWER_OFF_PULSE, i & 1);
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    std::thread::sleep(Duration::from_secs(2));
}

/// This device's name on the server and on the Start page: `KITCHEN_SLUG`
/// if built with one, else `tab5-` and the last three bytes of the chip's
/// MAC (`tab5-e2d6d8`), so one image fits every Tab5.
pub fn device_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        if let Some(slug) = option_env!("KITCHEN_SLUG") {
            return slug.to_string();
        }
        let mut mac = [0u8; 6];
        // SAFETY: the function fills the 6-byte buffer.
        if unsafe { sys::esp_efuse_mac_get_default(mac.as_mut_ptr()) } != 0 {
            return "tab5".to_string();
        }
        format!("tab5-{:02x}{:02x}{:02x}", mac[3], mac[4], mac[5])
    })
}

/// Battery charging on/off (E2.P7) and fast charging on/off (E2.P5,
/// active low). Both lines are push-pull outputs since init().
pub fn set_charging(enable: bool, fast: bool) {
    // SAFETY: the expander was set up in init().
    unsafe {
        let e2 = bsp::bsp_io_expander1_init();
        if e2.is_null() {
            return;
        }
        bsp::esp_io_expander_set_level(e2, FAST_CHARGE_N, u8::from(!fast));
        bsp::esp_io_expander_set_level(e2, CHARGE_EN, u8::from(enable));
    }
}

/// The charger's status line (E2.P6): 1 while charging.
pub fn charge_stat() -> u8 {
    let mut level = 0u32;
    // SAFETY: the expander was set up in init(); read-only query.
    unsafe {
        let e2 = bsp::bsp_io_expander1_init();
        if e2.is_null() || bsp::esp_io_expander_get_level(e2, CHARGE_STAT, &mut level) != 0 {
            return 2;
        }
    }
    u8::from(level != 0)
}
