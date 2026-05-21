//! Pickup-order flow: place_order + helpers shared across /checkout and /orders/:id.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickupSlot {
    /// ISO-ish "HH:MM" — what we render to the user.
    pub label: String,
    /// Same string the server will accept back as the chosen time.
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckoutContext {
    /// "ASAP" plus today's remaining 15-minute slots (after `prep_minutes` buffer).
    pub slots: Vec<PickupSlot>,
    /// Default ETA in minutes for the ASAP option.
    pub asap_minutes: i64,
    /// Cart subtotal as already-formatted euro string for the side panel.
    pub subtotal_label: String,
    pub subtotal_cents: i64,
    pub line_count: i64,
    /// Active delivery zones (with name + fee + min order). Empty list is
    /// "delivery currently disabled" — the UI hides the delivery option then.
    pub delivery_zones: Vec<DeliveryZone>,
    /// Cents threshold above which the delivery fee is waived. 0 means
    /// "no free-delivery promo, always charge the zone fee". Edited by
    /// admin from /admin/settings.
    #[serde(default)]
    pub free_delivery_threshold_cents: i64,
    /// Snapshot of the shop's address line ("Wolfsberger Str. 18, 59348
    /// Lüdinghausen") for the pickup-instructions hint at the bottom of
    /// the form. Empty when the admin hasn't filled in /admin/branding.
    /// Read inside the SAME server-fn that builds the rest of the
    /// checkout context — that way SSR and hydrate render identical
    /// DOM (the value travels through the resource, not via
    /// per-context BrandingHandle reads which would diverge between
    /// SSR and hydrate).
    #[serde(default)]
    pub shop_address_line: String,
    /// Same rationale: the customer's city field pre-fills with the
    /// shop's city. Travels through the resource so SSR and hydrate
    /// produce the same `<input value="...">` attribute.
    #[serde(default)]
    pub shop_city: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryZone {
    pub id: String,
    pub name: String,
    pub fee_cents: i64,
    pub min_order_cents: i64,
    pub eta_minutes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacedOrder {
    pub id: String,
    pub order_number: String,
    /// Present only when payment_method = "card". The Stripe Payment Element
    /// uses this to confirm payment in-browser. Customers paying cash never
    /// see this — the server already redirects them to the confirmation page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripe_client_secret: Option<String>,
    /// Present only when payment_method = "card". Stripe.js uses the
    /// publishable key to bootstrap. Echoed back so we don't have to expose
    /// it as a global window variable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripe_publishable_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderDetail {
    pub id: String,
    pub order_number: String,
    pub status: String,
    pub pickup_time_label: String,
    pub contact_name: String,
    pub contact_phone: String,
    pub contact_email: String,
    pub items: Vec<OrderDetailItem>,
    pub subtotal_cents: i64,
    pub total_cents: i64,
    pub created_at: String,
    #[serde(default)]
    pub payment_status: String,
    /// Short label of the instrument that settled the payment, e.g.
    /// "Apple Pay", "Karte (Visa)", "Link". None for unpaid / cash orders.
    #[serde(default)]
    pub payment_method_detail: Option<String>,
    /// "pickup" | "delivery"
    #[serde(default)]
    pub order_type: String,
    #[serde(default)]
    pub delivery_fee_cents: i64,
    /// Parsed back from delivery_address_json. None for pickup.
    #[serde(default)]
    pub delivery_address: Option<DeliveryAddress>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeliveryAddress {
    pub zone_name: String,
    pub street: String,
    pub house_number: String,
    pub postcode: String,
    pub city: String,
    #[serde(default)]
    pub notes: Option<String>,
}

impl DeliveryAddress {
    pub fn street_with_number(&self) -> String {
        format!("{} {}", self.street.trim(), self.house_number.trim())
            .trim()
            .to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderDetailItem {
    /// Snapshotted menu number ("22a", "51", …). None for items that didn't
    /// have one at order time. Rendered in front of the name on the kitchen
    /// ticket and the customer/admin invoice.
    #[serde(default)]
    pub menu_number: Option<String>,
    pub name: String,
    pub variant: Option<String>,
    pub quantity: i64,
    pub unit_price_cents: i64,
    pub line_total_cents: i64,
    /// Picked extras (Tabasco, Extra Käse…) snapshotted at order time. The
    /// kitchen reads them under the line; admin & customer invoice list
    /// them with their per-piece price contribution.
    #[serde(default)]
    pub extras: Vec<rusterando_shared::models::CartExtra>,
}

/// What the phone-recall lookup returns when a known number is typed at
/// checkout. Empty `addresses` is fine — pickup-only customers exist.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomerSnapshot {
    pub name: String,
    pub email: String,
    pub addresses: Vec<SavedAddress>,
}

/// What `validate_address` returns. `ok = true` means the customer can pay;
/// `ok = false` carries a German `reason` string ready for display.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AddressValidation {
    pub ok: bool,
    pub reason: Option<String>,
    /// Resolved zone — populated when `ok == true`.
    pub zone_id: Option<String>,
    pub zone_name: Option<String>,
    pub fee_cents: Option<i64>,
    pub min_order_cents: Option<i64>,
    pub eta_minutes: Option<i64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Echo of normalised parts — UI can show what we resolved against.
    pub matched_label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedAddress {
    pub id: String,
    pub street: String,
    pub house_number: String,
    pub postcode: String,
    pub city: String,
    pub notes: Option<String>,
    /// Pre-resolved zone, if Phase 2 has filled it in. None for now; the
    /// Form falls back to letting the customer pick the zone manually.
    pub delivery_zone_id: Option<String>,
    /// Used for sort order ("most-recent first") and as a freshness hint.
    pub last_used_at: String,
}

// -- Phase 3: tour types ----------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TourSummary {
    pub id: String,
    pub driver_name: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub total_distance_m: Option<i64>,
    pub total_duration_s: Option<i64>,
    /// "ors" when the optimiser ran, "fallback" when stops are oldest-first.
    pub optimised_by: String,
    pub stops: Vec<TourStopView>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TourStopView {
    pub sequence: i64,
    pub order_id: String,
    pub order_number: String,
    pub eta_seconds: Option<i64>,
    pub delivered_at: Option<String>,
    pub contact_name: String,
    pub contact_phone: String,
    pub address_line: String,
    pub city: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub total_cents: i64,
    pub payment_status: String,
}

// ---------------------------------------------------------------------------
// Notifier trait — lives here so the frontend's place_order can call it.
// The server crate provides a concrete impl (LogNotifier today, EmailNotifier later).
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
pub mod notify {
    use rusterando_shared::models::format_eur;
    use std::sync::Arc;

    #[derive(Debug, Clone)]
    pub struct OrderItemSummary {
        pub quantity: i64,
        /// Snapshotted menu number ("22a", "51", …). Rendered before the
        /// name on the email/invoice so kitchen + customer share one
        /// vocabulary.
        pub menu_number: Option<String>,
        pub name: String,
        pub variant: Option<String>,
        pub line_total_cents: i64,
        /// Picked extras snapshotted at order time. Listed under the item
        /// line in the email body.
        pub extras: Vec<rusterando_shared::models::CartExtra>,
    }

    #[derive(Debug, Clone)]
    pub struct OrderSummary {
        pub order_number: String,
        pub contact_name: String,
        pub contact_phone: String,
        pub contact_email: String,
        pub pickup_time_label: String,
        pub total_cents: i64,
        pub items: Vec<OrderItemSummary>,
        pub public_url: String,
        /// Either "cash" (bar bei Abholung) or "card" (online bezahlt).
        pub payment_method: String,
        /// "pickup" or "delivery"
        pub order_type: String,
        /// Pre-formatted address block, only for delivery. Empty for pickup.
        pub delivery_address_text: String,
        pub delivery_fee_cents: i64,
        pub subtotal_cents: i64,
        /// Code typed by the customer, e.g. "WILLKOMMEN10". Empty when
        /// no voucher was applied.
        pub voucher_code: String,
        /// Cents shaved off; 0 when no voucher.
        pub voucher_discount_cents: i64,
        /// Snapshot of the shop's contact data at order time. Captured
        /// here so the email-sender (which runs in a detached tokio
        /// task with no Leptos context) can render shop name / phone /
        /// address into the email body without re-reading the DB.
        pub branding: crate::branding::Branding,
    }

    pub trait OrderNotifier: Send + Sync {
        fn send_confirmation(
            &self,
            order: &OrderSummary,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>;
    }

    pub type NotifierHandle = Arc<dyn OrderNotifier>;

    pub struct LogNotifier;

    impl OrderNotifier for LogNotifier {
        fn send_confirmation(
            &self,
            order: &OrderSummary,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>
        {
            let body = render_plain_text(order);
            let order_number = order.order_number.clone();
            let to_email = order.contact_email.clone();
            let to_phone = order.contact_phone.clone();
            Box::pin(async move {
                log::info!(
                    "[order {order_number}] would send confirmation to {to_email} / {to_phone}\n{body}"
                );
                Ok(())
            })
        }
    }

    /// Sends order confirmations as plain-text email via SMTP. Reads its
    /// connection parameters from environment variables once at startup
    /// (see `EmailNotifier::from_env`).
    pub struct EmailNotifier {
        host: String,
        port: u16,
        username: String,
        password: String,
        from: String, // RFC 5322 "Name <addr>"
        kitchen_bcc: Option<String>,
    }

    impl EmailNotifier {
        /// Build an `EmailNotifier` from `SMTP_HOST/SMTP_PORT/SMTP_USER/SMTP_PASS/EMAIL_FROM`.
        /// Returns `None` if any required value is missing — callers should fall
        /// back to `LogNotifier` so dev environments without SMTP still work.
        pub fn from_env() -> Option<Self> {
            let host = std::env::var("SMTP_HOST").ok()?;
            let port = std::env::var("SMTP_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(465_u16);
            let username = std::env::var("SMTP_USER").ok()?;
            let password = std::env::var("SMTP_PASS").ok()?;
            let from = std::env::var("EMAIL_FROM").unwrap_or_else(|_| username.clone());
            // Default: BCC the order to our own bestellung@ inbox so the kitchen
            // has a copy of every confirmation. Override with KITCHEN_EMAIL=… or
            // disable with KITCHEN_EMAIL= (empty).
            let kitchen_bcc = match std::env::var("KITCHEN_EMAIL") {
                Ok(s) => Some(s).filter(|s| !s.is_empty()),
                Err(_) => Some(username.clone()),
            };
            Some(Self {
                host,
                port,
                username,
                password,
                from,
                kitchen_bcc,
            })
        }
    }

    impl OrderNotifier for EmailNotifier {
        fn send_confirmation(
            &self,
            order: &OrderSummary,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>
        {
            use lettre::message::{header::ContentType, Mailbox};
            use lettre::transport::smtp::authentication::Credentials;
            use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

            let host = self.host.clone();
            let port = self.port;
            let username = self.username.clone();
            let password = self.password.clone();
            let from = self.from.clone();
            let kitchen_bcc = self.kitchen_bcc.clone();
            let to_email = order.contact_email.clone();
            let to_name = order.contact_name.clone();
            let body = render_plain_text(order);
            let subject = format!(
                "Bestellbestätigung {} — {}",
                order.order_number,
                order.branding.display_name(),
            );

            Box::pin(async move {
                let from_mbox: Mailbox = from
                    .parse()
                    .map_err(|e| anyhow::anyhow!("EMAIL_FROM not a valid mailbox: {e}"))?;
                let to_mbox = Mailbox::new(
                    if to_name.trim().is_empty() {
                        None
                    } else {
                        Some(to_name)
                    },
                    to_email
                        .parse()
                        .map_err(|e| anyhow::anyhow!("invalid recipient email: {e}"))?,
                );

                let mut builder = Message::builder()
                    .from(from_mbox)
                    .to(to_mbox)
                    .subject(&subject)
                    .header(ContentType::TEXT_PLAIN);
                if let Some(bcc_str) = &kitchen_bcc {
                    if let Ok(bcc) = bcc_str.parse::<Mailbox>() {
                        builder = builder.bcc(bcc);
                    }
                }
                let email = builder
                    .body(body)
                    .map_err(|e| anyhow::anyhow!("build email: {e}"))?;

                // IONOS uses port 465 (implicit TLS). Port 587 = STARTTLS.
                let creds = Credentials::new(username, password);
                let transport: AsyncSmtpTransport<Tokio1Executor> = if port == 587 {
                    AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)
                        .map_err(|e| anyhow::anyhow!("smtp init: {e}"))?
                        .port(port)
                        .credentials(creds)
                        .build()
                } else {
                    AsyncSmtpTransport::<Tokio1Executor>::relay(&host)
                        .map_err(|e| anyhow::anyhow!("smtp init: {e}"))?
                        .port(port)
                        .credentials(creds)
                        .build()
                };

                transport
                    .send(email)
                    .await
                    .map_err(|e| anyhow::anyhow!("smtp send: {e}"))?;
                Ok(())
            })
        }
    }

    pub fn render_plain_text(o: &OrderSummary) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "{} — Bestellbestätigung\n",
            o.branding.display_name()
        ));
        s.push_str(&format!("Bestellnummer: {}\n", o.order_number));

        let is_delivery = o.order_type == "delivery";
        if is_delivery {
            s.push_str(&format!("Lieferung: {}\n", o.pickup_time_label));
            if !o.delivery_address_text.is_empty() {
                s.push_str(&format!("Lieferadresse:\n{}\n", o.delivery_address_text));
            }
        } else {
            s.push_str(&format!("Abholung: {}\n", o.pickup_time_label));
        }

        s.push_str("\nIhre Bestellung:\n");
        for it in &o.items {
            let variant = it
                .variant
                .as_deref()
                .map(|v| format!(" ({v})"))
                .unwrap_or_default();
            let prefix = it
                .menu_number
                .as_deref()
                .map(|n| format!("{n}. "))
                .unwrap_or_default();
            s.push_str(&format!(
                "  {} × {}{}{}  {}\n",
                it.quantity,
                prefix,
                it.name,
                variant,
                format_eur(it.line_total_cents)
            ));
            for ex in &it.extras {
                let p = if ex.price_cents == 0 {
                    "gratis".to_string()
                } else {
                    format!("+{}", format_eur(ex.price_cents))
                };
                s.push_str(&format!("        + {} ({p})\n", ex.label));
            }
        }
        if o.delivery_fee_cents > 0 || o.voucher_discount_cents > 0 {
            s.push_str(&format!(
                "\nZwischensumme: {}\n",
                format_eur(o.subtotal_cents)
            ));
            if o.delivery_fee_cents > 0 {
                s.push_str(&format!(
                    "Lieferzuschlag: {}\n",
                    format_eur(o.delivery_fee_cents)
                ));
            }
            if o.voucher_discount_cents > 0 {
                s.push_str(&format!(
                    "Gutschein {}: -{}\n",
                    o.voucher_code,
                    format_eur(o.voucher_discount_cents)
                ));
            }
        }
        s.push_str(&format!("\nGesamt: {}\n", format_eur(o.total_cents)));
        let payment_line = match (o.payment_method.as_str(), is_delivery) {
            ("card", _) => "Zahlung: Online bezahlt — keine weitere Aktion nötig.",
            ("cash", true) => "Zahlung: Bar bei Lieferung an den Fahrer.",
            ("cash", false) => "Zahlung: Bar bei Abholung",
            _ => "Zahlung: ?",
        };
        s.push_str(&format!("\n{payment_line}\n"));
        s.push_str(&format!("\nDetails & Status: {}\n", o.public_url));
        let addr = o.branding.full_address();
        if !addr.is_empty() {
            s.push_str(&format!("\nAdresse: {addr}\n"));
        }
        if !o.branding.shop_phone.is_empty() {
            s.push_str(&format!("Telefon: {}\n", o.branding.shop_phone));
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Server-side helpers
// ---------------------------------------------------------------------------

/// Shape of the orders row returned by `get_order`. Field order matches the
/// SELECT column order — keep them in sync.
/// (id, order_number, status, contact_name, contact_phone, contact_email,
///  scheduled_for, subtotal_cents, total_cents, created_at, payment_status,
///  order_type, delivery_fee_cents, delivery_address_json, payment_method_detail)
#[cfg(feature = "ssr")]
type OrderRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    i64,
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
);

/// `lookup_customer_by_phone` address rows.
/// (id, street, house_number, postcode, city, notes, delivery_zone_id, last_used_at)
#[cfg(feature = "ssr")]
type SavedAddressRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
);

/// `validate_address` DB-cache lookup. (latitude, longitude, suburb, delivery_zone_id)
#[cfg(feature = "ssr")]
type AddressCacheRow = (Option<f64>, Option<f64>, Option<String>, Option<String>);

/// `get_tour` header row.
/// (id, driver_name, started_at, finished_at, total_distance_m, total_duration_s, optimised_by)
#[cfg(feature = "ssr")]
type TourRow = (
    String,
    Option<String>,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    String,
);

/// `get_tour` stops join.
/// (sequence, order_id, eta_seconds, delivered_at, order_number, contact_name,
///  contact_phone, total_cents, payment_status, delivery_address_json, lat, lon)
#[cfg(feature = "ssr")]
type TourStopRow = (
    i64,
    String,
    Option<i64>,
    Option<String>,
    String,
    String,
    String,
    i64,
    String,
    Option<String>,
    Option<f64>,
    Option<f64>,
);

#[cfg(feature = "ssr")]
pub mod ssr {
    use super::*;
    use chrono::{Datelike, Local, NaiveTime, TimeZone, Timelike};

    pub const ASAP_DEFAULT_MIN: i64 = 30;
    pub const PREP_BUFFER_MIN: i64 = 20;

    pub fn now_local() -> chrono::DateTime<Local> {
        Local::now()
    }

    /// Return the "HH:MM" for `now + minutes`.
    pub fn fmt_offset(minutes: i64) -> String {
        let t = now_local() + chrono::Duration::minutes(minutes);
        t.format("%H:%M").to_string()
    }

    /// Generate today's pickup time slots in 15-min increments, starting from
    /// `now + PREP_BUFFER_MIN`, bounded by the rows in `opening_hours` that
    /// match today's weekday and are not closed. Slots from past hours are
    /// dropped automatically because their start time is < now.
    pub async fn today_slots(db: &sqlx::SqlitePool) -> Vec<PickupSlot> {
        let now = now_local();
        // SQLite weekday: 0=Sun..6=Sat — same convention as our seed.
        let weekday = now.weekday().num_days_from_sunday() as i64;

        let rows = sqlx::query_as::<_, (String, String, i64)>(
            "SELECT open_time, close_time, is_closed FROM opening_hours WHERE weekday = ?1",
        )
        .bind(weekday)
        .fetch_all(db)
        .await
        .unwrap_or_default();

        let earliest = now + chrono::Duration::minutes(PREP_BUFFER_MIN);
        let mut slots = Vec::new();

        for (open_s, close_s, closed) in rows {
            if closed != 0 {
                continue;
            }
            let Ok(open_t) = NaiveTime::parse_from_str(&open_s, "%H:%M") else {
                continue;
            };
            let Ok(close_t) = NaiveTime::parse_from_str(&close_s, "%H:%M") else {
                continue;
            };

            let day = now.date_naive();
            let open_dt = Local.from_local_datetime(&day.and_time(open_t)).single();
            let close_dt = Local.from_local_datetime(&day.and_time(close_t)).single();
            let (Some(mut t), Some(close_dt)) = (open_dt, close_dt) else {
                continue;
            };
            if t < earliest {
                t = earliest;
            }
            // Round up to next 15-minute boundary
            let m = t.minute();
            let bump = (15 - (m % 15)) % 15;
            if bump > 0 {
                t += chrono::Duration::minutes(bump as i64);
            }
            t = t
                .with_second(0)
                .unwrap_or(t)
                .with_nanosecond(0)
                .unwrap_or(t);

            while t + chrono::Duration::minutes(15) <= close_dt {
                let label = t.format("%H:%M").to_string();
                slots.push(PickupSlot {
                    label: label.clone(),
                    value: label,
                });
                t += chrono::Duration::minutes(15);
            }
        }
        slots.sort_by(|a, b| a.value.cmp(&b.value));
        slots
    }

    /// `DP-DDMM-NNNN` — daily 4-digit sequence, padded.
    pub async fn next_order_number(db: &sqlx::SqlitePool) -> Result<String, sqlx::Error> {
        let now = now_local();
        let prefix = format!("DP-{:02}{:02}-", now.day(), now.month());
        let pattern = format!("{prefix}%");
        let last: Option<(String,)> = sqlx::query_as(
            "SELECT order_number FROM orders WHERE order_number LIKE ?1 ORDER BY order_number DESC LIMIT 1",
        )
        .bind(&pattern)
        .fetch_optional(db)
        .await?;
        let n = match last {
            Some((s,)) => {
                s.rsplit('-')
                    .next()
                    .and_then(|t| t.parse::<u32>().ok())
                    .unwrap_or(0)
                    + 1
            }
            None => 1,
        };
        Ok(format!("{prefix}{:04}", n))
    }

    pub fn pickup_label(scheduled_for: Option<&chrono::NaiveDateTime>) -> String {
        match scheduled_for {
            Some(dt) => format!("Heute {}", dt.format("%H:%M")),
            None => format!("ASAP — gegen {} Uhr", fmt_offset(ASAP_DEFAULT_MIN)),
        }
    }

    /// Create a Stripe PaymentIntent for the order. Returns (intent id,
    /// client_secret). The customer's browser uses the client_secret to mount
    /// the Payment Element; the intent id is stored on our order row so the
    /// webhook can resolve back to it (in addition to metadata.order_id).
    ///
    /// `stripe_secret` is the mode-specific key resolved by the caller
    /// from `StripeModeHandle::active_keys()`. We don't read env here
    /// so the same function works for both sandbox and live without
    /// branching internally.
    pub async fn create_payment_intent(
        order_id: &str,
        order_number: &str,
        total_cents: i64,
        receipt_email: &str,
        shop_name: &str,
        stripe_secret: &str,
    ) -> Result<(String, String), leptos::prelude::ServerFnError> {
        use leptos::prelude::ServerFnError;
        use std::collections::HashMap;
        use stripe::{
            Client, CreatePaymentIntent, CreatePaymentIntentAutomaticPaymentMethods, Currency,
            PaymentIntent,
        };

        if stripe_secret.is_empty() {
            return Err(ServerFnError::new(
                "Stripe Secret Key fehlt — bitte S_STRIPE_SECRET_KEY / L_STRIPE_SECRET_KEY in .env setzen",
            ));
        }
        let client = Client::new(stripe_secret.to_string());

        let mut params = CreatePaymentIntent::new(total_cents, Currency::EUR);
        params.automatic_payment_methods = Some(CreatePaymentIntentAutomaticPaymentMethods {
            enabled: true,
            allow_redirects: None,
        });

        let mut metadata = HashMap::new();
        metadata.insert("order_id".to_string(), order_id.to_string());
        metadata.insert("order_number".to_string(), order_number.to_string());
        params.metadata = Some(metadata);

        let receipt_email_owned = receipt_email.to_string();
        if !receipt_email_owned.is_empty() {
            params.receipt_email = Some(&receipt_email_owned);
        }

        let description = format!("{shop_name} — Bestellung {order_number}");
        params.description = Some(&description);

        let intent = PaymentIntent::create(&client, params)
            .await
            .map_err(|e| ServerFnError::new(format!("stripe create intent: {e}")))?;

        let secret = intent
            .client_secret
            .clone()
            .ok_or_else(|| ServerFnError::new("Stripe returned no client_secret"))?;

        Ok((intent.id.to_string(), secret))
    }

    /// Strip everything but digits and a leading `+`. "0 2591 / 12 34" and
    /// "025911234" land on the same key; "+49 2591 1234" stays distinct from
    /// its national-format twin (intentional — different write conventions
    /// across customer base, we'd rather keep two rows than merge wrongly).
    pub fn normalize_phone(raw: &str) -> String {
        let trimmed = raw.trim();
        let leading_plus = trimmed.starts_with('+');
        let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
        if leading_plus {
            format!("+{digits}")
        } else {
            digits
        }
    }

    /// Insert (or refresh) the customer keyed by phone. Name/email are
    /// updated on every order — the latest order wins, since people fix
    /// typos as they go. last_seen_at bumps unconditionally.
    pub async fn upsert_customer(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        phone: &str,
        name: &str,
        email: &str,
    ) -> Result<String, ServerFnError> {
        let existing: Option<(String,)> =
            sqlx::query_as("SELECT id FROM customers WHERE phone = ?1")
                .bind(phone)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| ServerFnError::new(format!("lookup customer: {e}")))?;

        if let Some((id,)) = existing {
            sqlx::query(
                "UPDATE customers
                 SET name = CASE WHEN ?2 = '' THEN name ELSE ?2 END,
                     email = CASE WHEN ?3 = '' THEN email ELSE ?3 END,
                     last_seen_at = CURRENT_TIMESTAMP,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
            )
            .bind(&id)
            .bind(name)
            .bind(email)
            .execute(&mut **tx)
            .await
            .map_err(|e| ServerFnError::new(format!("update customer: {e}")))?;
            return Ok(id);
        }

        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO customers (id, phone, name, email)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&id)
        .bind(phone)
        .bind(name)
        .bind(email)
        .execute(&mut **tx)
        .await
        .map_err(|e| ServerFnError::new(format!("insert customer: {e}")))?;
        Ok(id)
    }

    /// Resolved geo for an address — populated by the Phase 2 geocoder and
    /// persisted on the `customer_addresses` row so subsequent lookups skip
    /// Nominatim entirely.
    #[derive(Debug, Clone, Default)]
    pub struct AddressGeo {
        pub zone_id: String,
        pub latitude: Option<f64>,
        pub longitude: Option<f64>,
    }

    /// Borrowed address parts — passed as one struct so callers (and clippy)
    /// don't see five str args in a row.
    pub struct AddressInput<'a> {
        pub street: &'a str,
        pub house_number: &'a str,
        pub postcode: &'a str,
        pub city: &'a str,
        pub notes: Option<&'a str>,
    }

    /// Insert (or refresh) the address for this customer. Uniqueness is
    /// `(customer_id, street, house_number, postcode)` — same household
    /// resubmitting the same place bumps `last_used_at` instead of stacking.
    /// `geo` carries the geocoder result; lat/lon/zone are persisted on
    /// insert and refreshed on update so the cache stays warm.
    pub async fn upsert_address(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        customer_id: &str,
        input: AddressInput<'_>,
        geo: Option<AddressGeo>,
    ) -> Result<String, ServerFnError> {
        let AddressInput {
            street,
            house_number,
            postcode,
            city,
            notes,
        } = input;
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM customer_addresses
             WHERE customer_id = ?1 AND street = ?2 AND house_number = ?3 AND postcode = ?4",
        )
        .bind(customer_id)
        .bind(street)
        .bind(house_number)
        .bind(postcode)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| ServerFnError::new(format!("lookup address: {e}")))?;

        let (zone, lat, lon) = match &geo {
            Some(g) => (Some(g.zone_id.as_str()), g.latitude, g.longitude),
            None => (None, None, None),
        };
        let geocoded_now = if geo.is_some() {
            Some("CURRENT_TIMESTAMP")
        } else {
            None
        };
        let _ = geocoded_now; // we set the timestamp via SQL CASE below

        if let Some((id,)) = existing {
            sqlx::query(
                "UPDATE customer_addresses
                 SET city = ?2,
                     notes = ?3,
                     delivery_zone_id = COALESCE(?4, delivery_zone_id),
                     latitude = COALESCE(?5, latitude),
                     longitude = COALESCE(?6, longitude),
                     geocoded_at = CASE WHEN ?5 IS NOT NULL THEN CURRENT_TIMESTAMP ELSE geocoded_at END,
                     geocode_provider = CASE WHEN ?5 IS NOT NULL THEN 'nominatim' ELSE geocode_provider END,
                     last_used_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
            )
            .bind(&id)
            .bind(city)
            .bind(notes)
            .bind(zone)
            .bind(lat)
            .bind(lon)
            .execute(&mut **tx)
            .await
            .map_err(|e| ServerFnError::new(format!("update address: {e}")))?;
            return Ok(id);
        }

        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO customer_addresses
                (id, customer_id, street, house_number, postcode, city, notes,
                 delivery_zone_id, latitude, longitude, geocoded_at, geocode_provider)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     CASE WHEN ?9 IS NOT NULL THEN CURRENT_TIMESTAMP ELSE NULL END,
                     CASE WHEN ?9 IS NOT NULL THEN 'nominatim' ELSE NULL END)",
        )
        .bind(&id)
        .bind(customer_id)
        .bind(street)
        .bind(house_number)
        .bind(postcode)
        .bind(city)
        .bind(notes)
        .bind(zone)
        .bind(lat)
        .bind(lon)
        .execute(&mut **tx)
        .await
        .map_err(|e| ServerFnError::new(format!("insert address: {e}")))?;
        Ok(id)
    }

    // -- Phase 2: geocoding + classification --------------------------------

    /// Pizzeria coordinates — used for the sanity-radius check in
    /// classify_zone(). Resolution order:
    ///   1. PIZZERIA_LAT / PIZZERIA_LON env vars (legacy override).
    ///   2. BrandingHandle.shop_lat / shop_lon (admin-set, in-DB).
    ///   3. (0.0, 0.0) — sentinel meaning "shop hasn't set coords yet".
    ///      classify_zone treats this as "skip the radius check" so
    ///      fresh installs without a configured address still let
    ///      orders through.
    pub fn pizzeria_coords() -> (f64, f64) {
        if let (Ok(lat), Ok(lon)) = (
            std::env::var("PIZZERIA_LAT").map(|s| s.parse::<f64>()),
            std::env::var("PIZZERIA_LON").map(|s| s.parse::<f64>()),
        ) {
            if let (Ok(lat), Ok(lon)) = (lat, lon) {
                if lat != 0.0 || lon != 0.0 {
                    return (lat, lon);
                }
            }
        }
        if let Some(b) =
            leptos::prelude::use_context::<crate::branding::BrandingHandle>().map(|h| h.get())
        {
            if b.shop_lat != 0.0 || b.shop_lon != 0.0 {
                return (b.shop_lat, b.shop_lon);
            }
        }
        (0.0, 0.0)
    }

    /// 30 km is a generous radius around the pizzeria — anything farther is
    /// clearly out of delivery range, so we reject before we'd ever reach
    /// the suburb classifier (avoids "Lüdinghausen, Sachsen-Anhalt" surprises).
    pub const SANITY_RADIUS_KM: f64 = 30.0;

    pub fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
        const R: f64 = 6371.0;
        let (lat1, lon1) = (a.0.to_radians(), a.1.to_radians());
        let (lat2, lon2) = (b.0.to_radians(), b.1.to_radians());
        let dlat = lat2 - lat1;
        let dlon = lon2 - lon1;
        let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
        2.0 * R * h.sqrt().asin()
    }

    /// Pure classifier — testable, no I/O. Maps an OSM result onto our three
    /// zone IDs. Returns None for "out of delivery area" (any reason: too
    /// far away, wrong city, unknown suburb shape).
    pub fn classify_zone(
        lat: f64,
        lon: f64,
        suburb: Option<&str>,
        city: Option<&str>,
        expected_city: &str,
    ) -> Option<&'static str> {
        // Skip the sanity-radius check when the shop hasn't set its
        // own coordinates yet (fresh install / demo deploy). Same
        // fail-open posture as the expected_city check below — better
        // to let an order through than block every customer until
        // /admin/branding is filled in.
        let shop = pizzeria_coords();
        if shop != (0.0, 0.0) && haversine_km(shop, (lat, lon)) > SANITY_RADIUS_KM {
            return None;
        }
        let city = city.unwrap_or("");
        // expected_city comes from BrandingHandle.shop_city. If the
        // shop hasn't filled it in yet we fail open here (return None
        // would block all orders) — admin nags via /admin/branding to
        // set it before going live.
        if !expected_city.is_empty() && !city.eq_ignore_ascii_case(expected_city) {
            return None;
        }
        let suburb = suburb.unwrap_or("").trim();
        if suburb.eq_ignore_ascii_case("Seppenrade") {
            return Some("dz-seppenrade");
        }
        // Empty suburb OR Nominatim's "<City> (Stadt)" district label both
        // mean "city proper" — base zone. Any *other* suburb name within
        // the expected city → outlying-zone fallback.
        if suburb.is_empty() || is_city_proper_district(suburb, city) {
            return Some("dz-luedinghausen");
        }
        Some("dz-bauerschaften")
    }

    /// Nominatim sometimes labels the city centre as `<City> (Stadt)` in
    /// `city_district`. That's still the base zone, not an outlying suburb.
    fn is_city_proper_district(suburb: &str, city: &str) -> bool {
        let s = suburb.trim();
        // Strip a trailing "(Stadt)" / "(Stadtteil)" / "(Kernstadt)" marker.
        let bare = s.split('(').next().unwrap_or(s).trim();
        bare.eq_ignore_ascii_case(city)
    }

    /// Nominatim raw response — only the fields we care about.
    /// `display_name` and `address.postcode` aren't read today but we still
    /// deserialise them: they're documentation of the wire shape and we may
    /// surface them in admin diagnostics later.
    #[derive(Debug, serde::Deserialize)]
    pub struct NominatimResult {
        pub lat: String,
        pub lon: String,
        #[serde(default)]
        #[allow(dead_code)]
        pub display_name: String,
        #[serde(default)]
        pub address: NominatimAddress,
    }

    #[derive(Debug, Default, serde::Deserialize)]
    pub struct NominatimAddress {
        pub city: Option<String>,
        pub town: Option<String>,
        pub village: Option<String>,
        pub suburb: Option<String>,
        pub city_district: Option<String>,
        #[allow(dead_code)]
        pub postcode: Option<String>,
    }

    impl NominatimAddress {
        /// OSM's "city-ish" field varies by record type; collapse them.
        pub fn place(&self) -> Option<&str> {
            self.city.as_deref().or(self.town.as_deref())
        }
        /// Sub-locality. Nominatim populates one of these depending on the
        /// shape of the result:
        ///   - `suburb`: typical for big cities
        ///   - `city_district`: used as "<City> (Stadt)" for the city centre
        ///   - `village`: when the place is a Stadtteil that's geographically
        ///     a separate village under a parent town (Seppenrade is one).
        pub fn suburb_or_district(&self) -> Option<&str> {
            self.suburb
                .as_deref()
                .or(self.city_district.as_deref())
                .or(self.village.as_deref())
        }
    }

    /// Global rate-limit gate for Nominatim. Their AUP requires ≤ 1 req/s.
    /// We share one gate process-wide; for our volume that's plenty.
    static NOMINATIM_GATE: std::sync::OnceLock<tokio::sync::Mutex<std::time::Instant>> =
        std::sync::OnceLock::new();

    async fn acquire_nominatim_slot() {
        let gate = NOMINATIM_GATE.get_or_init(|| {
            tokio::sync::Mutex::new(std::time::Instant::now() - std::time::Duration::from_secs(2))
        });
        let mut last = gate.lock().await;
        let elapsed = last.elapsed();
        let min = std::time::Duration::from_millis(1100); // 100ms slack
        if elapsed < min {
            tokio::time::sleep(min - elapsed).await;
        }
        *last = std::time::Instant::now();
    }

    /// Geocode a structured address against Nominatim. Returns `Ok(Some(_))`
    /// on a hit, `Ok(None)` for "address syntactically fine but unknown",
    /// and `Err` only for hard infrastructure failures (network, 5xx).
    /// The hard-reject UX in validate_address relies on Err vs Ok distinction.
    ///
    /// We attempt two queries: (1) the structured address as the customer
    /// typed it, (2) postcode-only fallback that drops the city field. Reason:
    /// Seppenrade is a Stadtteil of Lüdinghausen, so "Bahnhofstr. 1, 59348
    /// Seppenrade" returns 0 hits — but with the city dropped, Nominatim
    /// resolves it correctly. Two requests respects the 1 req/sec limit
    /// thanks to the shared `acquire_nominatim_slot()` gate.
    pub async fn nominatim_lookup(
        street: &str,
        house_number: &str,
        postcode: &str,
        city: &str,
    ) -> Result<Option<NominatimResult>, ServerFnError> {
        // Nominatim asks for a User-Agent identifying the application.
        // Rusterando ships a fixed name; per-shop email is appended when
        // configured so OSM ops can reach the operator if a shop misbehaves.
        let contact = use_context::<crate::branding::BrandingHandle>()
            .map(|h| h.get().shop_email)
            .unwrap_or_default();
        let ua = if contact.is_empty() {
            "Rusterando/1.0".to_string()
        } else {
            format!("Rusterando/1.0 ({contact})")
        };
        let client = reqwest::Client::builder()
            .user_agent(ua)
            .timeout(std::time::Duration::from_secs(6))
            .build()
            .map_err(|e| ServerFnError::new(format!("geocoder client: {e}")))?;

        let queries = [
            format!(
                "{} {}, {} {}, Deutschland",
                street.trim(),
                house_number.trim(),
                postcode.trim(),
                city.trim()
            ),
            // Drop the city; postcode + street + house number is usually
            // enough and avoids the "Seppenrade is a suburb, not a city" trap.
            format!(
                "{} {}, {}, Deutschland",
                street.trim(),
                house_number.trim(),
                postcode.trim()
            ),
        ];

        for q in &queries {
            acquire_nominatim_slot().await;
            let resp = client
                .get("https://nominatim.openstreetmap.org/search")
                .query(&[
                    ("q", q.as_str()),
                    ("format", "jsonv2"),
                    ("addressdetails", "1"),
                    ("countrycodes", "de"),
                    ("limit", "1"),
                ])
                .send()
                .await
                .map_err(|e| ServerFnError::new(format!("geocoder unreachable: {e}")))?;
            if !resp.status().is_success() {
                return Err(ServerFnError::new(format!(
                    "geocoder status {}",
                    resp.status()
                )));
            }
            let results: Vec<NominatimResult> = resp
                .json()
                .await
                .map_err(|e| ServerFnError::new(format!("geocoder parse: {e}")))?;
            if let Some(first) = results.into_iter().next() {
                return Ok(Some(first));
            }
        }
        Ok(None)
    }

    // -- Phase 3: ORS tour optimisation -------------------------------------

    #[derive(Debug, Clone, serde::Serialize)]
    struct OrsJob<'a> {
        id: u64,
        location: [f64; 2], // [lon, lat] — ORS convention
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<&'a str>,
    }

    #[derive(Debug, Clone, serde::Serialize)]
    struct OrsVehicle {
        id: u64,
        profile: &'static str,
        start: [f64; 2],
        end: [f64; 2],
    }

    #[derive(Debug, Clone, serde::Serialize)]
    struct OrsRequest<'a> {
        jobs: Vec<OrsJob<'a>>,
        vehicles: Vec<OrsVehicle>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct OrsResponse {
        #[serde(default)]
        routes: Vec<OrsRoute>,
        #[serde(default)]
        summary: OrsSummary,
    }

    #[derive(Debug, Default, serde::Deserialize)]
    struct OrsSummary {
        #[serde(default)]
        duration: u64,
        #[serde(default)]
        distance: u64,
    }

    #[derive(Debug, serde::Deserialize)]
    struct OrsRoute {
        #[serde(default)]
        steps: Vec<OrsStep>,
        #[serde(default)]
        duration: u64,
        #[serde(default)]
        distance: u64,
    }

    #[derive(Debug, serde::Deserialize)]
    struct OrsStep {
        #[serde(rename = "type", default)]
        step_type: String, // "start" | "job" | "end"
        #[serde(default)]
        job: Option<u64>,
        #[serde(default)]
        arrival: u64, // seconds since vehicle start
    }

    #[derive(Debug, Clone)]
    pub struct OptimisedStop {
        pub order_id: String,
        pub eta_seconds: Option<u64>,
    }

    #[derive(Debug, Clone)]
    pub struct OptimisedTour {
        /// In delivery order. Length matches the input (or the input is
        /// returned in the original order on fallback).
        pub stops: Vec<OptimisedStop>,
        pub total_distance_m: Option<u64>,
        pub total_duration_s: Option<u64>,
        /// Source label persisted on the tour row for diagnostics.
        pub source: &'static str, // "ors" | "fallback"
        pub raw_json: Option<String>,
    }

    /// Optimise the stop sequence for a tour. Falls back to the input order
    /// (caller pre-sorts oldest-first) when ORS is unreachable, the API key
    /// is unset, or the response is malformed. Failure is *never* fatal —
    /// the driver still gets a tour, just not an optimal one.
    pub async fn optimise_tour(
        stops: &[(
            String, /* order_id */
            f64,    /* lat */
            f64,    /* lon */
        )],
    ) -> OptimisedTour {
        if stops.is_empty() {
            return OptimisedTour {
                stops: Vec::new(),
                total_distance_m: None,
                total_duration_s: None,
                source: "fallback",
                raw_json: None,
            };
        }

        let api_key = match std::env::var("ORS_API_KEY") {
            Ok(k) if !k.trim().is_empty() => k,
            _ => return fallback_tour(stops, "ORS_API_KEY not set"),
        };
        let base = std::env::var("ORS_BASE_URL")
            .unwrap_or_else(|_| "https://api.openrouteservice.org".to_string());

        let (piz_lat, piz_lon) = pizzeria_coords();
        let pizzeria_lonlat = [piz_lon, piz_lat];
        let order_ids: Vec<String> = stops.iter().map(|s| s.0.clone()).collect();

        let body = OrsRequest {
            jobs: stops
                .iter()
                .enumerate()
                .map(|(i, s)| OrsJob {
                    id: i as u64,
                    location: [s.2, s.1], // [lon, lat]
                    description: None,
                })
                .collect(),
            vehicles: vec![OrsVehicle {
                id: 1,
                profile: "driving-car",
                start: pizzeria_lonlat,
                end: pizzeria_lonlat,
            }],
        };

        let contact = use_context::<crate::branding::BrandingHandle>()
            .map(|h| h.get().shop_email)
            .unwrap_or_default();
        let ua = if contact.is_empty() {
            "Rusterando/1.0".to_string()
        } else {
            format!("Rusterando/1.0 ({contact})")
        };
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent(ua)
            .build()
        {
            Ok(c) => c,
            Err(e) => return fallback_tour(stops, &format!("ors client build: {e}")),
        };

        let resp = client
            .post(format!("{base}/optimization"))
            .header("Authorization", &api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;
        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                log::warn!(
                    "ors non-2xx status={status} body={}",
                    body.chars().take(500).collect::<String>()
                );
                return fallback_tour(stops, &format!("ors {status}"));
            }
            Err(e) => return fallback_tour(stops, &format!("ors send: {e}")),
        };

        let raw = match resp.text().await {
            Ok(t) => t,
            Err(e) => return fallback_tour(stops, &format!("ors read: {e}")),
        };
        let parsed: OrsResponse = match serde_json::from_str(&raw) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("ors response parse failed: {e}");
                return fallback_tour(stops, &format!("ors parse: {e}"));
            }
        };

        let Some(route) = parsed.routes.into_iter().next() else {
            return fallback_tour(stops, "ors empty routes");
        };
        let mut sequenced = Vec::with_capacity(stops.len());
        for step in route.steps {
            if step.step_type != "job" {
                continue;
            }
            let Some(idx) = step.job else {
                continue;
            };
            let Some(oid) = order_ids.get(idx as usize) else {
                continue;
            };
            sequenced.push(OptimisedStop {
                order_id: oid.clone(),
                eta_seconds: Some(step.arrival),
            });
        }

        // Defensive: if ORS skipped any jobs (rare but possible), append the
        // missing ones at the end so the tour is still complete.
        for (i, oid) in order_ids.iter().enumerate() {
            if sequenced.iter().any(|s| s.order_id == *oid) {
                continue;
            }
            log::warn!("ors omitted job idx={i} order_id={oid}, appending");
            sequenced.push(OptimisedStop {
                order_id: oid.clone(),
                eta_seconds: None,
            });
        }

        OptimisedTour {
            stops: sequenced,
            total_distance_m: Some(route.distance.max(parsed.summary.distance)),
            total_duration_s: Some(route.duration.max(parsed.summary.duration)),
            source: "ors",
            raw_json: Some(raw),
        }
    }

    fn fallback_tour(stops: &[(String, f64, f64)], reason: &str) -> OptimisedTour {
        log::info!(
            "tour falling back to input order: {} ({} stops)",
            reason,
            stops.len()
        );
        OptimisedTour {
            stops: stops
                .iter()
                .map(|(id, _, _)| OptimisedStop {
                    order_id: id.clone(),
                    eta_seconds: None,
                })
                .collect(),
            total_distance_m: None,
            total_duration_s: None,
            source: "fallback",
            raw_json: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Server functions
// ---------------------------------------------------------------------------

#[server(
    name = LoadCheckoutContext,
    prefix = "/api",
    endpoint = "load_checkout_context"
)]
pub async fn load_checkout_context() -> Result<CheckoutContext, ServerFnError> {
    use sqlx::SqlitePool;
    let cart_id = crate::pages::cart::ssr::current_cart_id().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let cart = crate::pages::cart::ssr::load_cart(&db, &cart_id).await?;
    let slots = ssr::today_slots(&db).await;

    let zones: Vec<DeliveryZone> = sqlx::query_as::<_, (String, Option<String>, i64, i64, i64)>(
        "SELECT id, name, fee_cents, min_order_cents, eta_minutes
         FROM delivery_zones
         WHERE is_active = 1
         ORDER BY fee_cents, id",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load zones: {e}")))?
    .into_iter()
    .map(|(id, name, fee, min_order, eta)| DeliveryZone {
        name: name.unwrap_or_else(|| id.clone()),
        id,
        fee_cents: fee,
        min_order_cents: min_order,
        eta_minutes: eta,
    })
    .collect();

    let free_delivery_threshold_cents =
        crate::pages::settings::ssr::free_delivery_threshold_cents(&db).await;

    // Snapshot the bits of branding we need into the resource payload so
    // the form's SSR + hydrate render identical DOM. Reading
    // `BrandingHandle` directly inside the component would diverge —
    // SSR sees the cached values, hydrate sees `Branding::default()`,
    // and the resulting `<input value=…>` / conditional `<p>` blocks
    // mismatch → tachys hydration panics.
    let branding = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get())
        .unwrap_or_default();

    Ok(CheckoutContext {
        slots,
        asap_minutes: ssr::ASAP_DEFAULT_MIN,
        subtotal_label: format_eur(cart.subtotal_cents),
        subtotal_cents: cart.subtotal_cents,
        line_count: cart.lines.iter().map(|l| l.quantity).sum(),
        delivery_zones: zones,
        free_delivery_threshold_cents,
        shop_address_line: branding.full_address(),
        shop_city: branding.shop_city,
    })
}

