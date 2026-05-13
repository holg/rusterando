//! Voucher subsystem — public-facing validation + redeem helpers.
//!
//! Admin-facing CRUD lives in `pages::admin::vouchers`. This module is
//! the boundary the checkout calls into: `validate_voucher` (live,
//! during checkout typing) returns a `VoucherPreview` describing the
//! discount, and `redeem_voucher` (called inside `place_order`'s
//! transaction) persists the redemption row + snapshots the discount
//! onto the order.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Three voucher kinds for phase 1. `FreeDelivery` waives the
/// delivery fee regardless of the free-delivery threshold; useful for
/// pickup-vs-delivery conversion experiments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoucherKind {
    Percent,
    Fixed,
    FreeDelivery,
}

impl VoucherKind {
    pub fn as_str(self) -> &'static str {
        match self {
            VoucherKind::Percent => "percent",
            VoucherKind::Fixed => "fixed",
            VoucherKind::FreeDelivery => "free_delivery",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "percent" => Some(Self::Percent),
            "fixed" => Some(Self::Fixed),
            "free_delivery" => Some(Self::FreeDelivery),
            _ => None,
        }
    }
}

/// Result of `validate_voucher`. `discount_cents` is what we'd shave
/// off (subtotal for percent/fixed, delivery fee for free_delivery)
/// given the current cart subtotal + delivery fee + phone. Zero means
/// the code is valid but doesn't apply yet (e.g. min subtotal not
/// reached) — the checkout shows `reason_de` as a hint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoucherPreview {
    pub code: String,
    pub label: Option<String>,
    pub kind: String,
    pub discount_cents: i64,
    /// True iff the discount applies right now (cart passes all
    /// rules). The checkout button stays enabled either way; the
    /// reason text just changes.
    pub applies: bool,
    /// One-line German hint shown next to the input. Always set —
    /// either "Code WILLKOMMEN10 — 10% Rabatt." (applies) or
    /// "Mindestbestellwert 20 €." (rejected).
    pub reason_de: String,
}

/// Normalise an admin-typed code: trim + uppercase + strip whitespace.
/// "  Willkommen10 " → "WILLKOMMEN10". Stored this way so the
/// case-insensitive UX is just an UPPER on the input.
pub fn normalise_code(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_uppercase())
        .collect()
}

// ---------------------------------------------------------------------------
// Live validation server fn — called from /checkout on every code edit
// ---------------------------------------------------------------------------

#[server(
    name = ValidateVoucher,
    prefix = "/api",
    endpoint = "validate_voucher"
)]
pub async fn validate_voucher(
    code: String,
    phone: String,
    subtotal_cents: i64,
    delivery_fee_cents: i64,
) -> Result<VoucherPreview, ServerFnError> {
    use sqlx::SqlitePool;

    let code_norm = normalise_code(&code);
    if code_norm.is_empty() {
        return Err(ServerFnError::new("Bitte Code eingeben"));
    }
    let phone_norm = crate::pages::order::ssr::normalize_phone(&phone);

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let v = ssr::load_active_voucher(&db, &code_norm).await?;

    let preview = ssr::evaluate(&db, &v, &phone_norm, subtotal_cents, delivery_fee_cents).await?;
    Ok(preview)
}

// ---------------------------------------------------------------------------
// SSR helpers — used by validate_voucher and by place_order on redeem
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
pub mod ssr {
    use super::*;
    use chrono::Utc;
    use sqlx::{Row, SqlitePool};

    /// In-memory voucher row, for ssr-side evaluation. We avoid
    /// re-using the admin DTO so this module can be used standalone.
    pub struct VoucherRow {
        pub id: String,
        pub code: String,
        pub kind: VoucherKind,
        pub percent_off: Option<i64>,
        pub amount_off_cents: Option<i64>,
        pub min_subtotal_cents: i64,
        pub first_order_only: bool,
        pub per_phone_cap: i64,
        pub global_cap: i64,
        pub valid_from: Option<String>,
        pub valid_until: Option<String>,
        pub label: Option<String>,
        /// Hard binding to a specific normalised phone. `None` =
        /// anonymous code; `Some(...)` = only that phone may redeem.
        pub customer_phone: Option<String>,
    }

