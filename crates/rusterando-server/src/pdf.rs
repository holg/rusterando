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
pub const TEMPLATE_SRC: &str = include_str!("../../../templates/menu.typ");
/// The theme is imported by the geometry layer as `/theme.typ`.
const THEME_VPATH: &str = "/theme.typ";

/// Geometry layer — the Typst MAIN source. Owns page size + fold layout per
/// `?format=`, imports the styling theme (`/theme.typ`). Not user-editable.
const GEOMETRY_SRC: &str = include_str!("../../../templates/geometry.typ");
const GEOMETRY_VPATH: &str = "/geometry.typ";

// Image assets the template can `image("/img/...")` for. Bytes are embedded
// at compile time so the running binary needs no filesystem access.
const ASSET_LADENFRONT: &[u8] = include_bytes!("../../../public/img/ladenfront.jpg");
const ASSET_LADENFRONT_VPATH: &str = "/img/ladenfront.jpg";
/// Tri-fold cover photo used on the left panel of page 1 (the
/// outside-front when the leporello is folded shut). Static asset;
/// to change it, replace public/img/cover.jpg and rebuild.
const ASSET_COVER: &[u8] = include_bytes!("../../../public/img/cover.jpg");
const ASSET_COVER_VPATH: &str = "/img/cover.jpg";

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
const NOTO_EMOJI: &[u8] = include_bytes!(concat!(env!("FONT_CACHE_DIR"), "/NotoColorEmoji.ttf"));

/// One-time initialised resources shared across PDF requests. Searching the
/// system + embedded fonts on every request would be wasteful.
struct PdfResources {
    book: LazyHash<FontBook>,
    fonts: Vec<typst_kit::fonts::FontSlot>,
}

fn resources() -> &'static PdfResources {
    static R: OnceLock<PdfResources> = OnceLock::new();
    R.get_or_init(|| {
        // Materialise our embedded fonts to a temp dir Typst can scan. We use
        // /tmp/davidspizzeria-fonts as a stable location so the same process
        // restarting doesn't pile up new dirs.
        let dir = std::env::temp_dir().join("davidspizzeria-fonts");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("font cache dir {dir:?}: {e} — falling back to system fonts");
        }
        for (name, bytes) in [
            ("Inter-Regular.ttf", INTER_REGULAR),
            ("Inter-Bold.ttf", INTER_BOLD),
            ("Inter-Italic.ttf", INTER_ITALIC),
            ("NotoColorEmoji.ttf", NOTO_EMOJI),
        ] {
            let dst = dir.join(name);
            // Write only if missing or size mismatches (faster than always
            // rewriting; also avoids racing concurrent builds).
            let need = match std::fs::metadata(&dst) {
                Ok(m) => m.len() as usize != bytes.len(),
                Err(_) => true,
            };
            if need {
                if let Err(e) = std::fs::write(&dst, bytes) {
                    tracing::warn!("write {name}: {e}");
                }
            }
        }

        let fonts = typst_kit::fonts::Fonts::searcher()
            .include_system_fonts(false)
            .include_embedded_fonts(true)
            .search_with([&dir]);
        tracing::info!(
            "pdf: registered {} fonts (4 bundled + Typst defaults)",
            fonts.fonts.len()
        );
        PdfResources {
            book: LazyHash::new(fonts.book),
            fonts: fonts.fonts,
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
        let json = serde_json::to_string(payload)?;
        let mut inputs = Dict::new();
        inputs.insert("data".into(), typst::foundations::Value::Str(json.into()));
        // `format` selects the page geometry / fold layout in the template.
        inputs.insert(
            "format".into(),
            typst::foundations::Value::Str(format.into()),
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

        let qr_svg = render_qr_svg(&payload.site_url);

        Ok(Self {
            library: LazyHash::new(library),
            main_id,
            main_source,
            theme_id,
            theme_source,
            qr_svg,
            cover_override: overrides.cover_image.clone(),
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
            _ => Err(FileError::NotFound(id.vpath().as_rootless_path().into())),
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        resources().fonts.get(index)?.get()
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

    let opts = typst_pdf::PdfOptions::default();
    let bytes = typst_pdf::pdf(&document, &opts).map_err(|errs| {
        let msgs: Vec<_> = errs.iter().map(|e| eco_format!("{}", e.message)).collect();
        anyhow::anyhow!("Typst PDF export failed: {}", msgs.join("; "))
    })?;

    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Build a payload from the live database
// ---------------------------------------------------------------------------

pub async fn load_menu_payload(
    db: &SqlitePool,
    site_url: String,
    shop_branding: &rusterando_frontend::branding::Branding,
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
    )> = sqlx::query_as(
        "SELECT id, category_id, menu_number, name, description,
                price_small_cents, price_large_cents, size_small_label, size_large_label,
                allergen_codes, additive_codes, is_spicy, is_available, sort_order
         FROM menu_items
         WHERE is_listed = 1 AND is_available = 1
         ORDER BY sort_order,
                  CAST(COALESCE(menu_number, '') AS INTEGER),
                  menu_number",
    )
    .fetch_all(db)
    .await?;

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
    // proper own-branded text cover instead of inheriting the bundled davids
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
        },
        categories,
        allergens,
        additives,
        site_url,
    })
}

fn default_hours_lines() -> Vec<String> {
    vec![
        "Mo, Di, Do, So: 11:30–14:30 · 17:00–22:00".into(),
        "Fr, Sa: 16:00–22:00".into(),
        "Mi Ruhetag".into(),
    ]
}
fn default_extras_pizza_lines() -> Vec<String> {
    vec![
        "Krabben 1 €".into(),
        "Lachs 2 €".into(),
        "Kräuterbutter 0,60 €".into(),
        "sonstige Extras 0,70 €".into(),
    ]
}
fn default_extras_pasta_lines() -> Vec<String> {
    vec![
        "Krabben 1 €".into(),
        "Lachs 2 €".into(),
        "sonstige Extras 0,70 €".into(),
    ]
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
    let id: i64 = read_setting(db, "pdf_theme_active_id")
        .await?
        .trim()
        .parse()
        .ok()?;
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
/// renders: `"trifold"` (default — the 443×210 three-panel sheet) or
/// `"a5-zickzack"` (DIN-A5 portrait, 6-panel accordion, 888×210 + 3 mm bleed
/// + fold marks for the 4c print job). Unknown values fall back to trifold in
/// the caller.
pub async fn build_menu_pdf(
    db: &SqlitePool,
    site_url: String,
    shop_branding: &rusterando_frontend::branding::Branding,
    uploads_dir: &str,
    format: &str,
    show_ingredients: bool,
    shared: Option<&SqlitePool>,
) -> anyhow::Result<Vec<u8>> {
    let payload = load_menu_payload(db, site_url, shop_branding).await?;
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
    Ok(())
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
