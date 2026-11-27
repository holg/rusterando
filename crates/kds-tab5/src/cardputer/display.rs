//! The Cardputer ADV's 1.14" ST7789V2 (240×135, landscape) on SPI3, as M5's
//! own driver (M5GFX, board `cardputer_adv`): RST G33, DC G34, MOSI G35,
//! SCLK G36, CS G37, backlight G38 (also the RGB LED's power). SPI2 is the
//! LoRa cap's and the SD card's.
//!
//! No frame buffer (the S3 has no PSRAM): Slint renders line by line
//! ([`Lines`]) and each line goes straight to the panel.

use std::ops::Range;

use anyhow::Result;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{AnyIOPin, Output, OutputPin, PinDriver};
use esp_idf_svc::hal::spi::{config, SpiAnyPins, SpiDeviceDriver, SpiDriver};
use esp_idf_svc::hal::units::FromValueType;
use slint::platform::software_renderer::{LineBufferProvider, Rgb565Pixel};

pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 135;

/// Where the visible 240×135 sit in the controller's 320×240 memory, in
/// landscape (MADCTL below), as M5GFX computes it.
const X_OFF: u16 = 40;
const Y_OFF: u16 = 53;
/// MADCTL: row/column exchange + column mirror (+ refresh order) =
/// landscape with the keys below; BGR panel.
const MADCTL: u8 = 0x6C;
/// M5GFX's ST7789 power and gamma settings (Panel_ST7789).
const INIT: &[(u8, &[u8])] = &[
    (0xB7, &[0x35]),
    (0xBB, &[0x28]),
    (0xC0, &[0x0C]),
    (0xC2, &[0x01, 0xFF]),
    (0xC3, &[0x10]),
    (0xC4, &[0x20]),
    (0xD0, &[0xA4, 0xA1]),
    (0xB0, &[0x00, 0xC0]),
    (0xE0, &[0xD0, 0x00, 0x02, 0x07, 0x0A, 0x28, 0x32, 0x44, 0x42, 0x06, 0x0E, 0x12, 0x14, 0x17]),
    (0xE1, &[0xD0, 0x00, 0x02, 0x07, 0x0A, 0x28, 0x31, 0x54, 0x47, 0x0E, 0x1C, 0x17, 0x1B, 0x1E]),
];

pub struct Display {
    spi: SpiDeviceDriver<'static, SpiDriver<'static>>,
    dc: PinDriver<'static, Output>,
    backlight: PinDriver<'static, Output>,
}

impl Display {
    #[allow(clippy::too_many_arguments)]
    pub fn new<S: SpiAnyPins + 'static>(
        spi: S,
        sclk: impl OutputPin + 'static,
        mosi: impl OutputPin + 'static,
        cs: impl OutputPin + 'static,
        dc: impl OutputPin + 'static,
        rst: impl OutputPin + 'static,
        backlight: impl OutputPin + 'static,
    ) -> Result<Self> {
        let mut rst = PinDriver::output(rst)?;
        let driver = SpiDriver::new(spi, sclk, mosi, None::<AnyIOPin<'static>>, &config::DriverConfig::new())?;
        let spi = SpiDeviceDriver::new(
            driver,
            Some(cs),
            &config::Config::new().baudrate(40.MHz().into()).data_mode(config::MODE_0),
        )?;
        let mut d = Display { spi, dc: PinDriver::output(dc)?, backlight: PinDriver::output(backlight)? };
        rst.set_low()?;
        FreeRtos::delay_ms(10);
        rst.set_high()?;
        FreeRtos::delay_ms(120);
        d.cmd(0x01, &[])?; // software reset
        FreeRtos::delay_ms(150);
        for (c, data) in INIT {
            d.cmd(*c, data)?;
        }
        d.cmd(0x11, &[])?; // out of sleep
        FreeRtos::delay_ms(130);
        d.cmd(0x38, &[])?; // idle mode off
        d.cmd(0x3A, &[0x55])?; // 16 bit per pixel
        d.cmd(0x36, &[MADCTL])?;
        d.cmd(0x21, &[])?; // inverted colours: this panel's normal
        d.cmd(0x13, &[])?; // normal mode
        d.clear(0)?;
        d.cmd(0x29, &[])?; // display on
        d.backlight.set_high()?;
        // Not kept: the reset pin stays high from here on.
        core::mem::forget(rst);
        Ok(d)
    }

    fn cmd(&mut self, c: u8, data: &[u8]) -> Result<()> {
        self.dc.set_low()?;
        self.spi.write(&[c])?;
        if !data.is_empty() {
            self.dc.set_high()?;
            self.spi.write(data)?;
        }
        Ok(())
    }

    /// Address a rectangle (inclusive ends) and start writing pixels to it.
    fn window(&mut self, x0: u16, y0: u16, x1: u16, y1: u16) -> Result<()> {
        let (x0, x1, y0, y1) = (x0 + X_OFF, x1 + X_OFF, y0 + Y_OFF, y1 + Y_OFF);
        self.cmd(0x2A, &[(x0 >> 8) as u8, x0 as u8, (x1 >> 8) as u8, x1 as u8])?;
        self.cmd(0x2B, &[(y0 >> 8) as u8, y0 as u8, (y1 >> 8) as u8, y1 as u8])?;
        self.cmd(0x2C, &[])?;
        self.dc.set_high()?;
        Ok(())
    }

    /// The whole screen in one colour (RGB565).
    pub fn clear(&mut self, rgb565: u16) -> Result<()> {
        self.window(0, 0, WIDTH as u16 - 1, HEIGHT as u16 - 1)?;
        let line = [(rgb565 >> 8) as u8, rgb565 as u8].repeat(WIDTH);
        for _ in 0..HEIGHT {
            self.spi.write(&line)?;
        }
        Ok(())
    }

    pub fn set_backlight(&mut self, on: bool) {
        let _ = if on { self.backlight.set_high() } else { self.backlight.set_low() };
    }
}

/// Slint's line-by-line output onto the panel.
pub struct Lines<'a> {
    pub display: &'a mut Display,
    line: [Rgb565Pixel; WIDTH],
    bytes: [u8; WIDTH * 2],
}

impl<'a> Lines<'a> {
    pub fn new(display: &'a mut Display) -> Self {
        Lines { display, line: [Rgb565Pixel(0); WIDTH], bytes: [0; WIDTH * 2] }
    }
}

impl LineBufferProvider for &mut Lines<'_> {
    type TargetPixel = Rgb565Pixel;

    fn process_line(
        &mut self,
        line: usize,
        range: Range<usize>,
        render_fn: impl FnOnce(&mut [Rgb565Pixel]),
    ) {
        render_fn(&mut self.line[range.clone()]);
        // The panel wants each pixel high byte first.
        for (i, px) in self.line[range.clone()].iter().enumerate() {
            let [lo, hi] = px.0.to_le_bytes();
            self.bytes[2 * i] = hi;
            self.bytes[2 * i + 1] = lo;
        }
        let n = range.len() * 2;
        let ok = self
            .display
            .window(range.start as u16, line as u16, range.end as u16 - 1, line as u16)
            .and_then(|_| Ok(self.display.spi.write(&self.bytes[..n])?));
        if let Err(e) = ok {
            log::warn!("display: line {line}: {e:#}");
        }
    }
}
