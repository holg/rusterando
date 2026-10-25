//! A composed kitchen receipt ([`ReceiptProgram`], from the server's
//! Bon-Designer) as ESC/POS bytes, ready for the printer's USB endpoint or
//! `/dev/usb/lp0`.
//!
//! The device is a pure interpreter: text/raster/QR/rule/feed/cut → printer
//! primitives, no themes, no layout — the server composed all of that. One
//! implementation for every printing kitchen device (the Pi/STM32 printer
//! client, the ESP32-P4 displays), so the bytes are the same everywhere.

#[cfg(feature = "svg")]
pub mod svg;

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{Context, Result};
use escpos::driver::Driver;
use escpos::printer::Printer;
use escpos::printer_options::PrinterOptions;
use escpos::utils::{JustifyMode, PageCode, Protocol, QRCodeCorrectionLevel, QRCodeModel, QRCodeOption};
use kitchen_protocol::bitmap::Bitmap;
use kitchen_protocol::receipt::{LineStyle, ReceiptLine, ReceiptProgram, RECEIPT_WIDTH};

/// ESC/POS receipt printers don't speak UTF-8. They ship with a
/// firmware-resident table of code pages and we tell them which one to use
/// via ESC `t n`. ISO-8859-15 ("Latin-9") is the right choice for a German
/// shop:
///
///   - Native code points for ä/ö/ü/ß and the € sign.
///   - Supported by Epson TM-T20III and basically every modern ESC/POS
///     receipt printer.
///
/// The `escpos` crate auto-encodes UTF-8 strings to the configured page
/// code's byte table on every `.writeln` / `.write` call, so the rest of the
/// render code stays Unicode.
pub const RECEIPT_PAGE_CODE: PageCode = PageCode::ISO8859_15;

/// Resolves a logo ([`ReceiptLine::RasterSvg`]) to a 1-bit raster:
/// `(hash, target_width, svg)` → bitmap, or `None` to leave the logo out
/// (the text banners still convey the channel).
pub type LogoResolver<'a> = dyn FnMut(&[u8; 32], u16, &[u8]) -> Option<Bitmap> + 'a;

/// The ESC/POS bytes for `program`.
pub fn render(program: &ReceiptProgram, logos: &mut LogoResolver<'_>) -> Result<Vec<u8>> {
    let sink = Memory::default();
    // Configure the printer with our target code page up front: every
    // .writeln below then goes through the table-based UTF-8 → ISO-8859-15
    // transcoder in escpos's `Protocol::text`. Without this, the crate sends
    // raw UTF-8 bytes and the firmware paints mojibake.
    let options = PrinterOptions::new(Some(RECEIPT_PAGE_CODE), None, RECEIPT_WIDTH as u8);
    let mut p = Printer::new(sink.clone(), Protocol::default(), Some(options));

    p.init()?;
    // Switch the firmware's active code-page table to match the bytes we
    // send (drives the same UTF-8 → ISO-8859-15 transcoder).
    p.page_code(RECEIPT_PAGE_CODE)?;

    for line in &program.lines {
        emit_line(&mut p, line, logos)?;
    }

    p.print().context("flush ESC/POS commands")?;
    Ok(sink.take())
}

/// Emit one composed [`ReceiptLine`] as ESC/POS. Resets justify/size/bold
/// after each styled line so the next starts from a known state.
fn emit_line(p: &mut Printer<Memory>, line: &ReceiptLine, logos: &mut LogoResolver<'_>) -> Result<()> {
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
            p.justify(JustifyMode::LEFT)?.writeln(&"-".repeat(RECEIPT_WIDTH))?;
        }
        ReceiptLine::RasterSvg { hash, target_width, svg } => {
            if let Some(bm) = logos(hash, *target_width, svg) {
                p.justify(JustifyMode::CENTER)?
                    .custom(&gs_v0(&bm))?
                    .justify(JustifyMode::LEFT)?;
            }
        }
        ReceiptLine::Qr { url } => {
            // QR is drawn NATIVELY by the printer firmware from the URL —
            // never a pre-rendered bitmap. The default (.qrcode) prints with
            // module size 4 — too small on 80 mm thermal paper. Module size 8
            // (code ~22 mm), Model2 = the modern phone standard, correction
            // level H copes with print streaks.
            p.justify(JustifyMode::CENTER)?
                .qrcode_option(url, QRCodeOption::new(QRCodeModel::Model2, 8, QRCodeCorrectionLevel::H))?
                .justify(JustifyMode::LEFT)?;
        }
        ReceiptLine::Cut { feed } => {
            p.feeds(*feed)?.cut()?;
        }
    }
    Ok(())
}

