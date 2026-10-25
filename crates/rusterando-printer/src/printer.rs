use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use escpos::driver::{Driver, FileDriver};
use kitchen_protocol::bitmap::Bitmap;
use kitchen_protocol::receipt::ReceiptProgram;

use crate::usb::{UsbDriver, UsbSelector};

/// Where the ESC/POS bytes go: a device file (`/dev/usb/lp0` via the
/// kernel `usblp` driver, or a plain file as a mock printer) or the printer's
/// USB endpoint directly (`PRINTER_PATH=usb[:vid:pid]`, for kernels without
/// `usblp` such as the STM32MP135F-DK's).
#[derive(Debug, Clone)]
enum Target {
    File(PathBuf),
    Usb(UsbSelector),
    /// `PRINTER_PATH=none`: display-only terminal. The client never prints
    /// or acks, so another printer on the same shop channel (the Pi during
    /// a rollout) keeps every receipt.
    None,
}

/// `PRINTER_PATH` value for a display-only terminal.
pub const DISPLAY_ONLY: &str = "none";

/// Thin wrapper around the printer device. Opens it per print rather than
/// holding it open: `usblp` reclaims the device cleanly on detach/reattach
/// and the USB driver re-enumerates, so re-opening per receipt makes us
/// resilient to the printer being powered off and back on mid-shift.
pub struct Printer {
    target: Target,
    /// Serialize prints so we never interleave two receipts.
    write_lock: Mutex<()>,
}

impl Printer {
    /// Open and probe: fails fast at startup if the device isn't there
    /// (printer off, cable unplugged). The headless printer's choice —
    /// systemd restarts it until the printer shows up.
    pub fn open(path: &Path) -> Result<Self> {
        let p = Self::new(path);
        p.probe()?;
        Ok(p)
    }

    /// No probe: the kitchen display must come up (and keep showing orders)
    /// with the printer off. Prints fail and are retried until it's back.
    pub fn new(path: &Path) -> Self {
        let target = match path.to_str() {
            Some(DISPLAY_ONLY) => Target::None,
            Some(spec) => match UsbSelector::parse(spec) {
                Some(sel) => Target::Usb(sel),
                None => Target::File(path.to_path_buf()),
            },
            None => Target::File(path.to_path_buf()),
        };
        Self {
            target,
            write_lock: Mutex::new(()),
        }
    }

    /// Open the device once and drop it.
    pub fn probe(&self) -> Result<()> {
        if self.is_display_only() {
            return Ok(());
        }
        AnyDriver::open(&self.target).map(drop).context("probe printer")
    }

    pub fn is_display_only(&self) -> bool {
        matches!(self.target, Target::None)
    }

    /// Human-readable target for logs and the display's status line.
    pub fn describe(&self) -> String {
        match &self.target {
            Target::File(p) => p.display().to_string(),
            Target::Usb(sel) => sel.to_string(),
            Target::None => DISPLAY_ONLY.to_string(),
        }
    }

    /// Print a fully-composed [`ReceiptProgram`]. The server (the
    /// Bon-Designer) already resolved the theme and inlined every logo; the
    /// bytes come from the shared renderer (`receipt-escpos`, the same the
    /// ESP32-P4 displays use), this only resolves logos and writes them.
    pub fn print_program(
        &self,
        program: &ReceiptProgram,
        cache: &mut crate::raster_cache::RasterCache,
    ) -> Result<()> {
        let _guard = self.write_lock.lock().unwrap();
        let driver = AnyDriver::open(&self.target)?;
        let bytes = receipt_escpos::render(program, &mut |hash, target_width, svg| {
            resolve_logo(cache, hash, target_width, svg)
        })?;
        driver.write(&bytes).context("write ESC/POS")?;
        driver.flush().context("flush ESC/POS")?;
        Ok(())
    }
}

/// A logo's 1-bit raster: cache hit → use it; miss with SVG bytes →
/// rasterize + cache; miss with an empty svg (a server reference we somehow
/// don't have) → skip the logo. The text banners still convey the channel,
/// and the server re-sends the bytes on the next reconnect (fresh Hello
/// advertises our cache).
fn resolve_logo(
    cache: &mut crate::raster_cache::RasterCache,
    hash: &[u8; 32],
    target_width: u16,
    svg: &[u8],
) -> Option<Bitmap> {
    if let Some(bm) = cache.get(hash) {
        return Some(bm);
    }
    if svg.is_empty() {
        tracing::warn!("logo cache miss with no SVG bytes — skipping (will re-fetch on reconnect)");
        return None;
    }
    match receipt_escpos::svg::rasterize(svg, target_width) {
        Ok(bm) => {
            cache.put(*hash, &bm);
            Some(bm)
        }
        Err(e) => {
            tracing::warn!(error = %e, "logo SVG rasterize failed — skipping logo");
            None
        }
    }
}

/// The concrete escpos driver for one print.
enum AnyDriver {
    File(FileDriver),
    Usb(UsbDriver),
}

impl AnyDriver {
    fn open(target: &Target) -> Result<Self> {
        Ok(match target {
            Target::File(path) => Self::File(
                FileDriver::open(path).with_context(|| format!("open {}", path.display()))?,
            ),
            Target::Usb(sel) => Self::Usb(UsbDriver::open(*sel)?),
            Target::None => anyhow::bail!("display-only terminal has no printer"),
        })
    }
}

impl Driver for AnyDriver {
    fn name(&self) -> String {
        match self {
            Self::File(d) => d.name(),
            Self::Usb(d) => d.name(),
        }
    }
    fn write(&self, data: &[u8]) -> escpos::errors::Result<()> {
        match self {
            Self::File(d) => d.write(data),
            Self::Usb(d) => d.write(data),
        }
    }
    fn read(&self, buf: &mut [u8]) -> escpos::errors::Result<usize> {
        match self {
            Self::File(d) => d.read(buf),
            Self::Usb(d) => d.read(buf),
        }
    }
    fn flush(&self) -> escpos::errors::Result<()> {
        match self {
            Self::File(d) => d.flush(),
            Self::Usb(d) => d.flush(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kitchen_protocol::receipt::{LineStyle, ReceiptLine, ReceiptProgram};

    /// What the board writes to its printer is exactly the shared renderer's
    /// output (the ESP32-P4 displays print the same bytes).
    #[test]
    fn file_target_writes_the_shared_rendering() {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-tmp"));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("receipt.bin");
        let _ = std::fs::remove_file(&out);
        std::fs::write(&out, b"").unwrap();

        let big = LineStyle { center: true, bold: true, width: 2, height: 2 };
        let program = ReceiptProgram {
            display_number: 1234,
            is_reprint: false,
            lines: vec![
                ReceiptLine::Text { text: "Bestellung 1234".into(), style: big },
                ReceiptLine::Feed { n: 1 },
                ReceiptLine::Text { text: "2× Pizza Käse 7,50 €".into(), style: LineStyle::NORMAL },
                ReceiptLine::Text { text: "  ohne Zwiebeln, Größe L".into(), style: LineStyle::NORMAL },
                ReceiptLine::Rule,
                ReceiptLine::Qr { url: "https://example.org/o/1234".into() },
                ReceiptLine::Cut { feed: 4 },
            ],
        };

        let mut cache = crate::raster_cache::RasterCache::load(&dir).unwrap();
        Printer::new(&out).print_program(&program, &mut cache).unwrap();
        let written = std::fs::read(&out).unwrap();
        let shared = receipt_escpos::render(&program, &mut |_, _, _| None).unwrap();
        assert!(!written.is_empty());
        assert_eq!(written, shared);
    }
}
