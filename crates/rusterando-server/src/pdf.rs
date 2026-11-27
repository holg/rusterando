//! Server-side menu PDF rendering via Typst.
//!
//! The .typ template lives at templates/menu.typ and is embedded at compile
//! time. We build a JSON document from the live SQLite menu, hand it to the
//! template through `sys.inputs.data`, and let Typst do the layout.

use std::sync::OnceLock;

use ecow::eco_format;
use serde::Serialize;
use sqlx::SqlitePool;
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Dict};
use typst::layout::PagedDocument;
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

// ---------------------------------------------------------------------------
// Embedded template
// ---------------------------------------------------------------------------

/// Factory-default STYLING theme (palette + render fns, no page geometry).
/// Used when the admin hasn't supplied an override via the pdf_themes
/// library. Kept as the safety net for "Reset auf Werks-Template".
pub const TEMPLATE_SRC: &str = include_str!("../templates/menu.typ");
/// The theme is imported by the geometry layer as `/theme.typ`.
const THEME_VPATH: &str = "/theme.typ";

/// Geometry layer — the Typst MAIN source. Owns page size + fold layout per
/// `?format=`, imports the styling theme (`/theme.typ`). Not user-editable.
const GEOMETRY_SRC: &str = include_str!("../templates/geometry.typ");
/// Live fields on poster covers (prices, delivery fees drawn from the data
/// onto artwork that leaves them blank), imported by the geometry layer.
const COVER_LIVE_SRC: &str = include_str!("../templates/cover_live.typ");
const COVER_LIVE_VPATH: &str = "/cover_live.typ";
const GEOMETRY_VPATH: &str = "/geometry.typ";
/// A shop's own poster layouts, imported by the geometry layer (see
/// [`site_overlay`]).
const SITE_POSTERS_VPATH: &str = "/site_posters.typ";
const NO_POSTERS: &str = "#let posters = (:)\n";

/// What one deployment adds that is that shop's own and so not part of this
/// tree: its poster layouts (`posters.typ`, see `templates/cover_live.typ`).
/// From the directory `SITE_OVERLAY_DIR` (the deploy script ships
/// `site/<profile>/` there), read once; without it, no poster layouts.
struct SiteOverlay {
    posters_src: String,
}

fn site_overlay() -> &'static SiteOverlay {
    static S: OnceLock<SiteOverlay> = OnceLock::new();
    S.get_or_init(|| {
        let dir = std::env::var("SITE_OVERLAY_DIR")
            .ok()
            .filter(|d| !d.trim().is_empty());
        let posters_src = dir
            .and_then(|d| {
                let path = std::path::Path::new(&d).join("posters.typ");
                match std::fs::read_to_string(&path) {
                    Ok(src) => {
                        tracing::info!("pdf: poster layouts from {}", path.display());
                        Some(src)
                    }
                    Err(e) => {
                        tracing::warn!("pdf: no poster layouts at {}: {e}", path.display());
                        None
                    }
                }
            })
            .unwrap_or_else(|| NO_POSTERS.to_string());
        SiteOverlay { posters_src }
    })
}

// Image assets the template can `image("/img/...")` for. Bytes are embedded
// at compile time so the running binary needs no filesystem access.
const ASSET_LADENFRONT: &[u8] = include_bytes!("../assets/ladenfront.jpg");
const ASSET_LADENFRONT_VPATH: &str = "/img/ladenfront.jpg";
/// Tri-fold cover photo used on the left panel of page 1 (the
/// outside-front when the leporello is folded shut). Static asset;
/// to change it, replace public/img/cover.jpg and rebuild.
const ASSET_COVER: &[u8] = include_bytes!("../assets/cover.jpg");
const ASSET_COVER_VPATH: &str = "/img/cover.jpg";
/// The "Tafel" theme's blackboard (466×232 mm at 200 dpi, generated — no
/// third-party image; see assets/README.md).
const ASSET_TAFEL: &[u8] = include_bytes!("../assets/tafel.jpg");
const ASSET_TAFEL_VPATH: &str = "/img/tafel.jpg";
/// The built-in "Tafel" theme (blackboard + chalk), chosen in the PDF-Editor
/// like a saved one: `pdf_theme_active_id` = [`TAFEL_THEME_ID`].
pub const TAFEL_SRC: &str = include_str!("../templates/tafel.typ");
pub const TAFEL_THEME_ID: &str = "builtin:tafel";
/// The built-in "Trattoria" theme: the look of an Italian poster cover (dark
/// paper, red/green/cream, spaced capitals), pointer [`TRATTORIA_THEME_ID`].
pub const TRATTORIA_SRC: &str = include_str!("../templates/trattoria.typ");
pub const TRATTORIA_THEME_ID: &str = "builtin:trattoria";
/// Its paper (466×232 mm at 170 dpi, generated — scripts/gen_paper.py).
const ASSET_TRATTORIA: &[u8] = include_bytes!("../assets/trattoria.jpg");
const ASSET_TRATTORIA_VPATH: &str = "/img/trattoria.jpg";

/// Line drawings a theme can put behind each category card, served as
/// `/img/motif/<name>.svg` (the "Tafel" theme draws them in faint chalk).
/// Tabler Icons, MIT — see assets/motifs/LICENSE.
macro_rules! motifs {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../assets/motifs/", $name, ".svg")) as &[u8])),*]
    };
}
const MOTIFS: &[(&str, &[u8])] = motifs![
    "baguette",
    "beer",
    "bottle",
    "bowl",
    "bowl-chopsticks",
    "bowl-spoon",
    "bread",
    "burger",
    "cake",
    "cheese",
    "chef-hat",
    "coffee",
    "egg",
    "egg-fried",
    "fish",
    "flame",
    "glass-cocktail",
    "glass-full",
    "ice-cream-2",
    "leaf",
    "meat",
    "mushroom",
    "pepper",
    "pizza",
    "salad",
    "sausage",
    "seedling",
    "soup",
    "tools-kitchen-2",
    "tools-kitchen-3",
    "tree",
    "map-pin",
];
const MOTIF_VPATH_PREFIX: &str = "/img/motif/";

fn motif_file(path: &str) -> Option<Bytes> {
    let name = path
        .strip_prefix(MOTIF_VPATH_PREFIX)?
        .strip_suffix(".svg")?;
    MOTIFS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, b)| Bytes::new(*b))
}

/// Virtual paths the template uses for admin-uploaded ad slots. The
/// template can reference these unconditionally; we serve a 1×1
/// transparent PNG if the slot is empty so `#image(...)` doesn't err.
const ASSET_AD_COVER_VPATH: &str = "/img/ad-cover.png";
const ASSET_AD_CENTER_VPATH: &str = "/img/ad-center.png";
const ASSET_AD_BACK_VPATH: &str = "/img/ad-back.png";
/// 1×1 transparent PNG. Returned for empty ad slots so the template
/// can reference the vpath unconditionally without crashing.
const BLANK_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

/// Path the template's `image()` call uses to fetch the QR. Bytes are built
/// per-payload from the live `site_url`, served by `MenuWorld::file()`.
const ASSET_QR_VPATH: &str = "/img/qr.svg";

// ---------------------------------------------------------------------------
// JSON shape consumed by the template
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct MenuPdfPayload {
    pub branding: Branding,
    pub categories: Vec<PdfCategory>,
    pub allergens: Vec<Legend>,
    pub additives: Vec<Legend>,
    pub site_url: String,
}

#[derive(Debug, Serialize)]
pub struct Branding {
    pub name: String,
    pub tagline: String,
    pub address: String,
    pub phone: String,
    pub hours_lines: Vec<String>,
    pub delivery_lines: Vec<String>,
    pub extras_pizza_lines: Vec<String>,
    pub extras_pasta_lines: Vec<String>,
    /// PDF front-cover mode: `"image"` (photo from the Cover-Bibliothek /
    /// bundled fallback) or `"text"` (typeset cover from this branding). Read
    /// from the `pdf_cover_mode` setting; `"text"` is the default so a fresh
    /// tenant never inherits another shop's baked cover photo.
    pub cover_mode: String,
    /// The active delivery zones as data, for a theme or cover that lays
    /// them out itself (`delivery_lines` holds them as finished text).
    pub delivery_zones: Vec<PdfZone>,
    /// Free delivery from this order value on, in cents (0 = never).
    pub free_delivery_cents: i64,
    /// A poster cover's title lines and slogan (`pdf_cover_title`,
    /// `pdf_cover_slogan`); empty = the poster's own.
    pub cover_title: Vec<String>,
    pub cover_slogan: String,
}

#[derive(Debug, Serialize)]
pub struct PdfZone {
    pub name: String,
    pub min_order_cents: i64,
    pub fee_cents: i64,
}

#[derive(Debug, Serialize)]
pub struct PdfCategory {
    pub id: String,
    pub name: String,
    pub items: Vec<PdfItem>,
}

#[derive(Debug, Serialize)]
pub struct PdfItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub menu_number: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allergen_codes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive_codes: Option<String>,
    pub is_spicy: bool,
    pub price_small_cents: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_large_cents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_small_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_large_label: Option<String>,
    /// Toppings included in the price (the Pizzablech: 3).
    #[serde(skip_serializing_if = "is_zero")]
    pub included_extras_count: i64,
    /// Flat price per further topping, in cents (Pizzablech 3 €, Pizza
    /// 36 cm 1 €); `None` = the extras catalogue prices.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flat_extra_price_cents: Option<i64>,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

#[derive(Debug, Serialize)]
pub struct Legend {
    pub code: String,
    pub name_de: String,
}

// ---------------------------------------------------------------------------
// World
// ---------------------------------------------------------------------------

// Fonts embedded in the binary at compile time via build.rs. Bundling them
// guarantees byte-identical PDF rendering on Mac dev, the Linux server, and
// any future build host — irrespective of what's installed system-wide.
const INTER_REGULAR: &[u8] = include_bytes!(concat!(env!("FONT_CACHE_DIR"), "/Inter-Regular.ttf"));
const INTER_BOLD: &[u8] = include_bytes!(concat!(env!("FONT_CACHE_DIR"), "/Inter-Bold.ttf"));
const INTER_ITALIC: &[u8] = include_bytes!(concat!(env!("FONT_CACHE_DIR"), "/Inter-Italic.ttf"));
/// Noto Color Emoji, subset to the ~50 food/status glyphs a menu uses
/// (🌶 above all). The full font is 10 MB; the subset is 108 KB. Rebuild
/// with fonttools' `pyftsubset` — see assets/fonts/README.md.
const NOTO_EMOJI: &[u8] = include_bytes!("../assets/fonts/NotoColorEmoji-subset.ttf");
/// The "Tafel" theme's chalk lettering (SIL OFL, google/fonts), subset to
/// Latin + German punctuation and € with pyftsubset (assets/fonts/README.md).
const CABIN_SKETCH_REGULAR: &[u8] =
    include_bytes!("../assets/fonts/CabinSketch-Regular-subset.ttf");
const CABIN_SKETCH_BOLD: &[u8] = include_bytes!("../assets/fonts/CabinSketch-Bold-subset.ttf");
const KALAM_REGULAR: &[u8] = include_bytes!("../assets/fonts/Kalam-Regular-subset.ttf");
const KALAM_BOLD: &[u8] = include_bytes!("../assets/fonts/Kalam-Bold-subset.ttf");
/// Storefront lettering for poster covers (site poster layouts):
/// Montserrat ExtraBold (SIL OFL, JulietaUla/Montserrat v7.222), subset as
/// the chalk fonts.
const MONTSERRAT_EXTRABOLD: &[u8] =
    include_bytes!("../assets/fonts/Montserrat-ExtraBold-subset.ttf");

/// One-time initialised resources shared across PDF requests. Searching the
/// system + embedded fonts on every request would be wasteful.
struct PdfResources {
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
}

/// The bundled emoji font's pictures, by code point: its glyphs are PNGs
/// (CBDT). The menu shows emoji as these images (geometry.typ
/// `emoji-images`) instead of as font glyphs: a bitmap font can only go into
/// a PDF as a Type 3 font, which print-shop checks report as not embedded.
fn emoji_pngs() -> &'static std::collections::BTreeMap<u32, Bytes> {
    static PNGS: OnceLock<std::collections::BTreeMap<u32, Bytes>> = OnceLock::new();
    PNGS.get_or_init(|| {
        let mut out = std::collections::BTreeMap::new();
        let Some(font) = Font::new(Bytes::new(NOTO_EMOJI), 0) else {
            return out;
        };
        let face = font.ttf();
        let Some(cmap) = face.tables().cmap else {
            return out;
        };
        for sub in cmap.subtables {
            if !sub.is_unicode() {
                continue;
            }
            sub.codepoints(|cp| {
                // Pictographs only: not the joiner / variation selector.
                if cp < 0x2190 || cp == 0xFE0F || out.contains_key(&cp) {
                    return;
                }
                let png = sub
                    .glyph_index(cp)
                    .and_then(|g| face.glyph_raster_image(g, u16::MAX));
                if let Some(png) = png {
                    out.insert(cp, Bytes::new(png.data.to_vec()));
                }
            });
        }
        out
    })
}

/// `/emoji/<hex>.png` → that emoji's picture.
fn emoji_file(path: &str) -> Option<Bytes> {
    let hex = path
        .strip_prefix(EMOJI_VPATH_PREFIX)?
        .strip_suffix(".png")?;
    let cp = u32::from_str_radix(hex, 16).ok()?;
    emoji_pngs().get(&cp).cloned()
}

const EMOJI_VPATH_PREFIX: &str = "/emoji/";

/// The bundled fonts, parsed once from the binary's own bytes.
///
/// No temp-dir copies, no font searcher, no embedded Typst default set: the
/// templates only ever ask for these, so they are the whole book. `Bytes::new`
/// on a `&'static [u8]` stores the reference — the font data is never
/// duplicated onto the heap.
fn resources() -> &'static PdfResources {
    static R: OnceLock<PdfResources> = OnceLock::new();
    R.get_or_init(|| {
        let mut fonts = Vec::with_capacity(8);
        for (name, bytes) in [
            ("Inter-Regular", INTER_REGULAR),
            ("Inter-Bold", INTER_BOLD),
            ("Inter-Italic", INTER_ITALIC),
            ("NotoColorEmoji-subset", NOTO_EMOJI),
            ("CabinSketch-Regular", CABIN_SKETCH_REGULAR),
            ("CabinSketch-Bold", CABIN_SKETCH_BOLD),
            ("Kalam-Regular", KALAM_REGULAR),
            ("Kalam-Bold", KALAM_BOLD),
            ("Montserrat-ExtraBold", MONTSERRAT_EXTRABOLD),
        ] {
            match Font::new(Bytes::new(bytes), 0) {
                Some(f) => fonts.push(f),
                None => tracing::warn!("pdf: bundled font {name} is unparseable — skipped"),
            }
        }
        let book = FontBook::from_fonts(&fonts);
        tracing::info!(
            "pdf: registered {} bundled fonts (Inter, emoji subset, Tafel chalk, Montserrat)",
            fonts.len()
        );
        PdfResources {
            book: LazyHash::new(book),
            fonts,
        }
    })
}

