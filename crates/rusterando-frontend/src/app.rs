use leptos::prelude::*;
#[cfg(feature = "ssr")]
use leptos_meta::Title;
use leptos_meta::{provide_meta_context, HashedStylesheet, MetaTags};
use leptos_router::components::{Route, Router, Routes};
use leptos_router::{ParamSegment, StaticSegment};

use crate::components::cart_drawer::{provide_cart_ctx, CartDrawer, CartFab};
use crate::components::site_header::SiteHeader;
use crate::components::test_mode_banner::TestModeBanner;
use crate::pages::admin::broadcast::BroadcastAdminPage;
use crate::pages::admin::customers::{AdminCustomerDetailPage, AdminCustomersPage};
use crate::pages::admin::extras_admin::ExtrasAdminPage;
use crate::pages::admin::history::AdminHistoryPage;
use crate::pages::admin::home::AdminHomePage;
use crate::pages::admin::home_admin::HomeAdminPage;
use crate::pages::admin::login::AdminLoginPage;
use crate::pages::admin::menu_admin::AdminMenuPage;
use crate::pages::admin::orders::{AdminOrderDetailPage, AdminOrdersPage};
use crate::pages::admin::pdf::PdfAdminPage;
use crate::pages::admin::pricing::PricingAdminPage;
use crate::pages::admin::settings_admin::SettingsAdminPage;
use crate::pages::admin::vouchers::VouchersAdminPage;
use crate::pages::admin::zones::ZonesAdminPage;
use crate::pages::driver::board::DriverBoardPage;
use crate::pages::driver::login::DriverLoginPage;
use crate::pages::home::Home;
use crate::pages::kitchen::board::KitchenBoardPage;
use crate::pages::kitchen::login::KitchenLoginPage;
use crate::pages::legal::{DatenschutzPage, ImpressumPage};
use crate::pages::menu::MenuPage;
use crate::pages::order::{CheckoutPage, OrderConfirmationPage};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    // SSR-time theme lookup. The `ThemeHandle` is provided by the server
    // crate's route context (see main.rs); the read is sync (RwLock) so
    // the very first byte of the response carries the right class.
    // Hydration falls back to "warm" when the handle isn't in scope (which
    // doesn't happen on prod but keeps the type-checker happy on the
    // hydrate side).
    let theme_class = {
        #[cfg(feature = "ssr")]
        {
            use_context::<crate::pages::settings::ThemeHandle>()
                .map(|h| h.get())
                .unwrap_or_else(|| "warm".to_string())
        }
        #[cfg(not(feature = "ssr"))]
        {
            "warm".to_string()
        }
    };
    let html_class = format!("theme-{theme_class}");

    view! {
        <!DOCTYPE html>
        <html lang="de" class=html_class>
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options=options.clone()/>
                <HashedStylesheet id="leptos" options/>
                <MetaTags/>
                <script>
                    {r#"
                    // Translate vertical mouse-wheel into horizontal scroll on .category-tabs.
                    document.addEventListener('wheel', function (e) {
                        var el = e.target.closest && e.target.closest('.category-tabs');
                        if (!el) return;
                        if (e.deltaY === 0 || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
                        el.scrollLeft += e.deltaY;
                        e.preventDefault();
                    }, { passive: false });

                    // Stripe Payment Element bootstrap. Called from Rust (via wasm_bindgen
                    // extern "C") once the server fn returns a client_secret. We lazily load
                    // Stripe.js so customers paying cash never download it.
                    window.__dpInitStripe = function (pk, clientSecret, returnUrl) {
                        function ensureStripeLoaded(cb) {
                            if (window.Stripe) return cb();
                            var s = document.createElement('script');
                            s.src = 'https://js.stripe.com/v3/';
                            s.async = true;
                            s.onload = cb;
                            document.head.appendChild(s);
                        }
                        ensureStripeLoaded(function () {
                            // The mount target is rendered by Leptos; wait for it.
                            function wait(retries) {
                                var mount = document.getElementById('payment-element');
                                if (!mount) {
                                    if (retries > 0) setTimeout(function(){ wait(retries-1); }, 50);
                                    return;
                                }
                                var stripe = window.Stripe(pk);
                                var elements = stripe.elements({
                                    clientSecret: clientSecret,
                                    appearance: { theme: 'stripe' }
                                });
                                var pay = elements.create('payment', { layout: 'tabs' });
                                pay.mount(mount);

                                var btn = document.getElementById('payment-submit');
                                var msg = document.getElementById('payment-message');
                                if (btn) {
                                    btn.addEventListener('click', async function () {
                                        btn.disabled = true;
                                        msg.style.display = 'none';
                                        var fullReturnUrl = returnUrl || (window.location.origin + '/');
                                        var res = await stripe.confirmPayment({
                                            elements: elements,
                                            confirmParams: { return_url: fullReturnUrl }
                                        });
                                        // confirmPayment only resolves with an error here; on
                                        // success Stripe redirects to return_url itself.
                                        if (res.error) {
                                            msg.textContent = res.error.message || 'Zahlung fehlgeschlagen.';
                                            msg.style.display = 'block';
                                            btn.disabled = false;
                                        }
                                    });
                                }
                            }
                            wait(50);
                        });
                    };
                    "#}
                </script>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    provide_cart_ctx();

    // i18n: install the locale context AND derive the Router base
    // path so the inner Routes block matches `/menu` regardless of
    // whether the request is `/menu` or `/en/menu`. Always-on now —
    // the runtime `i18n_enabled` admin toggle separately decides
    // whether the locale switcher renders and whether prefixed routes
    // are served (server-side middleware enforces the 404 when off).
    //
    // SSR + hydrate both compute the same value from the same path so
    // there's no DOM mismatch at hydrate time. Hydrate reads
    // window.location.pathname; SSR reads it from leptos_router's
    // RequestUrl context.
    let path = current_path();
    let (loc, _rest) = crate::i18n::split_locale_prefix(&path);
    let initial = loc.unwrap_or(crate::i18n::Locale::DEFAULT);
    let _ = crate::i18n::provide_locale_ctx(initial);
    // Map the resolved locale to a *static* base string — required
    // by Router's `base: Cow<'static, str>` prop. Default locale
    // uses "" so canonical URLs have no prefix.
    let router_base: &'static str = match initial {
        crate::i18n::Locale::De => "",
        crate::i18n::Locale::En => "/en",
        crate::i18n::Locale::Fr => "/fr",
        crate::i18n::Locale::It => "/it",
        crate::i18n::Locale::Es => "/es",
        crate::i18n::Locale::Pt => "/pt",
        crate::i18n::Locale::Ru => "/ru",
        crate::i18n::Locale::Cn => "/cn",
    };

    // <title> resolution:
    //   - SSR side: reads BrandingHandle from context and emits the
    //     real shop name into the SSR HTML's <title> tag.
    //   - Hydrate side: SSR already wrote the correct title, so we
    //     skip the <Title> emit and leave document.title alone.
    //
    // The previous implementation used `OnceResource::new(get_shop_name())`
    // here, but a resource read **outside** a <Suspense> caused tachys
    // to panic during hydration with "entered unreachable code" — the
    // resource's await-point couldn't be coordinated with the SSR
    // stream markers at the document root. Splitting the path by
    // feature gate eliminates the resource entirely.
    // On SSR, read the real shop name from BrandingHandle and emit a
    // <Title> so the streamed HTML carries the correct browser-tab
    // text. On hydrate, do nothing — the title is already in the DOM
    // and re-emitting it would either flicker or panic (the previous
    // `OnceResource::new` version triggered the tachys "entered
    // unreachable code" hydration panic because a resource read
    // outside a <Suspense> at the document root can't be coordinated
    // with the SSR stream markers).
    let title_node = {
        #[cfg(feature = "ssr")]
        {
            let name = use_context::<crate::branding::BrandingHandle>()
                .map(|h| h.get().shop_name)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Mein Restaurant".to_string());
            Some(view! { <Title text=name/> })
        }
        #[cfg(not(feature = "ssr"))]
        {
            None::<leptos::tachys::view::any_view::AnyView>
        }
    };

    view! {
        {title_node}

        <Router base=router_base>
            // Sandbox-only banner. Renders nothing in live mode so the
            // public site looks identical to a production build.
            <TestModeBanner/>
            <SiteHeader/>
            <main>
                <Routes fallback=|| crate::t!("errors.page_not_found")>
                    <Route path=StaticSegment("") view=Home/>
                    <Route path=StaticSegment("menu") view=MenuPage/>
                    <Route path=StaticSegment("checkout") view=CheckoutPage/>
                    <Route path=(StaticSegment("orders"), ParamSegment("id")) view=OrderConfirmationPage/>
                    <Route path=StaticSegment("admin") view=AdminHomePage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("login")) view=AdminLoginPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("menu")) view=AdminMenuPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("extras")) view=ExtrasAdminPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("home")) view=HomeAdminPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("settings")) view=SettingsAdminPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("broadcast")) view=BroadcastAdminPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("orders")) view=AdminOrdersPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("orders"), ParamSegment("id")) view=AdminOrderDetailPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("history")) view=AdminHistoryPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("customers")) view=AdminCustomersPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("customers"), ParamSegment("id")) view=AdminCustomerDetailPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("vouchers")) view=VouchersAdminPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("zones")) view=ZonesAdminPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("pdf")) view=PdfAdminPage/>
                    <Route path=(StaticSegment("admin"), StaticSegment("pricing")) view=PricingAdminPage/>
                    <Route path=StaticSegment("kitchen") view=KitchenBoardPage/>
                    <Route path=(StaticSegment("kitchen"), StaticSegment("login")) view=KitchenLoginPage/>
                    <Route path=StaticSegment("driver") view=DriverBoardPage/>
                    <Route path=(StaticSegment("driver"), StaticSegment("login")) view=DriverLoginPage/>
                    <Route path=StaticSegment("datenschutz") view=DatenschutzPage/>
                    <Route path=StaticSegment("impressum") view=ImpressumPage/>
                </Routes>
            </main>
            <CartFab/>
            <CartDrawer/>
        </Router>
    }
}

