//! Single source of truth for the **receipt layout** — the "Bon-Designer".
//!
//! The receipt is described as data, not printer bytes. There are two
//! representations and one composition step between them:
//!
//!   * [`build_layout`] turns an [`OrderForKitchen`] into a list of
//!     [`LayoutLine`]s — *what the ticket says and how each line is
//!     styled*, with logos still as theme-keyed [`RasterSlot`]s. The
//!     admin preview renders this directly (slot → `<img>` of the
//!     `.1bit.png` twin), so the preview can't drift from the layout.
//!
//!   * [`compose`] resolves every [`RasterSlot`] into **inline 1-bit
//!     bytes** via a [`RasterProvider`], yielding a [`ReceiptProgram`] —
//!     a fully self-contained, serializable list of [`ReceiptLine`]s.
//!     This is what the **server sends to the Pi**: the Pi holds no
//!     themes and no assets, it just executes primitives (text, raster
//!     bytes, native QR, rule, feed, cut). New themes/artwork are a
//!     server-only change.
//!
//! This module is column-math + composition but **not** ESC/POS: it
//! knows the receipt is `RECEIPT_WIDTH` columns wide and packs 1-bit
//! rasters, but emits no printer bytes. The Pi maps a [`ReceiptLine`] →
//! ESC/POS; the frontend maps a [`LayoutLine`] → CSS/`<img>`.

use serde::{Deserialize, Serialize};

use crate::{OrderChannel, OrderForKitchen, PaymentStatus};

/// Single-width character columns on an 80-mm thermal receipt.
pub const RECEIPT_WIDTH: usize = 42;

/// Per-shop, admin-editable receipt configuration (the "Bon-Editor"
/// output). Drives the optional/configurable parts of the layout; the
/// data-driven parts (items, totals, customer) still come from the order.
/// Resolved from `app_settings` server-side and passed into
/// [`build_layout`]. `Default` = the historical hardcoded behaviour, so
/// an un-migrated DB renders exactly as before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptConfig {
    /// Optional override for the top brand line. Empty → use the order's
    /// `shop_name` (the prior behaviour).
    pub header_text: String,
    /// Optional free-text line printed at the very bottom of EVERY receipt
    /// (e.g. a thank-you / promo). Empty → nothing printed. Distinct from
    /// the per-order customer `note` (Anmerkung), which is unaffected.
    pub footer_text: String,
    /// Show the allergen reminder block.
    pub show_allergens: bool,
    /// Show the theme logo row (channel icon + brand mark).
    pub show_logo: bool,
    /// Admin-overridable receipt badge/banner texts. Each empty → built-in
    /// German default (see [`ReceiptLabels::or`]).
    #[serde(default)]
    pub labels: ReceiptLabels,
}

impl Default for ReceiptConfig {
    fn default() -> Self {
        Self {
            header_text: String::new(),
            footer_text: String::new(),
            show_allergens: true,
            show_logo: true,
            labels: ReceiptLabels::default(),
        }
    }
}

/// Per-shop overrides for the fixed banner/badge texts on the receipt.
/// Every field is an override: empty string → the built-in default. This
/// keeps the wire payload tiny (defaults aren't sent) and means an
/// un-migrated shop renders exactly as before.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptLabels {
    // Channel banners
    #[serde(default)]
    pub channel_delivery: String, // "LIEFERUNG"
    #[serde(default)]
    pub channel_pickup: String, // "ABHOLUNG"
    #[serde(default)]
    pub channel_dinein: String, // "TISCH" (table number appended)
    // Payment markers
    #[serde(default)]
    pub paid_badge: String, // "BEZAHLT"
    #[serde(default)]
    pub unpaid_badge: String, // "NICHT BEZAHLT"
    #[serde(default)]
    pub paid_confirm: String, // "BESTELLUNG IST BEZAHLT" (printed as two lines)
    #[serde(default)]
    pub paid_by: String, // "Bezahlt von:"
    #[serde(default)]
    pub cash: String, // "Bar"
    #[serde(default)]
    pub collect_delivery: String, // "Bei Lieferung kassieren:"
    #[serde(default)]
    pub collect_pickup: String, // "Bei Abholung kassieren:"
    #[serde(default)]
    pub collect_dinein: String, // "Am Tisch kassieren:"
    // Footer / legal
    #[serde(default)]
    pub disclaimer: String, // "Das ist keine Rechnung"
    #[serde(default)]
    pub qr_caption: String, // "Zum Bestelldetail scannen"
    #[serde(default)]
    pub allergen_notice: String, // the allergen reminder block (\n-separated)
    // Banners
    #[serde(default)]
    pub reprint_banner: String, // "*** REPRINT ***"
    #[serde(default)]
    pub test_banner: String, // "*** TEST ***"
}

