//! The gong: the kitchen's new-order sound (the same gong.wav as the STM32
//! board and the staff app) through the Tab5's speaker — ES8388 codec and
//! power amplifier, set up by the board package (48 kHz, 16 bit, mono).
//!
//! Plays on its own thread so the screen keeps running; rings that come in
//! while it plays are merged into the one playing. The power amplifier is
//! only switched on while the gong plays (it idles on battery otherwise).

use std::sync::mpsc::{self, Sender};

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use esp_idf_svc::sys::bsp;

static GONG_WAV: &[u8] = include_bytes!("../../rusterando-printer/assets/sounds/gong.wav");
/// Speaker volume, 0–100.
const VOLUME: i32 = 95;
/// The amplifier's start-up time before sound, and the DMA tail after the
/// last write, so neither the start nor the end of the gong is cut.
const AMP_ON: Duration = Duration::from_millis(30);
const AMP_TAIL: Duration = Duration::from_millis(150);

/// Switch the speaker's power amplifier (SPK_EN on the I/O expander 0x43).
pub fn amplifier(on: bool) {
    // SAFETY: board-package feature switch on the shared I2C bus.
    let r = unsafe { bsp::bsp_feature_enable(bsp::bsp_feature_t_BSP_FEATURE_SPEAKER, on) };
    if r != 0 {
        log::warn!("gong: amplifier {}: error {r}", if on { "on" } else { "off" });
    }
}

#[derive(Clone)]
pub struct Gong(Sender<()>);

impl Gong {
    pub fn start() -> Result<Self> {
        let pcm = pcm_data(GONG_WAV)?;
        let (tx, rx) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("gong".into())
            .stack_size(8 * 1024)
            .spawn(move || {
                let speaker = match Speaker::open() {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("gong: {e:#}");
                        return;
                    }
                };
                while rx.recv().is_ok() {
                    while rx.try_recv().is_ok() {}
                    speaker.play(pcm);
                }
            })?;
        Ok(Self(tx))
    }

    pub fn ring(&self) {
        let _ = self.0.send(());
    }
}

struct Speaker(bsp::esp_codec_dev_handle_t);

impl Speaker {
    fn open() -> Result<Self> {
        // SAFETY: board-package init (I2S, codec over the shared I2C bus,
        // amplifier on); the handle lives for the program.
        unsafe {
            let dev = bsp::bsp_audio_codec_speaker_init();
            if dev.is_null() {
                bail!("speaker codec not found");
            }
            let mut fs: bsp::esp_codec_dev_sample_info_t = core::mem::zeroed();
            fs.sample_rate = 48_000;
            fs.channel = 1;
            fs.bits_per_sample = 16;
            check(bsp::esp_codec_dev_open(dev, &mut fs), "open")?;
            check(bsp::esp_codec_dev_set_out_vol(dev, VOLUME), "volume")?;
            amplifier(false);
            Ok(Self(dev))
        }
    }

    fn play(&self, pcm: &[u8]) {
        amplifier(true);
        std::thread::sleep(AMP_ON);
        for chunk in pcm.chunks(4096) {
            // SAFETY: the codec copies from the buffer before returning.
            let r = unsafe { bsp::esp_codec_dev_write(self.0, chunk.as_ptr() as *mut _, chunk.len() as i32) };
            if r != 0 {
                log::warn!("gong: write failed ({r})");
                break;
            }
        }
        std::thread::sleep(AMP_TAIL);
        amplifier(false);
    }
}

fn check(code: i32, what: &str) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(anyhow!("speaker {what}: error {code}"))
    }
}

/// The sample data of a PCM WAV file (16 bit, as gong.wav is).
fn pcm_data(wav: &'static [u8]) -> Result<&'static [u8]> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        bail!("gong.wav is not a WAV file");
    }
    let mut pos = 12;
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let len = u32::from_le_bytes([wav[pos + 4], wav[pos + 5], wav[pos + 6], wav[pos + 7]]) as usize;
        let body = pos + 8;
        if id == b"data" {
            return Ok(&wav[body..(body + len).min(wav.len())]);
        }
        pos = body + len + (len & 1);
    }
    bail!("gong.wav has no data chunk")
}