    pub async fn load_active_voucher(
        db: &SqlitePool,
        code: &str,
    ) -> Result<VoucherRow, ServerFnError> {
        let row = sqlx::query(
            "SELECT id, code, kind, percent_off, amount_off_cents,
                    min_subtotal_cents, first_order_only,
                    per_phone_cap, global_cap, valid_from, valid_until, label,
                    customer_phone
             FROM vouchers
             WHERE code = ?1 AND active = 1
             LIMIT 1",
        )
        .bind(code)
        .fetch_optional(db)
        .await
        .map_err(|e| ServerFnError::new(format!("load voucher: {e}")))?
        .ok_or_else(|| ServerFnError::new("Code unbekannt oder deaktiviert."))?;

        let kind_str: String = row.get("kind");
        let kind = VoucherKind::parse(&kind_str)
            .ok_or_else(|| ServerFnError::new(format!("unknown voucher kind: {kind_str}")))?;

        Ok(VoucherRow {
            id: row.get("id"),
            code: row.get("code"),
            kind,
            percent_off: row.get("percent_off"),
            amount_off_cents: row.get("amount_off_cents"),
            min_subtotal_cents: row.get("min_subtotal_cents"),
            first_order_only: row.get::<i64, _>("first_order_only") != 0,
            per_phone_cap: row.get("per_phone_cap"),
            global_cap: row.get("global_cap"),
            valid_from: row.get("valid_from"),
            valid_until: row.get("valid_until"),
            label: row.get("label"),
            customer_phone: row.get("customer_phone"),
        })
    }

    /// Compute the gross discount this voucher would produce, given a
    /// subtotal + current delivery fee. Returns 0 for free_delivery
    /// when the cart is pickup-only.
    ///
    /// Fixed vouchers cap against `subtotal + delivery_fee` (not just
    /// subtotal). Rationale: a 25€ voucher on a 14€ pizza + 1,50€
    /// delivery should leave 0€ to pay, not 1,50€. Otherwise the
    /// customer faces a tiny Stripe charge that defeats the whole
    /// point of "voucher covers it all".
    fn gross_discount(v: &VoucherRow, subtotal_cents: i64, delivery_fee_cents: i64) -> i64 {
        match v.kind {
            VoucherKind::Percent => {
                let p = v.percent_off.unwrap_or(0).clamp(0, 100);
                subtotal_cents.saturating_mul(p) / 100
            }
            VoucherKind::Fixed => {
                let f = v.amount_off_cents.unwrap_or(0).max(0);
                f.min(subtotal_cents + delivery_fee_cents.max(0))
            }
            VoucherKind::FreeDelivery => delivery_fee_cents.max(0),
        }
    }

    fn label_for(v: &VoucherRow) -> String {
        if let Some(l) = v.label.as_deref().filter(|s| !s.trim().is_empty()) {
            return l.to_string();
        }
        match v.kind {
            VoucherKind::Percent => {
                let p = v.percent_off.unwrap_or(0);
                format!("{p}% Rabatt")
            }
            VoucherKind::Fixed => {
                let f = v.amount_off_cents.unwrap_or(0);
                format!("{} Rabatt", rusterando_shared::models::format_eur(f))
            }
            VoucherKind::FreeDelivery => "Gratis-Lieferung".to_string(),
        }
    }

