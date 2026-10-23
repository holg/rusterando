//! The Tab5's 5" MIPI-DSI panel through Espressif's board package: it detects
//! which of the three panel variants is fitted, sets up DSI and the backlight.
//!
//! The panel is 720×1280 portrait, RGB565. Frames are drawn whole from a
//! buffer in PSRAM; the board package copies them into the panel's own frame
//! buffer with the 2D-DMA engine.

use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::{ensure, Result};
use esp_idf_svc::sys::{bsp, esp};

pub const WIDTH: usize = bsp::BSP_LCD_H_RES as usize;
pub const HEIGHT: usize = bsp::BSP_LCD_V_RES as usize;

/// Backlight in normal use (%). The backlight is the biggest consumer on
/// battery: measured on the Tab5 at 7.2 V, 100 % → 280 mA, 60 % → 207–210 mA,
/// 15 % → 151–163 mA (the rest is processor, Wi-Fi and panel). Clocking the
/// processor down would save little (60 % at 180 MHz: 198 mA) and stops the
/// picture stream to the panel, so it stays at 360 MHz (sdkconfig.defaults).
pub const BRIGHTNESS: i32 = 60;
/// Backlight after a while without anything happening (%).
pub const DIMMED: i32 = 15;

static CURRENT: AtomicI32 = AtomicI32::new(-1);

/// Set the backlight (%); a no-op if it's already there.
pub fn set_brightness(percent: i32) {
    if CURRENT.swap(percent, Ordering::Relaxed) != percent {
        // SAFETY: LEDC duty update; the backlight is set up by Display::new.
        if let Err(e) = esp!(unsafe { bsp::bsp_display_brightness_set(percent) }) {
            log::warn!("display: brightness: {e}");
        }
    }
}


pub struct Display {
    panel: bsp::esp_lcd_panel_handle_t,
}

impl Display {
    /// Bring up panel and backlight. Needs `board::init` first (I2C bus).
    pub fn new() -> Result<Self> {
        // SAFETY: plain C config struct, fully set below.
        let mut cfg: bsp::bsp_display_config_t = unsafe { core::mem::zeroed() };
        cfg.dsi_bus.phy_clk_src = 0; // driver default
        cfg.dsi_bus.lane_bit_rate_mbps = bsp::BSP_LCD_MIPI_DSI_LANE_BITRATE_MBPS;
        let mut panel = core::ptr::null_mut();
        let mut io = core::ptr::null_mut();
        // SAFETY: out-parameters receive valid handles on success.
        unsafe {
            esp!(bsp::bsp_display_new(&cfg, &mut panel, &mut io))?;
            esp!(bsp::esp_lcd_panel_disp_on_off(panel, true))?;
        }
        set_brightness(BRIGHTNESS);
        log::info!("display: {WIDTH}x{HEIGHT} up, backlight {BRIGHTNESS} %");
        Ok(Self { panel })
    }

    /// Show a full frame: RGB565, `WIDTH`×`HEIGHT`, row-major. The buffer
    /// must stay untouched until the next call (the copy runs on DMA).
    pub fn draw(&self, frame: &[u16]) -> Result<()> {
        ensure!(frame.len() == WIDTH * HEIGHT, "frame has {} pixels", frame.len());
        // SAFETY: panel is live; frame covers the whole area passed.
        esp!(unsafe {
            bsp::esp_lcd_panel_draw_bitmap(
                self.panel,
                0,
                0,
                WIDTH as i32,
                HEIGHT as i32,
                frame.as_ptr().cast(),
            )
        })?;
        Ok(())
    }
}
