//! `rusterando-printer --test-print [PATH]`: print one sample receipt and
//! exit — no server, no tunnel. For checking a printer on new hardware (the
//! kitchen display board's USB path) before it takes over the shop's
//! receipts. PATH defaults to `usb`; any `PRINTER_PATH` value works.
//!
//! Composed locally with the same `receipt::compose` the server uses
//! (text-only theme — no logo), so it exercises the real Bon layout,
//! the ISO-8859-15 code page (umlauts, €), the native QR code and the cut.
//!
//! Don't run it on the Pi while its service runs: both would open
//! `/dev/usb/lp0` at once.

use std::path::Path;

use anyhow::{bail, Result};
use kitchen_protocol::receipt::{
    self, ComposedSvg, RasterProvider, RasterSlot, ReceiptConfig,
};
use kitchen_protocol::{
    Customer, LineItem, OrderChannel, OrderForKitchen, OrderId, PaymentStatus,
};

use crate::printer::Printer;

struct NoLogos;

impl RasterProvider for NoLogos {
    fn resolve(&self, _theme: &str, _slot: RasterSlot) -> Option<ComposedSvg> {
        None
    }
}

pub fn run(path: &str) -> Result<()> {
    if path == crate::printer::DISPLAY_ONLY {
        bail!("PRINTER_PATH=none is display-only — pass a printer, e.g. --test-print usb");
    }
    let printer = Printer::new(Path::new(path));
    printer.probe()?;
    let program = receipt::compose(&sample(), false, &ReceiptConfig::default(), &NoLogos);
    // The raster cache is only consulted for logos, which this receipt has
    // none of; a scratch one keeps the real cache untouched.
    let scratch = std::env::temp_dir().join("rusterando-test-print");
    let mut cache = crate::raster_cache::RasterCache::load(&scratch)?;
    printer.print_program(&program, &mut cache)?;
    println!("test receipt sent to {}", printer.describe());
    Ok(())
}

fn sample() -> OrderForKitchen {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let item = |qty: u32, name: &str, mods: &[&str], cents: u32| LineItem {
        qty,
        name: name.to_string(),
        modifications: mods.iter().map(|m| m.to_string()).collect(),
        unit_price_cents: cents,
        category: "Testdruck".to_string(),
        modification_prices_cents: Vec::new(),
    };
    OrderForKitchen {
        order_id: OrderId(0),
        display_number: 0,
        display_label: "TEST-0000".to_string(),
        created_at_unix: now,
        created_at_label: chrono::Local::now().format("%d.%m. %H:%M").to_string(),
        channel: OrderChannel::Pickup,
        customer: Customer {
            name: "Küchen-Display Testdruck".to_string(),
            phone: None,
        },
        items: vec![
            item(1, "#22a Pizza Funghi - Größe XL", &["extra Käse", "ohne Zwiebeln"], 1150),
            item(2, "#12 Pizzabrötchen", &["Sauce: Kräuterbutter"], 450),
        ],
        subtotal_cents: 2050,
        delivery_fee_cents: 0,
        total_cents: 2050,
        payment: PaymentStatus::CollectOnDelivery { amount_cents: 2050 },
        note: Some("Testdruck vom Küchen-Display - äöüß € - bitte wegwerfen".to_string()),
        shop_name: "Testdruck".to_string(),
        accepted_at_unix: None,
        accepted_at_label: None,
        qr_url: Some("https://davidspizzeria.de".to_string()),
        is_test_mode: true,
        voucher_code: String::new(),
        voucher_discount_cents: 0,
        pickup_time_label: None,
        printer_theme: String::new(),
    }
}
