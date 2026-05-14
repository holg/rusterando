use leptos::prelude::*;
use leptos_meta::{provide_meta_context, HashedStylesheet, MetaTags, Title};
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

    // <title> driven by a server-fn-backed Resource so SSR and hydrate
    // render the same value. A direct context read would diverge: SSR
    // sees the BrandingHandle and writes the real shop name, hydrate
    // has no handle and would fall back to a literal — leptos_meta's
    // Title then overwrites the SSR title with whatever hydrate passed,
    // leaking the build-time fallback into the browser tab.
    let shop_name = OnceResource::new(crate::branding::get_shop_name());
    let title_text = move || {
        shop_name
            .get()
            .and_then(|res| res.ok())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Mein Restaurant".to_string())
    };

    view! {
        <Title text=title_text/>

        <Router>
            // Sandbox-only banner. Renders nothing in live mode so the
            // public site looks identical to a production build.
            <TestModeBanner/>
            <SiteHeader/>
            <main>
                <Routes fallback=|| "Seite nicht gefunden.">
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