/// `#[allow(clippy::too_many_arguments)]`: server-fn args mirror the flat
/// checkout form fields (name, phone, email, pickup time, payment method,
/// order type, address parts). Bundling into a struct would break the
/// `<ActionForm>`'s URL-encoded wire format with no caller benefit.
#[server(
    name = PlaceOrder,
    prefix = "/api",
    endpoint = "place_order"
)]
#[allow(clippy::too_many_arguments)]
pub async fn place_order(
    name: String,
    phone: String,
    email: String,
    pickup_time: String,    // "asap" | "HH:MM"
    payment_method: String, // "cash" | "card"
    order_type: String,     // "pickup" | "delivery"
    delivery_zone_id: Option<String>,
    street: Option<String>,
    house_number: Option<String>,
    postcode: Option<String>,
    city: Option<String>,
    address_notes: Option<String>,
    voucher_code: Option<String>,
) -> Result<PlacedOrder, ServerFnError> {
    use crate::pages::cart::ssr::{current_cart_id, load_cart};
    use crate::pages::order::notify::{NotifierHandle, OrderItemSummary, OrderSummary};
    use sqlx::SqlitePool;

    let name = name.trim().to_string();
    let phone = phone.trim().to_string();
    let email = email.trim().to_string();
    if name.is_empty() {
        return Err(ServerFnError::new("Bitte Name angeben"));
    }
    if phone.len() < 5 {
        return Err(ServerFnError::new("Bitte gültige Telefonnummer angeben"));
    }
    if !email.contains('@') {
        return Err(ServerFnError::new("Bitte gültige E-Mail-Adresse angeben"));
    }
    let payment_method = match payment_method.trim() {
        "card" => "card",
        _ => "cash",
    };
    let order_type = match order_type.trim() {
        "delivery" => "delivery",
        _ => "pickup",
    };

    let cart_id = current_cart_id().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let notifier = use_context::<NotifierHandle>()
        .ok_or_else(|| ServerFnError::new("notifier missing from context"))?;

    if payment_method == "cash" {
        let phone_norm = ssr::normalize_phone(&phone);
        let blocked: Option<(String,)> = sqlx::query_as(
            "SELECT blacklisted_at FROM customers \
             WHERE phone = ? AND blacklisted_at IS NOT NULL LIMIT 1",
        )
        .bind(&phone_norm)
        .fetch_optional(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("blacklist check: {e}")))?;
        if blocked.is_some() {
            return Err(ServerFnError::new(
                "Bar-Bestellungen sind für diese Nummer derzeit nicht möglich. Bitte wählen Sie Online-Zahlung oder kontaktieren Sie uns.",
            ));
        }
    }

    let cart = load_cart(&db, &cart_id).await?;
    if cart.lines.is_empty() {
        return Err(ServerFnError::new("Warenkorb ist leer"));
    }
    let subtotal = cart.subtotal_cents;

    // Delivery: re-run the geocoder + classifier on the server, regardless of
    // the client-supplied zone hint. Phase 2: address derives the zone, the
    // dropdown is now purely cosmetic on the wire.
    let (delivery_fee, delivery_address_json, delivery_zone_name, resolved_geo) = if order_type
        == "delivery"
    {
        let street_v = street.unwrap_or_default().trim().to_string();
        let house_v = house_number.unwrap_or_default().trim().to_string();
        let postcode_v = postcode.unwrap_or_default().trim().to_string();
        let fallback_city = use_context::<crate::branding::BrandingHandle>()
            .map(|h| h.get().shop_city)
            .unwrap_or_default();
        let city_v = city
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback_city);
        if street_v.is_empty() || house_v.is_empty() || postcode_v.is_empty() {
            return Err(ServerFnError::new(
                "Bitte Straße, Hausnummer und PLZ angeben",
            ));
        }

        let v = validate_address(
            street_v.clone(),
            house_v.clone(),
            postcode_v.clone(),
            city_v.clone(),
        )
        .await?;
        if !v.ok {
            return Err(ServerFnError::new(
                v.reason.unwrap_or_else(|| "Adresse ungültig.".to_string()),
            ));
        }
        let zone_id_resolved = v.zone_id.clone().unwrap_or_default();
        let zname = v.zone_name.clone();
        let zone_fee_cents = v.fee_cents.unwrap_or(0);
        let min_order_cents = v.min_order_cents.unwrap_or(0);
        let _ = delivery_zone_id; // hint from client, intentionally ignored

        if subtotal < min_order_cents {
            let missing = min_order_cents - subtotal;
            return Err(ServerFnError::new(format!(
                "Mindestbestellwert für Lieferung: {}. Es fehlen noch {}.",
                format_eur(min_order_cents),
                format_eur(missing),
            )));
        }

        // Free-delivery promo: if the admin-set threshold is reached,
        // waive the zone fee entirely. We re-read the live setting
        // here (cheap; it's one indexed PK lookup) so a price change
        // applies immediately to in-flight orders.
        let free_threshold = crate::pages::settings::ssr::free_delivery_threshold_cents(&db).await;
        let fee_cents = if free_threshold > 0 && subtotal >= free_threshold {
            0
        } else {
            zone_fee_cents
        };

        let notes = address_notes
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let address = serde_json::json!({
            "zone_id": zone_id_resolved,
            "zone_name": zname,
            "street": street_v,
            "house_number": house_v,
            "postcode": postcode_v,
            "city": city_v,
            "notes": notes,
        })
        .to_string();
        let geo = ssr::AddressGeo {
            zone_id: zone_id_resolved,
            latitude: v.latitude,
            longitude: v.longitude,
        };
        (fee_cents, Some(address), zname, Some(geo))
    } else {
        (0_i64, None, None, None)
    };

    let total = subtotal + delivery_fee;

    // Resolve pickup time
    let scheduled_for = if pickup_time == "asap" || pickup_time.is_empty() {
        None
    } else {
        let t = chrono::NaiveTime::parse_from_str(&pickup_time, "%H:%M")
            .map_err(|_| ServerFnError::new("Ungültiges Zeitformat"))?;
        let day = ssr::now_local().date_naive();
        Some(day.and_time(t))
    };

    let order_id = uuid::Uuid::new_v4().to_string();
    let order_number = ssr::next_order_number(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("order number: {e}")))?;
    let pickup_label = ssr::pickup_label(scheduled_for.as_ref());

    // Compute the voucher discount PRE-flight so we can:
    //   (a) decide initial_status/payment_status with the real total
    //   (b) INSERT INTO orders with the post-voucher total in one shot
    //   (c) skip Stripe entirely when voucher covers everything
    //
    // We use `evaluate` (read-only) here. The full `redeem` (which
    // writes the voucher_redemptions row) runs inside the txn after
    // the order INSERT — it re-validates against committed state so
    // caps still hold under concurrent submits.
    let phone_norm_preview = ssr::normalize_phone(&phone);
    let voucher_preview_discount: i64 = if let Some(raw_code) = voucher_code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let code_norm = crate::pages::vouchers::normalise_code(raw_code);
        match crate::pages::vouchers::ssr::load_active_voucher(&db, &code_norm).await {
            Ok(v) => {
                let preview = crate::pages::vouchers::ssr::evaluate(
                    &db,
                    &v,
                    &phone_norm_preview,
                    subtotal,
                    delivery_fee,
                )
                .await?;
                if preview.applies {
                    preview.discount_cents
                } else {
                    // The voucher exists but doesn't apply (rule
                    // failure). Surface the reason so the customer
                    // sees why it was rejected, mirroring what the
                    // live validator already shows on the checkout
                    // form.
                    return Err(ServerFnError::new(preview.reason_de));
                }
            }
            Err(e) => return Err(e),
        }
    } else {
        0
    };
    let total_after_voucher = (total - voucher_preview_discount).max(0);

    // Cash orders are immediately "received" in the kitchen queue. Card orders
    // start as "pending_payment" and only move to "received" after the webhook
    // confirms payment. payment_status mirrors that distinction.
    //
    // Special case: a voucher that drops total_cents to 0 (e.g. fixed-25€
    // voucher on a 24€ cart, or 100% percent) means there's nothing for the
    // customer to pay. We treat that like a cash-on-pickup order — straight
    // to the kitchen, no Stripe Intent. Without this branch the customer
    // would face a Stripe Payment Element with a €0 amount, which is invalid.
    let total_after_voucher_is_zero = total_after_voucher == 0;
    let (initial_status, initial_payment_status) =
        if payment_method == "card" && !total_after_voucher_is_zero {
            ("pending_payment", "pending")
        } else if total_after_voucher_is_zero && voucher_preview_discount > 0 {
            // Order is effectively free — fully covered by voucher. Mark as
            // "received" so the kitchen sees it; payment_status='voucher_paid'
            // distinguishes it from cash-on-pickup for the Buchhaltung.
            ("received", "voucher_paid")
        } else {
            ("received", "cash_on_pickup")
        };

    // Atomic write: customers + customer_addresses + orders + order_items,
    // then clear cart.
    let mut tx = db
        .begin()
        .await
        .map_err(|e| ServerFnError::new(format!("begin tx: {e}")))?;

    // Upsert the customer keyed by phone. Phone is normalised lightly so
    // "0 2591 / 12 34" and "025911234" land on the same row. We keep the
    // *raw* phone on the order row for receipts; only the customers table
    // uses the normalised form.
    let phone_norm = ssr::normalize_phone(&phone);
    let customer_id = ssr::upsert_customer(&mut tx, &phone_norm, &name, &email).await?;

    // For delivery, upsert the address against this customer. JSON snapshot
    // on the order remains the source of truth for the printed receipt; the
    // structured row carries lat/lon/zone for Phase 3 routing + future cache
    // hits.
    let address_id: Option<String> = if order_type == "delivery" {
        if let Some(snapshot) = delivery_address_json.as_deref() {
            let v: serde_json::Value = serde_json::from_str(snapshot).unwrap_or_default();
            let street = v.get("street").and_then(|x| x.as_str()).unwrap_or("");
            let house = v.get("house_number").and_then(|x| x.as_str()).unwrap_or("");
            let plz = v.get("postcode").and_then(|x| x.as_str()).unwrap_or("");
            let city = v.get("city").and_then(|x| x.as_str()).unwrap_or("");
            let notes = v.get("notes").and_then(|x| x.as_str());
            let id = ssr::upsert_address(
                &mut tx,
                &customer_id,
                ssr::AddressInput {
                    street,
                    house_number: house,
                    postcode: plz,
                    city,
                    notes,
                },
                resolved_geo.clone(),
            )
            .await?;
            Some(id)
        } else {
            None
        }
    } else {
        None
    };

    // Snapshot the active Stripe mode at order-creation time. Orders
    // that pay through Stripe later overwrite this from the payment-
    // intent's mode; voucher_paid and cash_on_pickup orders never
    // go through that path, so without this snapshot they'd inherit
    // the schema's `DEFAULT 'sandbox'` and get flagged as TEST
    // everywhere (kitchen ticket banner, admin order list, stats
    // exclusion in Buchhaltung) — even when the shop is in live mode.
    let stripe_mode_snapshot: String = use_context::<crate::stripe::StripeModeHandle>()
        .map(|h| h.get().as_setting().to_string())
        .unwrap_or_else(|| "sandbox".to_string());

    sqlx::query(
        "INSERT INTO orders (id, order_number, order_type, status,
            contact_name, contact_phone, contact_email,
            delivery_address_json, scheduled_for,
            subtotal_cents, delivery_fee_cents, tax_cents, total_cents,
            payment_status, notes,
            customer_id, address_id, stripe_mode)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, ?12, ?13, NULL, ?14, ?15, ?16)",
    )
    .bind(&order_id)
    .bind(&order_number)
    .bind(order_type)
    .bind(initial_status)
    .bind(&name)
    .bind(&phone)
    .bind(&email)
    .bind(&delivery_address_json)
    .bind(scheduled_for.map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()))
    .bind(subtotal)
    .bind(delivery_fee)
    .bind(total_after_voucher) // post-voucher total — Stripe-branch sees correct amount
    .bind(initial_payment_status)
    .bind(&customer_id)
    .bind(address_id.as_deref())
    .bind(&stripe_mode_snapshot)
    .execute(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("insert order: {e}")))?;
    let _ = delivery_zone_name; // reserved for the email body when we wire that branch later

    // Voucher redeem. Total + status are already correct from the
    // pre-flight evaluate; this call only re-validates inside the txn
    // (caps need to read committed state under concurrent submits) and
    // writes the voucher_redemptions row + snapshot columns onto the
    // order. The discount it returns SHOULD equal what we computed
    // pre-INSERT; we assert that to catch any divergence.
    let mut voucher_discount: i64 = 0;
    let mut voucher_discount_items: i64 = 0;
    let mut voucher_discount_delivery: i64 = 0;
    let mut voucher_meta: Option<(String, String)> = None; // (id, code)
    if let Some(raw_code) = voucher_code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let red = crate::pages::vouchers::ssr::redeem(
            &mut tx,
            raw_code,
            &phone_norm,
            subtotal,
            delivery_fee,
            &order_id,
        )
        .await?;
        if red.discount_cents != voucher_preview_discount {
            // Pre-flight evaluate said X, in-txn redeem said Y. Means
            // a concurrent redemption hit a cap between the two reads.
            // Reject conservatively — the customer will retry.
            return Err(ServerFnError::new(
                "Gutschein-Stand hat sich gerade geändert — bitte Bestellung erneut absenden.",
            ));
        }
        voucher_discount = red.discount_cents;
        voucher_discount_items = red.discount_items_cents;
        voucher_discount_delivery = red.discount_delivery_cents;
        voucher_meta = Some((red.voucher_id, red.code));
    }
    if voucher_discount > 0 {
        let (vid, vcode) = voucher_meta.as_ref().expect("set with discount").clone();
        // Snapshot only the voucher columns — total_cents was already
        // written with the post-voucher value in the INSERT above.
        // voucher_discount_cents stays as the total (= items + delivery)
        // for kitchen-protocol / printer compat; the split columns are
        // what Buchhaltung uses to attribute revenue correctly.
        sqlx::query(
            "UPDATE orders SET voucher_id = ?2, voucher_code = ?3,
                               voucher_discount_cents = ?4,
                               voucher_discount_items_cents = ?5,
                               voucher_discount_delivery_cents = ?6
             WHERE id = ?1",
        )
        .bind(&order_id)
        .bind(&vid)
        .bind(&vcode)
        .bind(voucher_discount)
        .bind(voucher_discount_items)
        .bind(voucher_discount_delivery)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("apply voucher: {e}")))?;
    }
    let total = total_after_voucher;

    for line in &cart.lines {
        let line_id = uuid::Uuid::new_v4().to_string();
        let opts = serde_json::json!({
            "size": line.size.label(),
            "size_label": line.size_label,
        })
        .to_string();
        // Carry the picked extras snapshot from cart line → order line so the
        // kitchen ticket and customer invoice see exactly what the customer
        // ticked at the time of the order.
        let extras_json = if line.extras.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&line.extras)
                    .map_err(|e| ServerFnError::new(format!("serialise extras: {e}")))?,
            )
        };
        // Same shape, but for required-choice option groups (dressing
        // etc.). The cart already validated min/max at add-time; this
        // is just the snapshot.
        let selected_options_json = if line.selected_options.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&line.selected_options)
                    .map_err(|e| ServerFnError::new(format!("serialise options: {e}")))?,
            )
        };
        sqlx::query(
            "INSERT INTO order_items
                (id, order_id, menu_item_id, name_snapshot, menu_number_snapshot,
                 quantity, options_json, unit_price_cents, line_total_cents,
                 extras_json, selected_options_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .bind(&line_id)
        .bind(&order_id)
        .bind(&line.menu_item_id)
        .bind(&line.name)
        .bind(line.menu_number.as_deref())
        .bind(line.quantity)
        .bind(&opts)
        .bind(line.unit_price_cents)
        .bind(line.line_total_cents)
        .bind(extras_json.as_deref())
        .bind(selected_options_json.as_deref())
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("insert order_item: {e}")))?;
    }

    sqlx::query("DELETE FROM cart_items WHERE cart_id = ?1")
        .bind(&cart_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("clear cart: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| ServerFnError::new(format!("commit: {e}")))?;

    let public_url = std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".into());
    let detail_url = format!("{public_url}/orders/{order_id}");

    let delivery_address_text = if order_type == "delivery" {
        delivery_address_json
            .as_deref()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
            .map(|v| {
                let street = v.get("street").and_then(|x| x.as_str()).unwrap_or("");
                let house = v.get("house_number").and_then(|x| x.as_str()).unwrap_or("");
                let plz = v.get("postcode").and_then(|x| x.as_str()).unwrap_or("");
                let city = v.get("city").and_then(|x| x.as_str()).unwrap_or("");
                let zone = v.get("zone_name").and_then(|x| x.as_str()).unwrap_or("");
                let notes = v
                    .get("notes")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty());
                let mut s = format!("  {street} {house}\n  {plz} {city}\n  Liefergebiet: {zone}");
                if let Some(n) = notes {
                    s.push_str(&format!("\n  Hinweis: {n}"));
                }
                s
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Snapshot branding into the summary so the email-sender tokio task
    // can render shop name / address / phone without re-reading the DB.
    let branding_snapshot = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get())
        .unwrap_or_default();

    let summary = OrderSummary {
        order_number: order_number.clone(),
        contact_name: name.clone(),
        contact_phone: phone.clone(),
        contact_email: email.clone(),
        pickup_time_label: pickup_label,
        total_cents: total,
        items: cart
            .lines
            .iter()
            .map(|l| OrderItemSummary {
                quantity: l.quantity,
                menu_number: l.menu_number.clone(),
                name: l.name.clone(),
                variant: l.size_label.clone(),
                line_total_cents: l.line_total_cents,
                extras: l.extras.clone(),
            })
            .collect(),
        public_url: detail_url,
        payment_method: payment_method.to_string(),
        order_type: order_type.to_string(),
        delivery_address_text,
        delivery_fee_cents: delivery_fee,
        subtotal_cents: subtotal,
        voucher_code: voucher_meta
            .as_ref()
            .map(|(_, c)| c.clone())
            .unwrap_or_default(),
        voucher_discount_cents: voucher_discount,
        branding: branding_snapshot.clone(),
    };

    if payment_method == "card" && !total_after_voucher_is_zero {
        // Resolve the active Stripe key set RIGHT BEFORE creating
        // the intent and snapshot the mode onto the order row. A
        // mid-checkout admin flip won't strand this order: the
        // webhook handler matches by the row's snapshot, not the
        // current active mode.
        let stripe_keys = use_context::<crate::stripe::StripeModeHandle>()
            .map(|h| h.active_keys())
            .ok_or_else(|| ServerFnError::new("StripeModeHandle missing from context"))?;
        if !stripe_keys.is_complete() {
            return Err(ServerFnError::new(format!(
                "Stripe-Schlüssel für Modus '{}' unvollständig — \
                 bitte {prefix}STRIPE_PUBLISH_KEY, {prefix}STRIPE_SECRET_KEY \
                 und {prefix}STRIPE_WEBHOOK_SECRET in .env setzen.",
                stripe_keys.mode.label_de(),
                prefix = stripe_keys.mode.env_prefix(),
            )));
        }

        let shop_name = branding_snapshot.display_name();
        let (intent_id, client_secret) = ssr::create_payment_intent(
            &order_id,
            &order_number,
            total,
            &email,
            &shop_name,
            &stripe_keys.secret_key,
        )
        .await?;

        // Persist the intent id AND the mode it was minted in. The
        // mode snapshot is the durable record — the webhook handler
        // uses it to pick the right signing secret, even after the
        // admin flips the active mode.
        sqlx::query(
            "UPDATE orders SET stripe_payment_intent_id = ?1, stripe_mode = ?2 WHERE id = ?3",
        )
        .bind(&intent_id)
        .bind(stripe_keys.mode.as_setting())
        .bind(&order_id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("save intent id: {e}")))?;

        // Email is deferred — we only confirm by email once payment succeeds
        // (in the webhook). Returning client_secret tells the browser to
        // mount the Payment Element.
        return Ok(PlacedOrder {
            id: order_id,
            order_number,
            stripe_client_secret: Some(client_secret),
            stripe_publishable_key: Some(stripe_keys.publish_key),
        });
    }

    // Cash path: push staff (kitchen + admin always, driver on delivery)
    // and send the confirmation email. Card orders get the same push from
    // the Stripe webhook once payment_intent.succeeded fires.
    if let Some(sink) = use_context::<crate::pages::push::BroadcastSinkHandle>().flatten() {
        let push_body = format!(
            "{name} · {total} · {n} Pos.",
            name = summary.contact_name,
            total = rusterando_shared::models::format_eur(summary.total_cents),
            n = summary.items.len(),
        );
        let push_title = format!("Neue Bestellung {}", summary.order_number);
        let mut roles: Vec<&str> = vec!["kitchen", "admin"];
        if summary.order_type == "delivery" {
            roles.push("driver");
        }
        sink.notify_roles(&roles, &push_title, &push_body, Some(&summary.public_url));
    }

    // Cash path: also fire the kitchen-printer broadcast so the Pi
    // prints the receipt. Card orders get their broadcast from the
    // Stripe webhook AFTER payment succeeds (see stripe_webhook_handler
    // in main.rs) — we don't want to print a receipt for an order
    // that may end up declined.
    //
    // Trait-erased handle so this crate doesn't need to depend on
    // rusterando-server / kitchen-protocol directly (the server crate
    // provides the impl + DB→DTO translation; we just pass the order
    // id). `use_context::<Option<_>>` returns None on printerless
    // deploys; flatten short-circuits cleanly.
    if let Some(k) = use_context::<crate::pages::push::KitchenSinkHandle>().flatten() {
        if let Err(e) = k.broadcast_new_order(&db, &order_id).await {
            log::warn!("[order {order_number}] kitchen broadcast failed: {e}");
        }
    }

    let order_number_for_log = summary.order_number.clone();
    tokio::spawn(async move {
        if let Err(e) = notifier.send_confirmation(&summary).await {
            log::warn!("[order {order_number_for_log}] notifier failed: {e}");
        } else {
            log::info!("[order {order_number_for_log}] confirmation sent");
        }
    });

    leptos_axum::redirect(&format!("/orders/{order_id}"));
    Ok(PlacedOrder {
        id: order_id,
        order_number,
        stripe_client_secret: None,
        stripe_publishable_key: None,
    })
}

#[server(
    name = GetOrder,
    prefix = "/api",
    endpoint = "get_order"
)]
pub async fn get_order(id: String) -> Result<OrderDetail, ServerFnError> {
    use sqlx::SqlitePool;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let row: OrderRow = sqlx::query_as(
        "SELECT id, order_number, status, contact_name, contact_phone, contact_email,
                scheduled_for, subtotal_cents, total_cents, created_at, payment_status,
                order_type, delivery_fee_cents, delivery_address_json,
                payment_method_detail
         FROM orders WHERE id = ?1",
    )
    .bind(&id)
    .fetch_optional(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load order: {e}")))?
    .ok_or_else(|| ServerFnError::new("Bestellung nicht gefunden"))?;

    let item_rows = sqlx::query_as::<
        _,
        (
            Option<String>,
            String,
            String,
            i64,
            i64,
            i64,
            Option<String>,
        ),
    >(
        "SELECT menu_number_snapshot, name_snapshot, options_json,
                quantity, unit_price_cents, line_total_cents, extras_json
         FROM order_items WHERE order_id = ?1
         ORDER BY id",
    )
    .bind(&id)
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load items: {e}")))?;

    let items = item_rows
        .into_iter()
        .map(|(num, name, opts, qty, unit, total, extras_json)| {
            let v: serde_json::Value = serde_json::from_str(&opts).unwrap_or_default();
            let variant = v
                .get("size_label")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            let extras: Vec<rusterando_shared::models::CartExtra> = extras_json
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_default();
            OrderDetailItem {
                menu_number: num.filter(|s| !s.is_empty()),
                name,
                variant,
                quantity: qty,
                unit_price_cents: unit,
                line_total_cents: total,
                extras,
            }
        })
        .collect();

    let pickup_label = match row.6.as_deref() {
        Some(s) => format!("Heute {}", s.split_whitespace().nth(1).unwrap_or(s)),
        None => "ASAP".to_string(),
    };

    let delivery_address = row.13.as_deref().and_then(parse_delivery_address);

    Ok(OrderDetail {
        id: row.0,
        order_number: row.1,
        status: row.2,
        contact_name: row.3,
        contact_phone: row.4,
        contact_email: row.5,
        pickup_time_label: pickup_label,
        items,
        subtotal_cents: row.7,
        total_cents: row.8,
        created_at: row.9,
        payment_status: row.10,
        payment_method_detail: row.14,
        order_type: row.11,
        delivery_fee_cents: row.12,
        delivery_address,
    })
}

/// Phase 1 — phone recall. Looks up a customer by exact phone match and
/// returns their saved name/email plus addresses (most-recent first).
///
/// Privacy:
/// - phone is normalised the same way upsert does, so customers find their
///   own row regardless of formatting
/// - exact-match only, no prefix search, no enumeration aid
/// - returns Ok(None) for unknown numbers — never leaks "this number was
///   tried but doesn't exist" vs "rate limited" via different shapes
/// - rate-limited at the route layer (5 req/min/IP) by the server crate
#[server(
    name = LookupCustomerByPhone,
    prefix = "/api",
    endpoint = "lookup_customer_by_phone"
)]
pub async fn lookup_customer_by_phone(
    phone: String,
) -> Result<Option<CustomerSnapshot>, ServerFnError> {
    use sqlx::SqlitePool;

    let phone_norm = ssr::normalize_phone(&phone);
    if phone_norm.chars().filter(|c| c.is_ascii_digit()).count() < 6 {
        // Too short to be a real number — don't bother the DB.
        return Ok(None);
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let customer: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, name, email FROM customers
         WHERE phone = ?1 AND blacklisted_at IS NULL",
    )
    .bind(&phone_norm)
    .fetch_optional(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("lookup customer: {e}")))?;

    let Some((customer_id, name, email)) = customer else {
        return Ok(None);
    };

    let address_rows: Vec<SavedAddressRow> = sqlx::query_as(
        "SELECT id, street, house_number, postcode, city, notes,
                delivery_zone_id, last_used_at
         FROM customer_addresses
         WHERE customer_id = ?1
         ORDER BY last_used_at DESC
         LIMIT 5",
    )
    .bind(&customer_id)
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("lookup addresses: {e}")))?;

    Ok(Some(CustomerSnapshot {
        name: name.unwrap_or_default(),
        email: email.unwrap_or_default(),
        addresses: address_rows
            .into_iter()
            .map(
                |(id, street, house_number, postcode, city, notes, zone, last_used)| SavedAddress {
                    id,
                    street,
                    house_number,
                    postcode,
                    city,
                    notes,
                    delivery_zone_id: zone,
                    last_used_at: last_used,
                },
            )
            .collect(),
    }))
}

