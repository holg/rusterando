//! M5Stack Tab5 Keyboard (SKU A164): 70 keys, an STM32F030 on I2C 0x6D at
//! the Tab5's EXT header (SDA GPIO0, SCL GPIO1, INT GPIO50 active low).
//!
//! Used in its character mode: the keyboard does Shift (Aa), Sym and the
//! key names itself and queues one record per key press — the modifier
//! byte (bit 0 Ctrl, bit 2 Alt) and the key's name: "a", "A", "?", " ",
//! or a word for the special keys ("enter", "backspace", "up", "esc", …;
//! upper case while Aa is on). Source: M5Stack's keyboard firmware
//! (github.com/m5stack/M5Tab5-Keyboard-Internal-FW, user_keyboard_handle.c)
//! and its I2C protocol sheet (Tab5_Keyboard-I2C-Protocol-EN-V1.0).
//!
//! On its own I2C bus (new master driver, any free port): the board's bus
//! (GPIO31/32) is a different one.

use std::time::{Duration, Instant};

use anyhow::Result;
use esp_idf_svc::sys::{self, esp};

use crate::terminal::Tone;
use crate::{say, Out};

const SDA: i32 = 0;
const SCL: i32 = 1;
const INT: i32 = 50;
const ADDRESS: u16 = 0x6D;

const REG_INT_STA: u8 = 0x01;
const REG_EVENT_NUM: u8 = 0x02;
const REG_BRIGHTNESS: u8 = 0x03;
const REG_MODE: u8 = 0x10;
const REG_CHAR_LEN: u8 = 0x40;
const REG_CHAR_EVENT: u8 = 0x50;
const REG_VERSION: u8 = 0xFE;

const MODE_CHAR: u8 = 2;
const INT_CHAR: u8 = 0x04;
const MOD_CTRL: u8 = 0x01;
const MOD_ALT: u8 = 0x04;

/// Read the queue even without the INT line this often (INT not wired, or
/// a missed edge).
const POLL_ANYWAY: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Alt(char),
    Enter,
    Backspace,
    Delete,
    Esc,
    Tab,
    Up,
    Down,
    Left,
    Right,
}

/// Look for the keyboard this often while it isn't there (it powers up
/// with the bus 5 V, and may be plugged in later).
const DETECT_EVERY: Duration = Duration::from_secs(1);
/// This many failed reads in a row: the keyboard is gone.
const LOST_AFTER: u8 = 5;

pub struct Keyboard {
    dev: sys::i2c_master_dev_handle_t,
    out: Out,
    last_read: Instant,
    present: bool,
    next_detect: Instant,
    failures: u8,
    pub version: u8,
}

// SAFETY: only used from the UI thread; the I2C driver locks its bus.
unsafe impl Send for Keyboard {}