struct MenuWorld {
    library: LazyHash<Library>,
    /// Main source = the geometry layer (`/geometry.typ`).
    main_id: FileId,
    main_source: Source,
    /// The styling theme (`/theme.typ`), imported by the geometry layer.
    /// Either the admin's active `pdf_themes.source` override or the bundled
    /// `TEMPLATE_SRC`.
    theme_id: FileId,
    theme_source: Source,
    /// `cover_live.typ` (live fields on poster covers).
    cover_live_id: FileId,
    cover_live_source: Source,
    /// The site's poster layouts (`site_overlay`).
    site_posters_id: FileId,
    site_posters_source: Source,
    /// SVG bytes for the QR code that points at `payload.site_url`. Generated
    /// once per request — Typst caches the resulting `Image`.
    qr_svg: Vec<u8>,
    /// Admin-uploaded cover bytes. `None` falls back to the bundled
    /// `ASSET_LADENFRONT`.
    cover_override: Option<Vec<u8>>,
    /// Admin-uploaded ad bytes per slot.
    ad_cover: Option<Vec<u8>>,
    ad_center: Option<Vec<u8>>,
    ad_back: Option<Vec<u8>>,
}

impl MenuWorld {
    fn new(
        payload: &MenuPdfPayload,
        overrides: &PdfOverrides,
        format: &str,
        show_ingredients: bool,
    ) -> anyhow::Result<Self> {
        use typst::foundations::Value;
        let json = serde_json::to_string(payload)?;
        let mut inputs = Dict::new();
        inputs.insert("data".into(), Value::Str(json.into()));
        // `format` selects the page geometry / fold layout in the template.
        // "a5-zickzack-marks" is the Zickzack with crop + fold marks: the
        // theme only ever sees the format itself.
        let (format, marks) = match format {
            "a5-zickzack-marks" => ("a5-zickzack", true),
            f => (f, false),
        };
        inputs.insert("format".into(), Value::Str(format.into()));
        inputs.insert("marks".into(), Value::Bool(marks));
        // Which cover picture this is: `cover_live.typ` draws live prices
        // onto the one artwork it knows, found by this hash (the
        // Cover-Bibliothek names uploads by it too).
        inputs.insert(
            "cover-sha".into(),
            Value::Str(sha256_hex(overrides.cover_image.as_deref().unwrap_or(ASSET_COVER)).into()),
        );
        // The emoji to show as pictures (`emoji_pngs`).
        inputs.insert(
            "emoji".into(),
            Value::Array(
                emoji_pngs()
                    .keys()
                    .map(|&cp| Value::Int(i64::from(cp)))
                    .collect(),
            ),
        );
        // `show-ingredients` toggles the per-item description line. Default
        // (true) is the full hand-out menu; `?condensed=1` flips it off for
        // the airy title+price-only in-house menu.
        inputs.insert(
            "show-ingredients".into(),
            typst::foundations::Value::Bool(show_ingredients),
        );
        let library = Library::builder().with_inputs(inputs).build();

        // Main = the geometry layer (page setup + fold layout per format).
        let main_id = FileId::new(None, VirtualPath::new(GEOMETRY_VPATH));
        let main_source = Source::new(main_id, GEOMETRY_SRC.to_string());

        // Theme = admin override (pdf_themes.source) or the bundled default.
        let theme_str = overrides
            .template_source
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(TEMPLATE_SRC);
        let theme_id = FileId::new(None, VirtualPath::new(THEME_VPATH));
        let theme_source = Source::new(theme_id, theme_str.to_string());
        let cover_live_id = FileId::new(None, VirtualPath::new(COVER_LIVE_VPATH));
        let cover_live_source = Source::new(cover_live_id, COVER_LIVE_SRC.to_string());
        let site_posters_id = FileId::new(None, VirtualPath::new(SITE_POSTERS_VPATH));
        let site_posters_source =
            Source::new(site_posters_id, site_overlay().posters_src.clone());

        let qr_svg = render_qr_svg(&payload.site_url);

        Ok(Self {
            library: LazyHash::new(library),
            main_id,
            main_source,
            theme_id,
            theme_source,
            cover_live_id,
            cover_live_source,
            site_posters_id,
            site_posters_source,
            qr_svg,
            cover_override: tafel_cover(theme_str, overrides.cover_image.as_deref())
                .or_else(|| overrides.cover_image.clone()),
            ad_cover: overrides.ad_cover_image.clone(),
            ad_center: overrides.ad_center_image.clone(),
            ad_back: overrides.ad_back_image.clone(),
        })
    }
}

/// Admin-supplied overrides resolved out of `app_settings` + the
/// uploads directory before render time. Empty fields fall back to
/// the binary-embedded factory defaults.
#[derive(Debug, Clone, Default)]
pub struct PdfOverrides {
    pub template_source: Option<String>,
    pub cover_image: Option<Vec<u8>>,
    pub ad_cover_image: Option<Vec<u8>>,
    pub ad_center_image: Option<Vec<u8>>,
    pub ad_back_image: Option<Vec<u8>>,
}

/// Build a QR SVG that points at `url`. Same shape as the homepage QR but
/// rendered without a quiet-zone tweak (the template handles padding).
fn render_qr_svg(url: &str) -> Vec<u8> {
    use qrcode::render::svg;
    use qrcode::QrCode;
    match QrCode::new(url.as_bytes()) {
        Ok(code) => code
            .render::<svg::Color>()
            .min_dimensions(400, 400)
            .quiet_zone(true)
            .dark_color(svg::Color("#2a1a0f"))
            .light_color(svg::Color("#ffffff"))
            .build()
            .into_bytes(),
        Err(_) => Vec::new(),
    }
}