impl ReceiptLabels {
    /// Pick the override if non-empty (trimmed), else the built-in default.
    pub fn or<'a>(over: &'a str, default: &'a str) -> &'a str {
        let t = over.trim();
        if t.is_empty() {
            default
        } else {
            t
        }
    }
}
/// Common dot-height both header logos are scaled to before composing.
/// Keeps the combined band inside the ~512-dot print head.
pub const HEADER_LOGO_HEIGHT: usize = 120;
/// White columns between the channel icon and the brand mark.
pub const HEADER_LOGO_GAP: usize = 28;

/// A theme raster slot — the *intent* of a logo, before its pixels are
/// resolved. [`compose`] turns each slot into inline bytes via a
/// [`RasterProvider`]; the preview maps it to the matching `.1bit.png`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterSlot {
    /// The header row: the per-channel icon (e-car for delivery, bag for
    /// pickup) on the LEFT, the Rusterando brand mark on the RIGHT, both
    /// on ONE line to save paper. Pre-composed into a single band.
    ///
    /// `is_delivery` picks the left icon. `is_paid` picks the RIGHT brand
    /// mark: a distinct "(R) RUSTERANDO" variant for prepaid vs. cash, so
    /// the kitchen reads channel AND payment from the logo row at a glance.
    ChannelAndBrand { is_delivery: bool, is_paid: bool },
}

/// A composed SVG ready to ship to the Pi: the SVG document bytes plus
/// the dot-width the Pi should rasterize it at. The Pi caches the
/// resulting 1-bit raster keyed by the content hash (see [`compose`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedSvg {
    pub svg: Vec<u8>,
    /// Target raster width in printer dots (height follows aspect).
    pub target_width: u16,
}

/// Supplies the composed [`ComposedSvg`] for a [`RasterSlot`]. The server
/// implements this by reading the theme's source `.svg` files and laying
/// them out side-by-side. Returns `None` for a missing asset / unknown
/// theme (the slot is then dropped, and the text banners still convey the
/// channel). The SVG is rasterized to 1-bit ON THE PI, not here.
pub trait RasterProvider {
    fn resolve(&self, theme: &str, slot: RasterSlot) -> Option<ComposedSvg>;
}

/// Content hash for a composed raster: sha256 of the SVG bytes plus the
/// target width (so a width change re-rasterizes). The Pi keys its raster
/// cache by this; the server decides ref-vs-bytes by whether the Pi
/// already advertised it in Hello.
pub fn raster_hash(svg: &[u8], target_width: u16) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(svg);
    h.update(target_width.to_le_bytes());
    h.finalize().into()
}

/// How a line of receipt text is emphasised. Maps to ESC/POS on the Pi
/// and to CSS classes in the preview. `width`/`height` are ESC/POS
/// `GS !` multipliers (1 = normal, 2 = double, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineStyle {
    pub center: bool,
    pub bold: bool,
    pub width: u8,
    pub height: u8,
}

impl LineStyle {
    pub const NORMAL: LineStyle = LineStyle {
        center: false,
        bold: false,
        width: 1,
        height: 1,
    };
    pub const fn centered() -> LineStyle {
        LineStyle {
            center: true,
            ..LineStyle::NORMAL
        }
    }
    pub const fn bold(mut self) -> LineStyle {
        self.bold = true;
        self
    }
    pub const fn size(mut self, w: u8, h: u8) -> LineStyle {
        self.width = w;
        self.height = h;
        self
    }
}

/// One element of a receipt *layout*, in print order. Logos are still
/// theme-keyed [`RasterSlot`]s here — this is the pre-composition form
/// the admin preview renders. Not serialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutLine {
    Text { text: String, style: LineStyle },
    Feed { n: u8 },
    Rule,
    Raster(RasterSlot),
    Qr { url: String },
}

impl LayoutLine {
    fn text(text: impl Into<String>, style: LineStyle) -> LayoutLine {
        LayoutLine::Text {
            text: text.into(),
            style,
        }
    }
}

/// One element of a **composed** receipt, in print order — fully
/// self-contained and serializable. This is the wire form: logos are
/// inline 1-bit [`Bitmap`]s, QR is still a `url` the Pi draws natively.
/// The Pi executes these as ESC/POS primitives; it knows no themes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiptLine {
    /// A styled text line, already wrapped/padded to `RECEIPT_WIDTH`.
    Text { text: String, style: LineStyle },
    /// Vertical whitespace — `n` blank feeds.
    Feed { n: u8 },
    /// A full-width `-` rule.
    Rule,
    /// A centered logo as SVG. The Pi rasterizes it to 1-bit at
    /// `target_width` dots and emits `GS v 0`, caching the result keyed by
    /// `hash` (= [`raster_hash`]). On a repeat the server may send the same
    /// `hash` with an EMPTY `svg` (a reference) once the Pi advertised it
    /// in Hello — the Pi then uses its cache. Miss with empty svg → the
    /// logo is skipped (text banners still convey the channel) and re-sent
    /// with bytes on the next reconnect.
    RasterSvg {
        hash: [u8; 32],
        target_width: u16,
        svg: Vec<u8>,
    },
    /// A QR code the Pi draws NATIVELY (ESC/POS `GS ( k`) from this URL —
    /// never a pre-rendered bitmap.
    Qr { url: String },
    /// Feed paper and cut. Terminates the receipt.
    Cut { feed: u8 },
}

