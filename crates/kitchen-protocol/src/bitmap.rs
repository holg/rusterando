//! 1-bit monochrome bitmap primitives, shared between the server (the
//! "Bon-Designer", which composes receipt artwork) and the Pi (which
//! emits it as an ESC/POS raster).
//!
//! This module is deliberately **printer-agnostic**: it parses 1-bit
//! BMPs, scales and composes them, and packs the result MSB-first with
//! `set bit = black dot` — the layout an ESC/POS `GS v 0` raster wants —
//! but it emits no printer bytes itself. The Pi turns a [`Bitmap`] into
//! the actual `GS v 0` command; the server inlines a [`Bitmap`]'s
//! `width`/`height`/`bytes` straight into a `ReceiptLine::Raster` and
//! ships it down the wire. One decode/scale/compose path, two consumers.

use serde::{Deserialize, Serialize};

/// A 1-bit monochrome bitmap. Row-major, 1 bit/pixel, MSB-first, a set
/// bit = BLACK dot. `bytes.len() == width.div_ceil(8) * height`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    pub bytes: Vec<u8>,
}

impl Bitmap {
    /// Bytes per packed row.
    pub fn row_bytes(&self) -> usize {
        self.width.div_ceil(8)
    }

    /// Read the bit at (x, y): true = black dot. Out-of-range → white.
    pub fn get(&self, x: usize, y: usize) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let rb = self.row_bytes();
        let byte = self.bytes[y * rb + x / 8];
        (byte >> (7 - (x % 8))) & 1 == 1
    }

    /// Nearest-neighbour resize to exactly `target_h` dots tall, width
    /// scaled to preserve aspect. No anti-aliasing — there are only two
    /// tones, and thermal line art reads fine without it.
    pub fn scale_to_height(&self, target_h: usize) -> Bitmap {
        if target_h == 0 || self.height == 0 {
            return Bitmap {
                width: 0,
                height: 0,
                bytes: vec![],
            };
        }
        let target_w = (self.width * target_h).div_ceil(self.height).max(1);
        let rb = target_w.div_ceil(8);
        let mut bytes = vec![0u8; rb * target_h];
        for dy in 0..target_h {
            let sy = dy * self.height / target_h;
            for dx in 0..target_w {
                let sx = dx * self.width / target_w;
                if self.get(sx, sy) {
                    bytes[dy * rb + dx / 8] |= 1 << (7 - (dx % 8));
                }
            }
        }
        Bitmap {
            width: target_w,
            height: target_h,
            bytes,
        }
    }

    /// Place `self` (left) and `right` (right) side-by-side with `gap`
    /// white columns between, both vertically centered in a canvas as
    /// tall as the taller input. Used to put the channel icon + brand
    /// mark on one receipt band.
    pub fn beside(&self, right: &Bitmap, gap: usize) -> Bitmap {
        let height = self.height.max(right.height);
        let width = self.width + gap + right.width;
        let rb = width.div_ceil(8);
        let mut bytes = vec![0u8; rb * height];

        let mut blit = |src: &Bitmap, x_off: usize| {
            let y_off = (height - src.height) / 2;
            for sy in 0..src.height {
                for sx in 0..src.width {
                    if src.get(sx, sy) {
                        let dx = x_off + sx;
                        let dy = y_off + sy;
                        bytes[dy * rb + dx / 8] |= 1 << (7 - (dx % 8));
                    }
                }
            }
        };
        blit(self, 0);
        blit(right, self.width + gap);

        Bitmap {
            width,
            height,
            bytes,
        }
    }
}

/// Parse a 1-bit, uncompressed BMP (BMP3) into a [`Bitmap`] whose bits
/// are MSB-first with set=black. Returns `None` on any malformation —
/// the caller then simply skips the logo (the receipt still prints).
///
/// Layout relied on: "BM" magic, u32@10 pixel offset, u32@18 width,
/// i32@22 height (positive = bottom-up), u16@28 bpp (must be 1), u32@30
/// compression (must be 0), then a 2-entry palette and 4-byte-padded
/// packed rows. The palette is read to learn which index is black.
pub fn parse_1bit_bmp(buf: &[u8]) -> Option<Bitmap> {
    if buf.len() < 54 || &buf[0..2] != b"BM" {
        return None;
    }
    let u16le = |o: usize| u16::from_le_bytes([buf[o], buf[o + 1]]);
    let u32le = |o: usize| u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);
    let i32le = |o: usize| i32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);

    let data_off = u32le(10) as usize;
    let dib_size = u32le(14) as usize;
    let width = u32le(18) as usize;
    let height_raw = i32le(22);
    let bpp = u16le(28);
    let compression = u32le(30);
    if bpp != 1 || compression != 0 || width == 0 || height_raw == 0 {
        return None;
    }
    let bottom_up = height_raw > 0;
    let height = height_raw.unsigned_abs() as usize;

    let pal_off = 14 + dib_size;
    if pal_off + 8 > buf.len() {
        return None;
    }
    let luma = |i: usize| {
        let b = buf[pal_off + i * 4] as u32;
        let g = buf[pal_off + i * 4 + 1] as u32;
        let r = buf[pal_off + i * 4 + 2] as u32;
        r + g + b
    };
    let one_is_black = luma(1) <= luma(0);

    let row_stride = width.div_ceil(32) * 4;
    let out_row_bytes = width.div_ceil(8);
    if data_off + row_stride * height > buf.len() {
        return None;
    }

    let mut out = vec![0u8; out_row_bytes * height];
    for y in 0..height {
        let src_row = if bottom_up { height - 1 - y } else { y };
        let src_base = data_off + src_row * row_stride;
        for x in 0..width {
            let src_byte = buf[src_base + x / 8];
            let src_bit = (src_byte >> (7 - (x % 8))) & 1;
            let is_black = (src_bit == 1) == one_is_black;
            if is_black {
                let di = y * out_row_bytes + x / 8;
                out[di] |= 1 << (7 - (x % 8));
            }
        }
    }
    Some(Bitmap {
        width,
        height,
        bytes: out,
    })
}
