pub mod address_attempts;
pub mod broadcast;
pub mod customers;
pub mod extras_admin;
pub mod history;
pub mod home;
pub mod home_admin;
pub mod hours;
pub mod login;
pub mod menu_admin;
pub mod options_admin;
pub mod orders;
pub mod pdf;
pub mod pricing;
pub mod printer_admin;
pub mod printer_status;
pub mod settings_admin;
pub mod shell;
pub mod translations;
pub mod vouchers;
pub mod zones;

/// Server-side helper to verify the admin cookie. Returns Ok(()) when allowed.
///
/// Delegates to the canonical session resolver (`session::ssr::current_role`),
/// which honors BOTH the modern `dp_session=admin` cookie (set by admin_login
/// since the 365-day migration) and the legacy `admin_session=ok`. Reading
/// only `admin_session` here was the bug that 401'd a freshly-logged-in admin.
#[cfg(feature = "ssr")]
pub async fn require_admin() -> Result<(), leptos::prelude::ServerFnError> {
    use crate::pages::session::Role;
    use leptos::prelude::ServerFnError;

    match crate::pages::session::ssr::current_role().await {
        Some(Role::Admin) => Ok(()),
        _ => Err(ServerFnError::new("nicht angemeldet")),
    }
}