/// A complete, self-contained receipt program: the server composes it
/// and ships it to the Pi, which executes the lines in order. Carries
/// the order's identity for the kitchen log / idempotency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptProgram {
    /// Numeric shorthand, for the "printed + acked order=N" log line.
    pub display_number: u32,
    /// Whether this is a reprint (the banner is already baked into
    /// `lines`; this is just metadata for logging).
    pub is_reprint: bool,
    pub lines: Vec<ReceiptLine>,
}

/// Build the full receipt **layout** for an order (logos as slots), with
/// the admin-editable [`ReceiptConfig`] applied. Pure: no I/O, no printer
/// bytes. The admin preview renders this; the server passes it to
/// [`compose`] to inline the rasters for the wire.
pub fn build_layout(
    order: &OrderForKitchen,
    is_reprint: bool,
    config: &ReceiptConfig,
) -> Vec<LayoutLine> {
    use LayoutLine as ReceiptLine;
    let mut out: Vec<ReceiptLine> = Vec::new();
    let center = LineStyle::centered();
    let l = &config.labels;

    // ===== TEST MODE banner (top) =====
    if order.is_test_mode {
        push_test_banner(&mut out, l);
    }

    // ===== Reprint banner =====
    if is_reprint {
        out.push(ReceiptLine::text(
            ReceiptLabels::or(&l.reprint_banner, "*** REPRINT ***"),
            center.bold(),
        ));
        out.push(ReceiptLine::Feed { n: 1 });
    }

    // ===== Brand line =====
    // Admin override (`receipt_header_text`) wins; else the order's
    // shop_name; else nothing.
    let header = if !config.header_text.is_empty() {
        config.header_text.as_str()
    } else {
        order.shop_name.as_str()
    };
    if !header.is_empty() {
        out.push(ReceiptLine::text(header, center.bold()));
    }

    // ===== Theme logo row =====
    // One combined band: channel icon (left) + Rusterando brand mark
    // (right). One row, minimal paper. Suppressed when the admin turns the
    // logo row off.
    if config.show_logo && theme_has_assets(&order.printer_theme) {
        let is_delivery = matches!(order.channel, OrderChannel::Delivery { .. });
        let is_paid = matches!(order.payment, PaymentStatus::Prepaid);
        out.push(ReceiptLine::Raster(RasterSlot::ChannelAndBrand {
            is_delivery,
            is_paid,
        }));
        out.push(ReceiptLine::Feed { n: 1 });
    }

    // ===== Channel banner (LIEFERUNG / ABHOLUNG / TISCH) =====
    let channel_label = match &order.channel {
        OrderChannel::Delivery { .. } => {
            ReceiptLabels::or(&l.channel_delivery, "LIEFERUNG").to_string()
        }
        OrderChannel::Pickup => ReceiptLabels::or(&l.channel_pickup, "ABHOLUNG").to_string(),
        OrderChannel::DineIn { table } => {
            format!("{} {table}", ReceiptLabels::or(&l.channel_dinein, "TISCH"))
        }
    };
    out.push(ReceiptLine::text(channel_label, center.bold().size(2, 2)));

    // ===== Eingang + scheduled fulfillment time =====
    let created_ts = if !order.created_at_label.is_empty() {
        format!("Eingang: {}", order.created_at_label)
    } else {
        format!("Eingang: {}", fmt_local_fallback(order.created_at_unix))
    };
    let scheduled_line = order
        .pickup_time_label
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|when| {
            let verb = match order.channel {
                OrderChannel::Delivery { .. } => "Lieferung",
                OrderChannel::Pickup => "Abholung",
                OrderChannel::DineIn { .. } => "Servieren",
            };
            format!("({verb} {when})")
        });

    let big = center.bold().size(1, 2);
    let scheduled_first = matches!(order.channel, OrderChannel::Delivery { .. });
    match (&scheduled_line, scheduled_first) {
        (Some(sched), true) => {
            out.push(ReceiptLine::text(sched, big));
            out.push(ReceiptLine::text(&created_ts, big));
        }
        (Some(sched), false) => {
            out.push(ReceiptLine::text(&created_ts, big));
            out.push(ReceiptLine::text(sched, big));
        }
        (None, _) => {
            out.push(ReceiptLine::text(&created_ts, big));
        }
    }
    out.push(ReceiptLine::Feed { n: 1 });

    // ===== Big bold display_label =====
    let label = if order.display_label.is_empty() {
        format!("# {}", order.display_number)
    } else {
        order.display_label.clone()
    };
    out.push(ReceiptLine::text(label, center.bold().size(2, 3)));

    out.push(ReceiptLine::Rule);

    // ===== Line items grouped by category =====
    let mut last_category: Option<&str> = None;
    for item in &order.items {
        let cat = item.category.trim();
        let cat_changed = match last_category {
            Some(prev) => prev != cat,
            None => true,
        };
        if cat_changed && !cat.is_empty() {
            out.push(ReceiptLine::Feed { n: 1 });
            out.push(ReceiptLine::text(cat, LineStyle::NORMAL.bold()));
        }
        last_category = Some(cat);

        let line_total = item.unit_price_cents * item.qty;
        let qty_name = format!("{} x {}", item.qty, item.name);
        push_wrapped_pair(&mut out, &qty_name, &format_euro(line_total));

        for (i, m) in item.modifications.iter().enumerate() {
            let pretty = format!("+ {m}");
            let price = item.modification_prices_cents.get(i).copied().unwrap_or(0);
            if price > 0 {
                push_wrapped_pair(&mut out, &format!("   {pretty}"), &format_euro(price));
            } else {
                out.push(ReceiptLine::text(format!("   {pretty}"), LineStyle::NORMAL));
            }
        }
    }

    out.push(ReceiptLine::Rule);

    // ===== Totals =====
    out.push(ReceiptLine::text(
        pad_line("Zwischensumme", &format_euro(order.subtotal_cents)),
        LineStyle::NORMAL,
    ));
    if order.delivery_fee_cents > 0 {
        out.push(ReceiptLine::text(
            pad_line("Liefergebühr", &format_euro(order.delivery_fee_cents)),
            LineStyle::NORMAL,
        ));
    }
    if order.voucher_discount_cents > 0 {
        let vlabel = if order.voucher_code.is_empty() {
            "Gutschein".to_string()
        } else {
            format!("Gutschein {}", order.voucher_code)
        };
        let value = format!("-{}", format_euro(order.voucher_discount_cents));
        out.push(ReceiptLine::text(
            pad_line(&vlabel, &value),
            LineStyle::NORMAL,
        ));
    }
    out.push(ReceiptLine::text(
        pad_line("Gesamtbetrag", &format_euro(order.total_cents)),
        LineStyle::NORMAL.bold(),
    ));

    // ===== Bezahlt von block =====
    out.push(ReceiptLine::Feed { n: 1 });
    match order.payment {
        PaymentStatus::Prepaid => {
            out.push(ReceiptLine::text(
                ReceiptLabels::or(&l.paid_by, "Bezahlt von:"),
                LineStyle::NORMAL.bold(),
            ));
            out.push(ReceiptLine::text(
                pad_line("Karte / Online", &format_euro(order.total_cents)),
                LineStyle::NORMAL,
            ));
        }
        PaymentStatus::CollectOnDelivery { amount_cents } => {
            let header = match order.channel {
                OrderChannel::Delivery { .. } => {
                    ReceiptLabels::or(&l.collect_delivery, "Bei Lieferung kassieren:")
                }
                OrderChannel::Pickup => {
                    ReceiptLabels::or(&l.collect_pickup, "Bei Abholung kassieren:")
                }
                OrderChannel::DineIn { .. } => {
                    ReceiptLabels::or(&l.collect_dinein, "Am Tisch kassieren:")
                }
            };
            out.push(ReceiptLine::text(header, LineStyle::NORMAL.bold()));
            out.push(ReceiptLine::text(
                pad_line(
                    ReceiptLabels::or(&l.cash, "Bar"),
                    &format_euro(amount_cents),
                ),
                LineStyle::NORMAL,
            ));
        }
    }

    // ===== Allergen reminder ===== (admin-toggleable + overridable text)
    if config.show_allergens {
        out.push(ReceiptLine::Feed { n: 1 });
        out.push(ReceiptLine::Rule);
        let notice = ReceiptLabels::or(
            &l.allergen_notice,
            "WICHTIG: Für Angaben zu\nLebensmittel-Allergenen\nrufen Sie das Restaurant an oder\nprüfen Sie die Speisekarte.",
        );
        // First two lines bold by convention; render each line of the
        // (possibly overridden) block, wrapped to width.
        for (i, raw) in notice.lines().enumerate() {
            for wrapped in wrap_left(raw, RECEIPT_WIDTH) {
                let style = if i < 2 {
                    LineStyle::NORMAL.bold()
                } else {
                    LineStyle::NORMAL
                };
                out.push(ReceiptLine::text(wrapped, style));
            }
        }
        out.push(ReceiptLine::Rule);
    }

    // ===== Optional note from the customer =====
    if let Some(note) = &order.note {
        out.push(ReceiptLine::Feed { n: 1 });
        out.push(ReceiptLine::text("Anmerkung:", LineStyle::NORMAL.bold()));
        out.push(ReceiptLine::text(note, LineStyle::NORMAL));
    }

    // ===== Big BEZAHLT banner (prepaid only) =====
    // Default prints "BESTELLUNG IST" / "BEZAHLT" on two lines; an override
    // is split into lines so multi-word custom text still fits double-width.
    if matches!(order.payment, PaymentStatus::Prepaid) {
        out.push(ReceiptLine::Feed { n: 1 });
        let confirm = ReceiptLabels::or(&l.paid_confirm, "BESTELLUNG IST\nBEZAHLT");
        for line in confirm.lines() {
            out.push(ReceiptLine::text(line, center.bold().size(2, 2)));
        }
    }

    // ===== Customer details =====
    out.push(ReceiptLine::Feed { n: 1 });
    out.push(ReceiptLine::text(
        "Details zum/zur Kund:in:",
        LineStyle::NORMAL.bold(),
    ));
    out.push(ReceiptLine::Feed { n: 1 });
    out.push(ReceiptLine::text(&order.customer.name, LineStyle::NORMAL));
    if let Some(phone) = &order.customer.phone {
        out.push(ReceiptLine::text(
            format!("Tel: {phone}"),
            LineStyle::NORMAL,
        ));
    }
    if let OrderChannel::Delivery { address } = &order.channel {
        out.push(ReceiptLine::Feed { n: 1 });
        out.push(ReceiptLine::text(&address.street, LineStyle::NORMAL));
        out.push(ReceiptLine::text(
            format!("{} {}", address.postal, address.city),
            LineStyle::NORMAL,
        ));
        if let Some(bell) = &address.bell {
            out.push(ReceiptLine::text(
                format!("Klingel: {bell}"),
                LineStyle::NORMAL,
            ));
        }
    }

    // ===== Bestellung aufgegeben / angenommen =====
    out.push(ReceiptLine::Feed { n: 1 });
    out.push(ReceiptLine::Rule);
    let created_line = if !order.created_at_label.is_empty() {
        order.created_at_label.clone()
    } else {
        fmt_local_fallback(order.created_at_unix)
    };
    if !created_line.is_empty() {
        out.push(ReceiptLine::text(
            pad_line("Bestellung aufgegeben", &created_line),
            LineStyle::NORMAL,
        ));
    }
    let accepted_line = order
        .accepted_at_label
        .clone()
        .or_else(|| order.accepted_at_unix.map(fmt_local_fallback));
    if let Some(line) = accepted_line {
        if !line.is_empty() {
            out.push(ReceiptLine::text(
                pad_line("Bestellung angenommen", &line),
                LineStyle::NORMAL,
            ));
        }
    }

    // ===== Payment status directly above the QR code =====
    out.push(ReceiptLine::Feed { n: 1 });
    match order.payment {
        PaymentStatus::Prepaid => {
            out.push(ReceiptLine::text(
                ReceiptLabels::or(&l.paid_badge, "BEZAHLT"),
                center.bold().size(2, 2),
            ));
        }
        PaymentStatus::CollectOnDelivery { .. } => {
            out.push(ReceiptLine::text(
                ReceiptLabels::or(&l.unpaid_badge, "NICHT BEZAHLT"),
                center.bold().size(2, 2),
            ));
            out.push(ReceiptLine::text(
                ReceiptLabels::or(&l.cash, "Bar"),
                center.bold(),
            ));
        }
    }

    // ===== QR code → order detail URL =====
    if let Some(url) = order.qr_url.as_deref() {
        if !url.is_empty() {
            out.push(ReceiptLine::Feed { n: 1 });
            out.push(ReceiptLine::Qr {
                url: url.to_string(),
            });
            out.push(ReceiptLine::text(
                ReceiptLabels::or(&l.qr_caption, "Zum Bestelldetail scannen"),
                center,
            ));
        }
    }

    // ===== Disclaimer =====
    out.push(ReceiptLine::Feed { n: 1 });
    out.push(ReceiptLine::text(
        ReceiptLabels::or(&l.disclaimer, "Das ist keine Rechnung"),
        center,
    ));

    // ===== Admin footer line ===== (free text on every receipt)
    if !config.footer_text.is_empty() {
        out.push(ReceiptLine::Feed { n: 1 });
        // Wrap to width so a long footer doesn't overrun.
        for line in wrap_left(&config.footer_text, RECEIPT_WIDTH) {
            out.push(ReceiptLine::text(line, center));
        }
    }

    // ===== TEST MODE banner (bottom) =====
    if order.is_test_mode {
        out.push(ReceiptLine::Feed { n: 1 });
        push_test_banner(&mut out, l);
    }

    out
}