/// Phase 2 — geocode an address against Nominatim, classify it into one of
/// our zones, and return the customer-facing fee + min-order. The frontend
/// calls this whenever the address fields settle, and gates the submit
/// button on `ok`.
///
/// Always re-validated server-side in `place_order`; this server-fn is
/// purely for UX feedback.
#[server(
    name = ValidateAddress,
    prefix = "/api",
    endpoint = "validate_address"
)]
pub async fn validate_address(
    street: String,
    house_number: String,
    postcode: String,
    city: String,
) -> Result<AddressValidation, ServerFnError> {
    use sqlx::SqlitePool;

    let street = street.trim().to_string();
    let house_number = house_number.trim().to_string();
    let postcode = postcode.trim().to_string();
    let city_trim = city.trim();
    let city_eff = if city_trim.is_empty() {
        use_context::<crate::branding::BrandingHandle>()
            .map(|h| h.get().shop_city)
            .unwrap_or_default()
    } else {
        city_trim.to_string()
    };

    if street.is_empty() || house_number.is_empty() || postcode.is_empty() {
        return Ok(AddressValidation {
            ok: false,
            reason: Some("Bitte Straße, Hausnummer und PLZ angeben.".to_string()),
            ..Default::default()
        });
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Cache-by-DB: if we've already geocoded this (street, house, postcode),
    // reuse the lat/lon/zone we stored last time. We don't scope by customer
    // here — the resolution itself is identical across customers, so first
    // hit warms the cache for everyone.
    let cached: Option<AddressCacheRow> = sqlx::query_as(
        "SELECT latitude, longitude, suburb, delivery_zone_id
             FROM customer_addresses
             WHERE street = ?1 AND house_number = ?2 AND postcode = ?3
                   AND latitude IS NOT NULL AND longitude IS NOT NULL
                   AND delivery_zone_id IS NOT NULL
             ORDER BY geocoded_at DESC LIMIT 1",
    )
    .bind(&street)
    .bind(&house_number)
    .bind(&postcode)
    .fetch_optional(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("address cache: {e}")))?;

    let (lat, lon, _suburb, zone_id) = match cached {
        Some((Some(lat), Some(lon), suburb, Some(zid))) => (lat, lon, suburb, zid),
        _ => {
            // Cache miss — call Nominatim.
            let hit = ssr::nominatim_lookup(&street, &house_number, &postcode, &city_eff)
                .await
                .map_err(|e| {
                    // Hard reject when the geocoder is down; the customer sees
                    // a helpful message rather than a 500.
                    ServerFnError::new(format!(
                        "Adressprüfung gerade nicht möglich, bitte gleich nochmal versuchen ({e})"
                    ))
                })?;

            let Some(found) = hit else {
                return Ok(AddressValidation {
                    ok: false,
                    reason: Some(
                        "Adresse konnte nicht gefunden werden. Bitte Straße/Hausnummer prüfen."
                            .to_string(),
                    ),
                    ..Default::default()
                });
            };
            let lat: f64 = found
                .lat
                .parse()
                .map_err(|_| ServerFnError::new("geocoder returned invalid latitude"))?;
            let lon: f64 = found
                .lon
                .parse()
                .map_err(|_| ServerFnError::new("geocoder returned invalid longitude"))?;
            let suburb_resolved = found.address.suburb_or_district().map(String::from);
            let resolved_city = found.address.place().map(String::from);

            // The customer's city-field input takes precedence when it
            // names a known Stadtteil. Reason: geocoders sometimes
            // normalise sub-areas under the parent city, so without
            // honouring the customer's hint we'd misclassify them all
            // as the base zone.
            let expected_city = use_context::<crate::branding::BrandingHandle>()
                .map(|h| h.get().shop_city)
                .unwrap_or_default();
            let effective_suburb = if !expected_city.is_empty()
                && !city_eff.eq_ignore_ascii_case(&expected_city)
                && suburb_resolved.is_none()
            {
                Some(city_eff.clone())
            } else {
                suburb_resolved.clone()
            };

            let Some(zid) = ssr::classify_zone(
                lat,
                lon,
                effective_suburb.as_deref(),
                resolved_city.as_deref(),
                &expected_city,
            ) else {
                return Ok(AddressValidation {
                    ok: false,
                    reason: Some(
                        "Diese Adresse liegt außerhalb unseres Liefergebiets.".to_string(),
                    ),
                    ..Default::default()
                });
            };
            (lat, lon, effective_suburb, zid.to_string())
        }
    };

    let zone: Option<(String, Option<String>, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, name, fee_cents, min_order_cents, eta_minutes
         FROM delivery_zones WHERE id = ?1 AND is_active = 1",
    )
    .bind(&zone_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("zone lookup: {e}")))?;

    let Some((zid, zname, fee, min_order, eta)) = zone else {
        let phone = use_context::<crate::branding::BrandingHandle>()
            .map(|h| h.get().shop_phone)
            .unwrap_or_default();
        let reason = if phone.is_empty() {
            "Zone derzeit nicht verfügbar — bitte rufen Sie uns an.".to_string()
        } else {
            format!("Zone derzeit nicht verfügbar — bitte rufen Sie uns an: {phone}.")
        };
        return Ok(AddressValidation {
            ok: false,
            reason: Some(reason),
            ..Default::default()
        });
    };

    Ok(AddressValidation {
        ok: true,
        reason: None,
        zone_id: Some(zid),
        zone_name: zname.clone(),
        fee_cents: Some(fee),
        min_order_cents: Some(min_order),
        eta_minutes: Some(eta),
        latitude: Some(lat),
        longitude: Some(lon),
        matched_label: zname,
    })
}

