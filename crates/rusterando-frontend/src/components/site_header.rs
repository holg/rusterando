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
                <a class="pill" href="/menu">{crate::t!("header.menu")}</a>
                <a class="pill" href="/#delivery">{crate::t!("header.delivery_area")}</a>
                <a class="pill" href="/#hours">{crate::t!("header.opening_hours")}</a>
                <LocaleSwitcher/>
            </nav>
        </header>
    }
}

/// Native-`<select>` locale switcher. Renders nothing in off-builds
/// so Davids' header stays unchanged. On feature=i18n it lists the
/// eight supported languages by their endonym; changing the value
/// navigates to the equivalent path under the chosen locale prefix.
///
/// Plain navigation (window.location.href) instead of the Leptos
/// router's use_navigate so the Router fully remounts with the new
/// `base` prop value — base is Cow<'static, str> and not reactive.
#[component]
fn LocaleSwitcher() -> impl IntoView {
    #[cfg(feature = "i18n")]
    {
        use crate::i18n::{current_locale, Locale};
        let active = current_locale();
        // The change handler navigates the browser; on SSR there's no
        // window so the handler is a no-op (SSR never fires events
        // anyway, but the closure still has to type-check). We gate
        // the web_sys block on hydrate-only deps.
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
                let _ = ev; // SSR: noop, hydrate replaces this handler
            }
        };
        view! {
            <select class="locale-switcher" on:change=on_change aria-label="Language">
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
        .into_any()
    }
    #[cfg(not(feature = "i18n"))]
    {
        ().into_any()
    }
}