/// Compose a layout into a self-contained [`ReceiptProgram`]: resolve
/// each [`RasterSlot`] to inline 1-bit bytes via `provider`, copy every
/// other line through, and append the final feed+cut. This is the
/// server's "render the theme" step; the result is what the Pi executes.
///
/// `theme` selects the artwork; a slot the provider can't resolve is
/// dropped (the text banners still carry the channel/payment info).
pub fn compose(
    order: &OrderForKitchen,
    is_reprint: bool,
    config: &ReceiptConfig,
    provider: &dyn RasterProvider,
) -> ReceiptProgram {
    let layout = build_layout(order, is_reprint, config);
    let theme = order.printer_theme.as_str();
    let mut lines: Vec<ReceiptLine> = Vec::with_capacity(layout.len() + 1);

    for line in layout {
        match line {
            LayoutLine::Text { text, style } => lines.push(ReceiptLine::Text { text, style }),
            LayoutLine::Feed { n } => lines.push(ReceiptLine::Feed { n }),
            LayoutLine::Rule => lines.push(ReceiptLine::Rule),
            LayoutLine::Qr { url } => lines.push(ReceiptLine::Qr { url }),
            LayoutLine::Raster(slot) => {
                if let Some(composed) = provider.resolve(theme, slot) {
                    let hash = raster_hash(&composed.svg, composed.target_width);
                    lines.push(ReceiptLine::RasterSvg {
                        hash,
                        target_width: composed.target_width,
                        svg: composed.svg,
                    });
                }
                // Unresolved → drop silently.
            }
        }
    }

    // Final feed + cut (was `feeds(4).cut()` in the old printer render).
    lines.push(ReceiptLine::Cut { feed: 4 });

    ReceiptProgram {
        display_number: order.display_number,
        is_reprint,
        lines,
    }
}

