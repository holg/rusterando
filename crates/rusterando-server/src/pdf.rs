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

const TEMPLATE_SRC: &str = include_str!("../../../templates/menu.typ");
const TEMPLATE_VPATH: &str = "/menu.typ";

// Image assets the template can `image("/img/...")` for. Bytes are embedded
// at compile time so the running binary needs no filesystem access.
const ASSET_LADENFRONT: &[u8] = include_bytes!("../../../public/img/ladenfront.jpg");
const ASSET_LADENFRONT_VPATH: &str = "/img/ladenfront.jpg";

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
    main_id: FileId,
    main_source: Source,
    /// SVG bytes for the QR code that points at `payload.site_url`. Generated
    /// once per request — Typst caches the resulting `Image`.
    qr_svg: Vec<u8>,
}

impl MenuWorld {
    fn new(payload: &MenuPdfPayload) -> anyhow::Result<Self> {
        let json = serde_json::to_string(payload)?;
        let mut inputs = Dict::new();
        inputs.insert("data".into(), typst::foundations::Value::Str(json.into()));
        let library = Library::builder().with_inputs(inputs).build();

        let main_id = FileId::new(None, VirtualPath::new(TEMPLATE_VPATH));
        let main_source = Source::new(main_id, TEMPLATE_SRC.to_string());

        let qr_svg = render_qr_svg(&payload.site_url);

        Ok(Self {
            library: LazyHash::new(library),
            main_id,
            main_source,
            qr_svg,
        })
    }
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
        } else {
            Err(FileError::NotFound(id.vpath().as_rootless_path().into()))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        let path = id.vpath().as_rooted_path();
        match path.to_str() {
            Some(p) if p == ASSET_LADENFRONT_VPATH => Ok(Bytes::new(ASSET_LADENFRONT.to_vec())),
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
pub fn render_menu_pdf(payload: &MenuPdfPayload) -> anyhow::Result<Vec<u8>> {
    let world = MenuWorld::new(payload)?;
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
    Ok(MenuPdfPayload {
        branding: Branding {
            name: shop_branding.display_name(),
            tagline: String::new(),
            address: shop_branding.full_address(),
            phone: phone_line,
            hours_lines: vec![
                "Mo, Di, Do, So: 11:30–14:30 · 17:00–22:00".into(),
                "Fr, Sa: 16:00–22:00".into(),
                "Mi Ruhetag".into(),
            ],
            delivery_lines,
            extras_pizza_lines: vec![
                "Krabben 1 €".into(),
                "Lachs 2 €".into(),
                "Kräuterbutter 0,60 €".into(),
                "sonstige Extras 0,70 €".into(),
            ],
            extras_pasta_lines: vec![
                "Krabben 1 €".into(),
                "Lachs 2 €".into(),
                "sonstige Extras 0,70 €".into(),
            ],
        },
        categories,
        allergens,
        additives,
        site_url,
    })
}

/// Convenience: load + render in one go. Compile runs on a blocking pool so
/// the async runtime stays responsive during the ~hundreds-of-ms render.
pub async fn build_menu_pdf(
    db: &SqlitePool,
    site_url: String,
    shop_branding: &rusterando_frontend::branding::Branding,
) -> anyhow::Result<Vec<u8>> {
    let payload = load_menu_payload(db, site_url, shop_branding).await?;
    tokio::task::spawn_blocking(move || render_menu_pdf(&payload)).await?
}
