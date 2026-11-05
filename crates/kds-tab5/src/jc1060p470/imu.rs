//! The JC1060P470 has no accelerometer: it hangs on the wall one way up.
//! The same interface as the Tab5's imu.rs, never turned over.

use anyhow::Result;

#[derive(Clone, Default)]
pub struct Orientation;

impl Orientation {
    pub fn flipped(&self) -> bool {
        false
    }
}

pub fn start(_orientation: Orientation) -> Result<()> {
    Ok(())
}