/// A source SVG with its intrinsic size, for header composition.
#[derive(Debug, Clone)]
pub struct SvgLogo<'a> {
    pub svg: &'a str,
    pub width: f64,
    pub height: f64,
}

/// Compose the combined header band SVG: channel icon (left) + brand mark
/// (right), each scaled to a common height and joined with a gap, laid out
/// in one SVG document. The output is a single SVG ready to ship; the
/// caller pairs it with a `target_width` (dots) for the Pi to rasterize
/// at. Pure string assembly — no rasterization happens here.
///
/// `band_height_px` is the SVG-space height each logo is scaled to;
/// `gap_px` the white space between them.
pub fn compose_header_svg(
    channel: &SvgLogo,
    brand: &SvgLogo,
    band_height_px: f64,
    gap_px: f64,
) -> String {
    let channel_svg = channel.svg;
    let brand_svg = brand.svg;
    // Scale each logo to the common band height, preserving aspect.
    let cscale = if channel.height > 0.0 {
        band_height_px / channel.height
    } else {
        1.0
    };
    let bscale = if brand.height > 0.0 {
        band_height_px / brand.height
    } else {
        1.0
    };
    let cw = channel.width * cscale;
    let bw = brand.width * bscale;
    let total_w = cw + gap_px + bw;

    // Inner SVGs are embedded as <svg> with explicit x/width/height so the
    // outer viewBox positions them side-by-side. We strip nothing — resvg
    // handles nested <svg> with viewBox correctly.
    // Declare BOTH the SVG and xlink namespaces on the composed root: the
    // source SVGs may use `xlink:` (e.g. potrace / brand-mark output) and
    // we strip their own `<svg>` (where they declared it), so the
    // declaration must live here or the parser rejects the prefix.
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{total_w}" height="{band_height_px}" viewBox="0 0 {total_w} {band_height_px}">
<rect x="0" y="0" width="{total_w}" height="{band_height_px}" fill="white"/>
<g transform="translate(0,0) scale({cscale})">{channel_svg}</g>
<g transform="translate({cx},0) scale({bscale})">{brand_svg}</g>
</svg>"#,
        total_w = total_w,
        band_height_px = band_height_px,
        cscale = cscale,
        bscale = bscale,
        cx = cw + gap_px,
        channel_svg = inner_svg_body(channel_svg),
        brand_svg = inner_svg_body(brand_svg),
    )
}

