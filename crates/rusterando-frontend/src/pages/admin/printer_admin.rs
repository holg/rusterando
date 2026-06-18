//! /admin/printer — a faithful preview of the kitchen receipt.
//!
//! The receipt LAYOUT is defined once in
//! `kitchen_protocol::receipt::build_receipt` and shared with the Pi
//! printer (`rusterando-printer`), which emits the same line list as
//! ESC/POS. Here we render that identical list as a 42-column monospace
//! "paper" so the admin sees exactly what the kitchen ticket will say —
//! banners, channel logo, line items, totals, payment marker, QR — not a
//! hand-built mock that drifts from the real output.
//!
//! Rasters (`RasterSlot`) resolve to the `.1bit.png` twins of the
//! embedded 1-bit BMPs the Pi prints, so even the logos are pixel-exact.
//!
//! The sample order is built in WASM (`kitchen-protocol` compiles to
//! wasm); channel + payment + theme are local toggles so the admin can
//! eyeball each receipt variant without placing a real order.

use leptos::prelude::*;

use kitchen_protocol::receipt::{
    build_layout, LayoutLine, LineStyle, RasterSlot, ReceiptConfig, RECEIPT_WIDTH,
};
use kitchen_protocol::{
    Customer, DeliveryAddress, LineItem, OrderChannel, OrderForKitchen, OrderId, PaymentStatus,
};

use crate::pages::admin::shell::AdminShell;

/// Compose + send a SAMPLE receipt to the kitchen printer so the admin can
/// see the current Bon-Editor config on real paper. Admin-gated; reuses the
/// live compose→Pi pipeline (the receipt is stamped TEST so it's never
/// mistaken for a real order).
#[server(name = PrintSampleReceipt, prefix = "/api", endpoint = "print_sample_receipt")]
pub async fn print_sample_receipt(
    delivery: bool,
    prepaid: bool,
    theme: String,
) -> Result<(), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    #[cfg(feature = "ssr")]
    {
        use sqlx::SqlitePool;
        let db = use_context::<SqlitePool>()
            .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
        let kitchen = use_context::<crate::pages::push::KitchenSinkHandle>()
            .flatten()
            .ok_or_else(|| {
                ServerFnError::new("Kein Drucker konfiguriert (KITCHEN_LISTEN_ADDR fehlt).")
            })?;
        kitchen
            .broadcast_test_print(&db, delivery, prepaid, theme)
            .await
            .map_err(|e| ServerFnError::new(format!("Probedruck fehlgeschlagen: {e}")))?;
    }
    #[cfg(not(feature = "ssr"))]
    let _ = (delivery, prepaid, theme);
    Ok(())
}

/// Which channel variant to preview.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Channel {
    Delivery,
    Pickup,
    DineIn,
}

/// Build a representative sample order for the chosen variant. Mirrors a
/// real mid-size order: two categories, an extra with a price, a voucher,
/// a delivery fee, a scheduled time.
fn sample_order(channel: Channel, prepaid: bool, theme: &str) -> OrderForKitchen {
    let order_channel = match channel {
        Channel::Delivery => OrderChannel::Delivery {
            address: DeliveryAddress {
                street: "Hauptstraße 12".into(),
                postal: "63739".into(),
                city: "Aschaffenburg".into(),
                bell: Some("Mustermann".into()),
            },
        },
        Channel::Pickup => OrderChannel::Pickup,
        Channel::DineIn => OrderChannel::DineIn { table: "7".into() },
    };

    let items = vec![
        LineItem {
            qty: 1,
            name: "Pizza Margherita".into(),
            modifications: vec!["extra Käse".into()],
            unit_price_cents: 850,
            category: "Pizza".into(),
            modification_prices_cents: vec![100],
        },
        LineItem {
            qty: 2,
            name: "Pizza Funghi e Prosciutto".into(),
            modifications: vec![],
            unit_price_cents: 1150,
            category: "Pizza".into(),
            modification_prices_cents: vec![],
        },
        LineItem {
            qty: 1,
            name: "Gemischter Salat".into(),
            modifications: vec!["ohne Zwiebeln".into()],
            unit_price_cents: 650,
            category: "Salate".into(),
            modification_prices_cents: vec![0],
        },
    ];

    // subtotal: 850+100 + 2*1150 + 650 = 3850
    let subtotal_cents = 3850;
    let delivery_fee_cents = if matches!(channel, Channel::Delivery) {
        150
    } else {
        0
    };
    let voucher_discount_cents = 300;
    let total_cents = subtotal_cents + delivery_fee_cents - voucher_discount_cents;

    let payment = if prepaid {
        PaymentStatus::Prepaid
    } else {
        PaymentStatus::CollectOnDelivery {
            amount_cents: total_cents,
        }
    };

    OrderForKitchen {
        order_id: OrderId(1105),
        display_number: 1,
        display_label: "DP-1105-0001".into(),
        created_at_unix: 0,
        created_at_label: "18. Juni, 17:40".into(),
        channel: order_channel,
        customer: Customer {
            name: "Maria Mustermann".into(),
            phone: Some("06021 123456".into()),
        },
        items,
        subtotal_cents,
        delivery_fee_cents,
        total_cents,
        payment,
        note: Some("Bitte klingeln, 2. Stock.".into()),
        shop_name: "Davids Pizzeria".into(),
        accepted_at_unix: None,
        accepted_at_label: Some("18. Juni, 17:42".into()),
        qr_url: Some("https://davidspizzeria.de/orders/DP-1105-0001".into()),
        is_test_mode: false,
        voucher_code: "WILLKOMMEN".into(),
        voucher_discount_cents,
        pickup_time_label: Some("18:30".into()),
        printer_theme: theme.into(),
    }
}