impl World for MenuWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &resources().book
    }

    fn main(&self) -> FileId {
        self.main_id
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main_id {
            Ok(self.main_source.clone())
        } else if id == self.theme_id {
            // The geometry layer `#import "/theme.typ"` resolves here.
            Ok(self.theme_source.clone())
        } else if id == self.cover_live_id {
            Ok(self.cover_live_source.clone())
        } else if id == self.site_posters_id {
            Ok(self.site_posters_source.clone())
        } else {
            Err(FileError::NotFound(id.vpath().as_rootless_path().into()))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        let path = id.vpath().as_rooted_path();
        match path.to_str() {
            // `ladenfront.jpg` is the in-body shop-front photo — always the
            // bundled asset (the admin cover library does NOT drive this).
            Some(p) if p == ASSET_LADENFRONT_VPATH => Ok(Bytes::new(ASSET_LADENFRONT.to_vec())),
            // `cover.jpg` is the PDF FRONT PAGE — this is what the admin
            // "Cover-Bibliothek" controls. Apply the active cover override
            // here; fall back to the bundled cover when none is active.
            Some(p) if p == ASSET_COVER_VPATH => {
                let bytes = self
                    .cover_override
                    .clone()
                    .unwrap_or_else(|| ASSET_COVER.to_vec());
                Ok(Bytes::new(bytes))
            }
            Some(p) if p == ASSET_AD_COVER_VPATH => Ok(Bytes::new(
                self.ad_cover.clone().unwrap_or_else(|| BLANK_PNG.to_vec()),
            )),
            Some(p) if p == ASSET_AD_CENTER_VPATH => Ok(Bytes::new(
                self.ad_center.clone().unwrap_or_else(|| BLANK_PNG.to_vec()),
            )),
            Some(p) if p == ASSET_AD_BACK_VPATH => Ok(Bytes::new(
                self.ad_back.clone().unwrap_or_else(|| BLANK_PNG.to_vec()),
            )),
            Some(p) if p == ASSET_QR_VPATH => Ok(Bytes::new(self.qr_svg.clone())),
            Some(p) if p == ASSET_TAFEL_VPATH => Ok(Bytes::new(ASSET_TAFEL)),
            Some(p) if p == ASSET_TRATTORIA_VPATH => Ok(Bytes::new(ASSET_TRATTORIA)),
            Some(p) if p.starts_with(EMOJI_VPATH_PREFIX) => emoji_file(p)
                .ok_or_else(|| FileError::NotFound(id.vpath().as_rootless_path().into())),
            Some(p) if p.starts_with(MOTIF_VPATH_PREFIX) => motif_file(p)
                .ok_or_else(|| FileError::NotFound(id.vpath().as_rootless_path().into())),
            _ => Err(FileError::NotFound(id.vpath().as_rootless_path().into())),
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        resources().fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        // Deterministic enough for a menu PDF; full date support can come later.
        None
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render the supplied menu payload to PDF bytes. Returns a (hopefully short)
/// human-readable error string on failure.
pub fn render_menu_pdf(
    payload: &MenuPdfPayload,
    overrides: &PdfOverrides,
    format: &str,
    show_ingredients: bool,
) -> anyhow::Result<Vec<u8>> {
    let document = compile_menu(payload, overrides, format, show_ingredients)?;
    let opts = typst_pdf::PdfOptions::default();
    let bytes = typst_pdf::pdf(&document, &opts).map_err(|errs| {
        let msgs: Vec<_> = errs.iter().map(|e| eco_format!("{}", e.message)).collect();
        anyhow::anyhow!("Typst PDF export failed: {}", msgs.join("; "))
    })?;
    Ok(bytes)
}

/// The front cover alone (format "cover", one A5 page, live prices on a
/// poster cover) as a JPEG `width_px` wide — the homepage's picture.
pub fn render_cover_jpeg(
    payload: &MenuPdfPayload,
    overrides: &PdfOverrides,
    width_px: u32,
) -> anyhow::Result<Vec<u8>> {
    let document = compile_menu(payload, overrides, "cover", true)?;
    let page = document
        .pages
        .first()
        .ok_or_else(|| anyhow::anyhow!("cover: no page"))?;
    let pixel_per_pt = width_px as f32 / page.frame.width().to_pt() as f32;
    let pixmap = typst_render::render(page, pixel_per_pt);
    // The cover is opaque, so premultiplied RGBA is plain RGBA: drop alpha.
    let rgb: Vec<u8> = pixmap
        .data()
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let img = image::RgbImage::from_raw(pixmap.width(), pixmap.height(), rgb)
        .ok_or_else(|| anyhow::anyhow!("cover: bad pixmap"))?;
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&img)?;
    Ok(out.into_inner())
}

/// Compile the menu document (geometry layer + theme) for `format`.
fn compile_menu(
    payload: &MenuPdfPayload,
    overrides: &PdfOverrides,
    format: &str,
    show_ingredients: bool,
) -> anyhow::Result<PagedDocument> {
    let world = MenuWorld::new(payload, overrides, format, show_ingredients)?;
    tracing::debug!(
        "pdf: compiling, fonts={}, categories={}, items={}",
        resources().fonts.len(),
        payload.categories.len(),
        payload
            .categories
            .iter()
            .map(|c| c.items.len())
            .sum::<usize>(),
    );
    let warned = typst::compile::<PagedDocument>(&world);
    // Drop memoised layout results older than a few compilations so the
    // cache stays bounded over the process lifetime.
    comemo::evict(4);

    for w in &warned.warnings {
        tracing::warn!("typst warn: {}", w.message);
    }

    let document = warned.output.map_err(|errs| {
        for e in &errs {
            tracing::error!("typst: {} (span={:?})", e.message, e.span);
        }
        let msgs: Vec<_> = errs.iter().map(|e| eco_format!("{}", e.message)).collect();
        anyhow::anyhow!("Typst compile failed: {}", msgs.join("; "))
    })?;
    Ok(document)
}

// ---------------------------------------------------------------------------
// Build a payload from the live database
// ---------------------------------------------------------------------------

/// `use_next_prices`: render the STAGED prices (`next_price_*`, falling back
/// to the live price where nothing is staged) instead of the live ones. This
/// is how the admin produces the print-shop PDF before the new prices go
/// live — see `/admin/next-prices` and migration 20260914000001.
pub async fn load_menu_payload(
    db: &SqlitePool,
    site_url: String,
    shop_branding: &rusterando_frontend::branding::Branding,
    use_next_prices: bool,
) -> anyhow::Result<MenuPdfPayload> {
    let cat_rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT id, name, sort_order FROM menu_categories WHERE is_active = 1 ORDER BY sort_order",
    )
    .fetch_all(db)
    .await?;

    let item_rows: Vec<(
        String,         // id
        String,         // category_id
        Option<String>, // menu_number
        String,         // name
        Option<String>, // description
        i64,            // price_small_cents
        Option<i64>,    // price_large_cents
        Option<String>, // size_small_label
        Option<String>, // size_large_label
        Option<String>, // allergen_codes
        Option<String>, // additive_codes
        i64,            // is_spicy
        i64,            // is_available
        i64,            // sort_order
        i64,            // included_extras_count
        Option<i64>,    // flat_extra_price_cents
    )> = {
        // Staged prices overlay the live ones per column; an item without a
        // large size stays without one even if a stray next_large is set.
        let (small_col, large_col) = if use_next_prices {
            (
                "COALESCE(next_price_small_cents, price_small_cents)",
                "CASE WHEN price_large_cents IS NULL THEN NULL \
                      ELSE COALESCE(next_price_large_cents, price_large_cents) END",
            )
        } else {
            ("price_small_cents", "price_large_cents")
        };
        let sql = format!(
            "SELECT id, category_id, menu_number, name, description,
                    {small_col}, {large_col}, size_small_label, size_large_label,
                    allergen_codes, additive_codes, is_spicy, is_available, sort_order,
                    included_extras_count, flat_extra_price_cents
             FROM menu_items
             WHERE is_listed = 1 AND is_available = 1
             ORDER BY sort_order,
                      CAST(COALESCE(menu_number, '') AS INTEGER),
                      menu_number"
        );
        sqlx::query_as(&sql).fetch_all(db).await?
    };

    let mut categories: Vec<PdfCategory> = cat_rows
        .into_iter()
        .map(|(id, name, _)| PdfCategory {
            id,
            name,
            items: Vec::new(),
        })
        .collect();

    for r in item_rows {
        let cat_id = r.1;
        if let Some(cat) = categories.iter_mut().find(|c| c.id == cat_id) {
            cat.items.push(PdfItem {
                menu_number: r.2,
                name: r.3,
                description: r.4.filter(|s| !s.is_empty()),
                price_small_cents: r.5,
                price_large_cents: r.6,
                size_small_label: r.7,
                size_large_label: r.8,
                allergen_codes: r.9.filter(|s| !s.is_empty()),
                additive_codes: r.10.filter(|s| !s.is_empty()),
                is_spicy: r.11 != 0,
                included_extras_count: r.14,
                flat_extra_price_cents: r.15,
            });
        }
    }
    categories.retain(|c| !c.items.is_empty());

    let allergens: Vec<Legend> = sqlx::query_as::<_, (String, String)>(
        "SELECT code, name_de FROM allergen_legend ORDER BY code",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|(code, name_de)| Legend { code, name_de })
    .collect();

    let additives: Vec<Legend> = sqlx::query_as::<_, (String, String)>(
        "SELECT code, name_de FROM additive_legend ORDER BY
         CASE WHEN length(code) = 1 AND code GLOB '[0-9]' THEN 0 ELSE 1 END, code",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|(code, name_de)| Legend { code, name_de })
    .collect();

    // Build the side-panel "Wir liefern" lines from the live
    // delivery_zones table so renaming a zone or changing a fee is one
    // /admin/settings click, not a code deploy. Free-delivery threshold
    // (also admin-tunable) is prepended so customers see it on the
    // printed flyer too.
    let zone_rows: Vec<(Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT name, fee_cents, min_order_cents
         FROM delivery_zones
         WHERE is_active = 1
         ORDER BY fee_cents, id",
    )
    .fetch_all(db)
    .await?;

    let format_eur = |cents: i64| -> String {
        let euros = cents / 100;
        let rest = (cents % 100).abs();
        format!("{euros},{rest:02} €")
    };

    let delivery_zones: Vec<PdfZone> = zone_rows
        .iter()
        .map(|(name, fee, min_order)| PdfZone {
            name: name.clone().unwrap_or_else(|| "?".into()),
            min_order_cents: *min_order,
            fee_cents: *fee,
        })
        .collect();
    let mut delivery_lines: Vec<String> = zone_rows
        .into_iter()
        .map(|(name, fee, min_order)| {
            format!(
                "{} ab {} + {}",
                name.unwrap_or_else(|| "?".into()),
                format_eur(min_order),
                format_eur(fee),
            )
        })
        .collect();

    let free_threshold =
        rusterando_frontend::pages::settings::ssr::free_delivery_threshold_cents(db).await;
    if free_threshold > 0 {
        delivery_lines.insert(
            0,
            format!("Lieferung gratis ab {}", format_eur(free_threshold)),
        );
    }

    let phone_line = if shop_branding.shop_phone.is_empty() {
        String::new()
    } else {
        format!("Tel. {}", shop_branding.shop_phone)
    };

    // Pull the editable text fields out of app_settings. Newline-split
    // for the list fields so the admin's <textarea> shape lands 1:1
    // in the PDF.
    let tagline = read_setting(db, "pdf_tagline").await.unwrap_or_default();
    let hours_lines = read_setting_lines(db, "pdf_hours")
        .await
        .unwrap_or_else(default_hours_lines);
    let extras_pizza_lines = read_setting_lines(db, "pdf_extras_pizza")
        .await
        .unwrap_or_else(default_extras_pizza_lines);
    let extras_pasta_lines = read_setting_lines(db, "pdf_extras_pasta")
        .await
        .unwrap_or_else(default_extras_pasta_lines);

    // Front-cover mode. Default "text" so a tenant with no cover photo gets a
    // proper own-branded text cover instead of the bundled placeholder
    // photo. Only "image" switches to the photo path.
    let cover_mode = match read_setting(db, "pdf_cover_mode").await.as_deref() {
        Some("image") => "image".to_string(),
        _ => "text".to_string(),
    };

    Ok(MenuPdfPayload {
        branding: Branding {
            name: shop_branding.display_name(),
            tagline,
            address: shop_branding.full_address(),
            phone: phone_line,
            hours_lines,
            delivery_lines,
            extras_pizza_lines,
            extras_pasta_lines,
            cover_mode,
            delivery_zones,
            free_delivery_cents: free_threshold,
            cover_title: read_setting_lines(db, "pdf_cover_title")
                .await
                .unwrap_or_default(),
            cover_slogan: read_setting(db, "pdf_cover_slogan")
                .await
                .unwrap_or_default(),
        },
        categories,
        allergens,
        additives,
        site_url,
    })
}

// Until the admin fills them in (PDF-Editor), the PDF shows no opening
// hours and no extras: a shop's own, never guessed.
fn default_hours_lines() -> Vec<String> {
    Vec::new()
}
fn default_extras_pizza_lines() -> Vec<String> {
    Vec::new()
}
fn default_extras_pasta_lines() -> Vec<String> {
    Vec::new()
}

/// Look up a single app_settings value. Returns `Some` only if the row
/// exists AND the value is non-empty after trimming — empty strings
/// fall back to defaults at the call site.
async fn read_setting(db: &SqlitePool, key: &str) -> Option<String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM app_settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    row.map(|(v,)| v).filter(|v| !v.trim().is_empty())
}
async fn read_setting_lines(db: &SqlitePool, key: &str) -> Option<Vec<String>> {
    read_setting(db, key).await.map(|s| {
        s.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    })
}

/// Load template source + admin-uploaded image bytes referenced by
/// `app_settings`. Falls back to bundled defaults silently when a
/// setting is missing or the upload file is unreadable.
///
/// The "cover" and "template_source" sides resolve through the
/// `pdf_cover_images` / `pdf_themes` library tables introduced by
/// migration 20260522000001. Each has an active-pointer setting
/// (`pdf_cover_active_id`, `pdf_theme_active_id`); empty pointer →
/// no override → renderer uses the compiled-in factory.
///
/// The ad-slot images (cover/center/back) are independent — they
/// still read directly from their single-value `pdf_ad_*_image`
/// settings. They're overlays, not the main cover.
/// `uploads_dir` is the on-disk base for admin uploads,
/// `<site_root>/img/uploads` (e.g. `html/img/uploads` on prod). Cover
/// files live in its `covers/` subdir; ad-slot images live directly under
/// it. Passed in (rather than hardcoded) so the path always matches
/// whatever directory the static server serves `/img/uploads/` from.
/// `shared` is the reserved `_shared` tenant pool (see
/// `tenant::load_shared_pool`). When this tenant hasn't configured its own
/// PDF cover image or theme/template, the value falls back to `_shared`'s
/// before the bundled `include_*!` default. Ad-slot overlays are tenant-only
/// (no shared fallback — they're per-shop promos, not chrome). Pass `None` to
/// disable the shared fallback (single-tenant Model A, or `_shared` unopenable).
pub async fn load_pdf_overrides(
    db: &SqlitePool,
    uploads_dir: &str,
    shared: Option<&SqlitePool>,
) -> PdfOverrides {
    let template_source = match read_active_theme_source(db).await {
        Some(s) => Some(s),
        // Fall back to the shared tenant's active theme. `_shared`'s uploads
        // live in the same uploads_dir tree (covers/ subdir), so cover bytes
        // resolve the same way.
        None => match shared {
            Some(s) => read_active_theme_source(s).await,
            None => None,
        },
    };
    let cover_image = match read_active_cover_bytes(db, uploads_dir).await {
        Some(b) => Some(b),
        None => match shared {
            Some(s) => read_active_cover_bytes(s, uploads_dir).await,
            None => None,
        },
    };
    let ad_cover_image = read_image_setting(db, "pdf_ad_cover_image", uploads_dir).await;
    let ad_center_image = read_image_setting(db, "pdf_ad_center_image", uploads_dir).await;
    let ad_back_image = read_image_setting(db, "pdf_ad_back_image", uploads_dir).await;
    PdfOverrides {
        template_source,
        cover_image,
        ad_cover_image,
        ad_center_image,
        ad_back_image,
    }
}

/// Resolve `pdf_theme_active_id` → `pdf_themes.source`. Returns
/// `None` if no theme is active or the active row was deleted out
/// from under the pointer.
async fn read_active_theme_source(db: &SqlitePool) -> Option<String> {
    let active = read_setting(db, "pdf_theme_active_id").await?;
    match active.trim() {
        TAFEL_THEME_ID => return Some(TAFEL_SRC.to_string()),
        TRATTORIA_THEME_ID => return Some(TRATTORIA_SRC.to_string()),
        _ => {}
    }
    let id: i64 = active.trim().parse().ok()?;
    sqlx::query_scalar::<_, String>("SELECT source FROM pdf_themes WHERE id = ?1")
        .bind(id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
}

/// Resolve `pdf_cover_active_id` → `pdf_cover_images.filename` →
/// bytes under `<uploads_dir>/covers/<filename>`. Falls back to
/// `<uploads_dir>/<filename>` so rows imported from the legacy
/// `pdf_cover_image` setting (where files lived in the parent
/// dir, not the `covers/` subdir) still resolve. Returns `None`
/// for any failure — the renderer then uses the bundled
/// `ASSET_COVER`.
async fn read_active_cover_bytes(db: &SqlitePool, uploads_dir: &str) -> Option<Vec<u8>> {
    let id: i64 = read_setting(db, "pdf_cover_active_id")
        .await?
        .trim()
        .parse()
        .ok()?;
    let filename: String =
        sqlx::query_scalar("SELECT filename FROM pdf_cover_images WHERE id = ?1")
            .bind(id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten()?;
    // Defence-in-depth: reject path traversal even though the upload
    // route only writes hash-derived names. Belt-and-suspenders for
    // legacy imports too.
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return None;
    }
    let base = std::path::Path::new(uploads_dir);
    let primary = base.join("covers").join(&filename);
    let raw = std::fs::read(&primary).ok().or_else(|| {
        // Legacy: rows imported from the old single-value pdf_cover_image
        // setting reference files that live under the uploads root directly.
        std::fs::read(base.join(&filename)).ok()
    })?;
    Some(normalise_cover_to_jpeg(raw))
}

/// Lower-case hex SHA-256, the same naming the Cover-Bibliothek upload uses.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The Typst template fetches the cover via `image("/img/cover.jpg")`, so
/// Typst infers the JPEG decoder from the `.jpg` extension regardless of
/// what bytes `MenuWorld::file` returns. Admins may upload PNG (the upload
/// route accepts both), which would feed PNG bytes into the JPEG decoder
/// and fail the whole render with "Illegal start bytes:8950" (the PNG
/// signature). So: if the bytes aren't already JPEG, transcode to JPEG.
/// Already-JPEG bytes pass through untouched (no lossy re-encode). On any
/// decode/encode failure we return the original bytes — the renderer logs
/// a clear Typst error rather than this swallowing it silently.
fn normalise_cover_to_jpeg(bytes: Vec<u8>) -> Vec<u8> {
    // JPEG SOI marker — already the right format, pass through.
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return bytes;
    }
    use image::ImageFormat;
    use std::io::Cursor;
    let Ok(img) = image::load_from_memory(&bytes) else {
        return bytes;
    };
    // PNG may carry alpha; flatten to RGB8 so the JPEG encoder (no alpha)
    // doesn't choke. White-background flatten matches how the cover sits
    // on the printed page.
    let rgb = img.to_rgb8();
    let mut out = Cursor::new(Vec::new());
    if image::DynamicImage::ImageRgb8(rgb)
        .write_to(&mut out, ImageFormat::Jpeg)
        .is_err()
    {
        return bytes;
    }
    out.into_inner()
}

/// Resolve a `/img/uploads/<hash>.<ext>` setting to its on-disk bytes.
/// `None` if the setting is empty, the path is malformed, or the file
/// has been deleted from the uploads directory.
async fn read_image_setting(db: &SqlitePool, key: &str, uploads_dir: &str) -> Option<Vec<u8>> {
    let value = read_setting(db, key).await?;
    let trimmed = value.trim().trim_start_matches('/');
    // Only honour our own uploads directory. Anything else is rejected
    // so a malicious admin can't read arbitrary files via this setting.
    let rel = trimmed.strip_prefix("img/uploads/")?;
    if rel.contains('/') || rel.contains('\\') {
        return None;
    }
    let path = std::path::Path::new(uploads_dir).join(rel);
    std::fs::read(&path).ok()
}

/// Convenience: load + render in one go. Compile runs on a blocking pool so
/// the async runtime stays responsive during the ~hundreds-of-ms render.
/// `format` selects the page geometry / fold layout the Typst template
/// renders: `"trifold"` (default — the 443×210 three-panel sheet),
/// `"a5-zickzack"` (DIN-A5 portrait, 6-panel accordion, 444×210 plus 3 mm
/// bleed for the 4c print job) or `"a5-zickzack-marks"` (the same with crop
/// and fold marks outside the bleed). Unknown values fall back to trifold in
/// the caller.
#[allow(clippy::too_many_arguments)]
pub async fn build_menu_pdf(
    db: &SqlitePool,
    site_url: String,
    shop_branding: &rusterando_frontend::branding::Branding,
    uploads_dir: &str,
    format: &str,
    show_ingredients: bool,
    shared: Option<&SqlitePool>,
    use_next_prices: bool,
) -> anyhow::Result<Vec<u8>> {
    let payload = load_menu_payload(db, site_url, shop_branding, use_next_prices).await?;
    let overrides = load_pdf_overrides(db, uploads_dir, shared).await;
    let format = format.to_string();
    tokio::task::spawn_blocking(move || {
        render_menu_pdf(&payload, &overrides, &format, show_ingredients)
    })
    .await?
}

// ---------------------------------------------------------------------------
// Static menu.pdf cache — the nginx `error_page 502/503/504` fallback. Served
// straight from the site root while the app is briefly down during a restart,
// and linked from the offline page, so customers always see the current menu.
// Re-rendered at boot + after every menu/extras/branding edit.
// ---------------------------------------------------------------------------

/// Render the tri-fold (full) + condensed menus to the site root, atomically
/// (temp file then rename so nginx never serves a half-written PDF).
///
/// `slug` scopes the filenames per tenant: empty (single-tenant / apex) keeps
/// the canonical `menu.pdf` / `menu-kompakt.pdf` (the nginx offline-fallback
/// names). A non-empty slug writes `<slug>-menu.pdf` / `<slug>-menu-kompakt.pdf`
/// so multi-tenant shops don't overwrite each other's cache. Best-effort:
/// returns the first error but always attempts both.
pub async fn write_menu_pdf_cache(
    db: &SqlitePool,
    site_root: &str,
    uploads_dir: &str,
    site_url: String,
    shop_branding: &rusterando_frontend::branding::Branding,
    slug: &str,
    shared: Option<&SqlitePool>,
) -> anyhow::Result<()> {
    let prefix = if slug.is_empty() {
        String::new()
    } else {
        format!("{slug}-")
    };
    for (stem, show_ingredients) in [("menu.pdf", true), ("menu-kompakt.pdf", false)] {
        let file = format!("{prefix}{stem}");
        let bytes = build_menu_pdf(
            db,
            site_url.clone(),
            shop_branding,
            uploads_dir,
            "trifold",
            show_ingredients,
            shared,
            // The offline-fallback cache is always the LIVE menu.
            false,
        )
        .await?;
        let dst = std::path::Path::new(site_root).join(&file);
        let tmp = std::path::Path::new(site_root).join(format!(".{file}.tmp"));
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &dst).await?;
        tracing::info!(
            "wrote static menu cache {} ({} bytes)",
            dst.display(),
            bytes.len()
        );
    }
    if let Err(e) = write_home_cover(db, uploads_dir, site_url, shop_branding, shared).await {
        tracing::warn!("homepage cover not written: {e}");
    }
    Ok(())
}

