//! Sticky site header — Italian-flag gradient wordmark on the left,
//! gradient pill nav links on the right. Mounted globally above
//! `<Routes>` in `app.rs`. The cart UX still lives in the floating
//! CartFab; the header intentionally does not duplicate it.
//!
//! Anchor links (`/#delivery`, `/#hours`) only resolve on the home
//! page; from other routes they navigate to home and scroll. That's the
//! expected browser behaviour for in-page anchors.

use leptos::prelude::*;

/// Read the shop name baked into `window.__appBootstrap` by the SSR
/// shell. SSR side reads from `BrandingHandle` directly. We avoid the
/// previous `OnceResource::new(get_shop_name())` because resource
/// reads in the chrome were tripping tachys's hydration walker with
/// "entered unreachable code" — a sync read agrees byte-for-byte
/// between SSR and hydrate.
fn shop_name_sync() -> String {
    #[cfg(feature = "ssr")]
    {
        use_context::<crate::branding::BrandingHandle>()
            .map(|h| h.get().display_name())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Mein Restaurant".to_string())
    }
    #[cfg(not(feature = "ssr"))]
    {
        // Read window.__appBootstrap.shop_name. The script tag that
        // sets it is the first thing in <body>, so it has already
        // executed by the time hydrate runs.
        use leptos::wasm_bindgen::JsValue;
        // Via leptos so the crate also builds with no feature (crates.io verify).
        use leptos::web_sys::{self, js_sys};
        let window = match web_sys::window() {
            Some(w) => w,
            None => return "Mein Restaurant".to_string(),
        };
        let bootstrap = js_sys::Reflect::get(&window, &JsValue::from_str("__appBootstrap"))
            .unwrap_or(JsValue::UNDEFINED);
        let name = js_sys::Reflect::get(&bootstrap, &JsValue::from_str("shop_name"))
            .ok()
            .and_then(|v| v.as_string())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Mein Restaurant".to_string());
        name
    }
}

