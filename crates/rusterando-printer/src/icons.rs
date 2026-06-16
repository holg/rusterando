//! Channel logos for the `rusterando-rando` receipt theme.
//!
//! These are simple 1-bit silhouettes (a bag for pickup, a scooter for
//! delivery) generated procedurally — no image crate, no asset files, no
//! cross-compile risk. They're drawn into a bit buffer and emitted as a
//! raw ESC/POS `GS v 0` raster bit-image via the printer's `custom()`
//! escape hatch.
//!
//! Swapping in real artwork later: replace `bag_bitmap()` /
//! `scooter_bitmap()` with bitmaps decoded from embedded PNGs (or hand
//! pixel data). The `Bitmap` type + `gs_v0()` encoder stay as-is.
//!
//! ## GS v 0 raster format
//!
//! `[GS, 'v', '0', m, xL, xH, yL, yH, <data>]`
//!   * m  = 0 (normal mode)
//!   * xL,xH = width in BYTES (little-endian) — width must be a multiple
//!     of 8 px; we pad the icon width to a byte boundary.
//!   * yL,yH = height in DOTS (little-endian).
//!   * data  = row-major, 1 bit/pixel, MSB-first, a 1-bit = black dot.

/// A 1-bit monochrome bitmap. `width` is in pixels (kept a multiple of 8
/// so each row is whole bytes); `set`/`get` address individual pixels.
pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    /// Row-major, 1 bit/pixel, MSB-first within each byte.
    bytes: Vec<u8>,
}

impl Bitmap {
    pub fn new(width: usize, height: usize) -> Self {
        // Round width up to a multiple of 8 so each row is whole bytes.
        let w = width.div_ceil(8) * 8;
        let row_bytes = w / 8;
        Bitmap {
            width: w,
            height,
            bytes: vec![0u8; row_bytes * height],
        }
    }

    #[inline]
    fn set(&mut self, x: usize, y: usize) {
        if x >= self.width || y >= self.height {
            return;
        }
        let row_bytes = self.width / 8;
        let idx = y * row_bytes + x / 8;
        let bit = 7 - (x % 8);
        self.bytes[idx] |= 1 << bit;
    }

    /// Fill an axis-aligned rectangle (inclusive of x0/y0, exclusive of
    /// x1/y1). The icon primitives are all built from these.
    fn rect(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        for y in y0..y1.min(self.height) {
            for x in x0..x1.min(self.width) {
                self.set(x, y);
            }
        }
    }

    /// A filled disc centred at (cx, cy) with radius r.
    fn disc(&mut self, cx: isize, cy: isize, r: isize) {
        let r2 = r * r;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r2 {
                    let x = cx + dx;
                    let y = cy + dy;
                    if x >= 0 && y >= 0 {
                        self.set(x as usize, y as usize);
                    }
                }
            }
        }
    }

    /// A ring (annulus): outer radius r, line thickness `t`.
    fn ring(&mut self, cx: isize, cy: isize, r: isize, t: isize) {
        let outer = r * r;
        let inner = (r - t).max(0) * (r - t).max(0);
        for dy in -r..=r {
            for dx in -r..=r {
                let d = dx * dx + dy * dy;
                if d <= outer && d >= inner {
                    let x = cx + dx;
                    let y = cy + dy;
                    if x >= 0 && y >= 0 {
                        self.set(x as usize, y as usize);
                    }
                }
            }
        }
    }

    /// Encode as an ESC/POS `GS v 0` raster bit-image command, ready to
    /// hand to `printer.custom(&bytes)`.
    pub fn gs_v0(&self) -> Vec<u8> {
        let row_bytes = self.width / 8;
        let xl = (row_bytes & 0xff) as u8;
        let xh = ((row_bytes >> 8) & 0xff) as u8;
        let yl = (self.height & 0xff) as u8;
        let yh = ((self.height >> 8) & 0xff) as u8;
        let mut out = Vec::with_capacity(8 + self.bytes.len());
        out.extend_from_slice(&[0x1d, b'v', b'0', 0x00, xl, xh, yl, yh]);
        out.extend_from_slice(&self.bytes);
        out
    }
}

