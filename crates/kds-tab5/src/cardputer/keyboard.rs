//! The Cardputer ADV's keyboard: 56 keys (4 × 14) behind a TCA8418 key-scan
//! chip on the internal I2C bus (SDA G8, SCL G9, interrupt G11, address
//! 0x34), wired as a 7 × 8 matrix. The chip scans by itself and queues up to
//! ten events; [`Keyboard::keys`] drains the queue and turns presses into
//! [`Key`]s, with Shift, Fn, Ctrl, Opt and Alt held as modifiers.
//!
//! Init, matrix remap and key table as M5's own library (M5Cardputer,
//! Adafruit_TCA8418 + Keyboard.h).

use anyhow::Result;
use esp_idf_svc::hal::i2c::I2cDriver;

const ADDR: u8 = 0x34;
const REG_CFG: u8 = 0x01;
const REG_INT_STAT: u8 = 0x02;
const REG_KEY_LCK_EC: u8 = 0x03;
const REG_KEY_EVENT_A: u8 = 0x04;
const REG_KP_GPIO1: u8 = 0x1D;
const REG_KP_GPIO2: u8 = 0x1E;
const REG_KP_GPIO3: u8 = 0x1F;
/// I2C timeout in RTOS ticks (1 ms each).
const TIMEOUT: u32 = 20;

/// What a key press means, modifiers applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    /// Fn + Backspace.
    Delete,
    Tab,
    /// Fn + `.
    Esc,
    /// Fn + ; , . / (the arrows printed on those keys).
    Up,
    Down,
    Left,
    Right,
    /// Ctrl + a key (the shell: Ctrl+C, Ctrl+L, …), lower case.
    Ctrl(char),
    /// Alt or Opt + a key.
    Alt(char),
}

/// A key press with the modifiers held at that moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Press {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    pub opt: bool,
}

#[derive(Clone, Copy)]
enum K {
    /// Normal and shifted character.
    C(char, char),
    Backspace,
    Tab,
    Fn,
    Shift,
    Enter,
    Ctrl,
    Opt,
    Alt,
}
use K::*;

/// The printed layout, row by row (M5Cardputer's `_key_value_map`).
const MAP: [[K; 14]; 4] = [
    [
        C('`', '~'), C('1', '!'), C('2', '@'), C('3', '#'), C('4', '$'), C('5', '%'), C('6', '^'),
        C('7', '&'), C('8', '*'), C('9', '('), C('0', ')'), C('-', '_'), C('=', '+'), Backspace,
    ],
    [
        Tab, C('q', 'Q'), C('w', 'W'), C('e', 'E'), C('r', 'R'), C('t', 'T'), C('y', 'Y'),
        C('u', 'U'), C('i', 'I'), C('o', 'O'), C('p', 'P'), C('[', '{'), C(']', '}'), C('\\', '|'),
    ],
    [
        Fn, Shift, C('a', 'A'), C('s', 'S'), C('d', 'D'), C('f', 'F'), C('g', 'G'),
        C('h', 'H'), C('j', 'J'), C('k', 'K'), C('l', 'L'), C(';', ':'), C('\'', '"'), Enter,
    ],
    [
        Ctrl, Opt, Alt, C('z', 'Z'), C('x', 'X'), C('c', 'C'), C('v', 'V'),
        C('b', 'B'), C('n', 'N'), C('m', 'M'), C(',', '<'), C('.', '>'), C('/', '?'), C(' ', ' '),
    ],
];

pub struct Keyboard {
    i2c: I2cDriver<'static>,
    shift: bool,
    func: bool,
    ctrl: bool,
    opt: bool,
    alt: bool,
}

impl Keyboard {
    pub fn new(i2c: I2cDriver<'static>) -> Result<Self> {
        let mut k = Keyboard { i2c, shift: false, func: false, ctrl: false, opt: false, alt: false };
        // The matrix: rows 0–6, columns 0–7 (the rest isn't wired).
        k.write(REG_KP_GPIO1, 0x7F)?;
        k.write(REG_KP_GPIO2, 0xFF)?;
        k.write(REG_KP_GPIO3, 0x00)?;
        // Whatever is queued from before, then key events on the interrupt.
        while k.read(REG_KEY_EVENT_A)? != 0 {}
        k.write(REG_INT_STAT, 0x03)?;
        let cfg = k.read(REG_CFG)?;
        k.write(REG_CFG, cfg | 0x03)?;
        Ok(k)
    }

    fn write(&mut self, reg: u8, val: u8) -> Result<()> {
        self.i2c.write(ADDR, &[reg, val], TIMEOUT)?;
        Ok(())
    }

    fn read(&mut self, reg: u8) -> Result<u8> {
        let mut b = [0u8];
        self.i2c.write_read(ADDR, &[reg], &mut b, TIMEOUT)?;
        Ok(b[0])
    }

    /// The key presses since the last call, oldest first. Modifier keys only
    /// change state; releases end them.
    pub fn keys(&mut self) -> Result<Vec<Press>> {
        let mut out = Vec::new();
        let count = self.read(REG_KEY_LCK_EC)? & 0x0F;
        for _ in 0..count {
            let ev = self.read(REG_KEY_EVENT_A)?;
            if ev == 0 {
                break;
            }
            let pressed = ev & 0x80 != 0;
            let n = (ev & 0x7F).saturating_sub(1);
            let (row, col) = (n / 10, n % 10);
            // The chip's 7 × 8 matrix onto the printed 4 × 14 layout.
            let x = row as usize * 2 + usize::from(col > 3);
            let y = (col % 4) as usize;
            let Some(k) = MAP.get(y).and_then(|r| r.get(x)).copied() else {
                log::warn!("keyboard: event {ev:#04x} outside the layout");
                continue;
            };
            match k {
                Shift => self.shift = pressed,
                Fn => self.func = pressed,
                Ctrl => self.ctrl = pressed,
                Opt => self.opt = pressed,
                Alt => self.alt = pressed,
                _ if !pressed => {}
                other => {
                    if let Some(key) = self.resolve(other) {
                        out.push(Press { key, ctrl: self.ctrl, alt: self.alt, opt: self.opt });
                    }
                }
            }
        }
        // Clear the key-event interrupt.
        self.write(REG_INT_STAT, 0x01)?;
        Ok(out)
    }

    fn resolve(&self, k: K) -> Option<Key> {
        Some(match k {
            Backspace if self.func => Key::Delete,
            Backspace => Key::Backspace,
            Tab => Key::Tab,
            Enter => Key::Enter,
            C('`', _) if self.func => Key::Esc,
            C(';', _) if self.func => Key::Up,
            C('.', _) if self.func => Key::Down,
            C(',', _) if self.func => Key::Left,
            C('/', _) if self.func => Key::Right,
            C(normal, _) if self.ctrl => Key::Ctrl(normal),
            C(normal, shifted) if self.alt || self.opt => Key::Alt(if self.shift { shifted } else { normal }),
            C(normal, shifted) => Key::Char(if self.shift { shifted } else { normal }),
            Fn | Shift | Ctrl | Opt | Alt => return None,
        })
    }
}
