//! Prominent shop open/closed status bar, shown at the top of the
//! customer-facing pages (home AND menu — the menu is what most people
//! bookmark, so the status must be visible there too).
//!
//! Takes the initial `(level, reason)` as props (the page passes it from
//! its own already-loaded data, so SSR + the initial hydrate render the
//! same DOM). After mount it subscribes to the global `/api/live/shop`
//! SSE channel and flips in place — no reload. The subscription lives in
//! a post-hydration `Effect` (inside `subscribe_shop_status`), so it never
//! alters the SSR/hydrate DOM shape.
//!
//! Three colours (see `ShopLevel`): green "Jetzt geöffnet", amber for a
//! temporary closure (opens later today, or a pause/snooze), red for a
//! hard close (Ruhetag / outside hours with nothing more today).

use leptos::prelude::*;
use rusterando_shared::models::ShopLevel;

#[component]
pub fn ShopStatusBar(
    /// Initial three-level state from the page's resource (SSR-correct).
    init_level: ShopLevel,
    /// Initial caption (German), shown when not plainly open.
    init_reason: String,
) -> impl IntoView {
    // Live override from SSE; `None` until a ShopStatus event arrives.
    let (shop_override, set_shop_override) =
        signal::<Option<(ShopLevel, String)>>(None);
    crate::utils::subscribe_shop_status(set_shop_override);

    move || {
        let (level, reason) = shop_override
            .get()
            .unwrap_or_else(|| (init_level, init_reason.clone()));
        let cls = format!("shop-status-bar {}", level.css_class());
        match level {
            ShopLevel::Open => view! {
                <div class=cls>
                    <strong>{crate::t!("shop_status.open")}</strong>
                    <span>{crate::t!("shop_status.open_hint")}</span>
                </div>
            }
            .into_any(),
            ShopLevel::OpensLater => view! {
                // Amber: closed right now but it resolves itself. `role` is
                // status (not alert) — it's informational, not an error.
                <div class=cls role="status">
                    <strong>{crate::t!("shop_status.closed_now")}</strong>
                    <span>{reason}</span>
                </div>
            }
            .into_any(),
            ShopLevel::Closed => view! {
                <div class=cls role="alert">
                    <strong>{crate::t!("shop_status.closed")}</strong>
                    <span>{reason}</span>
                </div>
            }
            .into_any(),
        }
    }
}