    /// Full rule check. Returns a `VoucherPreview` describing whether
    /// the code applies right now + a human-readable reason.
    pub async fn evaluate(
        db: &SqlitePool,
        v: &VoucherRow,
        phone_norm: &str,
        subtotal_cents: i64,
        delivery_fee_cents: i64,
    ) -> Result<VoucherPreview, ServerFnError> {
        let now = Utc::now().naive_utc();
        let parse_iso = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
            .ok();

        if let Some(from) = v.valid_from.as_deref().and_then(parse_iso) {
            if now < from {
                return Ok(reject(v, "Code ist noch nicht gültig."));
            }
        }
        if let Some(until) = v.valid_until.as_deref().and_then(parse_iso) {
            if now > until {
                return Ok(reject(v, "Code ist abgelaufen."));
            }
        }

        // Hard phone binding: a non-NULL customer_phone restricts this
        // code to exactly that number. Empty checkout phone gives the
        // friendlier "Bitte Nummer eingeben" so we don't expose the
        // fact the code is personalised.
        if let Some(bound) = v.customer_phone.as_deref().filter(|s| !s.is_empty()) {
            if phone_norm.is_empty() {
                return Ok(reject(v, "Bitte Telefonnummer eingeben."));
            }
            if bound != phone_norm {
                return Ok(reject(v, "Code gilt nicht für diese Nummer."));
            }
        }

        if subtotal_cents < v.min_subtotal_cents {
            let need = v.min_subtotal_cents;
            return Ok(reject(
                v,
                &format!(
                    "Mindestbestellwert {}.",
                    rusterando_shared::models::format_eur(need)
                ),
            ));
        }

        if v.kind == VoucherKind::FreeDelivery && delivery_fee_cents == 0 {
            return Ok(reject(
                v,
                "Gratis-Lieferung greift nur bei Lieferungen mit Liefergebühr.",
            ));
        }

        // Global cap: count all redemptions of this voucher_id.
        if v.global_cap > 0 {
            let used: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM voucher_redemptions WHERE voucher_id = ?1",
            )
            .bind(&v.id)
            .fetch_one(db)
            .await
            .map_err(|e| ServerFnError::new(format!("global cap check: {e}")))?;
            if used.0 >= v.global_cap {
                return Ok(reject(v, "Code ist ausgeschöpft."));
            }
        }

