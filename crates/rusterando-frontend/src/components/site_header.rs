//! Sticky site header — Italian-flag gradient wordmark on the left,
//! gradient pill nav links on the right. Mounted globally above
//! `<Routes>` in `app.rs`. The cart UX still lives in the floating
//! CartFab; the header intentionally does not duplicate it.
//!
//! Anchor links (`/#delivery`, `/#hours`) only resolve on the home
//! page; from other routes they navigate to home and scroll. That's the
//! expected browser behaviour for in-page anchors.

use leptos::prelude::*;

use crate::branding::get_shop_name;

#[component]
pub fn SiteHeader() -> impl IntoView {
    // Wordmark text comes from `BrandingHandle.shop_name` via a server
    // fn so SSR + hydrate render the same DOM. A direct context read
    // would diverge: SSR returns the cached value, hydrate has no
    // `BrandingHandle` (it's an SSR-only newtype) and would render
    // empty → tachys hydration mismatch.
    let name = OnceResource::new(get_shop_name());
    view! {
        <header class="site-header">
            <a class="brand" href="/">
                <Suspense fallback=|| view! {
                    <span class="brand-wordmark">"\u{00a0}"</span>
                }>
                    {move || name.get().map(|res| {
                        let upper = res
                            .ok()
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or_else(|| "Mein Restaurant".to_string())
                            .to_uppercase();
                        view! { <span class="brand-wordmark">{upper}</span> }
                    })}
                </Suspense>
            </a>
            <nav class="site-nav">
                <a class="pill" href="/menu">"Speisekarte"</a>
                <a class="pill" href="/#delivery">"Liefergebiet"</a>
                <a class="pill" href="/#hours">"Öffnungszeiten"</a>
            </nav>
        </header>
    }
}
