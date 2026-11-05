//! Guition JC1060P470 board setup through its board package
//! (boards/jc1060p470/bsp): the I2C bus the touch controller sits on.
//! No I/O expanders, no battery: the board runs on its power supply.

use anyhow::Result;
use esp_idf_svc::sys::{self, bsp, esp};

/// What this device reports as its arch on the print channel: the server
/// stages one update image per arch (artifacts/printer/<arch>/), and a Tab5
/// image must never reach this board.
pub const ARCH: &str = "esp32p4-jc1060p470";
/// The device on the test receipt.
pub const MODEL: &str = "JC1060P470";

pub fn init() -> Result<()> {
    // SAFETY: plain board-package call, made once from the main task before
    // anything else uses the bus.
    esp!(unsafe { bsp::bsp_i2c_init() })?;
    log::info!("board: JC1060P470, I2C up");
    Ok(())
}

/// This device's name on the server and on the Start page: `KITCHEN_SLUG`
/// if built with one, else `jc1060-` and the last three bytes of the chip's
/// MAC (`jc1060-d33c00`), so one image fits every board.
pub fn device_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        if let Some(slug) = option_env!("KITCHEN_SLUG") {
            return slug.to_string();
        }
        let mut mac = [0u8; 6];
        // SAFETY: the function fills the 6-byte buffer.
        if unsafe { sys::esp_efuse_mac_get_default(mac.as_mut_ptr()) } != 0 {
            return "jc1060".to_string();
        }
        format!("jc1060-{:02x}{:02x}{:02x}", mac[3], mac[4], mac[5])
    })
}