/// Best-effort read of the current URL path. Used by the i18n
/// resolver to spot a /<lang>/ prefix at App() boot — before the
/// Router has had a chance to set up its own location signal.
///
/// SSR: the path comes from the http::request::Parts that the leptos
/// integration layer makes available via context (provided by
/// leptos_axum's route handler). Hydrate: window.location.pathname.
fn current_path() -> String {
    #[cfg(feature = "ssr")]
    {
        // leptos_axum's provide_contexts() stores the URL as
        // `http://leptos.dev/<actual-path>` — the scheme/host are a
        // fake but the path is real. We parse with `url::Url` to
        // extract the path cleanly. Falling back to "/" if anything
        // goes wrong keeps the locale resolver defensive.
        if let Some(req_url) = use_context::<leptos_router::location::RequestUrl>() {
            let raw: &str = req_url.as_ref();
            // leptos_axum stores the URL as `http://leptos.dev/<path>`
            // — scheme + host are placeholders, the path after the
            // host is real. We extract by hand to avoid pulling in
            // the `url` crate as a direct dep.
            let no_scheme = raw.find("://").map(|i| &raw[i + 3..]).unwrap_or(raw);
            let path_and_q = no_scheme.find('/').map(|i| &no_scheme[i..]).unwrap_or("/");
            let path = path_and_q
                .split_once('?')
                .map(|(p, _)| p)
                .unwrap_or(path_and_q);
            return path.to_string();
        }
        "/".to_string()
    }
    #[cfg(all(feature = "hydrate", not(feature = "ssr")))]
    {
        if let Some(win) = web_sys::window() {
            if let Ok(p) = win.location().pathname() {
                return p;
            }
        }
        "/".to_string()
    }
    #[cfg(not(any(feature = "ssr", feature = "hydrate")))]
    {
        "/".to_string()
    }
}
