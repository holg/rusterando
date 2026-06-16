use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use chrono::{Local, TimeZone};
use escpos::driver::FileDriver;
use escpos::printer::Printer as EscposPrinter;
use escpos::printer_options::PrinterOptions;
use escpos::utils::{
    JustifyMode, PageCode, Protocol, QRCodeCorrectionLevel, QRCodeModel, QRCodeOption,
};
use kitchen_protocol::{OrderChannel, OrderForKitchen, PaymentStatus};

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

/// Thin wrapper around an ESC/POS device file. Opens the device per print
/// rather than holding the FD open: the kernel `usblp` driver reclaims the
/// device cleanly on detach/reattach, so re-opening per receipt makes us
/// resilient to the printer being powered off and back on mid-shift.
pub struct Printer {
    path: PathBuf,
    /// Serialize prints so we never interleave two receipts.
    write_lock: Mutex<()>,
}

impl Printer {
    pub fn open(path: &Path) -> Result<Self> {
        // Probe by opening and immediately dropping. If the device isn't
        // there (printer off, cable unplugged), fail fast at startup.
        let _probe =
            FileDriver::open(path).with_context(|| format!("probe open {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            write_lock: Mutex::new(()),
        })
    }

    pub fn print_order(&self, order: &OrderForKitchen, is_reprint: bool) -> Result<()> {
        let _guard = self.write_lock.lock().unwrap();
        let driver = FileDriver::open(&self.path)
            .with_context(|| format!("open {}", self.path.display()))?;

        // Configure the printer with our target code page up front: every
        // .writeln below will then go through the table-based UTF-8 →
        // ISO-8859-15 transcoder in escpos's `Protocol::text`. Without
        // this, the crate sends raw UTF-8 bytes and the firmware paints
        // mojibake.
        let options = PrinterOptions::new(Some(RECEIPT_PAGE_CODE), None, RECEIPT_WIDTH as u8);
        let mut p = EscposPrinter::new(driver, Protocol::default(), Some(options));

        render(&mut p, order, is_reprint).context("render order")?;
        p.print().context("flush ESC/POS commands")?;
        Ok(())
    }
}

