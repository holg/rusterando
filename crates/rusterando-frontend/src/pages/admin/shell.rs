//! Shared chrome for every admin page: top bar with logout + nav links.

use leptos::prelude::*;

use crate::pages::admin::login::{admin_logout, AdminLogout};

#[component]
pub fn AdminShell(children: Children) -> impl IntoView {
    let logout = ServerAction::<AdminLogout>::new();

    let do_logout = move |_| {
        logout.dispatch(AdminLogout {});
    };

    #[cfg(feature = "ssr")]
    let shop_name = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get().display_name())
        .unwrap_or_else(|| "Mein Restaurant".to_string());
    #[cfg(not(feature = "ssr"))]
    let shop_name = "Mein Restaurant".to_string();
    let brand_text = format!("{shop_name} — Admin");

    view! {
        <div class="admin-shell">
            <header class="admin-shell-bar no-print">
                <a class="brand" href="/admin">{brand_text}</a>
                <crate::pages::session::RoleSwitcher current=crate::pages::session::Role::Admin/>
                <nav class="admin-nav">
                    <a href="/admin">"Übersicht"</a>
                    <a href="/admin/orders">"Bestellungen"</a>
                    <a href="/admin/menu">"Speisekarte"</a>
                    <a href="/admin/extras">"Extras"</a>
                    <a href="/admin/home">"Startseite"</a>
                    <a href="/admin/broadcast">"Push"</a>
                    <a href="/admin/settings">"Einstellungen"</a>
                    <a href="/admin/history">"Buchhaltung"</a>
                    <a href="/menu.pdf" target="_blank" rel="noopener">"PDF"</a>
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
