//! Shared chrome for every admin page: top bar with logout + nav links.

use leptos::prelude::*;

use crate::branding::get_shop_name;
use crate::pages::admin::login::{admin_logout, AdminLogout};
use crate::stripe::get_stripe_mode;

#[component]
pub fn AdminShell(children: Children) -> impl IntoView {
    let logout = ServerAction::<AdminLogout>::new();

    let do_logout = move |_| {
        logout.dispatch(AdminLogout {});
    };

    // Brand text via Resource (same pattern as stripe chip + theme).
    // Previously this was a `#[cfg(feature = "ssr")]` context read, which
    // rendered "Davids Pizzeria — Admin" on the server but
    // "Mein Restaurant — Admin" on hydrate → tachys hit a DOM
    // mismatch on the very first node of the shell header and
    // aborted the rest of the hydration walk, leaving the top-nav
    // <a>-tags as un-bound DOM. The symptom: nav links did nothing
    // on /admin until the admin did one full-page navigation.
    let shop = OnceResource::new(get_shop_name());
    // Stripe mode chip — same OnceResource pattern as the customer
    // banner so SSR + hydrate render the same DOM. Green for Live,
    // amber for Sandbox. Always visible so an admin can't miss
    // which mode they're in.
    let mode = OnceResource::new(get_stripe_mode());

    // Shop online/offline chip — visible on EVERY admin page so staff
    // always see whether online ordering is live. Initial state from the
    // `shop_status` server fn (OnceResource, same hydration-safe pattern
    // as the mode chip); the SSE `/api/live/shop` channel flips it live
    // via a post-hydration Effect (so no reload needed after a toggle).
    let shop_status = OnceResource::new(crate::pages::menu::shop_status());
    let (shop_live, set_shop_live) =
        signal::<Option<(rusterando_shared::models::ShopLevel, String)>>(None);
    crate::utils::subscribe_shop_status(set_shop_live);

    view! {
        <div class="admin-shell">
            <header class="admin-shell-bar no-print">
                // No <Suspense> here: we read the resource directly and
                // render text-only. Without a Suspense boundary tachys
                // doesn't try to cast an element placeholder, so the
                // walker stays in sync between SSR (resolved value) and
                // hydrate (None on the first tick, then resolved). The
                // text-node update is invisible to the walker because
                // walker-vs-resource updates are decoupled.
                <a class="brand" href="/admin">
                    {move || {
                        let name = shop
                            .get()
                            .and_then(|r| r.ok())
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or_else(|| "Mein Restaurant".to_string());
                        format!("{name} — Admin")
                    }}
                </a>
                // Suspense wrap so the resource read happens inside.
                // Fallback + resolved both render the same <a> shape.
                <Suspense fallback=|| view! {
                    <a class="admin-mode-chip sandbox" href="/admin/settings" title="Sandbox">
                        <span class="dot"></span>
                        <span class="label">"TEST"</span>
                        <span class="hint">"Sandbox"</span>
                    </a>
                }>
                    {move || {
                        let is_live = mode
                            .get()
                            .and_then(|r| r.ok())
                            .map(|s| s == "live")
                            .unwrap_or(false);
                        let (cls, label, hint) = if is_live {
                            ("admin-mode-chip live", "LIVE", "Echtbetrieb")
                        } else {
                            ("admin-mode-chip sandbox", "TEST", "Sandbox")
                        };
                        view! {
                            <a class=cls href="/admin/settings" title=hint>
                                <span class="dot"></span>
                                <span class="label">{label}</span>
                                <span class="hint">{hint}</span>
                            </a>
                        }
                    }}
                </Suspense>
                // Shop online/offline chip — links to /admin/hours to toggle.
                // Own <Suspense> with a matching fallback shape (same as the
                // mode chip) so SSR + hydrate agree; SSE flips it live.
                <Suspense fallback=|| view! {
                    <a class="shop-online-chip" href="/admin/hours" title="Öffnungszeiten">
                        <span class="dot"></span>
                        <span class="label">"…"</span>
                    </a>
                }>
                    {move || {
                        use rusterando_shared::models::ShopLevel;
                        // Live override wins over the initial resource value.
                        let (level, reason) = shop_live.get().unwrap_or_else(|| {
                            shop_status
                                .get()
                                .and_then(|r| r.ok())
                                .unwrap_or((ShopLevel::Open, String::new()))
                        });
                        // Three states mirror the customer banner: green
                        // Online, amber "Öffnet später" (opens-later / pause),
                        // red Offline (Ruhetag / hard closed).
                        let (cls, label) = match level {
                            ShopLevel::Open => ("shop-online-chip online", "Online"),
                            ShopLevel::OpensLater => ("shop-online-chip opens-soon", "Öffnet später"),
                            ShopLevel::Closed => ("shop-online-chip offline", "Offline"),
                        };
                        let title = if reason.is_empty() { "Öffnungszeiten".to_string() } else { reason };
                        view! {
                            <a class=cls href="/admin/hours" title=title>
                                <span class="dot"></span>
                                <span class="label">{label}</span>
                            </a>
                        }
                    }}
                </Suspense>
                <crate::pages::session::RoleSwitcher current=crate::pages::session::Role::Admin/>
                <nav class="admin-nav">
                    <a href="/admin">"Übersicht"</a>
                    <a href="/admin/orders">"Bestellungen"</a>
                    <a href="/admin/menu">"Speisekarte"</a>
                    <a href="/admin/extras">"Extras"</a>
                    <a href="/admin/pricing">"Preis-Prüfung"</a>
                    <a href="/admin/home">"Startseite"</a>
                    <a href="/admin/broadcast">"Push"</a>
                    <a href="/admin/settings">"Einstellungen"</a>
                    <a href="/admin/history">"Buchhaltung"</a>
                    <a href="/admin/customers">"Kunden"</a>
                    <a href="/admin/vouchers">"Gutscheine"</a>
                    <a href="/admin/zones">"Liefergebiete"</a>
                    <a href="/admin/localities">"PLZ ↔ Ort"</a>
                    <a href="/admin/address-attempts">"Abgelehnte Adressen"</a>
                    <a href="/admin/hours">"Öffnungszeiten"</a>
                    <a href="/admin/pdf">"PDF-Editor"</a>
                    <button class="logout" on:click=do_logout>"Abmelden"</button>
                </nav>
            </header>
            <div class="admin-shell-body">
                {children()}
            </div>
        </div>
    }
}

// Suppress unused-import warning when admin_logout is only used by the macro expansion.
#[allow(dead_code)]
fn _admin_logout_typecheck() {
    let _ = admin_logout;
}
