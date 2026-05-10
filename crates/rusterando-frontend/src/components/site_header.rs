//! Sticky site header — Italian-flag gradient wordmark on the left,
//! gradient pill nav links on the right. Mounted globally above
//! `<Routes>` in `app.rs`. The cart UX still lives in the floating
//! CartFab; the header intentionally does not duplicate it.
//!
//! Anchor links (`/#delivery`, `/#hours`) only resolve on the home
//! page; from other routes they navigate to home and scroll. That's the
//! expected browser behaviour for in-page anchors.

use leptos::prelude::*;

#[component]
pub fn SiteHeader() -> impl IntoView {
    view! {
        <header class="site-header">
            <a class="brand" href="/">
                <span class="brand-wordmark">"DAVIDS PIZZERIA"</span>
            </a>
            <nav class="site-nav">
                <a class="pill" href="/menu">"Speisekarte"</a>
                <a class="pill" href="/#delivery">"Liefergebiet"</a>
                <a class="pill" href="/#hours">"Öffnungszeiten"</a>
            </nav>
        </header>
    }
}
