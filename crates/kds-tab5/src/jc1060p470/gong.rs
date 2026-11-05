//! No gong yet on the JC1060P470: the board has an ES8311 codec and a
//! speaker socket, but no speaker came with it. The same interface as the
//! Tab5's gong.rs; a ring is only logged. New orders still flash on screen.

use anyhow::Result;

#[derive(Clone)]
pub struct Gong;

impl Gong {
    pub fn start() -> Result<Self> {
        Ok(Self)
    }

    pub fn ring(&self) {
        log::info!("gong: no speaker on this board");
    }
}