/// Phase 3 — start a delivery tour. Driver passes the order_ids they want to
/// bundle; we load addresses, validate state, optimise via ORS (with
/// oldest-first fallback when ORS is unreachable), persist the tour + stops,
/// and flip each order to `out_for_delivery`. Returns the full tour view so
/// the driver UI can immediately render the optimised stop list.
#[server(
    name = StartTour,
    prefix = "/api",
    endpoint = "start_tour"
)]
pub async fn start_tour(
    order_ids: Vec<String>,
    driver_name: Option<String>,
) -> Result<TourSummary, ServerFnError> {
    use sqlx::SqlitePool;

    if order_ids.is_empty() {
        return Err(ServerFnError::new(
            "Bitte mindestens eine Bestellung wählen.",
        ));
    }

    let role = crate::pages::session::ssr::current_role().await;
    if !matches!(
        role,
        Some(crate::pages::session::Role::Driver) | Some(crate::pages::session::Role::Admin)
    ) {
        return Err(ServerFnError::new("Nicht autorisiert."));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Build placeholders for an IN (?,?,?) clause; we re-use them for both
    // the validation read and the eventual UPDATE.
    let placeholders = order_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");

    // Pull just the bits we need to validate + optimise. Address geo comes
    // from the JSON snapshot OR from the linked customer_addresses row,
    // since either may be the one with lat/lon (Phase 2 backfills the
    // structured row).
    let q = format!(
        "SELECT o.id, o.order_number, o.status, o.payment_status,
                o.contact_name, o.contact_phone,
                o.total_cents, o.delivery_address_json, o.created_at,
                ca.latitude, ca.longitude
         FROM orders o
         LEFT JOIN customer_addresses ca ON ca.id = o.address_id
         WHERE o.id IN ({placeholders})
         ORDER BY o.created_at ASC"
    );
    let mut query = sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            String,
            String,
            String,
            i64,
            Option<String>,
            String,
            Option<f64>,
            Option<f64>,
        ),
    >(&q);
    for id in &order_ids {
        query = query.bind(id);
    }
    let rows = query
        .fetch_all(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("load orders: {e}")))?;

    if rows.len() != order_ids.len() {
        return Err(ServerFnError::new(
            "Mindestens eine Bestellung wurde nicht gefunden.",
        ));
    }

    // Validate each candidate. We require the order to be ready_for_pickup
    // and either paid online or cash-on-pickup. Stops without lat/lon fall
    // back to the pizzeria coordinates (so the optimiser doesn't choke); the
    // driver still sees the address line and can navigate manually.
    let (piz_lat, piz_lon) = ssr::pizzeria_coords();
    let mut stops: Vec<(String, f64, f64)> = Vec::with_capacity(rows.len());
    let mut details: std::collections::HashMap<String, _> = std::collections::HashMap::new();

    for (id, num, status, pay, name, phone, total, addr_json, created_at, lat, lon) in rows {
        if status != "ready_for_pickup" {
            return Err(ServerFnError::new(format!(
                "Bestellung {num} ist nicht 'Abholbereit' (aktuell: {status})."
            )));
        }
        // Tour-eligible payment states:
        //   paid           — Stripe (or other online) completed
        //   cash_on_pickup — driver collects on arrival
        //   voucher_paid   — fully covered by a voucher, no money owed
        if !matches!(pay.as_str(), "paid" | "cash_on_pickup" | "voucher_paid") {
            return Err(ServerFnError::new(format!(
                "Bestellung {num} hat einen unklaren Zahlungsstatus ({pay})."
            )));
        }

        // Parse the delivery_address_json snapshot for the human-readable line.
        let (addr_line, city, fallback_lat, fallback_lon) = if let Some(j) = addr_json.as_deref() {
            let v: serde_json::Value = serde_json::from_str(j).unwrap_or_default();
            let s = v.get("street").and_then(|x| x.as_str()).unwrap_or("");
            let h = v.get("house_number").and_then(|x| x.as_str()).unwrap_or("");
            let p = v.get("postcode").and_then(|x| x.as_str()).unwrap_or("");
            let c = v.get("city").and_then(|x| x.as_str()).unwrap_or("");
            (
                format!("{s} {h}, {p} {c}").trim().to_string(),
                c.to_string(),
                None::<f64>,
                None::<f64>,
            )
        } else {
            return Err(ServerFnError::new(format!(
                "Bestellung {num} ist eine Abholung, nicht für die Tour geeignet."
            )));
        };
        let _ = (fallback_lat, fallback_lon);

        let stop_lat = lat.unwrap_or(piz_lat);
        let stop_lon = lon.unwrap_or(piz_lon);
        stops.push((id.clone(), stop_lat, stop_lon));
        details.insert(
            id,
            (
                num, name, phone, total, addr_line, city, lat, lon, pay, created_at,
            ),
        );
    }

    // ORS or fallback. Either way we get a Vec<OptimisedStop> in delivery order.
    let optimised = ssr::optimise_tour(&stops).await;

    let tour_id = uuid::Uuid::new_v4().to_string();
    let mut tx = db
        .begin()
        .await
        .map_err(|e| ServerFnError::new(format!("begin tx: {e}")))?;

    sqlx::query(
        "INSERT INTO delivery_tours
            (id, driver_name, total_distance_m, total_duration_s,
             optimised_by, ors_response_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&tour_id)
    .bind(driver_name.as_deref())
    .bind(optimised.total_distance_m.map(|x| x as i64))
    .bind(optimised.total_duration_s.map(|x| x as i64))
    .bind(optimised.source)
    .bind(optimised.raw_json.as_deref())
    .execute(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("insert tour: {e}")))?;

    let mut stop_views: Vec<TourStopView> = Vec::with_capacity(optimised.stops.len());
    for (i, st) in optimised.stops.iter().enumerate() {
        let seq = i as i64;
        sqlx::query(
            "INSERT INTO delivery_tour_stops
                (tour_id, sequence, order_id, eta_seconds)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&tour_id)
        .bind(seq)
        .bind(&st.order_id)
        .bind(st.eta_seconds.map(|x| x as i64))
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("insert stop: {e}")))?;

        sqlx::query(
            "UPDATE orders
             SET tour_id = ?1,
                 status = 'out_for_delivery',
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?2",
        )
        .bind(&tour_id)
        .bind(&st.order_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("update order: {e}")))?;

        if let Some((num, name, phone, total, addr_line, city, lat, lon, pay, _created_at)) =
            details.get(&st.order_id).cloned()
        {
            stop_views.push(TourStopView {
                sequence: seq,
                order_id: st.order_id.clone(),
                order_number: num,
                eta_seconds: st.eta_seconds.map(|x| x as i64),
                delivered_at: None,
                contact_name: name,
                contact_phone: phone,
                address_line: addr_line,
                city,
                latitude: lat,
                longitude: lon,
                total_cents: total,
                payment_status: pay,
            });
        }
    }

    tx.commit()
        .await
        .map_err(|e| ServerFnError::new(format!("commit tour: {e}")))?;

    Ok(TourSummary {
        id: tour_id,
        driver_name,
        started_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        finished_at: None,
        total_distance_m: optimised.total_distance_m.map(|x| x as i64),
        total_duration_s: optimised.total_duration_s.map(|x| x as i64),
        optimised_by: optimised.source.to_string(),
        stops: stop_views,
    })
}