/// Read the runtime i18n_enabled flag from the SSR-baked bootstrap.
fn i18n_enabled_sync() -> bool {
    #[cfg(feature = "ssr")]
    {
        use_context::<crate::pages::settings::I18nHandle>()
            .map(|h| h.get())
            .unwrap_or(false)
    }
    #[cfg(not(feature = "ssr"))]
    {
        use leptos::wasm_bindgen::JsValue;
        // Via leptos so the crate also builds with no feature (crates.io verify).
        use leptos::web_sys::{self, js_sys};
        let Some(window) = web_sys::window() else {
            return false;
        };
        let bootstrap = js_sys::Reflect::get(&window, &JsValue::from_str("__appBootstrap"))
            .unwrap_or(JsValue::UNDEFINED);
        js_sys::Reflect::get(&bootstrap, &JsValue::from_str("i18n_enabled"))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

/// The site theme and the one visitors may switch to ("" = no switcher),
/// from the SSR-baked bootstrap: the same on the server and in the browser.
fn themes_sync() -> (String, String) {
    #[cfg(feature = "ssr")]
    {
        use_context::<crate::pages::settings::ThemeHandle>()
            .map(|h| (h.get(), h.get_alt()))
            .unwrap_or_else(|| ("warm".to_string(), String::new()))
    }
    #[cfg(not(feature = "ssr"))]
    {
        use leptos::wasm_bindgen::JsValue;
        use leptos::web_sys::{self, js_sys};
        let Some(window) = web_sys::window() else {
            return ("warm".to_string(), String::new());
        };
        let bootstrap = js_sys::Reflect::get(&window, &JsValue::from_str("__appBootstrap"))
            .unwrap_or(JsValue::UNDEFINED);
        let read = |k: &str| {
            js_sys::Reflect::get(&bootstrap, &JsValue::from_str(k))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default()
        };
        (read("theme"), read("theme_alt"))
    }
}

/// Sun/moon button: switches between the shop's theme and its
/// `theme_alt`, remembered in the browser (`dp_theme`; app.rs applies it
/// before paint). Which icon shows is CSS, by the class on <html> — the
/// markup is the same everywhere.
#[component]
fn ThemeToggle() -> impl IntoView {
    let (theme, alt) = themes_sync();
    if alt.is_empty() || alt == theme {
        return ().into_any();
    }
    let toggle = move |_| {
        #[cfg(feature = "hydrate")]
        {
            use leptos::web_sys;
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let Some(html) = doc.document_element() else {
                return;
            };
            let now = html.class_name();
            let next = if now == format!("theme-{alt}") {
                theme.clone()
            } else {
                alt.clone()
            };
            html.set_class_name(&format!("theme-{next}"));
            if let Some(store) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
                let _ = store.set_item("dp_theme", &next);
            }
        }
    };
    // Not on admin pages: they are always light.
    let location = leptos_router::hooks::use_location();
    view! {
        <Show when=move || !crate::app::is_admin_path(&location.pathname.get())>
            <button class="theme-toggle" type="button" title="Hell / Dunkel" aria-label="Hell oder dunkel"
                on:click=toggle.clone()>
                <span class="to-dark" aria-hidden="true">"☾"</span>
                <span class="to-light" aria-hidden="true">"☀"</span>
            </button>
        </Show>
    }
    .into_any()
}

/// In the browser, on every page change: the admin light, the shop pages in
/// the visitor's choice again (what the <head> script does on a full load).
#[cfg(feature = "hydrate")]
fn follow_page_theme() {
    use leptos::web_sys;
    let location = leptos_router::hooks::use_location();
    let (theme, alt) = themes_sync();
    Effect::new(move |_| {
        let path = location.pathname.get();
        let Some(html) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
        else {
            return;
        };
        let stored = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .and_then(|s| s.get_item("dp_theme").ok().flatten())
            .filter(|t| !alt.is_empty() && (*t == theme || *t == alt));
        let want = if crate::app::is_admin_path(&path) {
            "warm".to_string()
        } else {
            stored.unwrap_or_else(|| theme.clone())
        };
        let class = format!("theme-{want}");
        if html.class_name() != class {
            html.set_class_name(&class);
        }
    });
}

#[component]
pub fn SiteHeader() -> impl IntoView {
    let name_upper = shop_name_sync().to_uppercase();

    // "Letzte Bestellung" link — the customer's most recent order, read
    // from the localStorage cache so they can always jump back to its
    // live status page after navigating away / reopening the tab.
    //
    // localStorage doesn't exist on SSR, so this MUST start empty and
    // populate only in a post-hydration Effect — otherwise SSR and the
    // first hydrate render would differ and tachys would panic. The link
    // simply pops in a moment after load on pages where a cached order
    // exists; absent otherwise. (Mirrors the hydration-safe pattern
    // used by the cart badge.)
    let last_order = RwSignal::new(None::<crate::order_cache::OrderRef>);
    #[cfg(feature = "hydrate")]
    {
        Effect::new(move |_| {
            last_order.set(crate::order_cache::latest());
        });
        follow_page_theme();
    }

    view! {
        <header class="site-header">
            <a class="brand" href="/">
                <span class="brand-wordmark">{crate::components::flag_letters::flag_letters(&name_upper)}</span>
            </a>
            <nav class="site-nav">
                // First, not last: at the right edge the admin's settings
                // button covers it.
                <ThemeToggle/>
                {move || last_order.get().map(|o| {
                    let href = format!("/orders/{}", o.id);
                    let label = crate::t!("header.last_order")
                        .replace("{number}", &o.order_number);
                    view! {
                        <a class="pill last-order" href=href>{label}</a>
                    }
                })}
                <a class="pill" href="/menu">{move || crate::t!("header.menu")}</a>
                <a class="pill" href="/#delivery">{move || crate::t!("header.delivery_area")}</a>
                <a class="pill" href="/#hours">{move || crate::t!("header.opening_hours")}</a>
                <LocaleSwitcher/>
            </nav>
        </header>
    }
}

/// Native-`<select>` locale switcher. The runtime `i18n_enabled` admin
/// toggle decides whether it renders. Reads the toggle synchronously
/// from `window.__appBootstrap` so SSR and hydrate emit the same DOM
/// without an `OnceResource`/Suspense pair.
///
/// Plain navigation (window.location.href) instead of the Leptos
/// router's use_navigate so the Router fully remounts with the new
/// `base` prop value — base is Cow<'static, str> and not reactive.
#[component]
fn LocaleSwitcher() -> impl IntoView {
    use crate::i18n::{current_locale, Locale};

    let on = i18n_enabled_sync();
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

    let cls = if on {
        "locale-switcher"
    } else {
        "locale-switcher hidden"
    };
    let aria = if on { "false" } else { "true" };
    let tabidx = if on { 0 } else { -1 };
    view! {
        <select class=cls
            aria-label="Language"
            aria-hidden=aria
            tabindex=tabidx
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
}