// The preview renders the SOURCE SVGs — the same vector files the server
// composes + ships to the Pi. The browser rasterizes them, so preview ≈
// print. (The real ticket additionally gets Floyd-Steinberg dithered on
// the Pi, so the print is slightly grainier than this crisp vector — an
// acceptable, arguably nicer-for-editing difference.)
const BRAND_PAID_SVG: &str = "/img/rusterando-printer-rando-brand-mark-paid.svg";
const BRAND_UNPAID_SVG: &str = "/img/rusterando-printer-rando-brand-mark-unpaid.svg";
const DELIVERY_SVG: &str = "/img/rusterando-printer-rando-delivery-e-car.svg";
const PICKUP_SVG: &str = "/img/rusterando-printer-rando-pickup-rust.svg";

/// The channel-icon SVG (left half of the combined header row).
fn channel_png(is_delivery: bool) -> &'static str {
    if is_delivery {
        DELIVERY_SVG
    } else {
        PICKUP_SVG
    }
}

/// The brand-mark SVG (right half) — paid vs. unpaid variant.
fn brand_png(is_paid: bool) -> &'static str {
    if is_paid {
        BRAND_PAID_SVG
    } else {
        BRAND_UNPAID_SVG
    }
}

/// CSS style string for a [`LineStyle`] — drives the monospace size/weight
/// so double-height/double-width banners look like the real ticket.
fn style_css(style: LineStyle) -> String {
    let mut s = String::new();
    if style.center {
        s.push_str("text-align:center;");
    }
    if style.bold {
        s.push_str("font-weight:700;");
    }
    // ESC/POS size multipliers → font scale. Width scales letter-spacing
    // feel via transform; we approximate with font-size on both axes,
    // which reads correctly on a monospace column.
    if style.height > 1 || style.width > 1 {
        let scale = style.height.max(style.width) as f32;
        s.push_str(&format!("font-size:{:.2}em;line-height:1.1;", scale));
        if style.width > style.height {
            s.push_str("letter-spacing:.05em;");
        }
    }
    s
}

#[component]
pub fn PrinterAdminPage() -> impl IntoView {
    view! {
        <AdminShell>
            <section class="printer-admin">
                <header class="admin-bar"><h1>"Bon-Editor"</h1></header>
                <p class="hint">
                    "Live-Vorschau des Küchen-Bons — exakt aus derselben Beschreibung "
                    "wie der echte Druck. Kopf-/Fußzeile und Blöcke bearbeiten, speichern, "
                    "und mit „Diesen Bon drucken“ einen Probedruck auf den Küchendrucker schicken."
                </p>
                <BonEditor/>
            </section>
        </AdminShell>
    }
}

/// Compact read-only preview for the /admin overview card.
#[component]
pub fn ReceiptPreviewPane() -> impl IntoView {
    view! { <BonEditor editable=false/> }
}