/// Returns a tour view by id, including its current stops (with
/// `delivered_at` reflecting driver progress). Driver UI calls this to
/// refresh the board after marking a stop delivered.
#[server(
    name = GetTour,
    prefix = "/api",
    endpoint = "get_tour"
)]
pub async fn get_tour(id: String) -> Result<TourSummary, ServerFnError> {
    use sqlx::SqlitePool;

    let role = crate::pages::session::ssr::current_role().await;
    if !matches!(
        role,
        Some(crate::pages::session::Role::Driver) | Some(crate::pages::session::Role::Admin)
    ) {
        return Err(ServerFnError::new("Nicht autorisiert."));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let tour: TourRow = sqlx::query_as(
        "SELECT id, driver_name, started_at, finished_at,
                total_distance_m, total_duration_s, optimised_by
         FROM delivery_tours WHERE id = ?1",
    )
    .bind(&id)
    .fetch_optional(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load tour: {e}")))?
    .ok_or_else(|| ServerFnError::new("Tour nicht gefunden."))?;

    let stop_rows: Vec<TourStopRow> = sqlx::query_as(
        "SELECT s.sequence, s.order_id, s.eta_seconds, s.delivered_at,
                o.order_number, o.contact_name, o.contact_phone,
                o.total_cents, o.payment_status,
                o.delivery_address_json, ca.latitude, ca.longitude
         FROM delivery_tour_stops s
         JOIN orders o ON o.id = s.order_id
         LEFT JOIN customer_addresses ca ON ca.id = o.address_id
         WHERE s.tour_id = ?1
         ORDER BY s.sequence ASC",
    )
    .bind(&id)
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load stops: {e}")))?;

    let stops = stop_rows
        .into_iter()
        .map(
            |(seq, oid, eta, delivered, num, name, phone, total, pay, addr_json, lat, lon)| {
                let (line, city) = if let Some(j) = addr_json.as_deref() {
                    let v: serde_json::Value = serde_json::from_str(j).unwrap_or_default();
                    let s = v.get("street").and_then(|x| x.as_str()).unwrap_or("");
                    let h = v.get("house_number").and_then(|x| x.as_str()).unwrap_or("");
                    let p = v.get("postcode").and_then(|x| x.as_str()).unwrap_or("");
                    let c = v.get("city").and_then(|x| x.as_str()).unwrap_or("");
                    (
                        format!("{s} {h}, {p} {c}").trim().to_string(),
                        c.to_string(),
                    )
                } else {
                    (String::new(), String::new())
                };
                TourStopView {
                    sequence: seq,
                    order_id: oid,
                    order_number: num,
                    eta_seconds: eta,
                    delivered_at: delivered,
                    contact_name: name,
                    contact_phone: phone,
                    address_line: line,
                    city,
                    latitude: lat,
                    longitude: lon,
                    total_cents: total,
                    payment_status: pay,
                }
            },
        )
        .collect();

    Ok(TourSummary {
        id: tour.0,
        driver_name: tour.1,
        started_at: tour.2,
        finished_at: tour.3,
        total_distance_m: tour.4,
        total_duration_s: tour.5,
        optimised_by: tour.6,
        stops,
    })
}

/// Mark a stop delivered. Server flips the order's status to `delivered`
/// and stamps `delivered_at` on the stop. If this was the last open stop,
/// the tour itself remains open until the driver hits "Tour beenden".
#[server(
    name = TourStopDelivered,
    prefix = "/api",
    endpoint = "tour_stop_delivered"
)]
pub async fn tour_stop_delivered(
    tour_id: String,
    sequence: i64,
) -> Result<TourSummary, ServerFnError> {
    use sqlx::SqlitePool;

    let role = crate::pages::session::ssr::current_role().await;
    if !matches!(
        role,
        Some(crate::pages::session::Role::Driver) | Some(crate::pages::session::Role::Admin)
    ) {
        return Err(ServerFnError::new("Nicht autorisiert."));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let order_id: Option<(String,)> = sqlx::query_as(
        "SELECT order_id FROM delivery_tour_stops WHERE tour_id = ?1 AND sequence = ?2",
    )
    .bind(&tour_id)
    .bind(sequence)
    .fetch_optional(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("lookup stop: {e}")))?;

    let Some((order_id,)) = order_id else {
        return Err(ServerFnError::new("Stop nicht gefunden."));
    };

    let mut tx = db
        .begin()
        .await
        .map_err(|e| ServerFnError::new(format!("begin tx: {e}")))?;

    sqlx::query(
        "UPDATE delivery_tour_stops
         SET delivered_at = CURRENT_TIMESTAMP
         WHERE tour_id = ?1 AND sequence = ?2",
    )
    .bind(&tour_id)
    .bind(sequence)
    .execute(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("update stop: {e}")))?;

    sqlx::query(
        "UPDATE orders SET status = 'delivered', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
    )
    .bind(&order_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("update order: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| ServerFnError::new(format!("commit: {e}")))?;

    get_tour(tour_id).await
}

/// Pull a single stop out of an active tour and put the order back in
/// `ready_for_pickup` so the driver can re-bundle later. Used when a
/// second driver shows up and they decide to split the load differently.
///
/// Already-delivered stops are rejected — the order has left the building.
/// If this was the last live stop on the tour, we auto-finish the tour
/// so it disappears from active lists.
#[server(
    name = RemoveTourStop,
    prefix = "/api",
    endpoint = "remove_tour_stop"
)]
pub async fn remove_tour_stop(tour_id: String, sequence: i64) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    let role = crate::pages::session::ssr::current_role().await;
    if !matches!(
        role,
        Some(crate::pages::session::Role::Driver) | Some(crate::pages::session::Role::Admin)
    ) {
        return Err(ServerFnError::new("Nicht autorisiert."));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let stop: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT order_id, delivered_at FROM delivery_tour_stops
         WHERE tour_id = ?1 AND sequence = ?2",
    )
    .bind(&tour_id)
    .bind(sequence)
    .fetch_optional(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("lookup stop: {e}")))?;

    let Some((order_id, delivered_at)) = stop else {
        return Err(ServerFnError::new("Stop nicht gefunden."));
    };
    if delivered_at.is_some() {
        return Err(ServerFnError::new(
            "Bereits geliefert — kann nicht zurückgenommen werden.",
        ));
    }

    let mut tx = db
        .begin()
        .await
        .map_err(|e| ServerFnError::new(format!("begin tx: {e}")))?;

    sqlx::query("DELETE FROM delivery_tour_stops WHERE tour_id = ?1 AND sequence = ?2")
        .bind(&tour_id)
        .bind(sequence)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("delete stop: {e}")))?;

    sqlx::query(
        "UPDATE orders
         SET tour_id = NULL,
             status = 'ready_for_pickup',
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
    )
    .bind(&order_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("revert order: {e}")))?;

    // If no live stops remain on the tour, auto-finish it. A tour with
    // only delivered stops is effectively done; one with zero stops
    // total is empty and should disappear too.
    let live: Option<(i64,)> = sqlx::query_as(
        "SELECT COUNT(*) FROM delivery_tour_stops
         WHERE tour_id = ?1 AND delivered_at IS NULL",
    )
    .bind(&tour_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("count live stops: {e}")))?;

    if live.map(|(n,)| n).unwrap_or(0) == 0 {
        sqlx::query(
            "UPDATE delivery_tours SET finished_at = CURRENT_TIMESTAMP
             WHERE id = ?1 AND finished_at IS NULL",
        )
        .bind(&tour_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("auto-finish tour: {e}")))?;
    }

    tx.commit()
        .await
        .map_err(|e| ServerFnError::new(format!("commit: {e}")))?;

    Ok(())
}