fn render(
    p: &mut EscposPrinter<FileDriver>,
    order: &OrderForKitchen,
    is_reprint: bool,
) -> Result<()> {
    p.init()?;

    // Emit ESC `t n` so the printer's firmware switches its active
    // table to match the bytes we'll send. The crate's
    // PrinterOptions::page_code (set in print_order) drives both this
    // selector AND the per-string UTF-8 transcoder — they have to
    // match or the receipt mojibakes.
    p.page_code(RECEIPT_PAGE_CODE)?;

    // ===== TEST MODE banner (top) =====
    // Big bold marker at the very top of the receipt for Stripe-
    // sandbox orders. Combined with the bottom banner this should
    // make it impossible for the kitchen to fulfill a test order
    // by accident. No-op on live orders (production-clean paper).
    if order.is_test_mode {
        render_test_banner(p)?;
    }

    // ===== Reprint banner =====
    if is_reprint {
        p.justify(JustifyMode::CENTER)?
            .bold(true)?
            .writeln("*** REPRINT ***")?
            .bold(false)?
            .feed()?;
    }

    // ===== Brand line =====
    // Small centered shop name at the very top. Empty when the server
    // didn't fill BrandingHandle.display_name.
    if !order.shop_name.is_empty() {
        p.justify(JustifyMode::CENTER)?
            .bold(true)?
            .writeln(&order.shop_name)?
            .bold(false)?;
    }

    // ===== Channel banner (LIEFERUNG / ABHOLUNG / TISCH) =====
    let channel_label = match &order.channel {
        OrderChannel::Delivery { .. } => "LIEFERUNG".to_string(),
        OrderChannel::Pickup => "ABHOLUNG".to_string(),
        OrderChannel::DineIn { table } => format!("TISCH {table}"),
    };
    p.justify(JustifyMode::CENTER)?
        .bold(true)?
        .size(2, 2)?
        .writeln(&channel_label)?
        .reset_size()?
        .bold(false)?;

    // ===== Fällig / Eingang timestamp =====
    // Prefer the server-formatted label (shop's local TZ baked in); fall
    // back to formatting the epoch with Local only for older servers that
    // didn't send a label (then the Pi's TZ matters, but that's the legacy
    // path; current servers always populate created_at_label).
    let created_ts = if !order.created_at_label.is_empty() {
        format!("Eingang: {}", order.created_at_label)
    } else {
        Local
            .timestamp_opt(order.created_at_unix, 0)
            .single()
            .map(|t| t.format("Eingang: %d. %B, %H:%M").to_string())
            .unwrap_or_else(|| "Eingang: —".into())
    };
    // Big + bold: this is when the order came in — a field the kitchen
    // reads at a glance, so it gets emphasis like the channel + label.
    // size(1,2) doubles the height only (keeps it on one ~42-col line;
    // a full 2×2 would overrun the width with the date + time).
    p.justify(JustifyMode::CENTER)?
        .bold(true)?
        .size(1, 2)?
        .writeln(&created_ts)?
        .reset_size()?
        .bold(false)?;

    // Scheduled fulfillment time, in brackets right under the Eingang
    // line. Channel decides the word: "Abholung" for pickup/dine-in,
    // "Lieferung" for delivery. A far-out pre-order (2h, or another day)
    // is exactly when the kitchen needs this. ASAP orders (None) print
    // nothing extra — the Eingang line already implies "now".
    if let Some(when) = order.pickup_time_label.as_deref().filter(|s| !s.is_empty()) {
        let verb = match order.channel {
            OrderChannel::Delivery { .. } => "Lieferung",
            OrderChannel::Pickup => "Abholung",
            OrderChannel::DineIn { .. } => "Servieren",
        };
        p.justify(JustifyMode::CENTER)?
            .bold(true)?
            .size(1, 2)?
            .writeln(&format!("({verb} {when})"))?
            .reset_size()?
            .bold(false)?;
    }

    p.justify(JustifyMode::CENTER)?.feed()?;

    // ===== Big bold display_label =====
    // Full server-assigned label ("DP-1105-0001") so the staff can
    // match the receipt against /admin/orders and the customer's
    // email at a glance. Older clients (or test fixtures) may send
    // an empty display_label; fall back to the legacy numeric one.
    let label = if order.display_label.is_empty() {
        format!("# {}", order.display_number)
    } else {
        order.display_label.clone()
    };
    p.justify(JustifyMode::CENTER)?
        .bold(true)?
        .size(2, 3)?
        .writeln(&label)?
        .reset_size()?
        .bold(false)?;

    p.justify(JustifyMode::LEFT)?
        .writeln(&"-".repeat(RECEIPT_WIDTH))?;

    // ===== Line items grouped by category =====
    //
    // Lieferando-style: each category gets its own bold heading; line
    // items underneath show `qty x #menu_no Name` left, price right;
    // extras/modifications each on their own line with `+ ` prefix and
    // a right-aligned price (which may be 0,00 EUR for "first N free").
    //
    // We render headings only when the category changes from the last
    // item's category — keeps the layout tight when several items
    // share one category.
    let mut last_category: Option<&str> = None;
    for item in &order.items {
        let cat = item.category.trim();
        let cat_changed = match last_category {
            Some(prev) => prev != cat,
            None => true,
        };
        if cat_changed && !cat.is_empty() {
            p.feed()?.bold(true)?.writeln(cat)?.bold(false)?;
        }
        last_category = Some(cat);

        let line_total = item.unit_price_cents * item.qty;
        let qty_name = format!("{} x {}", item.qty, item.name);
        // If the name is too long to fit on one line, wrap it under
        // the qty + indent. Looks just like Lieferando's right column.
        write_wrapped_pair(p, &qty_name, &format_euro(line_total))?;

        // Modifications: zip names with prices, render each with a
        // small indent and the `+ ` prefix.
        for (i, m) in item.modifications.iter().enumerate() {
            let pretty = format!("+ {m}");
            let price = item.modification_prices_cents.get(i).copied().unwrap_or(0);
            if price > 0 {
                write_wrapped_pair(p, &format!("   {pretty}"), &format_euro(price))?;
            } else {
                p.writeln(&format!("   {pretty}"))?;
            }
        }
    }

    p.writeln(&"-".repeat(RECEIPT_WIDTH))?;

    // ===== Totals =====
    p.writeln(&pad_line(
        "Zwischensumme",
        &format_euro(order.subtotal_cents),
    ))?;
    if order.delivery_fee_cents > 0 {
        p.writeln(&pad_line(
            "Liefergebühr",
            &format_euro(order.delivery_fee_cents),
        ))?;
    }
    if order.voucher_discount_cents > 0 {
        let label = if order.voucher_code.is_empty() {
            "Gutschein".to_string()
        } else {
            format!("Gutschein {}", order.voucher_code)
        };
        let value = format!("-{}", format_euro(order.voucher_discount_cents));
        p.writeln(&pad_line(&label, &value))?;
    }
    p.bold(true)?
        .writeln(&pad_line("Gesamtbetrag", &format_euro(order.total_cents)))?
        .bold(false)?;

    // ===== Bezahlt von block =====
    p.feed()?;
    match order.payment {
        PaymentStatus::Prepaid => {
            p.bold(true)?.writeln("Bezahlt von:")?.bold(false)?;
            p.writeln(&pad_line("Karte / Online", &format_euro(order.total_cents)))?;
        }
        PaymentStatus::CollectOnDelivery { amount_cents } => {
            // Bei Abholung wird an der Theke kassiert, nicht beim Fahrer —
            // die Kopfzeile muss zum Kanal passen, sonst steht auf dem
            // Abhol-Bon fälschlich "Bei Lieferung kassieren".
            let header = match order.channel {
                OrderChannel::Delivery { .. } => "Bei Lieferung kassieren:",
                OrderChannel::Pickup => "Bei Abholung kassieren:",
                OrderChannel::DineIn { .. } => "Am Tisch kassieren:",
            };
            p.bold(true)?.writeln(header)?.bold(false)?;
            p.writeln(&pad_line("Bar", &format_euro(amount_cents)))?;
        }
    }

    // ===== Allergen reminder =====
    p.feed()?
        .writeln(&"-".repeat(RECEIPT_WIDTH))?
        .bold(true)?
        .writeln("WICHTIG: Für Angaben zu")?
        .writeln("Lebensmittel-Allergenen")?
        .bold(false)?
        .writeln("rufen Sie das Restaurant an oder")?
        .writeln("prüfen Sie die Speisekarte.")?
        .writeln(&"-".repeat(RECEIPT_WIDTH))?;

    // ===== Optional note from the customer =====
    if let Some(note) = &order.note {
        p.feed()?
            .bold(true)?
            .writeln("Anmerkung:")?
            .bold(false)?
            .writeln(note)?;
    }

    // ===== Big BEZAHLT banner (prepaid only) =====
    if matches!(order.payment, PaymentStatus::Prepaid) {
        p.feed()?
            .justify(JustifyMode::CENTER)?
            .bold(true)?
            .size(2, 2)?
            .writeln("BESTELLUNG IST")?
            .writeln("BEZAHLT")?
            .reset_size()?
            .bold(false)?
            .justify(JustifyMode::LEFT)?;
    }

    // ===== Customer details =====
    p.feed()?
        .bold(true)?
        .writeln("Details zum/zur Kund:in:")?
        .bold(false)?
        .feed()?
        .writeln(&order.customer.name)?;
    if let Some(phone) = &order.customer.phone {
        p.writeln(&format!("Tel: {phone}"))?;
    }
    if let OrderChannel::Delivery { address } = &order.channel {
        p.feed()?
            .writeln(&address.street)?
            .writeln(&format!("{} {}", address.postal, address.city))?;
        if let Some(bell) = &address.bell {
            p.writeln(&format!("Klingel: {bell}"))?;
        }
    }

    // ===== Bestellung aufgegeben / angenommen =====
    // Prefer the server-formatted labels (shop's local TZ); fall back to
    // formatting the epoch with Local for older servers.
    p.feed()?.writeln(&"-".repeat(RECEIPT_WIDTH))?;
    let created_line = if !order.created_at_label.is_empty() {
        order.created_at_label.clone()
    } else {
        Local
            .timestamp_opt(order.created_at_unix, 0)
            .single()
            .map(|t| t.format("%H:%M %d. %b").to_string())
            .unwrap_or_default()
    };
    if !created_line.is_empty() {
        p.writeln(&pad_line("Bestellung aufgegeben", &created_line))?;
    }
    let accepted_line = order.accepted_at_label.clone().or_else(|| {
        order
            .accepted_at_unix
            .and_then(|u| Local.timestamp_opt(u, 0).single())
            .map(|t| t.format("%H:%M %d. %b").to_string())
    });
    if let Some(line) = accepted_line {
        if !line.is_empty() {
            p.writeln(&pad_line("Bestellung angenommen", &line))?;
        }
    }

    // ===== Payment status directly above the QR code =====
    // A prominent, unambiguous marker right where staff look when scanning:
    // "BEZAHLT" if the order is already paid (card/online or fully covered by
    // a voucher → Prepaid), or "NICHT BEZAHLT / Bar" when cash is still owed
    // (CollectOnDelivery, regardless of channel). The headline is double-size
    // (centered); the cash qualifier sits on a normal-size line below so the
    // text never overruns the ~21 double-width columns of an 80-mm receipt.
    p.feed()?.justify(JustifyMode::CENTER)?.bold(true)?;
    match order.payment {
        PaymentStatus::Prepaid => {
            p.size(2, 2)?.writeln("BEZAHLT")?.reset_size()?;
        }
        PaymentStatus::CollectOnDelivery { .. } => {
            p.size(2, 2)?.writeln("NICHT BEZAHLT")?.reset_size()?;
            p.writeln("Bar")?;
        }
    }
    p.bold(false)?.justify(JustifyMode::LEFT)?;

    // ===== QR code → order detail URL =====
    if let Some(url) = order.qr_url.as_deref() {
        if !url.is_empty() {
            // Der Default (.qrcode) druckt mit Modulgröße 4 — auf 80-mm-
            // Thermopapier zu klein, selbst ein iPhone 15 Pro scannt das
            // nicht zuverlässig. Modulgröße 8 verdoppelt die Punktgröße
            // (Code ~22 mm breit), Model2 ist der von Telefonen erwartete
            // moderne Standard, Korrekturstufe H verträgt Druckschlieren.
            p.feed()?
                .justify(JustifyMode::CENTER)?
                .qrcode_option(
                    url,
                    QRCodeOption::new(QRCodeModel::Model2, 8, QRCodeCorrectionLevel::H),
                )?
                .writeln("Zum Bestelldetail scannen")?
                .justify(JustifyMode::LEFT)?;
        }
    }

    // ===== Disclaimer =====
    p.feed()?
        .justify(JustifyMode::CENTER)?
        .writeln("Das ist keine Rechnung")?
        .justify(JustifyMode::LEFT)?;

    // ===== TEST MODE banner (bottom) =====
    // Mirror of the top banner. Double-sided marking means even if
    // the receipt gets torn or partially printed, the kitchen still
    // sees the warning.
    if order.is_test_mode {
        p.feed()?;
        render_test_banner(p)?;
    }

    // ===== Feed + cut =====
    p.feeds(4)?.cut()?;

    Ok(())
}