// ---------------------------------------------------------------------------
// Procedural icons (~200 px wide — comfortable on 80-mm / 384-dot paper).
// Simple bold silhouettes read best on a 1-bit thermal head.
// ---------------------------------------------------------------------------

const ICON_W: usize = 200;

/// Pickup: a shopping bag with two handles.
pub fn bag_bitmap() -> Bitmap {
    let h = 200;
    let mut b = Bitmap::new(ICON_W, h);
    // Bag body (a tall rounded-ish box).
    let bx0 = 40;
    let bx1 = 160;
    let by0 = 70;
    let by1 = 185;
    b.rect(bx0, by0, bx1, by1);
    // Hollow it out to leave a thick outline (so it reads as a bag, not
    // a solid block) — clear an inner rectangle.
    clear_rect(&mut b, bx0 + 14, by0 + 14, bx1 - 14, by1 - 14);
    // Two handles (arcs) rising from the top edge.
    b.ring(75, 70, 26, 8);
    b.ring(125, 70, 26, 8);
    // Mask off the lower half of each handle ring so only the upper arc
    // shows above the bag mouth.
    clear_rect(&mut b, 40, 70, 160, 200);
    // Redraw bag body + outline AFTER the handle mask (mask was broad).
    b.rect(bx0, by0, bx1, by1);
    clear_rect(&mut b, bx0 + 14, by0 + 14, bx1 - 14, by1 - 14);
    b
}

/// Delivery: a scooter silhouette (two wheels + body + handlebar).
pub fn scooter_bitmap() -> Bitmap {
    let h = 150;
    let mut b = Bitmap::new(ICON_W, h);
    // Wheels.
    b.ring(45, 120, 26, 8);
    b.disc(45, 120, 7);
    b.ring(160, 120, 26, 8);
    b.disc(160, 120, 7);
    // Deck + body bar connecting the wheels.
    b.rect(45, 104, 165, 116);
    // Front column up to the handlebar.
    b.rect(150, 50, 165, 116);
    // Handlebar.
    b.rect(150, 50, 190, 62);
    // Seat / rear body hump.
    b.rect(30, 80, 95, 110);
    b.disc(95, 80, 22);
    b
}

/// Clear (set to white) a rectangle — used to hollow shapes / mask arcs.
fn clear_rect(b: &mut Bitmap, x0: usize, y0: usize, x1: usize, y1: usize) {
    let row_bytes = b.width / 8;
    for y in y0..y1.min(b.height) {
        for x in x0..x1.min(b.width) {
            let idx = y * row_bytes + x / 8;
            let bit = 7 - (x % 8);
            b.bytes[idx] &= !(1u8 << bit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gs_v0_header_is_well_formed() {
        let b = Bitmap::new(16, 4); // 2 bytes wide, 4 rows
        let cmd = b.gs_v0();
        // [GS 'v' '0' 0 xL xH yL yH] + 2*4 data bytes
        assert_eq!(&cmd[0..4], &[0x1d, b'v', b'0', 0x00]);
        assert_eq!(cmd[4], 2); // row_bytes = 16/8
        assert_eq!(cmd[5], 0);
        assert_eq!(cmd[6], 4); // height
        assert_eq!(cmd[7], 0);
        assert_eq!(cmd.len(), 8 + 2 * 4);
    }

    #[test]
    fn width_padded_to_byte_boundary() {
        let b = Bitmap::new(ICON_W, 10);
        assert_eq!(b.width % 8, 0);
        assert!(b.width >= ICON_W);
    }

    #[test]
    fn icons_render_some_black_pixels() {
        for b in [bag_bitmap(), scooter_bitmap()] {
            let black = b.gs_v0()[8..].iter().filter(|&&x| x != 0).count();
            assert!(black > 0, "icon should have black pixels");
        }
    }
}