/// Cancel an entire active tour: every undelivered stop's order goes
/// back to `ready_for_pickup`, stop rows are deleted, the tour is
/// marked finished. Already-delivered stops stay attached as historical
/// record (the tour is finished, not erased).
#[server(
    name = CancelTour,
    prefix = "/api",
    endpoint = "cancel_tour"
)]
pub async fn cancel_tour(tour_id: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    let role = crate::pages::session::ssr::current_role().await;
    if !matches!(
        role,
        Some(crate::pages::session::Role::Driver) | Some(crate::pages::session::Role::Admin)
    ) {
        return Err(ServerFnError::new("Nicht autorisiert."));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let mut tx = db
        .begin()
        .await
        .map_err(|e| ServerFnError::new(format!("begin tx: {e}")))?;

    let live_orders: Vec<(String,)> = sqlx::query_as(
        "SELECT order_id FROM delivery_tour_stops
         WHERE tour_id = ?1 AND delivered_at IS NULL",
    )
    .bind(&tour_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("list live: {e}")))?;

    for (order_id,) in &live_orders {
        sqlx::query(
            "UPDATE orders
             SET tour_id = NULL,
                 status = 'ready_for_pickup',
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
        )
        .bind(order_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("revert order: {e}")))?;
    }

    sqlx::query(
        "DELETE FROM delivery_tour_stops
         WHERE tour_id = ?1 AND delivered_at IS NULL",
    )
    .bind(&tour_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("delete live stops: {e}")))?;

    sqlx::query(
        "UPDATE delivery_tours SET finished_at = CURRENT_TIMESTAMP
         WHERE id = ?1 AND finished_at IS NULL",
    )
    .bind(&tour_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("finish tour: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| ServerFnError::new(format!("commit: {e}")))?;

    Ok(())
}

/// Mark the whole tour finished. Driver calls this after delivering all
/// stops; we stamp finished_at so the tour disappears from "active" lists.
#[server(
    name = FinishTour,
    prefix = "/api",
    endpoint = "finish_tour"
)]
pub async fn finish_tour(tour_id: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    let role = crate::pages::session::ssr::current_role().await;
    if !matches!(
        role,
        Some(crate::pages::session::Role::Driver) | Some(crate::pages::session::Role::Admin)
    ) {
        return Err(ServerFnError::new("Nicht autorisiert."));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    sqlx::query(
        "UPDATE delivery_tours SET finished_at = CURRENT_TIMESTAMP WHERE id = ?1 AND finished_at IS NULL",
    )
    .bind(&tour_id)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("finish tour: {e}")))?;

    Ok(())
}

#[cfg(feature = "ssr")]
fn parse_delivery_address(json: &str) -> Option<DeliveryAddress> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(DeliveryAddress {
        zone_name: v
            .get("zone_name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        street: v
            .get("street")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        house_number: v
            .get("house_number")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        postcode: v
            .get("postcode")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        city: v
            .get("city")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        notes: v.get("notes").and_then(|x| x.as_str()).map(String::from),
    })
}

pub fn payment_status_label_de(s: &str) -> &'static str {
    match s {
        "cash_on_pickup" => "Bar bei Abholung",
        "pending" => "Zahlung ausstehend",
        "paid" => "Online bezahlt ✓",
        "failed" => "Zahlung fehlgeschlagen",
        "refunded" => "Erstattet",
        _ => "—",
    }
}

/// Status code → German label
pub fn status_label_de(s: &str) -> &'static str {
    match s {
        "pending_payment" => "Zahlung ausstehend",
        "received" => "Eingegangen",
        "preparing" => "Wird zubereitet",
        "ready_for_pickup" => "Abholbereit",
        "picked_up" => "Abgeholt",
        "out_for_delivery" => "Unterwegs",
        "delivered" => "Geliefert",
        "cancelled" => "Storniert",
        _ => "Unbekannt",
    }
}

// ---------------------------------------------------------------------------
// /orders/:id confirmation
// ---------------------------------------------------------------------------

#[component]
pub fn OrderConfirmationPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let id = move || params.read().get("id").unwrap_or_default();
    let order = Resource::new(id, |id| async move { get_order(id).await });

    view! {
        <section class="order-confirm">
            <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                {move || order.get().map(|res| match res {
                    Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                    Ok(o) => view! { <ConfirmationView o/> }.into_any(),
                })}
            </Suspense>
        </section>
    }
}