/// Strip the XML prolog / DOCTYPE and the outer `<svg …>`/`</svg>` wrapper
/// from a source SVG, returning just the inner drawing markup so it can be
/// embedded inside a `<g transform=…>` in the composed document. Keeps the
/// child geometry; the outer transform handles placement + scale.
fn inner_svg_body(svg: &str) -> String {
    // Drop everything up to and including the first '>' that closes the
    // opening <svg ...> tag, and the trailing </svg>.
    let body = match svg.find("<svg") {
        Some(start) => {
            let after_open = svg[start..]
                .find('>')
                .map(|i| start + i + 1)
                .unwrap_or(start);
            let end = svg.rfind("</svg>").unwrap_or(svg.len());
            &svg[after_open..end]
        }
        None => svg,
    };
    body.trim().to_string()
}

/// Themes that carry header rasters.
pub fn theme_has_assets(theme: &str) -> bool {
    matches!(theme, "rusterando-rando")
}

fn push_test_banner(out: &mut Vec<LayoutLine>, labels: &ReceiptLabels) {
    let c = LineStyle::centered();
    out.push(LayoutLine::text(
        ReceiptLabels::or(&labels.test_banner, "*** TEST ***"),
        c.bold().size(2, 2),
    ));
    out.push(LayoutLine::text("Keine echte Bestellung", c.bold()));
    out.push(LayoutLine::text("Stripe-Sandbox-Modus", c.bold()));
    out.push(LayoutLine::Feed { n: 1 });
}

