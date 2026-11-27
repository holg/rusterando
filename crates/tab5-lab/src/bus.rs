//! The M5-Bus on the Tab5's back: bus 5 V, the SPI bus, and which pins the
//! LAN module uses.
//!
//! Tab5 M5-Bus → ESP32-P4 (Tab5 schematic, "BUS same as CORE2"):
//!
//! | bus pin | GPIO | used by                                    |
//! |---------|------|--------------------------------------------|
//! | 7       | 18   | SPI MOSI                                   |
//! | 9       | 19   | SPI MISO                                   |
//! | 11      | 5    | SPI SCK                                    |
//! | 20      | 4    | LAN CS (jumper, as delivered)              |
//! | 23      | 47   | LAN CS (jumper)                            |
//! | 24      | 35   | LAN RST (jumper, as delivered) — boot strap |
//! | 22      | 48   | LAN RST (jumper)                           |
//! | 2, 26   | 16, 51 | LAN INT (not used: the W5500 is polled)  |
//! | 28      | 5 V  | SYS_EXT5VO, switched by expander 0x43 P2   |
//!
//! Module13.2 LAN (W5500) chooses CS/RST/INT with jumper caps. The positions
//! aren't known here, so [`probe`] finds them: both CS positions are asked
//! for the W5500's version register, and holding a RST position low tells
//! which reset line silences it. The LoRaWAN module's UART lines can share
//! some of these bus pins (rak.rs), so only the LAN module's own positions
//! are ever driven.

use std::borrow::Borrow;
use std::time::Duration;

use anyhow::{bail, Result};
use esp_idf_svc::hal::gpio::{Gpio18, Gpio19, Gpio5};
use esp_idf_svc::hal::spi::{config, Dma, SpiDeviceDriver, SpiDriver, SPI2};
use esp_idf_svc::hal::units::FromValueType;
use esp_idf_svc::sys::{self, bsp, esp};

/// E1 (0x43) P2: the load switch for SYS_EXT5VO (M5-Bus 5 V, EXT header 5 V).
const EXT5V_EN: u32 = bsp::esp_io_expander_pin_num_t_IO_EXPANDER_PIN_NUM_2;

/// The LAN module's CS jumper positions: bus 20, bus 23.
const CS_CANDIDATES: [i32; 2] = [4, 47];
/// Its RST jumper positions: bus 24 (as delivered) first, then bus 22 —
/// where the LoRaWAN module's TX may sit (DIP 8): tried only if needed.
const RST_CANDIDATES: [i32; 2] = [35, 48];

/// The M5-Bus pin number of a GPIO, for messages.
pub fn bus_pin(gpio: i32) -> u8 {
    match gpio {
        18 => 7,
        19 => 9,
        5 => 11,
        4 => 20,
        2 => 21,
        48 => 22,
        47 => 23,
        35 => 24,
        45 => 8,
        16 => 2,
        51 => 26,
        17 => 4,
        52 => 10,
        7 => 15,
        6 => 16,
        3 => 19,
        _ => 0,
    }
}

/// Switch the bus 5 V off (after [`ext5v_on`]): every module and the
/// keyboard lose power.
pub fn ext5v_off() -> Result<()> {
    // SAFETY: the cached expander handle; only the P2 line is touched.
    unsafe {
        let e1 = bsp::bsp_io_expander_init();
        if e1.is_null() {
            bail!("I/O expander 0x43 not found");
        }
        esp!(bsp::esp_io_expander_set_level(e1, EXT5V_EN, 0))?;
    }
    Ok(())
}