#[component]
fn ConfirmationView(o: OrderDetail) -> impl IntoView {
    let status_label = status_label_de(&o.status).to_string();
    let status_attr = o.status.clone();
    let payment_label = payment_status_label_de(&o.payment_status);
    let payment_attr = o.payment_status.clone();
    let is_delivery = o.order_type == "delivery";
    let is_pending = o.payment_status == "pending";

    // Dynamic hints reference shop branding (address for cash-on-pickup,
    // phone for payment-failed). Pull once from context.
    #[cfg(feature = "ssr")]
    let branding = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get())
        .unwrap_or_default();
    #[cfg(not(feature = "ssr"))]
    let branding = crate::branding::Branding::default();
    let shop_address = branding.full_address();
    let shop_phone = branding.shop_phone.clone();

    let pickup_label_de = if is_delivery {
        "Lieferzeit"
    } else {
        "Abholung"
    };
    let cash_label_de: String = if is_delivery {
        "Bezahlung bar bei Lieferung an den Fahrer.".to_string()
    } else if shop_address.is_empty() {
        "Bezahlung bar bei Abholung.".to_string()
    } else {
        format!("Bezahlung bar bei Abholung — {shop_address}.")
    };
    let payment_failed_de: String = if shop_phone.is_empty() {
        "Zahlung fehlgeschlagen. Bitte erneut versuchen.".to_string()
    } else {
        format!(
            "Zahlung fehlgeschlagen. Bitte erneut versuchen oder telefonisch bestellen: {shop_phone}."
        )
    };
    let payment_hint: String = match o.payment_status.as_str() {
        "cash_on_pickup" => cash_label_de,
        "paid" => "Zahlung bestätigt. Bestellung ist in der Küche.".to_string(),
        "pending" => "Zahlung wird verarbeitet — diese Seite aktualisiert sich, sobald die Bestätigung da ist.".to_string(),
        "failed" => payment_failed_de,
        _ => String::new(),
    };

    view! {
        <div class="confirm-card">
            <h1>"Vielen Dank für Ihre Bestellung!"</h1>
            <p class="sub">"Bestellnummer: " <strong>{o.order_number.clone()}</strong></p>
            <div class="status-pills">
                <div class="status-pill" data-order-type={o.order_type.clone()}>
                    {if is_delivery { "🛵 Lieferung" } else { "🏪 Abholung" }}
                </div>
                <div class="status-pill" data-status=status_attr>{status_label}</div>
                <div class="status-pill" data-payment=payment_attr>{payment_label}</div>
            </div>

            <dl class="confirm-meta">
                <dt>{pickup_label_de}</dt><dd>{o.pickup_time_label.clone()}</dd>
                <dt>"Name"</dt><dd>{o.contact_name.clone()}</dd>
                <dt>"Telefon"</dt><dd>{o.contact_phone.clone()}</dd>
                <dt>"E-Mail"</dt><dd>{o.contact_email.clone()}</dd>
                {o.delivery_address.as_ref().map(|a| {
                    let line = format!("{}, {} {}", a.street_with_number(), a.postcode, a.city);
                    let zone = a.zone_name.clone();
                    let notes = a.notes.clone();
                    view! {
                        <dt>"Lieferadresse"</dt>
                        <dd>
                            {line}
                            <br/>
                            <span class="muted">{format!("Liefergebiet: {zone}")}</span>
                            {notes.map(|n| view! { <br/> <span class="muted">{n}</span> })}
                        </dd>
                    }
                })}
            </dl>

            <h2>"Bestellung"</h2>
            <ul class="confirm-items">
                {o.items.into_iter().map(|it| {
                    let extras = it.extras.clone();
                    view! {
                        <li>
                            <span class="qty">{it.quantity} "×"</span>
                            <span class="name">
                                {it.menu_number.clone().map(|n| view! {
                                    <span class="num">{n} ". "</span>
                                })}
                                {it.name.clone()}
                                {it.variant.clone().map(|v| view! { " (" {v} ")" })}
                                {(!extras.is_empty()).then(|| view! {
                                    <ul class="extras">
                                        {extras.into_iter().map(|e| {
                                            let p = if e.price_cents == 0 {
                                                "gratis".to_string()
                                            } else {
                                                format!("+{}", format_eur(e.price_cents))
                                            };
                                            view! { <li>"+ " {e.label} " " <span class="muted">{p}</span></li> }
                                        }).collect_view()}
                                    </ul>
                                })}
                            </span>
                            <span class="total">{format_eur(it.line_total_cents)}</span>
                        </li>
                    }
                }).collect_view()}
            </ul>

            {(o.delivery_fee_cents > 0).then(|| view! {
                <p class="meta-row">"Zwischensumme: " <strong>{format_eur(o.subtotal_cents)}</strong></p>
                <p class="meta-row">"Lieferzuschlag: " <strong>{format_eur(o.delivery_fee_cents)}</strong></p>
            })}
            <p class="grand-total">"Gesamt: " <strong>{format_eur(o.total_cents)}</strong></p>
            <p class="hint">
                {payment_hint}
                {o.payment_method_detail.clone().filter(|s| !s.is_empty()).map(|m| view! {
                    <span class="muted">" · " {m}</span>
                })}
            </p>
            <p>
                <a class="btn ghost" href="/menu">"Weitere Speisekarte ansehen"</a>
            </p>
            {is_pending.then(|| view! {
                // Soft refresh every 3s so a 'pending' card flips to 'paid' shortly after the webhook lands.
                <meta http-equiv="refresh" content="3"/>
            })}
        </div>
    }
}

// ---------------------------------------------------------------------------
// /checkout component
// ---------------------------------------------------------------------------

#[component]
pub fn CheckoutPage() -> impl IntoView {
    let ctx = Resource::new(|| (), |_| async move { load_checkout_context().await });
    let placer = ServerAction::<PlaceOrder>::new();
    let cart_ctx = crate::components::cart_drawer::use_cart_ctx();

    // Bust cart cache and, on the card path, hand the client_secret to
    // Stripe.js. Cash path is server-side-redirected before this runs.
    Effect::new(move |_| {
        if let Some(Ok(p)) = placer.value().get() {
            cart_ctx.refresh();
            if let (Some(secret), Some(pk)) = (
                p.stripe_client_secret.clone(),
                p.stripe_publishable_key.clone(),
            ) {
                #[cfg(feature = "hydrate")]
                mount_stripe_payment_element(&pk, &secret, &p.id);
                let _ = (secret, pk);
            }
        }
    });

    let pickup_choice = RwSignal::new("asap".to_string());
    let payment_method = RwSignal::new("cash".to_string());
    let order_type = RwSignal::new("pickup".to_string());
    let delivery_zone_id = RwSignal::new(String::new());

    view! {
        <section class="checkout">
            <h1>{crate::t!("checkout.title")}</h1>
            <Suspense fallback=|| view! { <p>{crate::t!("common.loading")}</p> }>
                {move || ctx.get().map(|res| match res {
                    Err(e) => view! { <p class="error">{format!("{}: {e}", crate::t!("common.error"))}</p> }.into_any(),
                    Ok(c) if c.line_count == 0 => view! {
                        <p class="empty">{crate::t!("cart.empty")} " "
                            <a href="/menu">{crate::t!("checkout.to_menu")}</a></p>
                    }.into_any(),
                    Ok(c) => view! {
                        <Form ctx=c placer pickup_choice payment_method order_type delivery_zone_id/>
                    }.into_any(),
                })}
            </Suspense>

            // Stripe Payment Element appears once we have a client_secret.
            {move || match placer.value().get() {
                Some(Ok(p)) if p.stripe_client_secret.is_some() => Some(
                    view! { <StripePane order_id=p.id.clone()/> }.into_any()
                ),
                _ => None,
            }}
        </section>
    }
}

#[component]
fn StripePane(order_id: String) -> impl IntoView {
    let return_url = format!("/orders/{order_id}");
    view! {
        <div class="stripe-pane">
            <h2>{crate::t!("checkout.pay_online")}</h2>
            <div id="payment-element"></div>
            <button id="payment-submit" type="button" class="btn primary"
                data-return-url=return_url>{crate::t!("checkout.pay_now")}</button>
            <p id="payment-message" class="error" style="display:none"></p>
            <p class="hint">{crate::t!("checkout.stripe_hint")}</p>
        </div>
    }
}

/// Bootstrap Stripe.js with the given publishable key + client_secret. Loads
/// the JS lazily so it doesn't slow down customers who pay cash.
#[cfg(feature = "hydrate")]
fn mount_stripe_payment_element(pk: &str, client_secret: &str, order_id: &str) {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = window)]
        fn __dpInitStripe(pk: &str, secret: &str, return_url: &str);
    }

    // The init function lives in the inline <script> we add in app.rs shell.
    let return_url = format!("{}/orders/{order_id}", current_origin());
    __dpInitStripe(pk, client_secret, &return_url);
}

#[cfg(feature = "hydrate")]
fn current_origin() -> String {
    web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .unwrap_or_default()
}