/// Editable badge/label fields: (settings key, German UI label, default
/// shown as placeholder). Empty value → built-in default prints.
const BADGE_FIELDS: &[(&str, &str, &str)] = &[
    (
        "receipt_label_channel_delivery",
        "Banner: Lieferung",
        "LIEFERUNG",
    ),
    (
        "receipt_label_channel_pickup",
        "Banner: Abholung",
        "ABHOLUNG",
    ),
    ("receipt_label_channel_dinein", "Banner: Tisch", "TISCH"),
    ("receipt_label_paid_badge", "Badge: Bezahlt", "BEZAHLT"),
    (
        "receipt_label_unpaid_badge",
        "Badge: Nicht bezahlt",
        "NICHT BEZAHLT",
    ),
    (
        "receipt_label_paid_confirm",
        "Bezahlt-Bestätigung",
        "BESTELLUNG IST\\nBEZAHLT",
    ),
    ("receipt_label_paid_by", "„Bezahlt von“", "Bezahlt von:"),
    ("receipt_label_cash", "„Bar“", "Bar"),
    (
        "receipt_label_collect_delivery",
        "Kassieren (Lieferung)",
        "Bei Lieferung kassieren:",
    ),
    (
        "receipt_label_collect_pickup",
        "Kassieren (Abholung)",
        "Bei Abholung kassieren:",
    ),
    (
        "receipt_label_collect_dinein",
        "Kassieren (Tisch)",
        "Am Tisch kassieren:",
    ),
    (
        "receipt_label_disclaimer",
        "Fuß-Hinweis",
        "Das ist keine Rechnung",
    ),
    (
        "receipt_label_qr_caption",
        "QR-Beschriftung",
        "Zum Bestelldetail scannen",
    ),
    (
        "receipt_label_reprint_banner",
        "Nachdruck-Banner",
        "*** REPRINT ***",
    ),
    (
        "receipt_label_test_banner",
        "Testdruck-Banner",
        "*** TEST ***",
    ),
];

