//! Sticky top-bar that appears on every public page when the shop
//! is in Stripe sandbox mode. Disappears completely in live mode
//! so production traffic sees no chrome.
//!
//! Driven by a server-fn-backed `OnceResource` (`get_stripe_mode`)
//! so SSR and hydrate render the same DOM. Reading the
//! `StripeModeHandle` directly from context would diverge (handle
//! is SSR-only) and crash hydration — same trap the wordmark hit
//! before we ran it through a resource.

use leptos::prelude::*;

use crate::stripe::get_stripe_mode;

/// Yellow striped banner: "TEST-MODUS · Keine echten Zahlungen".
/// Mount above the SiteHeader; the CSS keeps it sticky at the top
/// of the viewport. Width-fills the page; renders nothing in
/// production (live) so the document layout is identical there.
#[component]
pub fn TestModeBanner() -> impl IntoView {
    let mode = OnceResource::new(get_stripe_mode());

    view! {
        // Suspense fallback intentionally empty: during the first
        // SSR pass we don't know the mode yet and we don't want a
        // visible flash. The banner appears once the resource
        // resolves (which on SSR is immediate after the server fn
        // runs; on hydrate it reads the streamed payload).
        <Suspense fallback=|| ()>
            {move || mode.get().map(|res| {
                let is_sandbox = res
                    .as_deref()
                    .map(|s| s == "sandbox")
                    .unwrap_or(false);
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
                }.into_any()
            })}
        </Suspense>
    }
}