/// Switch on the bus 5 V (SYS_EXT5VO): the modules' regulators and the
/// keyboard header hang on it. The kitchen firmware leaves it off.
pub fn ext5v_on() -> Result<()> {
    // SAFETY: the expander was created by board::init (cached handle); only
    // the P2 line is touched. Push-pull is needed: after the board
    // package's reset every line is high-Z with a pull-down.
    unsafe {
        let e1 = bsp::bsp_io_expander_init();
        if e1.is_null() {
            bail!("I/O expander 0x43 not found");
        }
        esp!(bsp::esp_io_expander_set_level(e1, EXT5V_EN, 1))?;
        esp!(bsp::esp_io_expander_set_dir(
            e1,
            EXT5V_EN,
            bsp::esp_io_expander_dir_t_IO_EXPANDER_OUTPUT
        ))?;
        esp!(bsp::esp_io_expander_set_output_mode(
            e1,
            EXT5V_EN,
            bsp::esp_io_expander_output_mode_t_IO_EXPANDER_OUTPUT_MODE_PUSH_PULL
        ))?;
    }
    // The W5500's regulator and PLL.
    std::thread::sleep(Duration::from_millis(50));
    Ok(())
}

/// The M5-Bus SPI, shared for the life of the program (the probe, then the
/// LAN driver's thread).
#[derive(Clone, Copy)]
pub struct SharedSpi(&'static SpiDriver<'static>);

// SAFETY: ESP-IDF's SPI master driver locks the bus for every device
// transaction; the Rust bus driver only hands its host id to the devices.
unsafe impl Send for SharedSpi {}
unsafe impl Sync for SharedSpi {}

impl Borrow<SpiDriver<'static>> for SharedSpi {
    fn borrow(&self) -> &SpiDriver<'static> {
        self.0
    }
}

/// A device on the M5-Bus SPI.
pub type Device = SpiDeviceDriver<'static, SharedSpi>;

/// SPI2 on the bus pins, DMA for the W5500's Ethernet frames.
pub fn spi(
    spi2: SPI2<'static>,
    sck: Gpio5<'static>,
    mosi: Gpio18<'static>,
    miso: Gpio19<'static>,
) -> Result<SharedSpi> {
    let cfg = config::DriverConfig::new().dma(Dma::Auto(4096));
    let driver = SpiDriver::new(spi2, sck, mosi, Some(miso), &cfg)?;
    Ok(SharedSpi(Box::leak(Box::new(driver))))
}

/// The chip found on the bus.
#[derive(Clone, Copy, Debug)]
pub struct Chip {
    pub cs: i32,
    pub rst: Option<i32>,
    pub version: u8,
}

impl Chip {
    pub fn describe(&self) -> String {
        let rst = match self.rst {
            Some(r) => format!("RST GPIO{r} (bus {})", bus_pin(r)),
            None => "RST not found".into(),
        };
        format!("CS GPIO{} (bus {}), {rst}", self.cs, bus_pin(self.cs))
    }
}

#[derive(Default, Debug)]
pub struct Found {
    pub w5500: Option<Chip>,
    /// Notes for the screen: conflicts, odd answers.
    pub notes: Vec<String>,
    /// What each position answered, for the log.
    pub raw: Vec<String>,
}

/// Find the LAN module. Leaves every candidate pin reset (input), the W5500
/// out of reset.
///
/// Only the pin being asked is ever driven; the other candidates idle as
/// inputs with a pull-up.
pub fn probe(spi: SharedSpi) -> Result<Found> {
    // CS by hand: one device without CS, the candidates as GPIOs.
    let dev_cfg = config::Config::new()
        .baudrate(1.MHz().into())
        .data_mode(config::MODE_0);
    let mut dev = SpiDeviceDriver::new(
        spi,
        Option::<esp_idf_svc::hal::gpio::AnyOutputPin>::None,
        &dev_cfg,
    )?;
    // SAFETY: plain GPIO configuration of pins nothing else uses yet.
    unsafe {
        // MISO pulled up: nothing selected reads 0xFF, not noise.
        sys::gpio_set_pull_mode(19, sys::gpio_pull_mode_t_GPIO_PULLUP_ONLY);
        // Idle select lines: let go, pulled up (the LAN module has its own
        // pull-up on CSn too).
        for pin in CS_CANDIDATES {
            sys::gpio_reset_pin(pin);
            release(pin);
        }
    }

    let mut found = Found::default();
    for cs in CS_CANDIDATES {
        let w = w5500_version(&mut dev, cs)?;
        found.raw.push(format!(
            "bus: GPIO{cs} (bus {}): W5500 version 0x{w:02x}",
            bus_pin(cs)
        ));
        if w != 0x04 {
            continue;
        }
        match found.w5500 {
            Some(prev) => found.notes.push(format!(
                "W5500 answers on GPIO{} and GPIO{cs}: two LAN CS jumpers?",
                prev.cs
            )),
            None => {
                found.w5500 = Some(Chip {
                    cs,
                    rst: None,
                    version: w,
                })
            }
        }
    }

    if let Some(chip) = found.w5500.as_mut() {
        chip.rst = find_reset(&mut dev, chip.cs)?;
    }

    drop(dev);
    // Hand the pins back: the drivers configure their own.
    for pin in CS_CANDIDATES.iter().chain(RST_CANDIDATES.iter()) {
        // SAFETY: plain GPIO reset.
        unsafe { sys::gpio_reset_pin(*pin) };
    }
    Ok(found)
}