/// Format epoch seconds as a plain "DD. Mon HH:MM" string WITHOUT pulling
/// in chrono — the shared crate stays dep-light. Used only as the
/// last-ditch fallback when the server didn't send a pre-formatted label
/// (which is the normal path). Produces a UTC wall-clock string; the
/// caller's server-side label is always preferred.
fn fmt_local_fallback(_unix: i64) -> String {
    // The server always populates `created_at_label` / `accepted_at_label`
    // in the shop's timezone, so this branch is effectively dead in
    // production. Emitting an em-dash here (rather than a misleading UTC
    // time) keeps the shared crate chrono-free without risking a wrong
    // local time on the ticket.
    "—".to_string()
}

// ===== column-fitting helpers (shared so preview == print) =====

fn format_euro(cents: u32) -> String {
    format!("{},{:02} EUR", cents / 100, cents % 100)
}

/// Left/right justified within `RECEIPT_WIDTH`, single space minimum gap.
fn pad_line(left: &str, right: &str) -> String {
    let l = left.chars().count();
    let r = right.chars().count();
    if l + r + 1 >= RECEIPT_WIDTH {
        format!("{left} {right}")
    } else {
        let pad = RECEIPT_WIDTH - l - r;
        format!("{left}{:pad$}{right}", "", pad = pad)
    }
}

/// Emit `left`/`right` on one line if they fit, else wrap `left` and put
/// `right` on the last wrapped line — Lieferando's column behaviour.
fn push_wrapped_pair(out: &mut Vec<LayoutLine>, left: &str, right: &str) {
    let right_w = right.chars().count();
    let max_left = RECEIPT_WIDTH.saturating_sub(right_w + 1);
    let lines = wrap_left(left, max_left);
    if lines.is_empty() {
        out.push(LayoutLine::text(pad_line("", right), LineStyle::NORMAL));
        return;
    }
    for (i, line) in lines.iter().enumerate() {
        if i + 1 == lines.len() {
            out.push(LayoutLine::text(pad_line(line, right), LineStyle::NORMAL));
        } else {
            out.push(LayoutLine::text(line, LineStyle::NORMAL));
        }
    }
}

