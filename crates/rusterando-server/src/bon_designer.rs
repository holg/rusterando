//! Server-side "Bon-Designer": resolves a receipt's theme logos into a
//! composed **SVG** the Pi rasterizes (and caches) itself, so the Pi holds
//! no themes or assets.
//!
//! Source artwork is `<site_root>/img/rusterando-printer-rando-*.svg`
//! (vector — editable, resolution-independent). For the header row we lay
//! the channel-icon SVG and the brand-mark SVG side-by-side into one SVG
//! document (`receipt::compose_header_svg`) and ship that. The Pi
//! rasterizes to 1-bit at the target width and caches by content hash;
//! after the first print the wire carries only a hash reference. Changing
//! a logo is a server-side `.svg` swap — no Pi redeploy.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use kitchen_protocol::receipt::{
    compose_header_svg, ComposedSvg, RasterProvider, RasterSlot, SvgLogo, HEADER_LOGO_GAP,
    HEADER_LOGO_HEIGHT, RECEIPT_WIDTH,
};

/// Print-head dot width to rasterize the header band at — this is the lever
/// for how BIG both top logos (channel icon + paid/unpaid brand mark) print.
/// The 80-mm head is ~512 dots. The band is scaled to fit this width, so a
/// larger value makes both logos bigger together (their side-by-side layout
/// and aspect ratios are preserved). We target 480 dots — nearly the full
/// head with a small safety margin so it can't clip on a slightly narrower
/// head. (Bumping HEADER_LOGO_HEIGHT alone does NOT help: it scales the
/// band's width and height together, which cancels out at this fit step.)
const HEADER_TARGET_WIDTH: u16 = 480;
/// Receipt column width in dots (42 cols × 8) — kept for reference; the
/// header band intentionally prints wider than the text column now.
const _TEXT_COLUMN_DOTS: u16 = (RECEIPT_WIDTH * 8) as u16;

/// A source SVG plus its parsed intrinsic size (for layout math).
#[derive(Clone)]
struct SourceSvg {
    text: String,
    width: f64,
    height: f64,
}

/// Resolves theme logos from `<img_dir>/*.svg`, caching the parsed source
/// + the composed header SVG so repeated receipts don't re-read/re-compose.
pub struct FileRasterProvider {
    img_dir: PathBuf,
    sources: Mutex<HashMap<String, Option<SourceSvg>>>,
    composed: Mutex<HashMap<(bool, bool), Option<ComposedSvg>>>,
}

impl FileRasterProvider {
    /// `site_root` is leptos's site root (`target/site` in dev, `html`
    /// on prod); assets sit under `<site_root>/img`.
    pub fn new(site_root: &str) -> Self {
        Self {
            img_dir: PathBuf::from(site_root).join("img"),
            sources: Mutex::new(HashMap::new()),
            composed: Mutex::new(HashMap::new()),
        }
    }

    /// Load + parse `<img_dir>/<stem>.svg`, memoized. `None` on a missing
    /// file or one we can't read dimensions from.
    fn load(&self, stem: &str) -> Option<SourceSvg> {
        if let Some(hit) = self.sources.lock().unwrap().get(stem) {
            return hit.clone();
        }
        let path = self.img_dir.join(format!("{stem}.svg"));
        let parsed = std::fs::read_to_string(&path).ok().and_then(|text| {
            let (width, height) = svg_intrinsic_size(&text)?;
            Some(SourceSvg {
                text,
                width,
                height,
            })
        });
        if parsed.is_none() {
            tracing::warn!(path = %path.display(), "receipt logo SVG missing/unsized");
        }
        self.sources
            .lock()
            .unwrap()
            .insert(stem.to_string(), parsed.clone());
        parsed
    }
}

impl RasterProvider for FileRasterProvider {
    fn resolve(&self, theme: &str, slot: RasterSlot) -> Option<ComposedSvg> {
        if theme != "rusterando-rando" {
            return None;
        }
        match slot {
            RasterSlot::ChannelAndBrand {
                is_delivery,
                is_paid,
            } => {
                let key = (is_delivery, is_paid);
                if let Some(hit) = self.composed.lock().unwrap().get(&key) {
                    return hit.clone();
                }
                let channel_stem = if is_delivery {
                    "rusterando-printer-rando-delivery-e-car"
                } else {
                    "rusterando-printer-rando-pickup-rust"
                };
                let brand_stem = if is_paid {
                    "rusterando-printer-rando-brand-mark-paid"
                } else {
                    "rusterando-printer-rando-brand-mark-unpaid"
                };

                let composed = (|| {
                    let channel = self.load(channel_stem)?;
                    let brand = self.load(brand_stem)?;
                    let svg = compose_header_svg(
                        &SvgLogo {
                            svg: &channel.text,
                            width: channel.width,
                            height: channel.height,
                        },
                        &SvgLogo {
                            svg: &brand.text,
                            width: brand.width,
                            height: brand.height,
                        },
                        HEADER_LOGO_HEIGHT as f64,
                        HEADER_LOGO_GAP as f64,
                    );
                    Some(ComposedSvg {
                        svg: svg.into_bytes(),
                        target_width: HEADER_TARGET_WIDTH,
                    })
                })();

                self.composed.lock().unwrap().insert(key, composed.clone());
                composed
            }
        }
    }
}