/// `app_settings` key naming the homepage's cover picture (a file in
/// `<uploads>/covers/`), empty = none. Written here, read by the homepage.
pub const HOME_COVER_KEY: &str = "home_cover_file";

/// The homepage's cover picture: the PDF's front cover (format "cover") as a
/// JPEG, made together with the static menu cache — on every save that
/// changes the menu, never per request. Only for a poster cover with live
/// prices (`cover_live.typ`); for any other cover the setting is cleared.
/// The file is named by everything that goes into it, so an unchanged menu
/// costs nothing and browsers may keep the file for good.
pub async fn write_home_cover(
    db: &SqlitePool,
    uploads_dir: &str,
    site_url: String,
    shop_branding: &rusterando_frontend::branding::Branding,
    shared: Option<&SqlitePool>,
) -> anyhow::Result<()> {
    let payload = load_menu_payload(db, site_url, shop_branding, false).await?;
    let overrides = load_pdf_overrides(db, uploads_dir, shared).await;
    let cover_sha = sha256_hex(overrides.cover_image.as_deref().unwrap_or(ASSET_COVER));
    let live = payload.branding.cover_mode == "image"
        && live_poster_shas().iter().any(|s| *s == cover_sha);
    let covers = std::path::Path::new(uploads_dir).join("covers");
    let name = if live {
        let mut key = serde_json::to_vec(&payload)?;
        for part in [
            cover_sha.as_str(),
            overrides.template_source.as_deref().unwrap_or(""),
            GEOMETRY_SRC,
            COVER_LIVE_SRC,
            site_overlay().posters_src.as_str(),
            COVER_BOARD_LOOK,
        ] {
            key.extend(part.bytes());
        }
        let name = format!("home-{}.jpg", &sha256_hex(&key)[..16]);
        let dst = covers.join(&name);
        if !dst.exists() {
            let jpeg = tokio::task::spawn_blocking(move || {
                render_cover_jpeg(&payload, &overrides, HOME_COVER_WIDTH_PX)
            })
            .await??;
            tokio::fs::create_dir_all(&covers).await?;
            let tmp = covers.join(format!(".{name}.tmp"));
            tokio::fs::write(&tmp, &jpeg).await?;
            tokio::fs::rename(&tmp, &dst).await?;
            tracing::info!(
                "wrote homepage cover {} ({} bytes)",
                dst.display(),
                jpeg.len()
            );
        }
        name
    } else {
        String::new()
    };
    let old = read_setting(db, HOME_COVER_KEY).await.unwrap_or_default();
    if name != old {
        sqlx::query(
            "INSERT INTO app_settings (key, value, label_de, hint_de) \
             VALUES (?1, ?2, 'Startseite: Titelblatt-Bild', 'Wird automatisch geschrieben.') \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(HOME_COVER_KEY)
        .bind(&name)
        .execute(db)
        .await?;
        // The previous picture is this shop's alone (its name hashes the
        // shop's own data): gone with the setting.
        if old.starts_with("home-") && !old.contains(['/', '\\']) {
            let _ = tokio::fs::remove_file(covers.join(&old)).await;
        }
    }
    Ok(())
}

/// 1000 px: sharp at the size the homepage shows it, on a phone too.
const HOME_COVER_WIDTH_PX: u32 = 1000;

/// Part of the homepage picture's name: change it when [`cover_on_board`]
/// changes, so the picture is made anew.
const COVER_BOARD_LOOK: &str = "tafel-board-lighten-1";

/// The cover for the "Tafel" theme: a live poster (dark stone ground) gets
/// the theme's blackboard instead — the front page then matches the board
/// pages and the website's Tafel look. `None`: another theme, or a cover
/// that isn't one of the posters (a photo stays as it is).
fn tafel_cover(theme: &str, cover: Option<&[u8]>) -> Option<Vec<u8>> {
    if theme != TAFEL_SRC {
        return None;
    }
    let cover = cover.unwrap_or(ASSET_COVER);
    let sha = sha256_hex(cover);
    if !live_poster_shas().contains(&sha) {
        return None;
    }
    static MADE: OnceLock<std::sync::Mutex<Option<(String, Vec<u8>)>>> = OnceLock::new();
    let made = MADE.get_or_init(Default::default);
    if let Some((k, bytes)) = made.lock().ok().and_then(|m| m.clone()) {
        if k == sha {
            return Some(bytes);
        }
    }
    match cover_on_board(cover) {
        Ok(bytes) => {
            if let Ok(mut m) = made.lock() {
                *m = Some((sha, bytes.clone()));
            }
            Some(bytes)
        }
        Err(e) => {
            tracing::warn!("Tafel cover: {e:#}");
            None
        }
    }
}

/// The poster laid on the Tafel board: each pixel keeps the lighter of
/// poster and board ("lighten"). The near-black stone shows the board;
/// lettering, prices and the photo keep their colours. The board's middle,
/// cut to the poster's shape and scaled to it.
fn cover_on_board(cover: &[u8]) -> anyhow::Result<Vec<u8>> {
    use image::imageops::{crop_imm, resize, FilterType};
    let mut poster = image::load_from_memory(cover)?.to_rgb8();
    let board = image::load_from_memory(ASSET_TAFEL)?.to_rgb8();
    let (w, h) = poster.dimensions();
    let (bw, bh) = board.dimensions();
    let (cw, ch) = if u64::from(bw) * u64::from(h) > u64::from(bh) * u64::from(w) {
        ((u64::from(bh) * u64::from(w) / u64::from(h)) as u32, bh)
    } else {
        (bw, (u64::from(bw) * u64::from(h) / u64::from(w)) as u32)
    };
    let cut = crop_imm(&board, (bw - cw) / 2, (bh - ch) / 2, cw, ch).to_image();
    let ground = resize(&cut, w, h, FilterType::Triangle);
    for (p, g) in poster.pixels_mut().zip(ground.pixels()) {
        for c in 0..3 {
            p.0[c] = p.0[c].max(g.0[c]);
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92).encode_image(&poster)?;
    Ok(out.into_inner())
}

/// The poster covers the site overlay knows (the keys of its `posters`).
fn live_poster_shas() -> &'static [String] {
    static SHAS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    SHAS.get_or_init(|| {
        let src = site_overlay().posters_src.as_str();
        let start = src.find("#let posters = (").unwrap_or(0);
        let block = src[start..].split("\n)").next().unwrap_or("");
        block
            .split('"')
            .skip(1)
            .step_by(2)
            .filter(|k| k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_string)
            .collect()
    })
}

/// Concrete `MenuPdfCache` so frontend admin server fns can refresh the static
/// cache after an edit without depending on this crate. Holds everything the
/// renderer needs except the `db` (passed per-call). Registered into Leptos
/// context in `main.rs`.
pub struct MenuPdfCacheImpl {
    pub site_root: String,
    pub uploads_dir: String,
    pub site_url: String,
    pub branding: rusterando_frontend::branding::BrandingHandle,
    /// Reserved `_shared` tenant pool — fallback for cover/theme when the
    /// rebuilt tenant has none. `None` if the shared DB couldn't be opened.
    pub shared: Option<SqlitePool>,
}

#[async_trait::async_trait]
impl rusterando_frontend::pages::push::MenuPdfCache for MenuPdfCacheImpl {
    async fn rebuild(&self, db: &SqlitePool) -> anyhow::Result<()> {
        // The boot/global cache keeps the canonical `menu.pdf` names (empty
        // slug) — it's the nginx offline fallback for the apex/main shop.
        // Per-tenant edit-triggered rebuilds go through `rebuild_for` with the
        // tenant's slug + branding + uploads dir.
        write_menu_pdf_cache(
            db,
            &self.site_root,
            &self.uploads_dir,
            self.site_url.clone(),
            &self.branding.get(),
            "",
            self.shared.as_ref(),
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Boot-time seed: ensure the PDF cover + theme libraries have at least
// one row on first run, so a fresh deployment doesn't show the admin
// an empty switcher. Called from main.rs right after migrations.
// ---------------------------------------------------------------------------

/// Idempotent. If `pdf_themes` is empty, insert one row with the
/// bundled `TEMPLATE_SRC` and set it active. If `pdf_cover_images`
/// is empty AND we can find `public/img/cover.jpg`, copy it into
/// `<uploads_dir>/covers/cover.jpg` and seed a row + activate it.
pub async fn seed_pdf_library_if_empty(db: &SqlitePool, uploads_dir: &str) -> anyhow::Result<()> {
    // Theme side.
    let theme_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pdf_themes")
        .fetch_one(db)
        .await?;
    if theme_count == 0 {
        let row: (i64,) =
            sqlx::query_as("INSERT INTO pdf_themes (name, source) VALUES (?1, ?2) RETURNING id")
                .bind("Werks-Vorlage")
                .bind(TEMPLATE_SRC)
                .fetch_one(db)
                .await?;
        sqlx::query("UPDATE app_settings SET value = ?1 WHERE key = 'pdf_theme_active_id'")
            .bind(row.0.to_string())
            .execute(db)
            .await?;
    }

    // Cover side.
    let cover_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pdf_cover_images")
        .fetch_one(db)
        .await?;
    if cover_count == 0 {
        // Try the bundled public/img/cover.jpg as the seed image.
        let src_path = std::path::Path::new("public/img/cover.jpg");
        if let Ok(bytes) = std::fs::read(src_path) {
            let dst_dir = std::path::Path::new(uploads_dir).join("covers");
            std::fs::create_dir_all(&dst_dir).ok();
            let dst_path = dst_dir.join("cover.jpg");
            if std::fs::write(&dst_path, &bytes).is_ok() {
                let row: (i64,) = sqlx::query_as(
                    "INSERT INTO pdf_cover_images (label, filename, mime_type, size_bytes)
                     VALUES (?1, ?2, ?3, ?4) RETURNING id",
                )
                .bind("Werks-Cover (cover.jpg)")
                .bind("cover.jpg")
                .bind("image/jpeg")
                .bind(bytes.len() as i64)
                .fetch_one(db)
                .await?;
                sqlx::query("UPDATE app_settings SET value = ?1 WHERE key = 'pdf_cover_active_id'")
                    .bind(row.0.to_string())
                    .execute(db)
                    .await?;
            }
        }
        // If public/img/cover.jpg isn't present, the library stays
        // empty. The renderer falls back to the compiled-in
        // ASSET_COVER (the bundled cover.jpg bytes) — no crash.
    }

    Ok(())
}

#[cfg(test)]
mod eink_tests {
    use super::*;

    fn sample(offer: bool) -> EinkOffer {
        EinkOffer {
            offer,
            name: "Broccoli Roma mit kleinem Salat".into(),
            detail: "Broccoli Roma · mit kl. Salat".into(),
            price: "9 €".into(),
            regular: "14 €".into(),
            motif: "tree".into(),
            shop: "Musterpizzeria".into(),
            url: "musterpizzeria.example".into(),
            phone: "01234-567890".into(),
        }
    }

    #[test]
    fn offer_renders_in_the_six_panel_colours() {
        for offer in [true, false] {
            let (png, eink) = render_eink_offer(&sample(offer)).expect("eink renders");
            assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
            assert_eq!(eink.len(), 192_000);
            let allowed: Vec<u8> = EINK_PALETTE.iter().map(|(_, c)| *c).collect();
            assert!(eink
                .iter()
                .all(|b| allowed.contains(&(b >> 4)) && allowed.contains(&(b & 0x0f))));
            // Not an empty board: some white, yellow or red is drawn.
            assert!(eink.iter().any(|b| b >> 4 != 0 || b & 0x0f != 0));
            // EINK_PREVIEW=<dir>: the pictures to look at.
            if let Ok(dir) = std::env::var("EINK_PREVIEW") {
                std::fs::write(format!("{dir}/eink-{offer}.png"), &png).unwrap();
            }
        }
    }
}

#[cfg(test)]
mod cover_tests {
    use super::*;

    fn sample_payload(cover_mode: &str) -> MenuPdfPayload {
        MenuPdfPayload {
            branding: Branding {
                name: "Test Pizzeria".into(),
                tagline: "frisch & lecker".into(),
                address: "Teststraße 1, 12345 Teststadt".into(),
                phone: "Tel. 0123-456789".into(),
                hours_lines: vec!["Mo–So 11:30–22:00".into()],
                delivery_lines: vec!["Stadt ab 15 €".into()],
                extras_pizza_lines: vec!["Krabben 1 €".into()],
                extras_pasta_lines: vec!["Krabben 1 €".into()],
                cover_mode: cover_mode.into(),
                delivery_zones: vec![PdfZone {
                    name: "Stadt".into(),
                    min_order_cents: 1500,
                    fee_cents: 150,
                }],
                free_delivery_cents: 0,
                cover_title: Vec::new(),
                cover_slogan: String::new(),
            },
            categories: vec![PdfCategory {
                id: "c1".into(),
                name: "Pizza".into(),
                items: vec![PdfItem {
                    menu_number: Some("1".into()),
                    name: "Margherita".into(),
                    description: Some("mit Tomaten und Mozzarella".into()),
                    allergen_codes: None,
                    additive_codes: None,
                    is_spicy: false,
                    price_small_cents: 500,
                    price_large_cents: Some(800),
                    size_small_label: Some("22cm".into()),
                    size_large_label: Some("30cm".into()),
                    included_extras_count: 0,
                    flat_extra_price_cents: None,
                }],
            }],
            allergens: vec![],
            additives: vec![],
            site_url: "https://example.com".into(),
        }
    }

    // The text cover must render without a Typst error (no image dependency).
    #[test]
    fn text_cover_renders() {
        let payload = sample_payload("text");
        let overrides = PdfOverrides {
            template_source: None,
            cover_image: None,
            ad_cover_image: None,
            ad_center_image: None,
            ad_back_image: None,
        };
        let bytes = render_menu_pdf(&payload, &overrides, "trifold", true)
            .expect("text cover should render");
        assert!(
            bytes.len() > 1000,
            "PDF suspiciously small: {}",
            bytes.len()
        );
    }

    // Emoji come from the bundled font as PNGs, so no Type 3 font goes in.
    #[test]
    fn emoji_are_pictures_from_the_bundled_font() {
        let pngs = emoji_pngs();
        assert!(pngs.len() > 40, "only {} emoji", pngs.len());
        let chili = emoji_file("/emoji/1f336.png").expect("🌶");
        assert_eq!(&chili[..4], b"\x89PNG");
        assert!(emoji_file("/emoji/fe0f.png").is_none());
        assert!(emoji_file("/emoji/zz.png").is_none());
    }

    // The Zickzack, with and without crop + fold marks.
    #[test]
    fn zickzack_renders_with_and_without_marks() {
        let payload = sample_payload("image");
        let overrides = PdfOverrides {
            template_source: None,
            cover_image: None,
            ad_cover_image: None,
            ad_center_image: None,
            ad_back_image: None,
        };
        for format in ["a5-zickzack", "a5-zickzack-marks"] {
            let bytes = render_menu_pdf(&payload, &overrides, format, true).expect(format);
            assert!(bytes.len() > 1000, "{format}: {} bytes", bytes.len());
        }
    }

    // The built-in blackboard theme, in every page format.
    #[test]
    fn tafel_theme_renders() {
        let payload = sample_payload("image");
        let overrides = PdfOverrides {
            template_source: Some(TAFEL_SRC.to_string()),
            cover_image: None,
            ad_cover_image: None,
            ad_center_image: None,
            ad_back_image: None,
        };
        for format in ["trifold", "a5-zickzack", "a5-zickzack-marks"] {
            let bytes = render_menu_pdf(&payload, &overrides, format, true).expect(format);
            assert!(
                bytes.len() > 100_000,
                "{format}: {} bytes (board missing?)",
                bytes.len()
            );
        }
    }

    #[test]
    fn trattoria_theme_renders() {
        let payload = sample_payload("image");
        let overrides = PdfOverrides {
            template_source: Some(TRATTORIA_SRC.to_string()),
            cover_image: None,
            ad_cover_image: None,
            ad_center_image: None,
            ad_back_image: None,
        };
        for format in ["trifold", "a5-zickzack", "a5-zickzack-marks"] {
            let bytes = render_menu_pdf(&payload, &overrides, format, true).expect(format);
            assert!(
                bytes.len() > 100_000,
                "{format}: {} bytes (paper missing?)",
                bytes.len()
            );
        }
    }

    /// All text on a laid-out frame, items separated by spaces.
    fn frame_text(frame: &typst::layout::Frame, out: &mut String) {
        for (_, item) in frame.items() {
            match item {
                typst::layout::FrameItem::Group(g) => frame_text(&g.frame, out),
                typst::layout::FrameItem::Text(t) => {
                    out.push_str(t.text.as_str());
                    out.push(' ');
                }
                _ => {}
            }
        }
    }

    /// The site's poster test picture, `$SITE_OVERLAY_DIR/tests/poster.jpg`
    /// (one of the overlay's `posters`). Poster layouts are a shop's own, so
    /// the poster tests run only with an overlay:
    /// `SITE_OVERLAY_DIR=$PWD/site/<profile> cargo test -p rusterando-server`.
    fn overlay_poster() -> Option<Vec<u8>> {
        let dir = std::env::var("SITE_OVERLAY_DIR").ok()?;
        let poster = std::fs::read(std::path::Path::new(&dir).join("tests/poster.jpg")).ok();
        if poster.is_none() {
            eprintln!("poster test skipped: no {dir}/tests/poster.jpg");
        }
        poster
    }

    // The Tafel theme lays its board under the poster: the dark stone gives
    // way to it (lighter, greener), the photo stays. Other themes and
    // non-poster covers are left alone. COVER_PREVIEW=<dir> writes the
    // homepage picture with and without the board.
    #[test]
    fn tafel_theme_lays_the_board_under_the_poster() {
        let Some(poster) = overlay_poster() else { return };
        assert!(tafel_cover(TRATTORIA_SRC, Some(&poster)).is_none());
        assert!(tafel_cover(TAFEL_SRC, Some(ASSET_LADENFRONT)).is_none());
        let on_board = tafel_cover(TAFEL_SRC, Some(&poster)).expect("board cover");
        let before = image::load_from_memory(&poster).unwrap().to_rgb8();
        let after = image::load_from_memory(&on_board).unwrap().to_rgb8();
        assert_eq!(before.dimensions(), after.dimensions());
        // The stone near the top left corner, inside the poster's frame line.
        let lum = |img: &image::RgbImage| {
            let mut sum = 0u64;
            for y in 30..90 {
                for x in 30..90 {
                    let p = img.get_pixel(x, y).0;
                    sum += u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]);
                }
            }
            sum / (60 * 60 * 3)
        };
        assert!(lum(&after) > lum(&before) + 8, "board: {} vs stone {}", lum(&after), lum(&before));
        if let Ok(dir) = std::env::var("COVER_PREVIEW") {
            let payload = sample_payload("image");
            for (name, theme) in [("cover-tafel.jpg", TAFEL_SRC), ("cover-trattoria.jpg", TRATTORIA_SRC)] {
                let overrides = PdfOverrides {
                    template_source: Some(theme.to_string()),
                    cover_image: Some(poster.clone()),
                    ad_cover_image: None,
                    ad_center_image: None,
                    ad_back_image: None,
                };
                let jpeg = render_cover_jpeg(&payload, &overrides, 1000).expect("cover");
                std::fs::write(std::path::Path::new(&dir).join(name), jpeg).unwrap();
            }
        }
    }

    // The site's poster gets today's prices and zones from the data, found by
    // the picture's hash; any other cover gets none of it. What it must show
    // for this payload is the overlay's `tests/poster-expect.txt` (one text
    // per line). The homepage picture renders from the same page.
    #[test]
    fn poster_cover_gets_live_prices() {
        let Some(poster) = overlay_poster() else { return };
        let dir = std::env::var("SITE_OVERLAY_DIR").unwrap();
        let expect = std::fs::read_to_string(std::path::Path::new(&dir).join("tests/poster-expect.txt"))
            .expect("tests/poster-expect.txt next to the test poster");
        assert!(live_poster_shas().contains(&sha256_hex(&poster)));
        let mut payload = sample_payload("image");
        let item = |name: &str, cents: i64, incl: i64, flat: i64| PdfItem {
            menu_number: None,
            name: name.into(),
            description: None,
            allergen_codes: None,
            additive_codes: None,
            is_spicy: false,
            price_small_cents: cents,
            price_large_cents: None,
            size_small_label: None,
            size_large_label: None,
            included_extras_count: incl,
            flat_extra_price_cents: Some(flat),
        };
        payload.categories[0]
            .items
            .push(item("Pizzablech 60x40cm", 2940, 3, 300));
        payload.categories[0]
            .items
            .push(item("Pizza 36 cm mit Goudakäse", 1470, 0, 100));
        payload.branding.delivery_zones.push(PdfZone {
            name: "Nebendorf".into(),
            min_order_cents: 3000,
            fee_cents: 250,
        });
        payload.branding.cover_title = vec!["Bella".into(), "Vita".into()];
        payload.branding.cover_slogan = "Frisch aus dem Ofen".into();
        payload.branding.hours_lines = vec!["Mo–So 11:30–22:00".into(), "Mi Ruhetag".into()];
        payload.branding.address = "Hauptstr. 1, 12345 Musterstadt".into();
        payload.branding.phone = "Tel. 0123-456".into();
        let overrides = |cover: Option<Vec<u8>>| PdfOverrides {
            template_source: None,
            cover_image: cover,
            ad_cover_image: None,
            ad_center_image: None,
            ad_back_image: None,
        };
        let text = |cover: Option<Vec<u8>>| {
            let doc = compile_menu(&payload, &overrides(cover), "cover", true).expect("cover");
            assert_eq!(doc.pages.len(), 1);
            let mut s = String::new();
            frame_text(&doc.pages[0].frame, &mut s);
            s
        };
        let live = text(Some(poster.clone()));
        for want in expect.lines().map(str::trim).filter(|l| !l.is_empty()) {
            assert!(
                live.contains(want),
                "{want:?} missing from the cover: {live}"
            );
        }
        assert_ne!(live, text(None), "the poster got no live fields");
        let jpeg = render_cover_jpeg(&payload, &overrides(Some(poster)), 1000).expect("jpeg");
        assert!(jpeg.starts_with(&[0xFF, 0xD8]) && jpeg.len() > 50_000);
    }

    // Every drawing the Tafel theme can pick (its `motif-map` and the
    // fallback) must be bundled: a missing one fails the whole PDF for any
    // shop with a category that matches it.
    #[test]
    fn tafel_motifs_are_bundled() {
        for src in [TAFEL_SRC, TRATTORIA_SRC] {
            theme_motifs_are_bundled(src);
        }
        assert!(motif_file("/img/motif/../tafel.jpg").is_none());
    }

    fn theme_motifs_are_bundled(src: &str) {
        let start = src.find("#let motif-map = (").expect("motif-map");
        let end = src.find("#let theme-card-motif").expect("theme-card-motif");
        // `  ("pattern", "name"),` and `#let motif-fallback = "name"`
        let names: Vec<&str> = src[start..end]
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('"').collect();
                match parts.len() {
                    5 => Some(parts[3]),
                    3 => Some(parts[1]),
                    _ => None,
                }
            })
            .collect();
        assert!(names.len() > 20, "parsed only {names:?}");
        for name in names {
            assert!(
                motif_file(&format!("{MOTIF_VPATH_PREFIX}{name}.svg")).is_some(),
                "a theme uses motif {name:?}, not in MOTIFS"
            );
        }
    }

    // The image cover must still render (falls back to bundled ASSET_COVER).
    #[test]
    fn image_cover_renders() {
        let payload = sample_payload("image");
        let overrides = PdfOverrides {
            template_source: None,
            cover_image: None,
            ad_cover_image: None,
            ad_center_image: None,
            ad_back_image: None,
        };
        let bytes = render_menu_pdf(&payload, &overrides, "trifold", true)
            .expect("image cover should render");
        assert!(
            bytes.len() > 1000,
            "PDF suspiciously small: {}",
            bytes.len()
        );
    }
}

