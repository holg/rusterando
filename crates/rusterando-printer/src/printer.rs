use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use escpos::driver::{Driver, FileDriver};
use escpos::printer::Printer as EscposPrinter;
use escpos::printer_options::PrinterOptions;
use escpos::utils::{
    JustifyMode, PageCode, Protocol, QRCodeCorrectionLevel, QRCodeModel, QRCodeOption,
};
use kitchen_protocol::bitmap::Bitmap;
use kitchen_protocol::receipt::{LineStyle, ReceiptLine, ReceiptProgram, RECEIPT_WIDTH};

use crate::usb::{UsbDriver, UsbSelector};

/// ESC/POS receipt printers don't speak UTF-8. They ship with a
/// firmware-resident table of code pages and we tell them which one
/// to use via ESC `t n`. ISO-8859-15 ("Latin-9") is the right choice
/// for a German shop:
///
///   - Native code points for ä/ö/ü/ß and the € sign.
///   - Supported by Epson TM-T20III and basically every modern
///     ESC/POS receipt printer.
///
/// The `escpos` crate auto-encodes UTF-8 strings to the configured
/// page code's byte table on every `.writeln` / `.write` call, so the
/// rest of the render code stays Unicode.
const RECEIPT_PAGE_CODE: PageCode = PageCode::ISO8859_15;

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
    /// Bon-Designer) already resolved the theme and inlined every logo;
    /// this is a pure interpreter — it maps each [`ReceiptLine`] to an
    /// ESC/POS primitive and knows nothing about themes or layout.
    pub fn print_program(
        &self,
        program: &ReceiptProgram,
        cache: &mut crate::raster_cache::RasterCache,
    ) -> Result<()> {
        let _guard = self.write_lock.lock().unwrap();
        let driver = AnyDriver::open(&self.target)?;

        // Configure the printer with our target code page up front: every
        // .writeln below will then go through the table-based UTF-8 →
        // ISO-8859-15 transcoder in escpos's `Protocol::text`. Without
        // this, the crate sends raw UTF-8 bytes and the firmware paints
        // mojibake.
        let options = PrinterOptions::new(Some(RECEIPT_PAGE_CODE), None, RECEIPT_WIDTH as u8);
        let mut p = EscposPrinter::new(driver, Protocol::default(), Some(options));

        p.init()?;
        // Switch the firmware's active code-page table to match the bytes
        // we send (drives the same UTF-8 → ISO-8859-15 transcoder).
        p.page_code(RECEIPT_PAGE_CODE)?;

        for line in &program.lines {
            emit_line(&mut p, line, cache)?;
        }

        p.print().context("flush ESC/POS commands")?;
        Ok(())
    }
}

/// Emit one composed [`ReceiptLine`] as ESC/POS. The Pi is a pure
/// interpreter: text/raster/QR/rule/feed/cut → printer primitives, with
/// no knowledge of themes or layout (the server composed all of that).
/// Resets justify/size/bold after each styled line so the next starts
/// from a known state.
fn emit_line(
    p: &mut EscposPrinter<AnyDriver>,
    line: &ReceiptLine,
    cache: &mut crate::raster_cache::RasterCache,
) -> Result<()> {
    match line {
        ReceiptLine::Text { text, style } => {
            apply_style(p, *style)?;
            p.writeln(text)?;
            // Return to a neutral state for the following line.
            p.reset_size()?.bold(false)?.justify(JustifyMode::LEFT)?;
        }
        ReceiptLine::Feed { n } => {
            for _ in 0..*n {
                p.feed()?;
            }
        }
        ReceiptLine::Rule => {
            p.justify(JustifyMode::LEFT)?
                .writeln(&"-".repeat(RECEIPT_WIDTH))?;
        }
        ReceiptLine::RasterSvg {
            hash,
            target_width,
            svg,
        } => {
            // Resolve the 1-bit raster: cache hit → use it; miss with SVG
            // bytes → rasterize + cache; miss with an empty svg (a server
            // reference we somehow don't have) → skip the logo. The text
            // banners still convey the channel, and the server re-sends the
            // bytes on the next reconnect (fresh Hello advertises our cache).
            let bm = if let Some(bm) = cache.get(hash) {
                Some(bm)
            } else if !svg.is_empty() {
                match crate::svg_raster::rasterize(svg, *target_width) {
                    Ok(bm) => {
                        cache.put(*hash, &bm);
                        Some(bm)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "logo SVG rasterize failed — skipping logo");
                        None
                    }
                }
            } else {
                tracing::warn!(
                    "logo cache miss with no SVG bytes — skipping (will re-fetch on reconnect)"
                );
                None
            };
            if let Some(bm) = bm {
                p.justify(JustifyMode::CENTER)?
                    .custom(&gs_v0(&bm))?
                    .justify(JustifyMode::LEFT)?;
            }
        }
        ReceiptLine::Qr { url } => {
            // QR is drawn NATIVELY by the printer firmware from the URL —
            // never a pre-rendered bitmap. Der Default (.qrcode) druckt mit
            // Modulgröße 4 — auf 80-mm-Thermopapier zu klein. Modulgröße 8
            // (Code ~22 mm), Model2 = moderner Telefon-Standard,
            // Korrekturstufe H verträgt Druckschlieren.
            p.justify(JustifyMode::CENTER)?
                .qrcode_option(
                    url,
                    QRCodeOption::new(QRCodeModel::Model2, 8, QRCodeCorrectionLevel::H),
                )?
                .justify(JustifyMode::LEFT)?;
        }
        ReceiptLine::Cut { feed } => {
            p.feeds(*feed)?.cut()?;
        }
    }
    Ok(())
}

/// Encode an inline [`Bitmap`] as an ESC/POS `GS v 0` raster bit-image
/// command. (The pixel math lives in `kitchen_protocol::bitmap`; this is
/// the only ESC/POS-specific bit, so it stays on the Pi.)
fn gs_v0(bm: &Bitmap) -> Vec<u8> {
    let row_bytes = bm.row_bytes();
    let xl = (row_bytes & 0xff) as u8;
    let xh = ((row_bytes >> 8) & 0xff) as u8;
    let yl = (bm.height & 0xff) as u8;
    let yh = ((bm.height >> 8) & 0xff) as u8;
    let mut out = Vec::with_capacity(8 + bm.bytes.len());
    out.extend_from_slice(&[0x1d, b'v', b'0', 0x00, xl, xh, yl, yh]);
    out.extend_from_slice(&bm.bytes);
    out
}

/// Translate a [`LineStyle`] into the matching ESC/POS state. The caller
/// resets afterwards.
fn apply_style(p: &mut EscposPrinter<AnyDriver>, style: LineStyle) -> Result<()> {
    p.justify(if style.center {
        JustifyMode::CENTER
    } else {
        JustifyMode::LEFT
    })?;
    p.bold(style.bold)?;
    p.size(style.width, style.height)?;
    Ok(())
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