#[component]
fn Form(
    ctx: CheckoutContext,
    placer: ServerAction<PlaceOrder>,
    pickup_choice: RwSignal<String>,
    payment_method: RwSignal<String>,
    order_type: RwSignal<String>,
    delivery_zone_id: RwSignal<String>,
) -> impl IntoView {
    let pending = placer.pending();
    let value = placer.value();

    // Pickup-address hint text comes from the resource payload (see
    // CheckoutContext), NOT from BrandingHandle directly — that way
    // SSR and hydrate render identical DOM. Pulling it via cfg-gated
    // context reads diverged once branding had non-empty values.

    let CheckoutContext {
        slots,
        asap_minutes,
        subtotal_label,
        subtotal_cents,
        line_count,
        delivery_zones,
        free_delivery_threshold_cents,
        shop_address_line: checkout_address_line,
        shop_city: default_city_from_ctx,
    } = ctx;

    let slots_for_dropdown = slots.clone();
    let has_slots = !slots.is_empty();
    // zones_for_select removed — Phase 2 derives the zone via geocoder feedback;
    // the dropdown is gone and the hidden delivery_zone_id is mirrored from
    // the validation result. Kept this line as a marker so the diff is clear.
    let _zones_for_select: Vec<DeliveryZone> = Vec::new();
    let zones_for_lookup = delivery_zones.clone();
    let delivery_available = !delivery_zones.is_empty();

    // Pre-select the cheapest zone so the dropdown isn't blank.
    if delivery_zone_id.get_untracked().is_empty() {
        if let Some(first) = delivery_zones.first() {
            delivery_zone_id.set(first.id.clone());
        }
    }

    // Memos so we can re-use these computed values inside multiple reactive
    // closures without running into FnOnce-by-move issues.
    let zones_sv = StoredValue::new(zones_for_lookup);
    let selected_zone = Memo::new(move |_| -> Option<DeliveryZone> {
        let id = delivery_zone_id.get();
        zones_sv.with_value(|zs| zs.iter().find(|z| z.id == id).cloned())
    });
    let is_delivery = Memo::new(move |_| order_type.get() == "delivery");
    // Validation result lives here so fee/min-order memos below can read it.
    let validation: RwSignal<Option<AddressValidation>> = RwSignal::new(None);
    // For Phase 2, fee/min-order come from the live validate_address response
    // (geocoder-derived zone). Until the validator has fired (or in pickup
    // mode), fall back to the dropdown's stored zone so the summary panel
    // isn't empty during the initial render.
    // Raw zone fee — what we'd charge before the free-delivery promo.
    // Sourced from the geocoder validation when present (Phase 2), else
    // from the legacy zone dropdown.
    let zone_fee_cents = Memo::new(move |_| {
        if !is_delivery.get() {
            return 0;
        }
        validation
            .with(|v| v.as_ref().and_then(|av| av.fee_cents))
            .or_else(|| selected_zone.get().map(|z| z.fee_cents))
            .unwrap_or(0)
    });
    // True for "subtotal at or above the admin-set free-delivery
    // threshold". `0` threshold disables the promo entirely.
    let free_delivery = Memo::new(move |_| {
        free_delivery_threshold_cents > 0 && subtotal_cents >= free_delivery_threshold_cents
    });
    // Effective fee: zero if the promo applies, else the zone fee.
    let fee_cents = Memo::new(move |_| {
        if free_delivery.get() {
            0
        } else {
            zone_fee_cents.get()
        }
    });
    // How much more the customer would need to add to unlock free
    // delivery. Used for the "Noch X € bis kostenlose Lieferung" hint.
    let missing_to_free = Memo::new(move |_| {
        if free_delivery_threshold_cents == 0 {
            0
        } else {
            (free_delivery_threshold_cents - subtotal_cents).max(0)
        }
    });
    let total_cents = Memo::new(move |_| subtotal_cents + fee_cents.get());
    let min_cents = Memo::new(move |_| {
        if !is_delivery.get() {
            return 0;
        }
        validation
            .with(|v| v.as_ref().and_then(|av| av.min_order_cents))
            .or_else(|| selected_zone.get().map(|z| z.min_order_cents))
            .unwrap_or(0)
    });
    let below_min = Memo::new(move |_| is_delivery.get() && subtotal_cents < min_cents.get());
    let missing_to_min = Memo::new(move |_| (min_cents.get() - subtotal_cents).max(0));

    // Hide the form once we have a client_secret — keep the page tidy while
    // the customer focuses on the Stripe panel.
    let in_card_flow = move || {
        matches!(
            value.get(),
            Some(Ok(p)) if p.stripe_client_secret.is_some()
        )
    };

    // -- Phase 1: phone-recall autofill -------------------------------------
    //
    // Controlled signals for the contact + address fields so we can pre-fill
    // them when the customer's phone number matches an existing record.
    let name_sig = RwSignal::new(String::new());
    let phone_sig = RwSignal::new(String::new());
    let email_sig = RwSignal::new(String::new());
    let street_sig = RwSignal::new(String::new());
    let house_sig = RwSignal::new(String::new());
    let postcode_sig = RwSignal::new(String::new());
    // Pre-fill the city field with the shop's configured city. Comes
    // through CheckoutContext (the resource), not a per-context
    // BrandingHandle read — that way SSR and hydrate render the same
    // `<input value="…">`. Mismatched defaults panicked tachys at
    // hydration once branding had non-empty values.
    let city_sig = RwSignal::new(default_city_from_ctx);
    let address_notes_sig = RwSignal::new(String::new());
    // Pick up ?code=… from the URL via the reactive query map. Reads
    // sync on both SSR and hydrate so the input ships pre-filled —
    // no flash, no DOM mismatch. Subsequent CSR navigations to the
    // checkout (e.g. from an admin clicking "code-Link") also pick
    // up the new value automatically.
    let query = leptos_router::hooks::use_query_map();
    let initial_voucher = query
        .read_untracked()
        .get("code")
        .map(|s| s.to_string())
        .unwrap_or_default();
    let voucher_sig = RwSignal::new(initial_voucher);
    // Latest voucher preview from the server. Drives the summary
    // discount line, the small green/red status next to the input,
    // and feeds total_after_voucher.
    let voucher_preview: RwSignal<Option<crate::pages::vouchers::VoucherPreview>> =
        RwSignal::new(None);
    // Latest validator error message, kept separate so we don't lose
    // the previous preview when a typo lands a "Code unbekannt" reply.
    let voucher_error: RwSignal<Option<String>> = RwSignal::new(None);

    // Saved-address chips for known customers (most-recent first; the most
    // recent is auto-applied, the rest become clickable chips).
    let saved_addresses: RwSignal<Vec<SavedAddress>> = RwSignal::new(Vec::new());
    // Has-known-customer flag — used purely for a friendly "Willkommen zurück"
    // hint once the lookup hits.
    let recognised = RwSignal::new(false);

    // Apply a saved address into the form fields.
    let apply_address = move |addr: SavedAddress| {
        street_sig.set(addr.street);
        house_sig.set(addr.house_number);
        postcode_sig.set(addr.postcode);
        city_sig.set(addr.city);
        address_notes_sig.set(addr.notes.unwrap_or_default());
        if let Some(zid) = addr.delivery_zone_id {
            if !zid.is_empty() {
                delivery_zone_id.set(zid);
            }
        }
    };

    // Debounced phone lookup: re-runs whenever phone_sig settles for ~700ms
    // and contains at least 6 digits. Uses `set_timeout` from gloo when on
    // the client; on the server this is a no-op (we only run from hydrate).
    let lookup = ServerAction::<LookupCustomerByPhone>::new();
    Effect::new(move |_| {
        let raw = phone_sig.get();
        let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() < 6 {
            return;
        }
        // Fire immediately; the network round-trip + 5/min server-side limit
        // already gates this. Keystroke storms beyond the limit get a 429
        // and recover quietly. Adding a debounce on top would mean importing
        // gloo-timers; not worth the dep for the same UX.
        lookup.dispatch(LookupCustomerByPhone { phone: raw });
    });

    // Apply lookup results when they land.
    Effect::new(move |_| {
        if let Some(Ok(snapshot)) = lookup.value().get() {
            match snapshot {
                Some(s) => {
                    recognised.set(true);
                    if name_sig.get_untracked().is_empty() && !s.name.is_empty() {
                        name_sig.set(s.name);
                    }
                    if email_sig.get_untracked().is_empty() && !s.email.is_empty() {
                        email_sig.set(s.email);
                    }
                    if let Some(first) = s.addresses.first().cloned() {
                        // Only auto-apply if the address fields are still empty
                        // — never overwrite something the customer typed.
                        if street_sig.get_untracked().is_empty() {
                            apply_address(first);
                        }
                    }
                    saved_addresses.set(s.addresses);
                }
                None => {
                    recognised.set(false);
                    saved_addresses.set(Vec::new());
                }
            }
        }
    });

    // -- Phase 2: live address validation -----------------------------------
    //
    // Whenever the address fields settle and the customer is in delivery mode,
    // ask the server to geocode + classify. The result drives:
    // - the displayed zone + fee (replaces the manual dropdown)
    // - the submit button's enabled state
    // (`validation` itself is declared higher up so the fee/min-order memos
    // can read from it.)
    let validator = ServerAction::<ValidateAddress>::new();

    // Trigger: re-validate when in delivery mode and all four fields are set.
    Effect::new(move |_| {
        if !is_delivery.get() {
            validation.set(None);
            return;
        }
        let s = street_sig.get();
        let h = house_sig.get();
        let p = postcode_sig.get();
        let c = city_sig.get();
        if s.trim().is_empty() || h.trim().is_empty() || p.trim().is_empty() {
            validation.set(None);
            return;
        }
        validator.dispatch(ValidateAddress {
            street: s,
            house_number: h,
            postcode: p,
            city: c,
        });
    });

    // Apply validator results. Surface server errors as a failed
    // AddressValidation so the submit button switches from the mute
    // "Adresse prüfen" placeholder to a real reason — otherwise the
    // customer just sees a permanently-greyed button with no idea why.
    Effect::new(move |_| match validator.value().get() {
        Some(Ok(v)) => {
            if let Some(zid) = v.zone_id.clone() {
                delivery_zone_id.set(zid);
            }
            validation.set(Some(v));
        }
        Some(Err(e)) => {
            validation.set(Some(AddressValidation {
                ok: false,
                reason: Some(format!("Adressprüfung fehlgeschlagen: {e}")),
                ..Default::default()
            }));
        }
        None => {}
    });

    // Voucher live-validation: re-runs whenever the code, phone,
    // or cart totals change. The reactive deps are gathered explicitly
    // so we don't refire on every unrelated signal in the form.
    let voucher_validator = ServerAction::<crate::pages::vouchers::ValidateVoucher>::new();
    Effect::new(move |_| {
        let code = voucher_sig.get();
        let phone = phone_sig.get();
        let sub = subtotal_cents;
        let fee = fee_cents.get();
        if code.trim().is_empty() {
            voucher_preview.set(None);
            voucher_error.set(None);
            return;
        }
        voucher_validator.dispatch(crate::pages::vouchers::ValidateVoucher {
            code,
            phone,
            subtotal_cents: sub,
            delivery_fee_cents: fee,
        });
    });
    Effect::new(move |_| match voucher_validator.value().get() {
        Some(Ok(p)) => {
            voucher_error.set(None);
            voucher_preview.set(Some(p));
        }
        Some(Err(e)) => {
            voucher_preview.set(None);
            voucher_error.set(Some(format!("{e}")));
        }
        None => {}
    });
    let voucher_discount = Memo::new(move |_| {
        voucher_preview
            .with(|p| p.as_ref().filter(|p| p.applies).map(|p| p.discount_cents))
            .unwrap_or(0)
    });
    let total_after_voucher =
        Memo::new(move |_| (total_cents.get() - voucher_discount.get()).max(0));

    // Convenience signals used in the template + submit gating.
    let address_valid = Memo::new(move |_| {
        if !is_delivery.get() {
            return true;
        }
        validation.with(|v| matches!(v, Some(av) if av.ok))
    });
    let validating = Memo::new(move |_| {
        if !is_delivery.get() {
            return false;
        }
        validator.pending().get()
    });

    view! {
        <div class="checkout-grid" class:hide-form=in_card_flow>
            <div class="summary-side">
                <h2>{crate::t!("checkout.summary")}</h2>
                <p>{line_count} " " {crate::t!("checkout.items")}</p>
                <p>{crate::t!("cart.subtotal")} ": " <strong>{subtotal_label}</strong></p>
                {move || (is_delivery.get() && fee_cents.get() > 0).then(|| view! {
                    <p>{crate::t!("checkout.delivery")} ": " <strong>{format_eur(fee_cents.get())}</strong></p>
                })}
                {move || (is_delivery.get() && free_delivery.get()).then(|| view! {
                    <p class="ok">{crate::t!("cart.free_delivery_unlocked")}</p>
                })}
                {move || (voucher_discount.get() > 0).then(|| {
                    let label = voucher_preview
                        .with(|p| p.as_ref().and_then(|p| p.label.clone())
                            .unwrap_or_else(|| crate::t!("voucher.label_placeholder")));
                    let code = voucher_preview
                        .with(|p| p.as_ref().map(|p| p.code.clone()).unwrap_or_default());
                    view! {
                        <p class="ok">
                            "✓ "{label}" ("{code}"): -"
                            <strong>{format_eur(voucher_discount.get())}</strong>
                        </p>
                    }
                })}
                <p class="big">{move || format_eur(total_after_voucher.get())}</p>
                {move || (is_delivery.get() && !free_delivery.get() && missing_to_free.get() > 0).then(|| view! {
                    <p class="hint">
                        {move || crate::t!("cart.free_delivery_progress")
                            .replace("{amount}", &format_eur(missing_to_free.get()))}
                    </p>
                })}
                {move || below_min.get().then(|| view! {
                    <p class="warn">
                        {crate::t!("checkout.min_order_warn")
                            .replace("{min}", &format_eur(min_cents.get()))
                            .replace("{missing}", &format_eur(missing_to_min.get()))}
                        " "
                        <a href="/menu">{crate::t!("checkout.to_menu")}</a>
                    </p>
                })}
                {{
                    let line = checkout_address_line.clone();
                    (!line.is_empty()).then(move || view! {
                        <p class="hint">
                            {crate::t!("checkout.address_label").replace("{line}", &line)}
                        </p>
                    })
                }}
            </div>

            <ActionForm action=placer attr:class="checkout-form">
                <fieldset>
                    <legend>{crate::t!("checkout.order_type")}</legend>
                    <label class="radio">
                        <input type="radio" name="order_type" value="pickup"
                               prop:checked=move || order_type.get() == "pickup"
                               on:change=move |_| order_type.set("pickup".to_string())/>
                        <span>{crate::t!("checkout.pickup")}</span>
                    </label>
                    {if delivery_available {
                        view! {
                            <label class="radio">
                                <input type="radio" name="order_type" value="delivery"
                                       prop:checked=move || order_type.get() == "delivery"
                                       on:change=move |_| order_type.set("delivery".to_string())/>
                                <span>{crate::t!("checkout.delivery")}</span>
                            </label>
                        }.into_any()
                    } else {
                        view! {
                            <p class="hint">{crate::t!("checkout.delivery_unavailable")}</p>
                        }.into_any()
                    }}
                </fieldset>

                <Show when=move || is_delivery.get() fallback=|| ()>
                    <fieldset>
                        <legend>{crate::t!("checkout.delivery_address")}</legend>
                        // Hidden field carries the resolved zone id for legacy
                        // form-submission compatibility. The server re-runs
                        // validate_address regardless, so this is a hint only.
                        <input type="hidden" name="delivery_zone_id"
                               prop:value=move || delivery_zone_id.get()/>

                        // Zone feedback panel — shows what the geocoder resolved
                        // the address to. Replaces the old manual dropdown.
                        {move || {
                            let v = validation.get();
                            let validating_now = validating.get();
                            if validating_now {
                                view! {
                                    <p class="zone-status checking">{format!("📍 {}", crate::t!("checkout.address_checking"))}</p>
                                }.into_any()
                            } else if let Some(av) = v {
                                if av.ok {
                                    let label = av.zone_name.clone().unwrap_or_default();
                                    let fee = av.fee_cents.unwrap_or(0);
                                    let min_o = av.min_order_cents.unwrap_or(0);
                                    let eta = av.eta_minutes.unwrap_or(0);
                                    view! {
                                        <p class="zone-status ok">
                                            {format!("✅ {}: ", crate::t!("home.delivery_title"))}
                                            <strong>{label}</strong>
                                            " — " {crate::t!("home.zone_fee_suffix")} " " <strong>{format_eur(fee)}</strong>
                                            ", " {crate::t!("home.zone_min_order_prefix")} " " {format_eur(min_o)}
                                            {format!(" ({}~{}", "", eta)} " min)"
                                        </p>
                                    }.into_any()
                                } else {
                                    let reason = av.reason.clone()
                                        .unwrap_or_else(|| crate::t!("checkout.address_invalid"));
                                    view! {
                                        <p class="zone-status err">"⚠️ " {reason}</p>
                                    }.into_any()
                                }
                            } else {
                                view! {
                                    <p class="zone-status hint">
                                        {crate::t!("checkout.address_validation_hint")}
                                    </p>
                                }.into_any()
                            }
                        }}
                        <label>
                            <span>{crate::t!("checkout.street")}</span>
                            <input type="text" name="street" autocomplete="address-line1"
                                   placeholder=crate::t!("checkout.street_placeholder")
                                   prop:value=move || street_sig.get()
                                   on:input=move |ev| street_sig.set(event_target_value(&ev))/>
                        </label>
                        <label>
                            <span>{crate::t!("checkout.house_number")}</span>
                            <input type="text" name="house_number"
                                   placeholder=crate::t!("checkout.house_number_placeholder")
                                   prop:value=move || house_sig.get()
                                   on:input=move |ev| house_sig.set(event_target_value(&ev))/>
                        </label>
                        <label>
                            <span>{crate::t!("checkout.postcode")}</span>
                            <input type="text" name="postcode" autocomplete="postal-code"
                                   placeholder="12345" inputmode="numeric"
                                   prop:value=move || postcode_sig.get()
                                   on:input=move |ev| postcode_sig.set(event_target_value(&ev))/>
                        </label>
                        <label>
                            <span>{crate::t!("checkout.city")}</span>
                            <input type="text" name="city" autocomplete="address-level2"
                                   placeholder="Lüdinghausen"
                                   prop:value=move || city_sig.get()
                                   on:input=move |ev| city_sig.set(event_target_value(&ev))/>
                        </label>
                        <label>
                            <span>{crate::t!("checkout.address_note")}</span>
                            <input type="text" name="address_notes"
                                   placeholder=crate::t!("checkout.address_note_placeholder")
                                   prop:value=move || address_notes_sig.get()
                                   on:input=move |ev| address_notes_sig.set(event_target_value(&ev))/>
                        </label>

                        // Older saved addresses for known customers, as
                        // clickable chips that overwrite the form fields.
                        {move || {
                            let addrs = saved_addresses.get();
                            if addrs.len() <= 1 { return ().into_any(); }
                            view! {
                                <div class="saved-addresses">
                                    <span class="hint">{crate::t!("checkout.saved_addresses")}</span>
                                    {addrs.into_iter().skip(1).map(|a| {
                                        let label = format!("{} {}, {}",
                                            a.street, a.house_number, a.postcode);
                                        let a_for_click = a.clone();
                                        view! {
                                            <button type="button" class="chip"
                                                on:click=move |_| {
                                                    apply_address(a_for_click.clone());
                                                }>
                                                {label}
                                            </button>
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_any()
                        }}
                    </fieldset>
                </Show>

                <fieldset>
                    <legend>{crate::t!("checkout.contact")}</legend>
                    {move || recognised.get().then(|| view! {
                        <p class="hint welcome-back">
                            {format!("👋 {}", crate::t!("checkout.welcome_back"))}
                        </p>
                    })}
                    <label>
                        <span>{crate::t!("checkout.phone")}</span>
                        <input type="tel" name="phone" required autocomplete="tel"
                               placeholder=crate::t!("checkout.phone_placeholder")
                               prop:value=move || phone_sig.get()
                               on:input=move |ev| phone_sig.set(event_target_value(&ev))/>
                    </label>
                    <label>
                        <span>{crate::t!("checkout.name")}</span>
                        <input type="text" name="name" required autocomplete="name"
                               prop:value=move || name_sig.get()
                               on:input=move |ev| name_sig.set(event_target_value(&ev))/>
                    </label>
                    <label>
                        <span>{crate::t!("checkout.email")}</span>
                        <input type="email" name="email" required autocomplete="email"
                               placeholder=crate::t!("checkout.email_placeholder")
                               prop:value=move || email_sig.get()
                               on:input=move |ev| email_sig.set(event_target_value(&ev))/>
                    </label>
                </fieldset>

                <fieldset>
                    <legend>{move || if is_delivery.get() { crate::t!("checkout.delivery_time") } else { crate::t!("checkout.pickup_time") }}</legend>
                    <label class="radio">
                        <input type="radio" name="pickup_time" value="asap"
                               prop:checked=move || pickup_choice.get() == "asap"
                               on:change=move |_| pickup_choice.set("asap".to_string())/>
                        <span>{crate::t!("checkout.asap").replace("{n}", &asap_minutes.to_string())}</span>
                    </label>

                    {if has_slots {
                        view! {
                            <label class="radio">
                                <input type="radio" name="pickup_time" value="scheduled"
                                       on:change=move |_| pickup_choice.set("scheduled".to_string())/>
                                <span>{crate::t!("checkout.pick_later")}</span>
                            </label>
                            <select name="pickup_time"
                                disabled=move || pickup_choice.get() != "scheduled">
                                {slots_for_dropdown.into_iter().map(|s| {
                                    view! { <option value=s.value>{s.label}</option> }
                                }).collect_view()}
                            </select>
                        }.into_any()
                    } else {
                        view! { <p class="hint">{crate::t!("checkout.no_slots")}</p> }.into_any()
                    }}
                </fieldset>

                <fieldset>
                    <legend>{crate::t!("checkout.payment")}</legend>
                    <label class="radio">
                        <input type="radio" name="payment_method" value="cash"
                               prop:checked=move || payment_method.get() == "cash"
                               on:change=move |_| payment_method.set("cash".to_string())/>
                        <span>{move || if is_delivery.get() { crate::t!("checkout.pay_cash_delivery") } else { crate::t!("checkout.pay_cash_pickup") }}</span>
                    </label>
                    <label class="radio">
                        <input type="radio" name="payment_method" value="card"
                               prop:checked=move || payment_method.get() == "card"
                               on:change=move |_| payment_method.set("card".to_string())/>
                        <span>{crate::t!("checkout.pay_card_long")}</span>
                    </label>
                </fieldset>

                <fieldset class="voucher-fieldset">
                    <legend>{crate::t!("checkout.voucher_legend")}</legend>
                    <input type="text" name="voucher_code" autocomplete="off"
                           placeholder=crate::t!("checkout.voucher_placeholder")
                           prop:value=move || voucher_sig.get()
                           on:input=move |ev| voucher_sig.set(event_target_value(&ev))/>
                    {move || {
                        if let Some(p) = voucher_preview.get() {
                            let cls = if p.applies { "ok" } else { "warn" };
                            view! { <p class=cls>{p.reason_de}</p> }.into_any()
                        } else if let Some(e) = voucher_error.get() {
                            view! { <p class="warn">{e}</p> }.into_any()
                        } else {
                            view! { <p class="hint muted">{crate::t!("checkout.voucher_hint")}</p> }.into_any()
                        }
                    }}
                </fieldset>

                <button type="submit" class="btn primary"
                        disabled=move || pending.get() || below_min.get()
                                       || validating.get() || !address_valid.get()>
                    {move || {
                        if pending.get() { crate::t!("checkout.please_wait") }
                        else if validating.get() { crate::t!("checkout.address_checking") }
                        else if !address_valid.get() {
                            // Pull the reason out of the validation if any.
                            validation.with(|v| v.as_ref()
                                .and_then(|av| av.reason.clone())
                                .unwrap_or_else(|| crate::t!("checkout.check_address")))
                        }
                        else if below_min.get() {
                            crate::t!("checkout.min_missing")
                                .replace("{amount}", &format_eur(missing_to_min.get()))
                        }
                        else if payment_method.get() == "card" && total_after_voucher.get() == 0 {
                            // Voucher covers everything — nothing to pay,
                            // no Stripe step. Match what place_order does.
                            crate::t!("checkout.place_order_btn")
                        }
                        else if payment_method.get() == "card" { crate::t!("checkout.proceed_payment") }
                        else { crate::t!("checkout.place_order_btn") }
                    }}
                </button>

                {move || match value.get() {
                    Some(Err(e)) => Some(view! { <p class="error">{format!("{}: {e}", crate::t!("common.error"))}</p> }.into_any()),
                    _ => None,
                }}
            </ActionForm>
        </div>
    }
}