/// Encode an inline [`Bitmap`] as an ESC/POS `GS v 0` raster bit-image
/// command. (The pixel math lives in `kitchen_protocol::bitmap`.)
pub fn gs_v0(bm: &Bitmap) -> Vec<u8> {
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
fn apply_style(p: &mut Printer<Memory>, style: LineStyle) -> Result<()> {
    p.justify(if style.center { JustifyMode::CENTER } else { JustifyMode::LEFT })?;
    p.bold(style.bold)?;
    p.size(style.width, style.height)?;
    Ok(())
}

/// An escpos driver that collects the bytes instead of writing a device.
#[derive(Clone, Default)]
struct Memory(Rc<RefCell<Vec<u8>>>);

impl Memory {
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

impl Driver for Memory {
    fn name(&self) -> String {
        "memory".into()
    }
    fn write(&self, data: &[u8]) -> escpos::errors::Result<()> {
        self.0.borrow_mut().extend_from_slice(data);
        Ok(())
    }
    fn read(&self, _buf: &mut [u8]) -> escpos::errors::Result<usize> {
        Ok(0)
    }
    fn flush(&self) -> escpos::errors::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(lines: Vec<ReceiptLine>) -> ReceiptProgram {
        ReceiptProgram { display_number: 1, is_reprint: false, lines }
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn german_text_in_latin9_qr_and_cut() {
        let p = program(vec![
            ReceiptLine::Text { text: "Größe: 1× Käse 7,50 €".into(), style: LineStyle::NORMAL },
            ReceiptLine::Rule,
            ReceiptLine::Qr { url: "https://example.org/o/1".into() },
            ReceiptLine::Cut { feed: 3 },
        ]);
        let bytes = render(&p, &mut |_, _, _| None).unwrap();
        assert!(bytes.starts_with(&[0x1b, b'@']), "ESC @ init first");
        // ISO-8859-15: ö = 0xF6, ß = 0xDF, ä = 0xE4, € = 0xA4.
        assert!(contains(&bytes, &[b'G', b'r', 0xF6, 0xDF, b'e']));
        assert!(contains(&bytes, &[b'K', 0xE4, b's', b'e']));
        assert!(bytes.contains(&0xA4), "euro sign");
        assert!(contains(&bytes, &[0x1d, b'(', b'k']), "native QR (GS ( k)");
        assert!(contains(&bytes, &[0x1d, b'V']), "cut (GS V)");
    }

    #[test]
    fn logo_through_resolver_or_left_out() {
        let logo = ReceiptLine::RasterSvg { hash: [7; 32], target_width: 16, svg: b"<svg/>".to_vec() };
        let p = program(vec![logo]);
        let without = render(&p, &mut |_, _, _| None).unwrap();
        assert!(!contains(&without, &[0x1d, b'v', b'0']));

        let mut asked = None;
        let with = render(&p, &mut |hash, width, svg| {
            asked = Some((*hash, width, svg.len()));
            Some(Bitmap { width: 16, height: 2, bytes: vec![0xff; 4] })
        })
        .unwrap();
        assert_eq!(asked, Some(([7; 32], 16, 6)));
        assert!(contains(&with, &[0x1d, b'v', b'0', 0x00, 2, 0, 2, 0, 0xff, 0xff, 0xff, 0xff]));
    }
}