// ===========================================================================
// Order-Beleg (single-order receipt / Rechnung) — a formal A4 PDF a tax clerk
// can file. Self-contained: it needs only the bundled fonts, no images. The
// server computes every money value so the PDF matches the confirmation page,
// admin detail, and CSV exactly — the template only lays it out.
// ===========================================================================

/// Beleg Typst source, embedded at compile time (see templates/beleg.typ).
const BELEG_SRC: &str = include_str!("../templates/beleg.typ");
const BELEG_VPATH: &str = "/beleg.typ";

/// Kurzliste / Einzelauflistung Typst source (see templates/kurzliste.typ).
const KURZLISTE_SRC: &str = include_str!("../templates/kurzliste.typ");
const KURZLISTE_VPATH: &str = "/kurzliste.typ";

#[derive(Debug, Serialize)]
pub struct OrderPdfPayload {
    pub shop: OrderPdfShop,
    /// Present only for a batch (range) export — drives the cover page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<OrderPdfSummary>,
    /// One entry for a single-order Beleg; many for a batch.
    pub orders: Vec<OrderPdfOrder>,
}

/// Cover-page figures for a batch export.
#[derive(Debug, Serialize)]
pub struct OrderPdfSummary {
    pub from: String,
    pub to: String,
    /// Non-cancelled orders in the export (each gets a Beleg page).
    pub count: i64,
    /// Sum of non-cancelled totals (cents) — matches the Buchhaltung "Umsatz".
    pub total_cents: i64,
    /// How many cancelled orders are in the range (shown for reconciliation).
    pub cancelled_count: i64,
    /// Non-cancelled orders with a voucher, and the discount they got: what
    /// the shop gave away (Werbungskosten); each Beleg shows its own line.
    pub voucher_count: i64,
    pub voucher_cents: i64,
    pub include_test: bool,
    pub include_cancelled: bool,
    /// Non-empty → a red "large export" note on the cover.
    pub truncated_note: String,
}

#[derive(Debug, Serialize)]
pub struct OrderPdfShop {
    pub name: String,
    pub address: String,
    pub phone: String,
    pub email: String,
    /// USt-IdNr., shown in the footer + payment area when set.
    pub vat_id: String,
}

#[derive(Debug, Serialize)]
pub struct OrderPdfOrder {
    pub number: String,
    /// Human date/time the order was placed ("02.07.2026 18:12").
    pub date: String,
    /// "Abholung" / "Lieferung" / "Tisch" — the fulfillment noun.
    pub channel_label: String,
    /// Scheduled/ETA time label ("18:30"), or "" when ASAP/none.
    pub fulfillment: String,
    pub customer_name: String,
    pub customer_phone: String,
    pub customer_email: String,
    /// One-line delivery address, "" for pickup.
    pub address: String,
    pub items: Vec<OrderPdfItem>,
    pub subtotal_cents: i64,
    pub delivery_fee_cents: i64,
    pub voucher_code: String,
    pub voucher_discount_cents: i64,
    pub total_cents: i64,
    /// "inkl. 7% MwSt. 1,08 €" or the Kleinunternehmer note. "" hides the line.
    pub tax_note: String,
    /// "Bar", "Karte (Visa)", "Gutschein", … — human payment label.
    pub payment_label: String,
    /// Non-empty (e.g. a Storno banner) draws a red note under the payment line.
    pub status_note: String,
}

#[derive(Debug, Serialize)]
pub struct OrderPdfItem {
    pub menu_number: String,
    pub name: String,
    pub variant: String,
    pub quantity: i64,
    pub unit_price_cents: i64,
    pub line_total_cents: i64,
    /// Human-readable extra labels ("Extra Käse").
    pub extras: Vec<String>,
    /// Removed ingredients ("Zwiebeln").
    pub removals: Vec<String>,
    /// Required-choice picks as "Dressing: Cocktail" / "Sauce: Kräuterbutter".
    #[serde(default)]
    pub selected_options: Vec<String>,
}

/// A minimal Typst `World` for the order Beleg: one source file + the
/// bundled fonts. No image/QR resolution, so `file()` always 404s.
struct OrderWorld {
    library: LazyHash<Library>,
    main_id: FileId,
    main_source: Source,
}

impl OrderWorld {
    fn new(payload: &OrderPdfPayload) -> anyhow::Result<Self> {
        let json = serde_json::to_string(payload)?;
        let mut inputs = Dict::new();
        inputs.insert("data".into(), typst::foundations::Value::Str(json.into()));
        let library = Library::builder().with_inputs(inputs).build();
        let main_id = FileId::new(None, VirtualPath::new(BELEG_VPATH));
        let main_source = Source::new(main_id, BELEG_SRC.to_string());
        Ok(Self {
            library: LazyHash::new(library),
            main_id,
            main_source,
        })
    }
}

impl World for OrderWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &resources().book
    }
    fn main(&self) -> FileId {
        self.main_id
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main_id {
            Ok(self.main_source.clone())
        } else {
            Err(FileError::NotFound(id.vpath().as_rootless_path().into()))
        }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        Err(FileError::NotFound(id.vpath().as_rootless_path().into()))
    }
    fn font(&self, index: usize) -> Option<Font> {
        resources().fonts.get(index).cloned()
    }
    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        None
    }
}

// ---- Tagesangebot for the colour e-ink frame -----------------------------

pub(crate) const EINK_OFFER_SRC: &str = include_str!("../templates/eink_offer.typ");
/// Bump when `quantize_eink` changes the look (the template counts by
/// itself): part of the picture's tag (eink.rs), so frames refresh.
pub(crate) const EINK_QUANTIZER: u32 = 3;
const EINK_OFFER_VPATH: &str = "/eink_offer.typ";
/// The frame's panel: Waveshare 7.3" e-Paper (E), 800 × 480.
pub const EINK_WIDTH: u32 = 800;
pub const EINK_HEIGHT: u32 = 480;