/// The Bon-Editor: live preview + (when `editable`) every per-shop receipt
/// setting in ONE place — header/footer text, theme, QR target, block
/// toggles, AND all badge/banner texts — with Save and a "print this"
/// button. The variant toggles (channel/payment) only change the preview.
/// Field state lives in a single key→value map so the 20+ settings stay
/// manageable.
#[component]
pub fn BonEditor(#[prop(default = true)] editable: bool) -> impl IntoView {
    use std::collections::HashMap;

    let channel = RwSignal::new(Channel::Delivery);
    let prepaid = RwSignal::new(true);

    // All editable settings, keyed by app_settings key. Strings; booleans
    // are stored as "1"/"0", theme/qr as their value.
    let fields: RwSignal<HashMap<String, String>> = RwSignal::new(HashMap::new());
    // Helpers to read/write one key reactively.
    let get = move |k: &'static str| fields.with(|m| m.get(k).cloned().unwrap_or_default());
    let getb = move |k: &'static str, default: bool| {
        fields.with(|m| match m.get(k).map(|s| s.as_str()) {
            Some("1") | Some("true") => true,
            Some("0") | Some("false") => false,
            _ => default,
        })
    };
    let set = move |k: &str, v: String| {
        fields.update(|m| {
            m.insert(k.to_string(), v);
        });
    };

    let saver = ServerAction::<crate::pages::settings::UpdateSetting>::new();
    let printer = ServerAction::<PrintSampleReceipt>::new();

    // theme drives the preview's logo rendering; mirror it from the map.
    let theme = move || {
        let t = get("printer_theme");
        if t.is_empty() {
            "rusterando-rando".to_string()
        } else {
            t
        }
    };

    // Load saved settings once and seed the map.
    let settings = Resource::new(
        || (),
        |_| async move { crate::pages::settings::list_settings_all().await },
    );
    Effect::new(move |_| {
        if let Some(Ok(rows)) = settings.get() {
            fields.update(|m| {
                for r in rows {
                    m.insert(r.key, r.value);
                }
            });
        }
    });

    // Build the live ReceiptConfig from the map.
    let config_now = move || {
        use kitchen_protocol::receipt::ReceiptLabels;
        ReceiptConfig {
            header_text: get("receipt_header_text"),
            footer_text: get("receipt_footer_text"),
            show_allergens: getb("receipt_show_allergens", true),
            show_logo: getb("receipt_show_logo", true),
            labels: ReceiptLabels {
                channel_delivery: get("receipt_label_channel_delivery"),
                channel_pickup: get("receipt_label_channel_pickup"),
                channel_dinein: get("receipt_label_channel_dinein"),
                paid_badge: get("receipt_label_paid_badge"),
                unpaid_badge: get("receipt_label_unpaid_badge"),
                // newline escape from the input → real newline for preview.
                paid_confirm: get("receipt_label_paid_confirm").replace("\\n", "\n"),
                paid_by: get("receipt_label_paid_by"),
                cash: get("receipt_label_cash"),
                collect_delivery: get("receipt_label_collect_delivery"),
                collect_pickup: get("receipt_label_collect_pickup"),
                collect_dinein: get("receipt_label_collect_dinein"),
                disclaimer: get("receipt_label_disclaimer"),
                qr_caption: get("receipt_label_qr_caption"),
                allergen_notice: get("receipt_label_allergen_notice").replace("\\n", "\n"),
                reprint_banner: get("receipt_label_reprint_banner"),
                test_banner: get("receipt_label_test_banner"),
            },
        }
    };

    let lines = Memo::new(move |_| {
        let order = sample_order(channel.get(), prepaid.get(), &theme());
        build_layout(&order, false, &config_now())
    });

    // Save EVERY known key (only those present in the map were either loaded
    // or edited; we save the full set so blanks clear an override).
    let save_all = move |_| {
        let snapshot = fields.get();
        for (k, v) in snapshot {
            // Only persist keys we own (receipt_* + printer_theme).
            if k.starts_with("receipt_") || k == "printer_theme" {
                saver.dispatch(crate::pages::settings::UpdateSetting { key: k, value: v });
            }
        }
    };

    let do_print = move |_| {
        printer.dispatch(PrintSampleReceipt {
            delivery: channel.get() == Channel::Delivery,
            prepaid: prepaid.get(),
            theme: theme(),
        });
    };

    view! {
        <div class="receipt-controls">
            <div class="seg">
                <span class="seg-label">"Kanal"</span>
                <button class="seg-btn" class:active=move || channel.get() == Channel::Delivery
                    on:click=move |_| channel.set(Channel::Delivery)>"Lieferung"</button>
                <button class="seg-btn" class:active=move || channel.get() == Channel::Pickup
                    on:click=move |_| channel.set(Channel::Pickup)>"Abholung"</button>
                <button class="seg-btn" class:active=move || channel.get() == Channel::DineIn
                    on:click=move |_| channel.set(Channel::DineIn)>"Tisch"</button>
            </div>
            <div class="seg">
                <span class="seg-label">"Zahlung"</span>
                <button class="seg-btn" class:active=move || prepaid.get()
                    on:click=move |_| prepaid.set(true)>"Bezahlt"</button>
                <button class="seg-btn" class:active=move || !prepaid.get()
                    on:click=move |_| prepaid.set(false)>"Bar"</button>
            </div>
            <div class="seg">
                <span class="seg-label">"Theme"</span>
                <button class="seg-btn" class:active=move || theme() == "rusterando-rando"
                    on:click=move |_| set("printer_theme", "rusterando-rando".to_string())>"rando (Logo)"</button>
                <button class="seg-btn" class:active=move || theme() == "rusterando-default"
                    on:click=move |_| set("printer_theme", "rusterando-default".to_string())>"klassisch"</button>
            </div>
        </div>

        {editable.then(|| view! {
            <div class="bon-edit-fields">
                <label class="bon-field">
                    <span>"Kopfzeile (überschreibt Shop-Name)"</span>
                    <input type="text" maxlength="60" placeholder="z. B. Davids Pizzeria"
                        prop:value=move || get("receipt_header_text")
                        on:input=move |ev| set("receipt_header_text", event_target_value(&ev))/>
                </label>
                <label class="bon-field">
                    <span>"Fußzeile (Freitext, jeder Bon)"</span>
                    <input type="text" maxlength="120" placeholder="z. B. Danke für Ihre Bestellung!"
                        prop:value=move || get("receipt_footer_text")
                        on:input=move |ev| set("receipt_footer_text", event_target_value(&ev))/>
                </label>

                <div class="bon-field">
                    <span>"QR-Code-Ziel"</span>
                    <select class="cell-input"
                        prop:value=move || {
                            let m = get("receipt_qr_mode");
                            if m.is_empty() { "order-url".to_string() } else { m }
                        }
                        on:change=move |ev| set("receipt_qr_mode", event_target_value(&ev))>
                        <option value="order-url">"Bestelldetail-URL — Kund:in empfängt Live-Nachrichten"</option>
                        <option value="static-url">"Feste URL (z. B. Startseite)"</option>
                    </select>
                </div>
                <label class="bon-field">
                    <span>"Feste QR-URL (nur bei „feste URL“)"</span>
                    <input type="text" placeholder="https://davidspizzeria.de"
                        prop:value=move || get("receipt_qr_static_url")
                        on:input=move |ev| set("receipt_qr_static_url", event_target_value(&ev))/>
                </label>

                <label class="bon-check">
                    <input type="checkbox" prop:checked=move || getb("receipt_show_logo", true)
                        on:change=move |ev| set("receipt_show_logo", if event_target_checked(&ev) { "1" } else { "0" }.to_string())/>
                    " Logo-Zeile anzeigen"
                </label>
                <label class="bon-check">
                    <input type="checkbox" prop:checked=move || getb("receipt_show_allergens", true)
                        on:change=move |ev| set("receipt_show_allergens", if event_target_checked(&ev) { "1" } else { "0" }.to_string())/>
                    " Allergen-Hinweis anzeigen"
                </label>

                <details class="bon-badges">
                    <summary>"Badges & Banner-Texte (leer = Standard)"</summary>
                    {BADGE_FIELDS.iter().map(|(key, label, default)| {
                        let key = *key;
                        view! {
                            <label class="bon-field">
                                <span>{label.to_string()}</span>
                                <input type="text" maxlength="200" placeholder=default.to_string()
                                    prop:value=move || get(key)
                                    on:input=move |ev| set(key, event_target_value(&ev))/>
                            </label>
                        }
                    }).collect_view()}
                </details>

                <div class="bon-actions">
                    <button class="btn primary" on:click=save_all>"Speichern"</button>
                    <button class="btn" on:click=do_print>"Diesen Bon drucken"</button>
                    {move || printer.value().get().map(|res| match res {
                        Ok(()) => view! { <span class="ok">"✓ an Drucker gesendet"</span> }.into_any(),
                        Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                    })}
                    {move || saver.value().get().map(|res| match res {
                        Ok(()) => view! { <span class="ok">"✓ gespeichert"</span> }.into_any(),
                        Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                    })}
                </div>
            </div>
        })}

        <div class="receipt-paper" style=format!("--cols:{RECEIPT_WIDTH}")>
            {move || {
                let theme = theme();
                lines.get().into_iter().map(|line| render_line(&theme, line)).collect_view()
            }}
        </div>
    }
}

