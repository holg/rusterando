//! Sticky top-bar that appears on every public page when the shop
//! is in Stripe sandbox mode. Disappears completely in live mode
//! so production traffic sees no chrome.
//!
//! Reads `window.__appBootstrap.stripe_sandbox` synchronously instead
//! of going through an `OnceResource`/Suspense pair. The previous
//! resource-driven version tripped tachys's hydration walker with
//! "entered unreachable code" — resource reads in the document-root
//! chrome can't be coordinated with the SSR stream markers. The SSR
//! shell now bakes the flag into an inline `<script>` so both SSR
//! and hydrate render the same DOM via a sync read.

use leptos::prelude::*;

fn stripe_sandbox_sync() -> bool {
    #[cfg(feature = "ssr")]
    {
        use_context::<crate::stripe::StripeModeHandle>()
            .map(|h| matches!(h.get(), crate::stripe::StripeMode::Sandbox))
            .unwrap_or(false)
    }
    #[cfg(not(feature = "ssr"))]
    {
        use leptos::wasm_bindgen::JsValue;
        let Some(window) = web_sys::window() else {
            return false;
        };
        let bootstrap = js_sys::Reflect::get(&window, &JsValue::from_str("__appBootstrap"))
            .unwrap_or(JsValue::UNDEFINED);
        js_sys::Reflect::get(&bootstrap, &JsValue::from_str("stripe_sandbox"))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

/// Yellow striped banner: "TEST-MODUS · Keine echten Zahlungen".
/// Mount above the SiteHeader; the CSS keeps it sticky at the top
/// of the viewport. Renders nothing in production (live) so the
/// document layout is identical there.
#[component]
pub fn TestModeBanner() -> impl IntoView {
    let is_sandbox = stripe_sandbox_sync();
    if !is_sandbox {
        return ().into_any();
    }
    view! {
        <div class="test-mode-banner" role="alert" aria-live="polite">
            <span class="label">"⚠ TEST-MODUS"</span>
            <span class="hint">
                "Keine echten Zahlungen · Stripe-Sandbox aktiv"
            </span>
        </div>
    }
    .into_any()
}