/// Parse an SVG's intrinsic size from `width`/`height` attrs, falling back
/// to the `viewBox` (cols 3+4). Strips a trailing unit like `pt`/`px`.
fn svg_intrinsic_size(svg: &str) -> Option<(f64, f64)> {
    // Prefer explicit width/height on the opening <svg> tag.
    let open = svg.find("<svg").map(|i| {
        let end = svg[i..].find('>').map(|j| i + j + 1).unwrap_or(svg.len());
        &svg[i..end]
    })?;
    let w = attr_number(open, "width");
    let h = attr_number(open, "height");
    if let (Some(w), Some(h)) = (w, h) {
        if w > 0.0 && h > 0.0 {
            return Some((w, h));
        }
    }
    // Fall back to viewBox "minx miny width height".
    if let Some(vb) = attr_value(open, "viewBox") {
        let nums: Vec<f64> = vb
            .split_whitespace()
            .filter_map(parse_leading_f64)
            .collect();
        if nums.len() == 4 && nums[2] > 0.0 && nums[3] > 0.0 {
            return Some((nums[2], nums[3]));
        }
    }
    None
}

/// Value of `name="..."` within a tag string.
fn attr_value<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let start = tag.find(&pat)? + pat.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Numeric value of an attribute, tolerating a trailing unit (`960`,
/// `994.96pt`, `120px`).
fn attr_number(tag: &str, name: &str) -> Option<f64> {
    parse_leading_f64(attr_value(tag, name)?)
}

/// Parse the leading float of a string, ignoring a trailing unit suffix.
fn parse_leading_f64(s: &str) -> Option<f64> {
    let s = s.trim();
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(s.len());
    s[..end].parse::<f64>().ok()
}

/// Process-wide provider, built once from the site root. The asset set
/// is tiny and static; one cache for the whole server is plenty.
static PROVIDER: OnceLock<FileRasterProvider> = OnceLock::new();

/// Get (or initialize) the shared provider. `site_root` is only used on
/// the first call; later calls return the existing instance.
pub fn provider(site_root: &str) -> &'static FileRasterProvider {
    PROVIDER.get_or_init(|| FileRasterProvider::new(site_root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_svg_sizes() {
        assert_eq!(
            svg_intrinsic_size(r#"<svg width="960" height="200"></svg>"#),
            Some((960.0, 200.0))
        );
        assert_eq!(
            svg_intrinsic_size(r#"<svg width="994.96pt" height="795.79pt"></svg>"#),
            Some((994.96, 795.79))
        );
        assert_eq!(
            svg_intrinsic_size(r#"<svg viewBox="0 0 100 50"></svg>"#),
            Some((100.0, 50.0))
        );
    }

    /// Composes the real source SVGs into one header SVG. Skips cleanly if
    /// the assets aren't in the site root yet.
    #[test]
    fn resolves_real_svgs_into_header() {
        let p = FileRasterProvider::new("target/site");
        let brand = p
            .img_dir
            .join("rusterando-printer-rando-brand-mark-paid.svg");
        if !brand.exists() {
            eprintln!("skipping: {} not present yet", brand.display());
            return;
        }
        let composed = p
            .resolve(
                "rusterando-rando",
                RasterSlot::ChannelAndBrand {
                    is_delivery: true,
                    is_paid: true,
                },
            )
            .expect("rando SVGs should resolve");
        assert_eq!(composed.target_width, HEADER_TARGET_WIDTH);
        let svg = String::from_utf8(composed.svg).unwrap();
        assert!(svg.contains("<svg") && svg.contains("translate("));
    }

    #[test]
    fn default_theme_resolves_to_none() {
        let p = FileRasterProvider::new("target/site");
        assert!(p
            .resolve(
                "rusterando-default",
                RasterSlot::ChannelAndBrand {
                    is_delivery: false,
                    is_paid: false
                }
            )
            .is_none());
    }
}
