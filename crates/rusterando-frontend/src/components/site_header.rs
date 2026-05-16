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
use crate::i18n::get_i18n_enabled;

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
                <a class="pill" href="/menu">{crate::t!("header.menu")}</a>
                <a class="pill" href="/#delivery">{crate::t!("header.delivery_area")}</a>
                <a class="pill" href="/#hours">{crate::t!("header.opening_hours")}</a>
                <LocaleSwitcher/>
            </nav>
        </header>
    }
}

/// Native-`<select>` locale switcher. The runtime `i18n_enabled` admin
/// toggle decides whether it renders — same OnceResource pattern as
/// the shop name / Stripe-mode chip so SSR + hydrate agree on the DOM.
///
/// Plain navigation (window.location.href) instead of the Leptos
/// router's use_navigate so the Router fully remounts with the new
/// `base` prop value — base is Cow<'static, str> and not reactive.
#[component]
fn LocaleSwitcher() -> impl IntoView {
    use crate::i18n::{current_locale, Locale};

    let enabled = OnceResource::new(get_i18n_enabled());
    let active = current_locale();

    let on_change = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            use crate::i18n::with_locale_prefix;
            use leptos::wasm_bindgen::JsCast;
            let select = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok());
            let Some(select) = select else { return };
            let code = select.value();
            let Some(target_loc) = Locale::parse(&code) else {
                return;
            };
            let cur_path = web_sys::window()
                .and_then(|w| w.location().pathname().ok())
                .unwrap_or_else(|| "/".to_string());
            let next = with_locale_prefix(target_loc, &cur_path);
            if let Some(win) = web_sys::window() {
                let _ = win.location().set_href(&next);
            }
        }
        #[cfg(not(feature = "hydrate"))]
        {
            let _ = ev;
        }
    };

    // The Suspense fallback + resolved branch render the same DOM
    // shape — a single <select> with the `.hidden` class toggled —
    // so tachys' hydration walker doesn't trip over a tag mismatch
    // between SSR and the first client tick.
    view! {
        <Suspense fallback=|| view! {
            <select class="locale-switcher hidden" aria-hidden="true" tabindex="-1">
                <option value="de" selected=true>"Deutsch"</option>
            </select>
        }>
            {move || {
                let on = enabled.get().and_then(|r| r.ok()).unwrap_or(false);
                let cls = if on { "locale-switcher" } else { "locale-switcher hidden" };
                let aria = if on { "false" } else { "true" };
                view! {
                    <select class=cls
                        aria-label="Language"
                        aria-hidden=aria
                        tabindex=move || if on { 0 } else { -1 }
                        on:change=on_change>
                        {Locale::ALL.iter().map(|&loc| {
                            let code = loc.code();
                            let label = loc.label();
                            let selected = loc == active;
                            view! {
                                <option value=code selected=selected>{label}</option>
                            }
                        }).collect_view()}
                    </select>
                }
            }}
        </Suspense>
    }
}
