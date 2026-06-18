//! Rasterize a server-composed logo SVG to a 1-bit [`Bitmap`] on the Pi.
//!
//! The server ships the header logo as SVG (vector, editable); the Pi
//! renders it once at the requested dot-width, Floyd-Steinberg dithers it
//! to 1-bit (matching the look of the old offline ImageMagick pipeline),
//! and caches the result by content hash (see `raster_cache`). All
//! pure-Rust (resvg/usvg/tiny-skia + a bundled Poppins font) so the
//! aarch64 cross-build stays C-free.

use anyhow::{Context, Result};
use kitchen_protocol::bitmap::Bitmap;
use resvg::tiny_skia;
use resvg::usvg;
use std::sync::OnceLock;

/// Bundled font(s) for `<text>` in the brand-mark SVGs (Poppins Bold). The
/// receipt must render identically regardless of what's installed on the
/// host, so we embed the font rather than scan the system.
///
/// NOTE: the TTF must be present at build time. If it's absent the
/// `include_bytes!` fails the build loudly — intentional, so we never ship
/// a binary that silently can't render the brand text.
static POPPINS_BOLD: &[u8] = include_bytes!("../assets/fonts/Poppins-Bold.ttf");

/// Shared usvg options with our font database loaded once.
fn usvg_options() -> &'static usvg::Options<'static> {
    static OPTS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTS.get_or_init(|| {
        let mut fontdb = usvg::fontdb::Database::new();
        fontdb.load_font_data(POPPINS_BOLD.to_vec());
        // Make "Poppins" (the family the SVGs request) resolvable, and set
        // it as the default so any unnamed text still uses it.
        fontdb.set_serif_family("Poppins");
        fontdb.set_sans_serif_family("Poppins");
        usvg::Options {
            fontdb: std::sync::Arc::new(fontdb),
            ..Default::default()
        }
    })
}

/// Render `svg` at `target_width` dots wide (height from aspect) and
/// dither to a 1-bit [`Bitmap`] (MSB-first, set bit = black dot).
pub fn rasterize(svg: &[u8], target_width: u16) -> Result<Bitmap> {
    let tree = usvg::Tree::from_data(svg, usvg_options()).context("parse composed logo SVG")?;
    let size = tree.size();
    if size.width() <= 0.0 || size.height() <= 0.0 || target_width == 0 {
        anyhow::bail!("degenerate SVG size");
    }

    let scale = target_width as f32 / size.width();
    let out_w = target_width as u32;
    let out_h = ((size.height() * scale).round() as u32).max(1);

    let mut pixmap = tiny_skia::Pixmap::new(out_w, out_h).context("alloc raster pixmap")?;
    // White background so transparent SVG areas become white (the SVGs
    // already paint a white rect, but be safe).
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    Ok(dither_to_1bit(
        pixmap.data(),
        out_w as usize,
        out_h as usize,
    ))
}

/// Floyd-Steinberg dither an RGBA8 premultiplied pixmap to 1-bit. We read
/// luminance, diffuse error, threshold at 50%, and pack MSB-first with
/// `set bit = black` (the `GS v 0` convention).
fn dither_to_1bit(rgba: &[u8], w: usize, h: usize) -> Bitmap {
    // Grayscale luminance buffer (0=black .. 255=white), f32 for error diffusion.
    let mut lum = vec![0f32; w * h];
    for i in 0..(w * h) {
        let r = rgba[i * 4] as f32;
        let g = rgba[i * 4 + 1] as f32;
        let b = rgba[i * 4 + 2] as f32;
        let a = rgba[i * 4 + 3] as f32 / 255.0;
        // Composite over white for any residual transparency.
        let gray = 0.299 * r + 0.587 * g + 0.114 * b;
        lum[i] = gray * a + 255.0 * (1.0 - a);
    }

    let row_bytes = w.div_ceil(8);
    let mut out = vec![0u8; row_bytes * h];

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let old = lum[idx];
            let new = if old < 128.0 { 0.0 } else { 255.0 };
            let err = old - new;
            if new < 128.0 {
                // black dot → set bit
                out[y * row_bytes + x / 8] |= 1 << (7 - (x % 8));
            }
            // Diffuse error (Floyd-Steinberg weights).
            let mut diffuse = |xx: isize, yy: isize, f: f32| {
                if xx < 0 || yy < 0 || xx as usize >= w || yy as usize >= h {
                    return;
                }
                let j = yy as usize * w + xx as usize;
                lum[j] = (lum[j] + err * f).clamp(0.0, 255.0);
            };
            let (x, y) = (x as isize, y as isize);
            diffuse(x + 1, y, 7.0 / 16.0);
            diffuse(x - 1, y + 1, 3.0 / 16.0);
            diffuse(x, y + 1, 5.0 / 16.0);
            diffuse(x + 1, y + 1, 1.0 / 16.0);
        }
    }

    Bitmap {
        width: w,
        height: h,
        bytes: out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterizes_a_simple_svg() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect x="0" y="0" width="100" height="50" fill="black"/></svg>"#;
        let bm = rasterize(svg, 200).expect("rasterize");
        assert_eq!(bm.width, 200);
        assert_eq!(bm.height, 100); // aspect 2:1
                                    // All-black rect → lots of set bits.
        let black = bm.bytes.iter().filter(|&&b| b != 0).count();
        assert!(black > 0);
    }
}

#[cfg(test)]
mod real_asset_tests {
    use super::*;
    /// Rasterize the REAL composed header SVG (channel + Poppins brand text)
    /// and assert it produced ink in both halves. Skips if assets absent.
    #[test]
    fn real_header_svg_renders_with_text() {
        let dir = std::path::Path::new("../../target/site/img");
        let brand = dir.join("rusterando-printer-rando-brand-mark-paid.svg");
        let chan = dir.join("rusterando-printer-rando-delivery-e-car.svg");
        if !brand.exists() || !chan.exists() {
            eprintln!("skipping: real SVGs not in target/site/img");
            return;
        }
        use kitchen_protocol::receipt::SvgLogo;
        let brand_svg = std::fs::read_to_string(&brand).unwrap();
        let chan_svg = std::fs::read_to_string(&chan).unwrap();
        let composed = kitchen_protocol::receipt::compose_header_svg(
            &SvgLogo {
                svg: &chan_svg,
                width: 994.965184,
                height: 795.795287,
            },
            &SvgLogo {
                svg: &brand_svg,
                width: 960.0,
                height: 200.0,
            },
            120.0,
            28.0,
        );
        let bm = rasterize(composed.as_bytes(), 336).expect("rasterize real header");
        assert!(bm.width == 336 && bm.height > 0);
        let half = bm.width / 2;
        let (mut left, mut right) = (0usize, 0usize);
        for y in 0..bm.height {
            for x in 0..bm.width {
                let byte = bm.bytes[y * bm.width.div_ceil(8) + x / 8];
                if (byte >> (7 - (x % 8))) & 1 == 1 {
                    if x < half {
                        left += 1;
                    } else {
                        right += 1;
                    }
                }
            }
        }
        assert!(left > 0, "channel icon should have ink");
        assert!(
            right > 0,
            "brand mark (Poppins text) should have ink — font loaded?"
        );
    }
}