/// The reset position that silences the W5500 on `cs` while held low.
fn find_reset(dev: &mut Device, cs: i32) -> Result<Option<i32>> {
    for rst in RST_CANDIDATES {
        // SAFETY: plain GPIO output on a candidate pin.
        unsafe { output(rst, 0) };
        std::thread::sleep(Duration::from_millis(1));
        let silenced = w5500_version(dev, cs)? != 0x04;
        let_go(rst);
        // PLL lock after a reset: 50 ms.
        std::thread::sleep(Duration::from_millis(60));
        if silenced {
            return Ok(Some(rst));
        }
    }
    Ok(None)
}

/// W5500 VERSIONR (common register 0x0039): 0x04. Frame: address (16 bit),
/// control (block 0, read, variable length), data.
fn w5500_version(dev: &mut Device, cs: i32) -> Result<u8> {
    let mut buf = [0x00, 0x39, 0x00, 0x00];
    selected(cs, || dev.transfer_in_place(&mut buf))?;
    Ok(buf[3])
}

/// Drive `cs` low around `f`, then let it go again.
fn selected<T>(cs: i32, f: impl FnOnce() -> Result<T, sys::EspError>) -> Result<T> {
    // SAFETY: a candidate pin no driver owns during the probe.
    unsafe { output(cs, 0) };
    let r = f();
    let_go(cs);
    Ok(r?)
}

/// Back to idle after driving `pin`: briefly high for a clean rising edge
/// (the W5500 ends a frame on it), then let go.
fn let_go(pin: i32) {
    // SAFETY: plain GPIO level on a candidate pin.
    unsafe { sys::gpio_set_level(pin, 1) };
    release(pin);
}

/// # Safety
/// `pin` must be a valid GPIO nothing else drives.
unsafe fn output(pin: i32, level: u32) {
    sys::gpio_reset_pin(pin);
    sys::gpio_set_level(pin, level);
    sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_OUTPUT);
}

/// Is something driving `pin` high? Read against the internal pull-down:
/// a UART module's TX idling, or a reset line with a strong pull-up (the
/// W5500 module's 10 kΩ). Left reset (input off, pulled up) afterwards.
pub fn driven_high(pin: i32) -> bool {
    // SAFETY: plain GPIO configuration of a free bus pin.
    unsafe {
        sys::gpio_reset_pin(pin);
        sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_INPUT);
        sys::gpio_set_pull_mode(pin, sys::gpio_pull_mode_t_GPIO_PULLDOWN_ONLY);
        sys::esp_rom_delay_us(200);
        let high = sys::gpio_get_level(pin) == 1;
        sys::gpio_reset_pin(pin);
        high
    }
}

/// Let a line go: input with pull-up (the module pulls up as well).
fn release(pin: i32) {
    // SAFETY: plain GPIO configuration of a candidate pin.
    unsafe {
        sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_INPUT);
        sys::gpio_set_pull_mode(pin, sys::gpio_pull_mode_t_GPIO_PULLUP_ONLY);
    }
}