/// Render one [`LayoutLine`] as an HTML element on the monospace paper.
/// (The preview renders the slot-based LAYOUT — logos as `<img>` of the
/// `.1bit.png` twins — not the composed inline-byte wire form. Same dither
/// either way, so it still shows exactly what prints.)
fn render_line(theme: &str, line: LayoutLine) -> AnyView {
    match line {
        LayoutLine::Text { text, style } => {
            view! { <div class="r-line" style=style_css(style)>{text}</div> }.into_any()
        }
        LayoutLine::Feed { n } => (0..n)
            .map(|_| view! { <div class="r-line r-feed">" "</div> })
            .collect_view()
            .into_any(),
        LayoutLine::Rule => {
            view! { <div class="r-line r-rule">{"-".repeat(RECEIPT_WIDTH)}</div> }.into_any()
        }
        LayoutLine::Raster(slot) => {
            // Only the rando theme carries rasters. The combined header
            // row: channel icon (left) + Rusterando brand mark (right),
            // mirroring the single ESC/POS band the Pi composes.
            if theme != "rusterando-rando" {
                return ().into_any();
            }
            let RasterSlot::ChannelAndBrand {
                is_delivery,
                is_paid,
            } = slot;
            view! {
                <div class="r-line r-raster">
                    <img src=channel_png(is_delivery) alt="" class="r-logo r-logo-channel"/>
                    <img src=brand_png(is_paid) alt="" class="r-logo r-logo-brand"/>
                </div>
            }
            .into_any()
        }
        LayoutLine::Qr { url } => view! {
            <div class="r-line r-qr">
                <div class="r-qr-box" title=url.clone()>"▣ QR"</div>
            </div>
        }
        .into_any(),
    }
}