/// The panel's six colours and their codes in its frame format (protocol
/// v1 of the frame's firmware: 4 bits a pixel, `4` unused).
pub const EINK_PALETTE: [([u8; 3], u8); 6] = [
    ([0, 0, 0], 0),
    ([255, 255, 255], 1),
    ([255, 255, 0], 2),
    ([255, 0, 0], 3),
    ([0, 0, 255], 5),
    ([0, 255, 0], 6),
];

/// What the e-ink Tagesangebot shows (templates/eink_offer.typ).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EinkOffer {
    /// false: no Tagesangebot today — the shop's card.
    pub offer: bool,
    pub name: String,
    pub detail: String,
    pub price: String,
    /// The regular price, struck through ("" = none).
    pub regular: String,
    /// The item's drawing (/img/motif/<motif>.svg).
    pub motif: String,
    pub shop: String,
    pub url: String,
    pub phone: String,
}

/// The e-ink picture's world: its template, the fonts, the drawings.
struct EinkWorld {
    library: LazyHash<Library>,
    main_id: FileId,
    main_source: Source,
}

impl World for EinkWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &resources().book
    }
    fn main(&self) -> FileId {
        self.main_id
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main_id {
            Ok(self.main_source.clone())
        } else {
            Err(FileError::NotFound(id.vpath().as_rootless_path().into()))
        }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        let path = id.vpath().as_rooted_path().to_string_lossy().to_string();
        motif_file(&path).ok_or_else(|| FileError::NotFound(id.vpath().as_rootless_path().into()))
    }
    fn font(&self, index: usize) -> Option<Font> {
        resources().fonts.get(index).cloned()
    }
    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        None
    }
}

/// The Tagesangebot as the frame shows it: a PNG (800 × 480, only the six
/// panel colours — for a preview, or the frame's iPhone app) and the
/// frame's own format (192 000 bytes: rows top to bottom, two pixels a
/// byte, the left one in the high nibble). Every pixel goes to the nearest
/// panel colour; the template draws in those colours only, so there is
/// nothing to dither.
pub fn render_eink_offer(o: &EinkOffer) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let mut inputs = Dict::new();
    inputs.insert(
        "data".into(),
        typst::foundations::Value::Str(serde_json::to_string(o)?.into()),
    );
    let main_id = FileId::new(None, VirtualPath::new(EINK_OFFER_VPATH));
    let world = EinkWorld {
        library: LazyHash::new(Library::builder().with_inputs(inputs).build()),
        main_id,
        main_source: Source::new(main_id, EINK_OFFER_SRC.to_string()),
    };
    let warned = typst::compile::<PagedDocument>(&world);
    comemo::evict(4);
    for w in &warned.warnings {
        tracing::warn!("typst warn (eink): {}", w.message);
    }
    let document = warned.output.map_err(|errs| {
        anyhow::anyhow!(
            "eink: {}",
            errs.iter()
                .map(|e| e.message.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    let page = document
        .pages
        .first()
        .ok_or_else(|| anyhow::anyhow!("eink: no page"))?;
    let pixmap = typst_render::render(page, 1.0);
    anyhow::ensure!(
        pixmap.width() == EINK_WIDTH && pixmap.height() == EINK_HEIGHT,
        "eink: {}×{} instead of 800×480",
        pixmap.width(),
        pixmap.height()
    );
    let (rgb, eink) = quantize_eink(pixmap.data());
    let img = image::RgbImage::from_raw(EINK_WIDTH, EINK_HEIGHT, rgb)
        .ok_or_else(|| anyhow::anyhow!("eink: bad pixmap"))?;
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png)?;
    Ok((png.into_inner(), eink))
}

/// The template's marker colours for textures (templates/eink_offer.typ).
const EINK_BOARD: [u8; 3] = [0, 40, 0];
const EINK_WOOD: [u8; 3] = [120, 60, 0];
/// 4×4 ordered dither thresholds (0..16).
const BAYER4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// A fixed pseudo-random value 0..1000 per pixel: the same picture every
/// time, so its ETag only changes with its content.
fn eink_noise(x: u32, y: u32, salt: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ salt.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h % 1000
}

/// A smooth value 0..1 over the picture (value noise, `cell` pixels a
/// cell): where the board was wiped less, it is a little lighter.
fn eink_smudge(x: u32, y: u32, cell: u32) -> f32 {
    let (cx, cy) = (x / cell, y / cell);
    let (fx, fy) = (
        (x % cell) as f32 / cell as f32,
        (y % cell) as f32 / cell as f32,
    );
    let v = |i: u32, j: u32| eink_noise(cx + i, cy + j, 21) as f32 / 1000.0;
    // Smoothstep between the corners, so no seams show.
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = v(0, 0) + (v(1, 0) - v(0, 0)) * sx;
    let bottom = v(0, 1) + (v(1, 1) - v(0, 1)) * sx;
    top + (bottom - top) * sy
}

/// The slate: black with a fine green pattern — dark green from a step
/// away — a little more green where it was smeared (soft patches), no
/// specks.
fn eink_board(x: u32, y: u32) -> u8 {
    let smudge = eink_smudge(x, y, 80) * 0.8 + eink_smudge(x, y, 29) * 0.2;
    // 2..4 green pixels in 16: soft, large patches.
    let green = 2 + (smudge * smudge * 3.0) as u8;
    if BAYER4[(y % 4) as usize][(x % 4) as usize] < green {
        6
    } else {
        0
    }
}

/// The wooden frame: red, yellow and black in a pattern that reads brown,
/// with darker grain lines along it.
fn eink_wood(x: u32, y: u32) -> u8 {
    let along = if x < 20 || x >= EINK_WIDTH - 20 { x } else { y };
    let grain = eink_noise(along / 3, 0, 11) < 180;
    // Brown ≈ a quarter red, a quarter yellow, half black.
    let t = BAYER4[(y % 4) as usize][(x % 4) as usize];
    match (grain, t) {
        (false, 0..=3) => 3,
        (false, 4..=7) => 2,
        (true, 0..=2) => 3,
        (true, 3..=4) => 2,
        _ => 0,
    }
}

/// Opaque RGBA → (the panel colours as RGB, the panel's 4-bit codes).
///
/// The template paints the board and the wood in marker colours and writes
/// in the panel colours, its edges smoothed into in-between shades. A pixel
/// near a marker becomes that texture. Any other is read as a mix of the
/// board with one chalk colour — the one it lies closest to the line
/// towards — and becomes that chalk when it is more than half of it, else
/// board: letters stay purely white or yellow, no stray colours at their
/// edges. Some chalk pixels are left out for the grain.
fn quantize_eink(rgba: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let colour_of = |code: u8| {
        EINK_PALETTE
            .iter()
            .find(|(_, c)| *c == code)
            .map(|(rgb, _)| *rgb)
            .unwrap_or([0, 0, 0])
    };
    let f = |c: [u8; 3]| [f32::from(c[0]), f32::from(c[1]), f32::from(c[2])];
    let board = f(EINK_BOARD);
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    let mut codes = Vec::with_capacity(rgba.len() / 4);
    for (i, p) in rgba.chunks_exact(4).enumerate() {
        let (x, y) = (i as u32 % EINK_WIDTH, i as u32 / EINK_WIDTH);
        let px = [f32::from(p[0]), f32::from(p[1]), f32::from(p[2])];
        let d2 = |c: [f32; 3]| (0..3).map(|k| (px[k] - c[k]).powi(2)).sum::<f32>();
        let code = if d2(f(EINK_WOOD)) < 900.0 {
            eink_wood(x, y)
        } else if d2(board) < 900.0 {
            eink_board(x, y)
        } else {
            // The chalk this pixel is a mix of (with the board), and how much.
            let (code, amount) = EINK_PALETTE
                .iter()
                .filter(|(_, code)| *code != 0)
                .map(|(c, code)| {
                    let c = f(*c);
                    let dir = [c[0] - board[0], c[1] - board[1], c[2] - board[2]];
                    let len2 = dir.iter().map(|v| v * v).sum::<f32>();
                    let a = ((0..3).map(|k| (px[k] - board[k]) * dir[k]).sum::<f32>() / len2)
                        .clamp(0.0, 1.0);
                    let on = [
                        board[0] + a * dir[0],
                        board[1] + a * dir[1],
                        board[2] + a * dir[2],
                    ];
                    (*code, a, d2(on))
                })
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .map(|(code, a, _)| (code, a))
                .unwrap_or((0, 0.0));
            if amount < 0.5 {
                eink_board(x, y)
            } else if eink_noise(x, y, 3) < 40 {
                // A light grain: a few pixels of the chalk show the board's
                // green through, not black holes.
                6
            } else {
                code
            }
        };
        rgb.extend_from_slice(&colour_of(code));
        codes.push(code);
    }
    let eink = codes
        .chunks_exact(2)
        .map(|two| (two[0] << 4) | two[1])
        .collect();
    (rgb, eink)
}

/// Render a single-order Beleg to PDF bytes. Human-readable error on failure.
pub fn render_order_pdf(payload: &OrderPdfPayload) -> anyhow::Result<Vec<u8>> {
    let world = OrderWorld::new(payload)?;
    let warned = typst::compile::<PagedDocument>(&world);
    // Drop memoised layout results older than a few compilations so the
    // cache stays bounded over the process lifetime.
    comemo::evict(4);
    for w in &warned.warnings {
        tracing::warn!("typst warn (beleg): {}", w.message);
    }
    let document = warned.output.map_err(|errs| {
        for e in &errs {
            tracing::error!("typst (beleg): {} (span={:?})", e.message, e.span);
        }
        let msgs: Vec<_> = errs.iter().map(|e| eco_format!("{}", e.message)).collect();
        anyhow::anyhow!("Typst compile failed: {}", msgs.join("; "))
    })?;
    let opts = typst_pdf::PdfOptions::default();
    typst_pdf::pdf(&document, &opts).map_err(|errs| {
        let msgs: Vec<_> = errs.iter().map(|e| eco_format!("{}", e.message)).collect();
        anyhow::anyhow!("Typst PDF export failed: {}", msgs.join("; "))
    })
}

/// Build the Beleg payload for one order from the live DB, then render it on a
/// blocking thread. `branding` supplies the shop header + VAT config.
pub async fn build_order_pdf(
    db: &SqlitePool,
    order_id: &str,
    branding: &rusterando_frontend::branding::Branding,
) -> anyhow::Result<Vec<u8>> {
    let payload = load_order_pdf_payload(db, order_id, branding).await?;
    tokio::task::spawn_blocking(move || render_order_pdf(&payload)).await?
}

// ===================================================================
// Kurzliste / Einzelauflistung — compact one-line-per-order statement
// ===================================================================

/// Full payload for the Kurzliste (see templates/kurzliste.typ).
#[derive(Debug, Serialize)]
pub struct KurzlistePayload {
    pub shop: OrderPdfShop,
    pub period: KurzlistePeriod,
    pub rows: Vec<KurzlisteRow>,
    /// The redeemed Gutscheine, oldest first (the Werbungskosten list).
    pub vouchers: Vec<KurzlisteVoucher>,
}

/// Summary header figures. Split by how the order was settled:
/// cash-at-handover, online (card/Apple Pay/…), or fully by voucher.
#[derive(Debug, Serialize)]
pub struct KurzlistePeriod {
    pub from: String,
    pub to: String,
    /// Completed orders (cash + online + voucher), count and summed cents.
    pub count_total: i64,
    pub sum_total_cents: i64,
    /// "Bei Auslieferung bezahlt" — payment_status = cash_on_pickup.
    pub count_cash: i64,
    pub sum_cash_cents: i64,
    /// "Online bezahlt" — real online settlement (payment_status = paid).
    /// Vouchers are NOT counted here — a Gutschein is a redemption, not money in.
    pub count_online: i64,
    pub sum_online_cents: i64,
    /// "Per Gutschein bezahlt" — payment_status = voucher_paid. These are fully
    /// covered by a Gutschein (total 0 €); no money changed hands.
    pub count_voucher: i64,
    pub sum_voucher_cents: i64,
    /// Every Gutschein redeemed in the period (orders with a voucher
    /// discount, fully or partly paid by it): count and discounts — the
    /// shop's advertising cost (Werbungskosten), listed in `vouchers`.
    pub count_redeemed: i64,
    pub sum_redeemed_cents: i64,
    pub sum_redeemed_items_cents: i64,
    pub sum_redeemed_delivery_cents: i64,
    /// Storno in the range (shown only when include_cancelled). Never counted.
    pub count_cancelled: i64,
    pub include_test: bool,
    pub include_cancelled: bool,
    /// Non-empty → a red note (e.g. large-export warning).
    pub note: String,
}

/// One redeemed Gutschein: the order, the code, the discount (on the items,
/// on the delivery fee, together).
#[derive(Debug, Serialize)]
pub struct KurzlisteVoucher {
    pub date: String,
    pub number: String,
    pub code: String,
    /// The voucher's admin label, empty when none.
    pub label: String,
    pub items_cents: i64,
    pub delivery_cents: i64,
    pub total_cents: i64,
}

/// One order row: date/time, the order number, its total, and a marker for how
/// it was paid — "" (cash), "*" (online), or "†" (voucher).
#[derive(Debug, Serialize)]
pub struct KurzlisteRow {
    pub date: String,
    pub number: String,
    pub total_cents: i64,
    /// Gutschein discount on this order (0 = none); the total is after it.
    pub voucher_cents: i64,
    /// "" = cash, "*" = online (paid), "†" = Gutschein (voucher_paid).
    pub marker: String,
}

/// A minimal Typst `World` for the Kurzliste: one source file + bundled fonts.
struct KurzlisteWorld {
    library: LazyHash<Library>,
    main_id: FileId,
    main_source: Source,
}

impl KurzlisteWorld {
    fn new(payload: &KurzlistePayload) -> anyhow::Result<Self> {
        let json = serde_json::to_string(payload)?;
        let mut inputs = Dict::new();
        inputs.insert("data".into(), typst::foundations::Value::Str(json.into()));
        let library = Library::builder().with_inputs(inputs).build();
        let main_id = FileId::new(None, VirtualPath::new(KURZLISTE_VPATH));
        let main_source = Source::new(main_id, KURZLISTE_SRC.to_string());
        Ok(Self {
            library: LazyHash::new(library),
            main_id,
            main_source,
        })
    }
}

impl World for KurzlisteWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &resources().book
    }
    fn main(&self) -> FileId {
        self.main_id
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main_id {
            Ok(self.main_source.clone())
        } else {
            Err(FileError::NotFound(id.vpath().as_rootless_path().into()))
        }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        Err(FileError::NotFound(id.vpath().as_rootless_path().into()))
    }
    fn font(&self, index: usize) -> Option<Font> {
        resources().fonts.get(index).cloned()
    }
    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        None
    }
}