/// Greedy word-wrap to `max` columns; words longer than `max` hard-break.
fn wrap_left(text: &str, max: usize) -> Vec<String> {
    if max == 0 {
        return vec![text.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let need = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()
        };
        if need <= max {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        } else {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            if word.chars().count() > max {
                let mut buf = String::new();
                for ch in word.chars() {
                    if buf.chars().count() + 1 > max {
                        out.push(std::mem::take(&mut buf));
                    }
                    buf.push(ch);
                }
                current = buf;
            } else {
                current = word.to_string();
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Customer, DeliveryAddress, LineItem};

    fn sample(channel: OrderChannel, payment: PaymentStatus) -> OrderForKitchen {
        OrderForKitchen {
            order_id: crate::OrderId(1),
            display_number: 1,
            display_label: "DP-1105-0001".into(),
            created_at_unix: 0,
            created_at_label: "8. Juni, 17:40".into(),
            channel,
            customer: Customer {
                name: "Mustermann".into(),
                phone: Some("0123".into()),
            },
            items: vec![LineItem {
                qty: 1,
                name: "Pizza Margherita".into(),
                modifications: vec!["extra Käse".into()],
                unit_price_cents: 850,
                category: "Pizza".into(),
                modification_prices_cents: vec![100],
            }],
            subtotal_cents: 950,
            delivery_fee_cents: 150,
            total_cents: 1100,
            payment,
            note: None,
            shop_name: "Davids Pizzeria".into(),
            accepted_at_unix: None,
            accepted_at_label: None,
            qr_url: Some("https://example.com/orders/x".into()),
            is_test_mode: false,
            voucher_code: String::new(),
            voucher_discount_cents: 0,
            pickup_time_label: Some("18:30".into()),
            printer_theme: "rusterando-rando".into(),
        }
    }

    #[test]
    fn delivery_puts_scheduled_above_eingang_and_has_rasters() {
        let o = sample(
            OrderChannel::Delivery {
                address: DeliveryAddress {
                    street: "Hauptstr. 1".into(),
                    postal: "12345".into(),
                    city: "Stadt".into(),
                    bell: None,
                },
            },
            PaymentStatus::CollectOnDelivery { amount_cents: 1100 },
        );
        let lines = build_layout(&o, false, &ReceiptConfig::default());

        // rando theme → one combined channel+brand raster row (delivery,
        // and this sample is CollectOnDelivery → unpaid brand mark).
        assert!(lines.iter().any(|l| matches!(
            l,
            LayoutLine::Raster(RasterSlot::ChannelAndBrand {
                is_delivery: true,
                is_paid: false
            })
        )));

        // Scheduled "(Lieferung …)" comes before "Eingang:" for delivery.
        let texts: Vec<&str> = lines
            .iter()
            .filter_map(|l| match l {
                LayoutLine::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let sched_idx = texts.iter().position(|t| t.contains("Lieferung 18:30"));
        let eingang_idx = texts.iter().position(|t| t.contains("Eingang:"));
        assert!(sched_idx.is_some() && eingang_idx.is_some());
        assert!(sched_idx.unwrap() < eingang_idx.unwrap());
    }

    #[test]
    fn every_text_line_fits_width_at_single_size() {
        let o = sample(OrderChannel::Pickup, PaymentStatus::Prepaid);
        for l in build_layout(&o, false, &ReceiptConfig::default()) {
            if let LayoutLine::Text { text, style } = l {
                if style.width == 1 {
                    assert!(
                        text.chars().count() <= RECEIPT_WIDTH,
                        "line over width: {text:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn brand_mark_variant_follows_payment() {
        // Prepaid → paid brand mark; cash → unpaid brand mark.
        let paid = sample(OrderChannel::Pickup, PaymentStatus::Prepaid);
        assert!(build_layout(&paid, false, &ReceiptConfig::default())
            .iter()
            .any(|l| matches!(
                l,
                LayoutLine::Raster(RasterSlot::ChannelAndBrand { is_paid: true, .. })
            )));

        let cash = sample(
            OrderChannel::Pickup,
            PaymentStatus::CollectOnDelivery { amount_cents: 1100 },
        );
        assert!(build_layout(&cash, false, &ReceiptConfig::default())
            .iter()
            .any(|l| matches!(
                l,
                LayoutLine::Raster(RasterSlot::ChannelAndBrand { is_paid: false, .. })
            )));
    }

    #[test]
    fn default_theme_has_no_rasters() {
        let mut o = sample(OrderChannel::Pickup, PaymentStatus::Prepaid);
        o.printer_theme = "rusterando-default".into();
        let lines = build_layout(&o, false, &ReceiptConfig::default());
        assert!(!lines.iter().any(|l| matches!(l, LayoutLine::Raster(_))));
    }

    // A stub provider returning a tiny SVG for the rando theme.
    struct StubProvider;
    impl RasterProvider for StubProvider {
        fn resolve(&self, theme: &str, _slot: RasterSlot) -> Option<ComposedSvg> {
            if theme == "rusterando-rando" {
                Some(ComposedSvg {
                    svg: br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="black"/></svg>"#.to_vec(),
                    target_width: 268,
                })
            } else {
                None
            }
        }
    }

    #[test]
    fn compose_emits_rastersvg_with_hash_and_ends_with_cut() {
        let o = sample(OrderChannel::Pickup, PaymentStatus::Prepaid);
        let prog = compose(&o, false, &ReceiptConfig::default(), &StubProvider);

        // The header slot is now a RasterSvg carrying bytes + a hash that
        // matches raster_hash of (svg, target_width).
        let svg_line = prog
            .lines
            .iter()
            .find(|l| matches!(l, ReceiptLine::RasterSvg { .. }))
            .expect("a RasterSvg line");
        if let ReceiptLine::RasterSvg {
            hash,
            target_width,
            svg,
        } = svg_line
        {
            assert!(!svg.is_empty());
            assert_eq!(*hash, raster_hash(svg, *target_width));
        }
        // Last line is the cut.
        assert!(matches!(prog.lines.last(), Some(ReceiptLine::Cut { .. })));
        // It serializes (it's the wire payload).
        let json = serde_json::to_string(&prog).unwrap();
        let back: ReceiptProgram = serde_json::from_str(&json).unwrap();
        assert_eq!(back, prog);
    }

    #[test]
    fn compose_default_theme_drops_unresolved_rasters() {
        let mut o = sample(OrderChannel::Pickup, PaymentStatus::Prepaid);
        o.printer_theme = "rusterando-default".into();
        let prog = compose(&o, false, &ReceiptConfig::default(), &StubProvider);
        assert!(!prog
            .lines
            .iter()
            .any(|l| matches!(l, ReceiptLine::RasterSvg { .. })));
    }

    #[test]
    fn compose_header_svg_lays_out_side_by_side() {
        let c = SvgLogo {
            svg: r#"<svg width="100" height="100"><circle cx="50" cy="50" r="40"/></svg>"#,
            width: 100.0,
            height: 100.0,
        };
        let b = SvgLogo {
            svg: r#"<svg width="200" height="100"><rect width="200" height="100"/></svg>"#,
            width: 200.0,
            height: 100.0,
        };
        let out = compose_header_svg(&c, &b, 120.0, 28.0);
        assert!(out.contains("<svg"));
        assert!(out.contains("translate(")); // brand placed to the right
        assert!(out.contains("circle") && out.contains("rect")); // both bodies embedded
    }
}
