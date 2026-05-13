//! Stripe mode + key resolution.
//!
//! The server boots holding two key sets — `S_STRIPE_*` (sandbox)
//! and `L_STRIPE_*` (live) — and an `app_settings.stripe_mode` row
//! decides which is active. Order-placement / webhook code reads
//! through [`StripeKeys::active`] at the moment of use.
//!
//! Mirrors the [`crate::branding::BrandingHandle`] pattern: one
//! Arc<RwLock<…>> cached in AppState, write-through on
//! `update_setting('stripe_mode', …)`, no DB hit per request.
//!
//! Snapshot-at-order-time: when [`pages::order::place_order`] mints
//! a Payment Intent it stamps `orders.stripe_mode` with whatever
//! mode it just used. The webhook handlers then route validation by
//! that snapshot — a mid-checkout admin flip can't strand pending
//! intents because the row remembers its origin.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use std::sync::{Arc, RwLock};

/// Which Stripe environment a key set targets.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum StripeMode {
    /// Stripe test keys (sk_test_…/pk_test_…). No real money moves.
    /// Default for a fresh install so a misconfigured deploy can't
    /// accidentally charge real cards.
    #[default]
    Sandbox,
    /// Stripe live keys (sk_live_…/pk_live_…). Real money.
    Live,
}

impl StripeMode {
    /// Parse the value persisted in `app_settings.stripe_mode`.
    /// Defaults to Sandbox for any unknown or empty value — safer
    /// than guessing "live".
    pub fn from_setting(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "live" => Self::Live,
            _ => Self::Sandbox,
        }
    }

    /// Inverse of `from_setting`. Stored verbatim in app_settings.
    pub fn as_setting(&self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Live => "live",
        }
    }

    /// Display label for the admin UI.
    pub fn label_de(&self) -> &'static str {
        match self {
            Self::Sandbox => "Sandbox (Test)",
            Self::Live => "Live (Echtbetrieb)",
        }
    }

    /// Per-mode env var prefix. The server boots with both
    /// `S_STRIPE_*` (sandbox) and `L_STRIPE_*` (live) set in `.env`;
    /// this picks the right one at lookup time.
    pub fn env_prefix(&self) -> &'static str {
        match self {
            Self::Sandbox => "S_",
            Self::Live => "L_",
        }
    }
}

/// Resolved Stripe keys for a single mode. Returned by
/// [`StripeKeys::for_mode`] so call sites get a clean struct rather
/// than a fistful of `std::env::var` lookups.
///
/// Empty fields mean the corresponding `.env` key was missing.
/// Callers should error out at the use site (e.g. "no publish key
/// in live mode → can't render checkout") rather than panicking at
/// boot.
#[derive(Debug, Clone)]
pub struct StripeKeys {
    pub mode: StripeMode,
    pub publish_key: String,
    pub secret_key: String,
    pub webhook_secret: String,
}

impl StripeKeys {
    /// Pull `<prefix>STRIPE_PUBLISH_KEY` / `<prefix>STRIPE_SECRET_KEY`
    /// / `<prefix>STRIPE_WEBHOOK_SECRET` for the given mode. Empty
    /// strings for any missing var; check before use.
    #[cfg(feature = "ssr")]
    pub fn for_mode(mode: StripeMode) -> Self {
        let p = mode.env_prefix();
        Self {
            mode,
            publish_key: std::env::var(format!("{p}STRIPE_PUBLISH_KEY"))
                .or_else(|_| std::env::var(format!("{p}STRIPE_PUBLISHABLE_KEY")))
                .unwrap_or_default(),
            secret_key: std::env::var(format!("{p}STRIPE_SECRET_KEY")).unwrap_or_default(),
            webhook_secret: std::env::var(format!("{p}STRIPE_WEBHOOK_SECRET")).unwrap_or_default(),
        }
    }

    /// True when all three secrets are present. False = a deploy
    /// is missing env vars for this mode; the admin shouldn't be
    /// allowed to flip into it.
    pub fn is_complete(&self) -> bool {
        !self.publish_key.is_empty()
            && !self.secret_key.is_empty()
            && !self.webhook_secret.is_empty()
    }
}

/// SSR-only handle to the currently-active Stripe mode. Cached in
/// AppState by `rusterando-server::main`, provided as Leptos context
/// so server fns can `use_context::<StripeModeHandle>()` cheaply
/// without a per-request DB read. Write-through on
/// `update_setting('stripe_mode', …)`.
#[cfg(feature = "ssr")]
#[derive(Clone, Default)]
pub struct StripeModeHandle(pub Arc<RwLock<StripeMode>>);

#[cfg(feature = "ssr")]
impl StripeModeHandle {
    pub fn new(initial: StripeMode) -> Self {
        Self(Arc::new(RwLock::new(initial)))
    }
    pub fn get(&self) -> StripeMode {
        self.0.read().map(|g| *g).unwrap_or(StripeMode::Sandbox)
    }
    pub fn set(&self, mode: StripeMode) {
        if let Ok(mut g) = self.0.write() {
            *g = mode;
        }
    }
    /// Convenience: read the handle and resolve the matching key
    /// set in one call. Used by `place_order` and the legacy
    /// `/api/webhooks/stripe` alias.
    pub fn active_keys(&self) -> StripeKeys {
        StripeKeys::for_mode(self.get())
    }
}

#[cfg(feature = "ssr")]
pub mod ssr {
    use super::*;
    use sqlx::SqlitePool;

    /// Load the persisted `stripe_mode` at boot. Defaults to
    /// Sandbox if the row is missing or unreadable.
    pub async fn load_mode(db: &SqlitePool) -> StripeMode {
        let raw: Option<String> =
            sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'stripe_mode'")
                .fetch_optional(db)
                .await
                .ok()
                .flatten();
        StripeMode::from_setting(raw.as_deref().unwrap_or(""))
    }
}

/// Public server fn — returns the active Stripe mode as a string
/// ("sandbox" | "live"). Used by the customer-facing TEST-MODUS
/// banner and the admin shell's mode chip via an `OnceResource` so
/// SSR + hydrate render the same DOM (a direct `use_context::<…>`
/// read would diverge — handle is SSR-only).
#[server(
    name = GetStripeMode,
    prefix = "/api",
    endpoint = "get_stripe_mode"
)]
pub async fn get_stripe_mode() -> Result<String, ServerFnError> {
    let mode = use_context::<StripeModeHandle>()
        .ok_or_else(|| ServerFnError::new("StripeModeHandle missing from context"))?
        .get();
    Ok(mode.as_setting().to_string())
}
