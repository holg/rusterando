//! The Tab5's LAN module (W5500 on the M5-Bus) for the shell: the Cardputer
//! has none, so the shell is always given `None` and answers "no LAN
//! module" — these only satisfy its types.

use crate::Out;

pub struct Lan;

impl Lan {
    pub fn print_status(&self, _out: &Out) {}
    pub fn restart(&self) {}
    pub fn reset(&self) {}
}

pub struct Tcp {
    pub peer: String,
}

impl Tcp {
    pub fn connect(target: String, _out: Out) -> Tcp {
        Tcp { peer: target }
    }
    pub fn is_open(&self) -> bool {
        false
    }
    pub fn send(&self, _line: String) {}
}

pub fn ping(_host: String, _out: Out) {}

pub fn get(_url: String, _out: Out) {}