/// Render a centered, large, bold "*** TEST ***" banner. Used at
/// both top and bottom of sandbox-mode receipts. Kept as a free
/// function so adjusting the banner shape (e.g. inverse mode later)
/// only touches one place.
fn render_test_banner(p: &mut EscposPrinter<FileDriver>) -> Result<()> {
    p.justify(JustifyMode::CENTER)?
        .bold(true)?
        .size(2, 2)?
        .writeln("*** TEST ***")?
        .reset_size()?
        .writeln("Keine echte Bestellung")?
        .writeln("Stripe-Sandbox-Modus")?
        .bold(false)?
        .justify(JustifyMode::LEFT)?
        .feed()?;
    Ok(())
}

/// Print `left` / `right` on one line if they fit, otherwise wrap
/// `left` onto multiple lines and right-align `right` on the LAST
/// wrapped line. Matches Lieferando's "item name wraps under itself,
/// price stays on the right of the bottom row" behaviour.
fn write_wrapped_pair(p: &mut EscposPrinter<FileDriver>, left: &str, right: &str) -> Result<()> {
    let right_w = right.chars().count();
    // Leave one space gap minimum between left and right.
    let max_left = RECEIPT_WIDTH.saturating_sub(right_w + 1);

    let lines = wrap_left(left, max_left);
    if lines.is_empty() {
        p.writeln(&pad_line("", right))?;
        return Ok(());
    }
    for (i, line) in lines.iter().enumerate() {
        if i + 1 == lines.len() {
            p.writeln(&pad_line(line, right))?;
        } else {
            p.writeln(line)?;
        }
    }
    Ok(())
}

/// Greedy word-wrap to `max` chars wide. Words longer than `max` get
/// hard-broken on character boundaries — degenerate case for very
/// long URLs / IDs that shouldn't appear in line-item names anyway.
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
            // word itself longer than max? hard-break.
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

const RECEIPT_WIDTH: usize = 42;

fn format_euro(cents: u32) -> String {
    format!("{},{:02} EUR", cents / 100, cents % 100)
}

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