impl Keyboard {
    /// Set up the EXT header's I2C bus; the keyboard itself is looked for
    /// by [`Keyboard::poll`] until it answers.
    pub fn open(out: Out) -> Result<Self> {
        // SAFETY: plain C config structs; out-parameters receive handles.
        let dev = unsafe {
            let mut bus_cfg: sys::i2c_master_bus_config_t = core::mem::zeroed();
            bus_cfg.i2c_port = -1; // any free port; the board's bus has its own
            bus_cfg.sda_io_num = SDA;
            bus_cfg.scl_io_num = SCL;
            bus_cfg.__bindgen_anon_1.clk_source = sys::soc_periph_i2c_clk_src_t_I2C_CLK_SRC_DEFAULT;
            bus_cfg.glitch_ignore_cnt = 7;
            bus_cfg.flags.set_enable_internal_pullup(1);
            let mut bus = core::ptr::null_mut();
            esp!(sys::i2c_new_master_bus(&bus_cfg, &mut bus))?;
            let mut dev_cfg: sys::i2c_device_config_t = core::mem::zeroed();
            dev_cfg.dev_addr_length = sys::i2c_addr_bit_len_t_I2C_ADDR_BIT_LEN_7;
            dev_cfg.device_address = ADDRESS;
            dev_cfg.scl_speed_hz = 100_000;
            let mut dev = core::ptr::null_mut();
            esp!(sys::i2c_master_bus_add_device(bus, &dev_cfg, &mut dev))?;

            // INT: active low, open drain on the keyboard side.
            let io = sys::gpio_config_t {
                pin_bit_mask: 1u64 << INT,
                mode: sys::gpio_mode_t_GPIO_MODE_INPUT,
                pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
                pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
                intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
                ..core::mem::zeroed()
            };
            esp!(sys::gpio_config(&io))?;
            dev
        };
        let mut kb = Self {
            dev,
            out,
            last_read: Instant::now(),
            present: false,
            next_detect: Instant::now(),
            failures: 0,
            version: 0,
        };
        // Give it a moment at start (it boots when the bus 5 V comes on).
        for _ in 0..20 {
            if kb.detect().is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(kb)
    }

    pub fn present(&self) -> bool {
        self.present
    }

    /// Ask for the firmware version, then switch to character mode
    /// (switching modes clears the queue and releases INT).
    fn detect(&mut self) -> Result<()> {
        self.version = self.read(REG_VERSION)?;
        self.write(REG_MODE, MODE_CHAR)?;
        self.write(REG_BRIGHTNESS, 20)?;
        self.present = true;
        self.failures = 0;
        say(
            &self.out,
            Tone::Ok,
            format!("keyboard: Tab5 Keyboard found, firmware v{}", self.version),
        );
        Ok(())
    }

    /// The keys pressed since the last call.
    pub fn poll(&mut self) -> Vec<Key> {
        if !self.present {
            if Instant::now() >= self.next_detect {
                self.next_detect = Instant::now() + DETECT_EVERY;
                let _ = self.detect();
            }
            return Vec::new();
        }
        // SAFETY: input pin configured in open().
        let pending = unsafe { sys::gpio_get_level(INT) } == 0;
        if !pending && self.last_read.elapsed() < POLL_ANYWAY {
            return Vec::new();
        }
        self.last_read = Instant::now();
        match self.drain() {
            Ok(keys) => {
                self.failures = 0;
                keys
            }
            Err(e) => {
                self.failures += 1;
                if self.failures >= LOST_AFTER {
                    self.present = false;
                    say(&self.out, Tone::Warn, format!("keyboard: gone ({e:#})"));
                }
                Vec::new()
            }
        }
    }

    fn drain(&mut self) -> Result<Vec<Key>> {
        let mut keys = Vec::new();
        let status = self.read(REG_INT_STA)?;
        if status & INT_CHAR == 0 {
            return Ok(keys);
        }
        let count = self.read(REG_EVENT_NUM)?.min(32);
        for _ in 0..count {
            let len = self.read(REG_CHAR_LEN)? as usize;
            if len == 0 || len > 15 {
                break;
            }
            let mut record = [0u8; 16];
            self.read_into(REG_CHAR_EVENT, &mut record[..len + 1])?;
            // The length counts the C string's terminating zero.
            let name = String::from_utf8_lossy(&record[1..len + 1])
                .trim_end_matches('\0')
                .to_string();
            if let Some(key) = decode(record[0], &name) {
                keys.push(key);
            }
        }
        // Releases the INT line.
        self.write(REG_INT_STA, 0)?;
        Ok(keys)
    }

    fn read(&self, reg: u8) -> Result<u8> {
        let mut b = [0u8];
        self.read_into(reg, &mut b)?;
        Ok(b[0])
    }

    fn read_into(&self, reg: u8, buf: &mut [u8]) -> Result<()> {
        // SAFETY: live device handle; buffers outlive the call.
        esp!(unsafe {
            sys::i2c_master_transmit_receive(self.dev, &reg, 1, buf.as_mut_ptr(), buf.len(), 50)
        })?;
        Ok(())
    }

    fn write(&self, reg: u8, value: u8) -> Result<()> {
        let data = [reg, value];
        // SAFETY: live device handle; the buffer outlives the call.
        esp!(unsafe { sys::i2c_master_transmit(self.dev, data.as_ptr(), data.len(), 50) })?;
        Ok(())
    }
}

/// A character-mode record → a key. Names are matched without case: with
/// Aa on, the keyboard sends "ENTER", "UP", "BACKSPACE".
fn decode(modifier: u8, name: &str) -> Option<Key> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(if modifier & MOD_CTRL != 0 {
            Key::Ctrl(c.to_ascii_lowercase())
        } else if modifier & MOD_ALT != 0 {
            Key::Alt(c)
        } else {
            Key::Char(c)
        });
    }
    Some(match name.to_ascii_lowercase().as_str() {
        "enter" => Key::Enter,
        "backspace" => Key::Backspace,
        "del" => Key::Delete,
        "esc" => Key::Esc,
        "tab" => Key::Tab,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        other => {
            log::info!("keyboard: unknown key name {other:?}");
            return None;
        }
    })
}