        // Per-phone cap: count redemptions for this phone.
        if !phone_norm.is_empty() && v.per_phone_cap > 0 {
            let used: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM voucher_redemptions \
                 WHERE voucher_id = ?1 AND phone = ?2",
            )
            .bind(&v.id)
            .bind(phone_norm)
            .fetch_one(db)
            .await
            .map_err(|e| ServerFnError::new(format!("per-phone cap check: {e}")))?;
            if used.0 >= v.per_phone_cap {
                return Ok(reject(v, "Code wurde bereits eingelöst."));
            }
        }

        // First-order-only: zero paid orders for this phone.
        if !phone_norm.is_empty() && v.first_order_only {
            let prior: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM orders o \
                 JOIN customers c ON c.id = o.customer_id \
                 WHERE c.phone = ?1 \
                   AND o.payment_status IN ('paid','cash_on_pickup') \
                   AND o.status <> 'cancelled'",
            )
            .bind(phone_norm)
            .fetch_one(db)
            .await
            .map_err(|e| ServerFnError::new(format!("first-order check: {e}")))?;
            if prior.0 > 0 {
                return Ok(reject(v, "Nur für Erstbesteller."));
            }
        }

        let discount = gross_discount(v, subtotal_cents, delivery_fee_cents);
        if discount <= 0 {
            return Ok(reject(v, "Kein Rabatt anwendbar."));
        }

        Ok(VoucherPreview {
            code: v.code.clone(),
            label: Some(label_for(v)),
            kind: v.kind.as_str().to_string(),
            discount_cents: discount,
            applies: true,
            reason_de: format!(
                "{} − {} Rabatt",
                label_for(v),
                rusterando_shared::models::format_eur(discount)
            ),
        })
    }

    fn reject(v: &VoucherRow, reason: &str) -> VoucherPreview {
        VoucherPreview {
            code: v.code.clone(),
            label: Some(label_for(v)),
            kind: v.kind.as_str().to_string(),
            discount_cents: 0,
            applies: false,
            reason_de: reason.to_string(),
        }
    }

    /// Called inside the place_order transaction. Re-runs the rules
    /// against the final cart (in case the cart changed between
    /// validate + place) and inserts a `voucher_redemptions` row.
    /// Returns (voucher_id, code, discount_cents). The caller is
    /// expected to use the same `tx` for its own writes so the
    /// redemption rolls back if the order fails.
    pub async fn redeem<'c>(
        tx: &mut sqlx::Transaction<'c, sqlx::Sqlite>,
        code: &str,
        phone_norm: &str,
        subtotal_cents: i64,
        delivery_fee_cents: i64,
        order_id: &str,
    ) -> Result<(String, String, i64), ServerFnError> {
        let code_norm = normalise_code(code);

        let row = sqlx::query(
            "SELECT id, code, kind, percent_off, amount_off_cents,
                    min_subtotal_cents, first_order_only,
                    per_phone_cap, global_cap, valid_from, valid_until, label,
                    customer_phone
             FROM vouchers
             WHERE code = ?1 AND active = 1
             LIMIT 1",
        )
        .bind(&code_norm)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| ServerFnError::new(format!("redeem: load: {e}")))?
        .ok_or_else(|| ServerFnError::new("Code unbekannt oder deaktiviert."))?;

        let kind_str: String = row.get("kind");
        let kind = VoucherKind::parse(&kind_str)
            .ok_or_else(|| ServerFnError::new(format!("unknown voucher kind: {kind_str}")))?;
        let v = VoucherRow {
            id: row.get("id"),
            code: row.get("code"),
            kind,
            percent_off: row.get("percent_off"),
            amount_off_cents: row.get("amount_off_cents"),
            min_subtotal_cents: row.get("min_subtotal_cents"),
            first_order_only: row.get::<i64, _>("first_order_only") != 0,
            per_phone_cap: row.get("per_phone_cap"),
            global_cap: row.get("global_cap"),
            valid_from: row.get("valid_from"),
            valid_until: row.get("valid_until"),
            label: row.get("label"),
            customer_phone: row.get("customer_phone"),
        };

        // Re-validate inside the txn — caps must read committed state.
        let now = Utc::now().naive_utc();
        let parse_iso = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
            .ok();
        if let Some(from) = v.valid_from.as_deref().and_then(parse_iso) {
            if now < from {
                return Err(ServerFnError::new("Code ist noch nicht gültig."));
            }
        }
        if let Some(until) = v.valid_until.as_deref().and_then(parse_iso) {
            if now > until {
                return Err(ServerFnError::new("Code ist abgelaufen."));
            }
        }
        // Phone binding inside the txn — same rule as evaluate().
        if let Some(bound) = v.customer_phone.as_deref().filter(|s| !s.is_empty()) {
            if phone_norm.is_empty() {
                return Err(ServerFnError::new("Bitte Telefonnummer eingeben."));
            }
            if bound != phone_norm {
                return Err(ServerFnError::new("Code gilt nicht für diese Nummer."));
            }
        }
        if subtotal_cents < v.min_subtotal_cents {
            return Err(ServerFnError::new(format!(
                "Mindestbestellwert {}.",
                rusterando_shared::models::format_eur(v.min_subtotal_cents)
            )));
        }
        if v.global_cap > 0 {
            let used: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM voucher_redemptions WHERE voucher_id = ?1",
            )
            .bind(&v.id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| ServerFnError::new(format!("global cap: {e}")))?;
            if used.0 >= v.global_cap {
                return Err(ServerFnError::new("Code ist ausgeschöpft."));
            }
        }
        if v.per_phone_cap > 0 {
            let used: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM voucher_redemptions WHERE voucher_id = ?1 AND phone = ?2",
            )
            .bind(&v.id)
            .bind(phone_norm)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| ServerFnError::new(format!("per-phone cap: {e}")))?;
            if used.0 >= v.per_phone_cap {
                return Err(ServerFnError::new("Code wurde bereits eingelöst."));
            }
        }
        if v.first_order_only {
            let prior: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM orders o \
                 JOIN customers c ON c.id = o.customer_id \
                 WHERE c.phone = ?1 \
                   AND o.payment_status IN ('paid','cash_on_pickup') \
                   AND o.status <> 'cancelled'",
            )
            .bind(phone_norm)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| ServerFnError::new(format!("first-order check: {e}")))?;
            if prior.0 > 0 {
                return Err(ServerFnError::new("Nur für Erstbesteller."));
            }
        }

        let discount = gross_discount(&v, subtotal_cents, delivery_fee_cents);
        if discount <= 0 {
            return Err(ServerFnError::new("Kein Rabatt anwendbar."));
        }

        let red_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO voucher_redemptions
               (id, voucher_id, order_id, phone, discount_cents)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&red_id)
        .bind(&v.id)
        .bind(order_id)
        .bind(phone_norm)
        .bind(discount)
        .execute(&mut **tx)
        .await
        .map_err(|e| ServerFnError::new(format!("insert redemption: {e}")))?;

        Ok((v.id, v.code, discount))
    }
}