/// Render a Kurzliste payload to PDF bytes.
pub fn render_kurzliste_pdf(payload: &KurzlistePayload) -> anyhow::Result<Vec<u8>> {
    let world = KurzlisteWorld::new(payload)?;
    let warned = typst::compile::<PagedDocument>(&world);
    // Drop memoised layout results older than a few compilations so the
    // cache stays bounded over the process lifetime.
    comemo::evict(4);
    for w in &warned.warnings {
        tracing::warn!("typst warn (kurzliste): {}", w.message);
    }
    let document = warned.output.map_err(|errs| {
        for e in &errs {
            tracing::error!("typst (kurzliste): {} (span={:?})", e.message, e.span);
        }
        let msgs: Vec<_> = errs.iter().map(|e| eco_format!("{}", e.message)).collect();
        anyhow::anyhow!("Typst compile failed: {}", msgs.join("; "))
    })?;
    let opts = typst_pdf::PdfOptions::default();
    typst_pdf::pdf(&document, &opts).map_err(|errs| {
        let msgs: Vec<_> = errs.iter().map(|e| eco_format!("{}", e.message)).collect();
        anyhow::anyhow!("Typst PDF export failed: {}", msgs.join("; "))
    })
}

/// UTC `created_at` string ("YYYY-MM-DD HH:MM:SS") → "DD.MM.YYYY HH:MM" in the
/// shop's local timezone. `chrono::Local` honours the TZ env of the systemd
/// unit (Europe/Berlin), matching the kitchen ticket + admin UI. Falls back to
/// the raw string if it can't be parsed.
fn kurzliste_local_datetime(created_at_utc: &str) -> String {
    use chrono::{Local, NaiveDateTime, TimeZone, Utc};
    NaiveDateTime::parse_from_str(created_at_utc.trim(), "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|naive| {
            let utc = Utc.from_utc_datetime(&naive);
            utc.with_timezone(&Local)
                .format("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| created_at_utc.to_string())
}

/// Build the compact Kurzliste for a filtered range. Same date-range + Sandbox
/// + Storno semantics as the Buchhaltung/Beleg export, but:
///   * excludes non-completed orders (pending/failed) — they aren't revenue,
///   * carries no line items (one summary query + one row query),
///   * splits payment into cash-at-handover vs. online-paid.
pub async fn build_kurzliste_pdf(
    db: &SqlitePool,
    from: &str,
    to: &str,
    include_test: bool,
    include_cancelled: bool,
    branding: &rusterando_frontend::branding::Branding,
) -> anyhow::Result<Vec<u8>> {
    use sqlx::Row;

    let from_ts = format!("{from} 00:00:00");
    let to_ts = format!("{to} 23:59:59");
    let mode_clause = if include_test {
        ""
    } else {
        " AND stripe_mode = 'live'"
    };

    // Only COMPLETED orders count toward the list: paid / voucher_paid (online)
    // and cash_on_pickup (cash at handover). pending/failed are dropped.
    // Cancelled is always excluded from the rows + money; it's surfaced only as
    // a separate reconciliation count when include_cancelled is on.
    const COMPLETED: &str =
        " AND payment_status IN ('paid','voucher_paid','cash_on_pickup') AND status != 'cancelled'";

    // Rows — one per completed order, oldest first (like the Lieferando list).
    let row_sql = format!(
        "SELECT o.order_number, o.created_at, o.total_cents, o.payment_status,
                o.voucher_code, o.voucher_discount_cents,
                o.voucher_discount_items_cents, o.voucher_discount_delivery_cents,
                COALESCE(v.label, '') AS voucher_label
         FROM orders o LEFT JOIN vouchers v ON v.id = o.voucher_id
         WHERE o.created_at BETWEEN ?1 AND ?2{mode_clause}{COMPLETED}
         ORDER BY o.created_at ASC",
    );
    let db_rows = sqlx::query(&row_sql)
        .bind(&from_ts)
        .bind(&to_ts)
        .fetch_all(db)
        .await?;

    let mut rows = Vec::with_capacity(db_rows.len());
    let mut count_cash = 0i64;
    let mut sum_cash_cents = 0i64;
    let mut count_online = 0i64;
    let mut sum_online_cents = 0i64;
    let mut count_voucher = 0i64;
    let mut sum_voucher_cents = 0i64;
    let mut vouchers = Vec::new();
    for r in &db_rows {
        let number: String = r.get("order_number");
        let created_at: String = r.get("created_at");
        let total_cents: i64 = r.get("total_cents");
        let payment_status: String = r.get("payment_status");
        // created_at is UTC 'YYYY-MM-DD HH:MM:SS'; convert to the shop's local
        // timezone (Local honours the TZ env of the systemd unit, same as the
        // kitchen ticket) and present it as "DD.MM.YYYY HH:MM".
        let date = kurzliste_local_datetime(&created_at);
        // Three settlement buckets. A voucher-paid order is fully covered by a
        // Gutschein (total 0 €) — it is NOT online revenue, so it gets its own
        // line + "†" marker rather than being folded into "Online bezahlt".
        let marker = match payment_status.as_str() {
            "cash_on_pickup" => {
                count_cash += 1;
                sum_cash_cents += total_cents;
                ""
            }
            "voucher_paid" => {
                count_voucher += 1;
                sum_voucher_cents += total_cents;
                "†"
            }
            // "paid" (card / Apple Pay / …) and any other completed online state.
            _ => {
                count_online += 1;
                sum_online_cents += total_cents;
                "*"
            }
        };
        let voucher_cents: i64 = r.try_get("voucher_discount_cents").unwrap_or(0);
        if voucher_cents > 0 {
            vouchers.push(KurzlisteVoucher {
                date: date.clone(),
                number: number.clone(),
                code: r
                    .try_get::<Option<String>, _>("voucher_code")
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                label: r.try_get("voucher_label").unwrap_or_default(),
                items_cents: r.try_get("voucher_discount_items_cents").unwrap_or(0),
                delivery_cents: r.try_get("voucher_discount_delivery_cents").unwrap_or(0),
                total_cents: voucher_cents,
            });
        }
        rows.push(KurzlisteRow {
            date,
            number,
            total_cents,
            voucher_cents,
            marker: marker.to_string(),
        });
    }
    let count_total = count_cash + count_online + count_voucher;
    let sum_total_cents = sum_cash_cents + sum_online_cents + sum_voucher_cents;

    // Storno count in the same range (for reconciliation; never in the money).
    let canc_sql = format!(
        "SELECT COUNT(*) AS c FROM orders
         WHERE created_at BETWEEN ?1 AND ?2{mode_clause} AND status = 'cancelled'",
    );
    let count_cancelled: i64 = sqlx::query(&canc_sql)
        .bind(&from_ts)
        .bind(&to_ts)
        .fetch_one(db)
        .await?
        .get("c");

    const LARGE_LIST: usize = 1500;
    let note = if rows.len() > LARGE_LIST {
        format!(
            "Großer Zeitraum: {} Bestellungen. Bei Performance-Problemen eingrenzen.",
            rows.len()
        )
    } else {
        String::new()
    };

    let payload = KurzlistePayload {
        shop: order_pdf_shop(branding),
        period: KurzlistePeriod {
            from: from.to_string(),
            to: to.to_string(),
            count_total,
            sum_total_cents,
            count_cash,
            sum_cash_cents,
            count_online,
            sum_online_cents,
            count_voucher,
            sum_voucher_cents,
            count_redeemed: vouchers.len() as i64,
            sum_redeemed_cents: vouchers.iter().map(|v| v.total_cents).sum(),
            sum_redeemed_items_cents: vouchers.iter().map(|v| v.items_cents).sum(),
            sum_redeemed_delivery_cents: vouchers.iter().map(|v| v.delivery_cents).sum(),
            count_cancelled,
            include_test,
            include_cancelled,
            note,
        },
        rows,
        vouchers,
    };
    tokio::task::spawn_blocking(move || render_kurzliste_pdf(&payload)).await?
}

/// The shop-header block, built once per payload from branding.
fn order_pdf_shop(branding: &rusterando_frontend::branding::Branding) -> OrderPdfShop {
    OrderPdfShop {
        name: branding.display_name(),
        address: branding.full_address(),
        phone: if branding.shop_phone.trim().is_empty() {
            String::new()
        } else {
            format!("Tel. {}", branding.shop_phone)
        },
        email: branding.shop_email.clone(),
        vat_id: branding.shop_vat_id.clone(),
    }
}

/// Single-order Beleg payload: one order, no summary cover.
pub async fn load_order_pdf_payload(
    db: &SqlitePool,
    order_id: &str,
    branding: &rusterando_frontend::branding::Branding,
) -> anyhow::Result<OrderPdfPayload> {
    let order = load_one_order(db, order_id, branding).await?;
    Ok(OrderPdfPayload {
        shop: order_pdf_shop(branding),
        summary: None,
        orders: vec![order],
    })
}

/// Assemble one [`OrderPdfOrder`] from the `orders` + `order_items` rows.
/// Errors if the order id is unknown.
pub async fn load_one_order(
    db: &SqlitePool,
    order_id: &str,
    branding: &rusterando_frontend::branding::Branding,
) -> anyhow::Result<OrderPdfOrder> {
    use anyhow::Context;

    let row = sqlx::query(
        "SELECT order_number, order_type, status, contact_name, contact_phone,
                contact_email, delivery_address_json, scheduled_for,
                subtotal_cents, delivery_fee_cents, total_cents,
                voucher_code, voucher_discount_cents,
                payment_status, payment_method_detail, created_at
         FROM orders WHERE id = ?1",
    )
    .bind(order_id)
    .fetch_optional(db)
    .await?
    .context("order not found")?;

    use sqlx::Row;
    let order_number: String = row.get("order_number");
    let order_type: String = row.get("order_type");
    let status: String = row.get("status");
    let contact_name: String = row.get("contact_name");
    let contact_phone: String = row.get("contact_phone");
    let contact_email: String = row.get("contact_email");
    let delivery_address_json: Option<String> = row.get("delivery_address_json");
    let scheduled_for: Option<String> = row.try_get("scheduled_for").ok().flatten();
    let subtotal_cents: i64 = row.get("subtotal_cents");
    let delivery_fee_cents: i64 = row.get("delivery_fee_cents");
    let total_cents: i64 = row.get("total_cents");
    let voucher_code: String = row
        .try_get::<Option<String>, _>("voucher_code")
        .ok()
        .flatten()
        .unwrap_or_default();
    let voucher_discount_cents: i64 = row.try_get("voucher_discount_cents").unwrap_or(0);
    let payment_status: String = row.get("payment_status");
    let payment_method_detail: Option<String> = row.try_get("payment_method_detail").ok().flatten();
    let created_at: String = row.get("created_at");

    // Items with their snapshotted extras/removals.
    let item_rows = sqlx::query(
        "SELECT menu_number_snapshot, name_snapshot, quantity, options_json,
                unit_price_cents, line_total_cents, extras_json, removals_json,
                selected_options_json
         FROM order_items WHERE order_id = ?1 ORDER BY id",
    )
    .bind(order_id)
    .fetch_all(db)
    .await?;

    let items: Vec<OrderPdfItem> = item_rows
        .into_iter()
        .map(|r| {
            let menu_number: Option<String> = r.get("menu_number_snapshot");
            let name: String = r.get("name_snapshot");
            let quantity: i64 = r.get("quantity");
            let options_json: String = r.get("options_json");
            let unit_price_cents: i64 = r.get("unit_price_cents");
            let line_total_cents: i64 = r.get("line_total_cents");
            let extras_json: Option<String> = r.get("extras_json");
            let removals_json: Option<String> = r.get("removals_json");
            let selected_options_json: Option<String> = r.get("selected_options_json");

            let variant = serde_json::from_str::<serde_json::Value>(&options_json)
                .ok()
                .and_then(|v| {
                    v.get("size_label")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_default();
            let extras: Vec<String> = extras_json
                .as_deref()
                .and_then(|j| {
                    serde_json::from_str::<Vec<rusterando_shared::models::CartExtra>>(j).ok()
                })
                .unwrap_or_default()
                .into_iter()
                .map(|e| e.label)
                .collect();
            let removals: Vec<String> = removals_json
                .as_deref()
                .and_then(|j| {
                    serde_json::from_str::<Vec<rusterando_shared::models::CartRemoval>>(j).ok()
                })
                .unwrap_or_default()
                .into_iter()
                .map(|r| r.label)
                .collect();
            let selected_options: Vec<String> = selected_options_json
                .as_deref()
                .and_then(|j| {
                    serde_json::from_str::<Vec<rusterando_shared::models::CartSelectedOption>>(j)
                        .ok()
                })
                .unwrap_or_default()
                .into_iter()
                .map(|o| {
                    // Admin-authored, translatable DB labels — rendered verbatim.
                    let group = o.group_label.trim();
                    let choice = o.option_label.trim();
                    if group.is_empty() {
                        choice.to_string()
                    } else {
                        format!("{group}: {choice}")
                    }
                })
                .collect();

            OrderPdfItem {
                menu_number: menu_number
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_default(),
                name,
                variant,
                quantity,
                unit_price_cents,
                line_total_cents,
                extras,
                removals,
                selected_options,
            }
        })
        .collect();

    // One-line delivery address (pickup → empty).
    let address = delivery_address_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .map(|v| {
            let get = |k: &str| {
                v.get(k)
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            };
            let street = get("street");
            let house = get("house_number");
            let postcode = get("postcode");
            let city = get("city");
            let line1 = format!("{street} {house}").trim().to_string();
            let line2 = format!("{postcode} {city}").trim().to_string();
            [line1, line2]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_default();

    let channel_label = if order_type == "delivery" {
        "Lieferung".to_string()
    } else {
        "Abholung".to_string()
    };
    // scheduled_for is a naive LOCAL wall-clock string — split the time,
    // NEVER epoch-round-trip it (see kitchen.rs for the +2h trap).
    let fulfillment = scheduled_for
        .as_deref()
        .and_then(|s| s.split_whitespace().nth(1))
        .map(|t| {
            t.rsplitn(2, ':')
                .nth(1)
                .map(str::to_string)
                .unwrap_or_else(|| t.to_string())
        })
        .unwrap_or_default();
    let date = created_at
        .split_once(' ')
        .map(|(d, t)| {
            // "YYYY-MM-DD" → "DD.MM.YYYY", keep HH:MM.
            let ymd: Vec<&str> = d.split('-').collect();
            let dmy = if ymd.len() == 3 {
                format!("{}.{}.{}", ymd[2], ymd[1], ymd[0])
            } else {
                d.to_string()
            };
            let hm = t.rsplitn(2, ':').nth(1).unwrap_or(t);
            format!("{dmy} {hm}")
        })
        .unwrap_or(created_at.clone());

    // Included VAT from the shop's configured rate. German gastronomy
    // pickup/delivery is the reduced rate; the value is INCLUDED in the gross
    // total (Brutto), so included_tax = gross · rate / (100 + rate). A rate of
    // 0 means Kleinunternehmer (§19 UStG) → the legally required note instead.
    let net_total = total_cents; // gross; voucher/fee already folded in
    let tax_note = if branding.shop_tax_rate > 0.0 {
        let rate = branding.shop_tax_rate;
        let included = ((net_total as f64) * rate / (100.0 + rate)).round() as i64;
        let euros = included / 100;
        let rest = (included % 100).abs();
        // rate as a trimmed number ("7" or "19", or "10,5").
        let rate_str = {
            let s = format!("{rate}");
            s.replace('.', ",")
        };
        format!("inkl. {rate_str}% MwSt. {euros},{rest:02} €")
    } else {
        "Kleinunternehmer gem. §19 UStG — kein Umsatzsteuerausweis.".to_string()
    };

    // Payment label: prefer the instrument detail, else derive from status.
    let payment_label = match payment_status.as_str() {
        "cash_on_pickup" => "Bar bei Abholung/Lieferung".to_string(),
        "voucher_paid" => "Gutschein".to_string(),
        _ => payment_method_detail
            .filter(|s| !s.trim().is_empty())
            .map(|d| format!("Karte ({d})"))
            .unwrap_or_else(|| match payment_status.as_str() {
                "paid" => "Karte".to_string(),
                "pending" => "Karte (ausstehend)".to_string(),
                "refunded" => "Erstattet".to_string(),
                other => other.to_string(),
            }),
    };

    let status_note = if status == "cancelled" {
        "STORNIERT — diese Bestellung wurde storniert.".to_string()
    } else {
        String::new()
    };

    Ok(OrderPdfOrder {
        number: order_number,
        date,
        channel_label,
        fulfillment,
        customer_name: contact_name,
        customer_phone: contact_phone,
        customer_email: contact_email,
        address,
        items,
        subtotal_cents,
        delivery_fee_cents,
        voucher_code,
        voucher_discount_cents,
        total_cents,
        tax_note,
        payment_label,
        status_note,
    })
}

/// Load all filtered orders in a range and build a batch payload with a
/// summary cover page. Mirrors the Buchhaltung filters (date range, Sandbox,
/// Storno). Renders every matched order — no silent cap — but sets a
/// `truncated_note` warning when the count is large so the user can narrow.
pub async fn build_orders_pdf(
    db: &SqlitePool,
    from: &str,
    to: &str,
    include_test: bool,
    include_cancelled: bool,
    branding: &rusterando_frontend::branding::Branding,
) -> anyhow::Result<Vec<u8>> {
    use sqlx::Row;

    // Same date-range + mode semantics as list_admin_history / the CSV.
    let from_ts = format!("{from} 00:00:00");
    let to_ts = format!("{to} 23:59:59");
    let mode_clause = if include_test {
        ""
    } else {
        " AND stripe_mode = 'live'"
    };
    let cancelled_clause = if include_cancelled {
        ""
    } else {
        " AND status != 'cancelled'"
    };

    // Ids in the same order the list shows (newest first). We rebuild each
    // order via the shared single-order loader so the batch pages are
    // byte-for-byte the same layout as a single Beleg.
    let id_sql = format!(
        "SELECT id FROM orders
         WHERE created_at BETWEEN ?1 AND ?2{mode_clause}{cancelled_clause}
         ORDER BY created_at ASC",
    );
    let id_rows = sqlx::query(&id_sql)
        .bind(&from_ts)
        .bind(&to_ts)
        .fetch_all(db)
        .await?;
    let ids: Vec<String> = id_rows.iter().map(|r| r.get::<String, _>("id")).collect();

    // Summary figures: non-cancelled count + revenue, and the Storno count,
    // computed with the same filters — one aggregate query.
    let agg_sql = format!(
        "SELECT
            COALESCE(SUM(CASE WHEN status != 'cancelled' THEN 1 ELSE 0 END), 0) AS cnt,
            COALESCE(SUM(CASE WHEN status != 'cancelled' THEN total_cents ELSE 0 END), 0) AS rev,
            COALESCE(SUM(CASE WHEN status  = 'cancelled' THEN 1 ELSE 0 END), 0) AS canc,
            COALESCE(SUM(CASE WHEN status != 'cancelled' AND voucher_discount_cents > 0 THEN 1 ELSE 0 END), 0) AS vcnt,
            COALESCE(SUM(CASE WHEN status != 'cancelled' THEN voucher_discount_cents ELSE 0 END), 0) AS vsum
         FROM orders
         WHERE created_at BETWEEN ?1 AND ?2{mode_clause}",
    );
    let agg = sqlx::query(&agg_sql)
        .bind(&from_ts)
        .bind(&to_ts)
        .fetch_one(db)
        .await?;
    let count: i64 = agg.get("cnt");
    let total_cents: i64 = agg.get("rev");
    let cancelled_count: i64 = agg.get("canc");
    let voucher_count: i64 = agg.get("vcnt");
    let voucher_cents: i64 = agg.get("vsum");

    // Warn (don't drop) above this many — a big export is slow + heavy.
    const LARGE_EXPORT: usize = 500;
    let truncated_note = if ids.len() > LARGE_EXPORT {
        format!(
            "Großer Export: {} Belege. Bei Performance-Problemen den Zeitraum eingrenzen.",
            ids.len()
        )
    } else {
        String::new()
    };

    let mut orders = Vec::with_capacity(ids.len());
    for id in &ids {
        match load_one_order(db, id, branding).await {
            Ok(o) => orders.push(o),
            // Skip a row that vanished mid-export rather than failing the whole PDF.
            Err(e) => tracing::warn!("belege batch: skipping {id}: {e}"),
        }
    }

    let payload = OrderPdfPayload {
        shop: order_pdf_shop(branding),
        summary: Some(OrderPdfSummary {
            from: from.to_string(),
            to: to.to_string(),
            count,
            total_cents,
            cancelled_count,
            voucher_count,
            voucher_cents,
            include_test,
            include_cancelled,
            truncated_note,
        }),
        orders,
    };
    tokio::task::spawn_blocking(move || render_order_pdf(&payload)).await?
}

#[cfg(test)]
mod beleg_tests {
    use super::*;

    fn sample_shop() -> OrderPdfShop {
        OrderPdfShop {
            name: "Musterpizzeria".into(),
            address: "Musterstr. 1, 12345 Musterstadt".into(),
            phone: "Tel. 01234-56789".into(),
            email: "hallo@example.com".into(),
            vat_id: "DE123456789".into(),
        }
    }

    fn sample_order() -> OrderPdfOrder {
        OrderPdfOrder {
            number: "DP-0207-0002".into(),
            date: "02.07.2026 18:12".into(),
            channel_label: "Abholung".into(),
            fulfillment: "18:30".into(),
            customer_name: "Max Mustermann".into(),
            customer_phone: "0170 1234567".into(),
            customer_email: "".into(),
            address: "".into(),
            items: vec![
                OrderPdfItem {
                    menu_number: "1".into(),
                    name: "Pizza Margherita".into(),
                    variant: "30cm".into(),
                    quantity: 1,
                    unit_price_cents: 800,
                    line_total_cents: 800,
                    extras: vec!["Extra Käse".into()],
                    removals: vec!["Zwiebeln".into()],
                    selected_options: vec![],
                },
                OrderPdfItem {
                    menu_number: "400".into(),
                    name: "Pizzabrötchen".into(),
                    variant: "".into(),
                    quantity: 2,
                    unit_price_cents: 300,
                    line_total_cents: 600,
                    extras: vec![],
                    removals: vec![],
                    selected_options: vec!["Sauce: Kräuterbutter".into()],
                },
            ],
            subtotal_cents: 1400,
            delivery_fee_cents: 0,
            voucher_code: "".into(),
            voucher_discount_cents: 0,
            total_cents: 1400,
            tax_note: "inkl. 7% MwSt. 92 €".into(),
            payment_label: "Karte (Visa)".into(),
            status_note: "".into(),
        }
    }

    fn single(order: OrderPdfOrder) -> OrderPdfPayload {
        OrderPdfPayload {
            shop: sample_shop(),
            summary: None,
            orders: vec![order],
        }
    }

    #[test]
    fn beleg_renders() {
        let bytes = render_order_pdf(&single(sample_order())).expect("beleg should render");
        assert!(
            bytes.len() > 1000,
            "PDF suspiciously small: {}",
            bytes.len()
        );
        // PDF magic.
        assert_eq!(&bytes[..5], b"%PDF-");
    }

    #[test]
    fn beleg_with_voucher_and_delivery_renders() {
        let mut o = sample_order();
        o.address = "Beispielweg 3, 12345 Musterstadt".into();
        o.channel_label = "Lieferung".into();
        o.delivery_fee_cents = 250;
        o.voucher_code = "WILLKOMMEN10".into();
        o.voucher_discount_cents = 500;
        o.total_cents = 1150;
        o.status_note = "STORNIERT — diese Bestellung wurde storniert.".into();
        let bytes = render_order_pdf(&single(o)).expect("beleg should render");
        assert!(bytes.len() > 1000);
    }

    #[test]
    fn batch_with_summary_renders() {
        // Two orders + a summary cover page.
        let o1 = sample_order();
        let mut o2 = sample_order();
        o2.number = "DP-0207-0003".into();
        o2.total_cents = 2300;
        let payload = OrderPdfPayload {
            shop: sample_shop(),
            summary: Some(OrderPdfSummary {
                from: "2026-07-01".into(),
                to: "2026-07-08".into(),
                count: 2,
                total_cents: 3700,
                cancelled_count: 1,
                voucher_count: 1,
                voucher_cents: 250,
                include_test: false,
                include_cancelled: false,
                truncated_note: "".into(),
            }),
            orders: vec![o1, o2],
        };
        let bytes = render_order_pdf(&payload).expect("batch beleg should render");
        assert!(bytes.len() > 1000);
        assert_eq!(&bytes[..5], b"%PDF-");
    }
}

#[cfg(test)]
mod kurzliste_tests {
    use super::*;

    fn shop() -> OrderPdfShop {
        OrderPdfShop {
            name: "Musterpizzeria".into(),
            address: "Musterstr. 1, 12345 Musterstadt".into(),
            phone: "".into(),
            email: "".into(),
            vat_id: "DE123456789".into(),
        }
    }

    /// `marker`: "" cash, "*" online, "†" voucher.
    fn row(date: &str, number: &str, cents: i64, marker: &str) -> KurzlisteRow {
        KurzlisteRow {
            date: date.into(),
            number: number.into(),
            total_cents: cents,
            voucher_cents: 0,
            marker: marker.into(),
        }
    }

    fn payload(rows: Vec<KurzlisteRow>, include_cancelled: bool) -> KurzlistePayload {
        let mut cash = (0i64, 0i64);
        let mut online = (0i64, 0i64);
        let mut voucher = (0i64, 0i64);
        for r in &rows {
            let bucket = match r.marker.as_str() {
                "*" => &mut online,
                "†" => &mut voucher,
                _ => &mut cash,
            };
            bucket.0 += 1;
            bucket.1 += r.total_cents;
        }
        KurzlistePayload {
            shop: shop(),
            period: KurzlistePeriod {
                from: "2026-07-01".into(),
                to: "2026-08-31".into(),
                count_total: cash.0 + online.0 + voucher.0,
                sum_total_cents: cash.1 + online.1 + voucher.1,
                count_cash: cash.0,
                sum_cash_cents: cash.1,
                count_online: online.0,
                sum_online_cents: online.1,
                count_voucher: voucher.0,
                sum_voucher_cents: voucher.1,
                count_redeemed: 0,
                sum_redeemed_cents: 0,
                sum_redeemed_items_cents: 0,
                sum_redeemed_delivery_cents: 0,
                count_cancelled: if include_cancelled { 2 } else { 0 },
                include_test: false,
                include_cancelled,
                note: String::new(),
            },
            rows,
            vouchers: Vec::new(),
        }
    }

    // Redeemed Gutscheine: a "G" on their orders, a summary line and the
    // Werbungskosten list at the end.
    #[test]
    fn kurzliste_with_vouchers_renders() {
        let mut rows = vec![
            row("02.07.2026 13:54", "DP-0207-0001", 1500, "*"),
            row("03.07.2026 17:46", "DP-0307-0001", 0, "†"),
        ];
        rows[0].voucher_cents = 500;
        rows[1].voucher_cents = 1890;
        let mut p = payload(rows, false);
        p.vouchers = vec![
            KurzlisteVoucher {
                date: "02.07.2026 13:54".into(),
                number: "DP-0207-0001".into(),
                code: "SOMMER5".into(),
                label: "Sommer-Aktion".into(),
                items_cents: 500,
                delivery_cents: 0,
                total_cents: 500,
            },
            KurzlisteVoucher {
                date: "03.07.2026 17:46".into(),
                number: "DP-0307-0001".into(),
                code: "GRATIS".into(),
                label: String::new(),
                items_cents: 1740,
                delivery_cents: 150,
                total_cents: 1890,
            },
        ];
        p.period.count_redeemed = 2;
        p.period.sum_redeemed_cents = 2390;
        p.period.sum_redeemed_items_cents = 2240;
        p.period.sum_redeemed_delivery_cents = 150;
        let bytes = render_kurzliste_pdf(&p).expect("kurzliste with vouchers should render");
        assert!(bytes.len() > 1000);
    }

    #[test]
    fn kurzliste_renders() {
        // One of each settlement kind so all three summary lines render.
        let rows = vec![
            row("02.07.2026 13:54", "DP-0207-0001", 1000, "*"),
            row("02.07.2026 18:04", "DP-0207-0002", 4374, ""),
            row("03.07.2026 17:46", "DP-0307-0001", 0, "†"),
        ];
        let bytes = render_kurzliste_pdf(&payload(rows, false)).expect("kurzliste should render");
        assert!(
            bytes.len() > 1000,
            "PDF suspiciously small: {}",
            bytes.len()
        );
        assert_eq!(&bytes[..5], b"%PDF-");
    }

    #[test]
    fn kurzliste_empty_renders() {
        // No orders in range → still a valid one-page PDF with the header.
        let bytes = render_kurzliste_pdf(&payload(vec![], false)).expect("empty should render");
        assert!(bytes.len() > 500);
        assert_eq!(&bytes[..5], b"%PDF-");
    }

    #[test]
    fn kurzliste_with_storno_renders() {
        let rows = vec![row("06.08.2026 21:14", "DP-0608-0003", 550, "")];
        let bytes =
            render_kurzliste_pdf(&payload(rows, true)).expect("storno variant should render");
        assert!(bytes.len() > 1000);
    }

    #[test]
    fn local_datetime_formats_and_falls_back() {
        // Parseable UTC → DD.MM.YYYY HH:MM (exact local offset depends on the
        // test host TZ, so just assert the shape: 16 chars, dots + colon).
        let out = kurzliste_local_datetime("2026-07-02 16:04:11");
        assert_eq!(out.len(), 16, "got {out:?}");
        assert_eq!(&out[2..3], ".");
        assert_eq!(&out[13..14], ":");
        // Garbage in → echoed back unchanged.
        assert_eq!(kurzliste_local_datetime("not-a-date"), "not-a-date");
    }
}
