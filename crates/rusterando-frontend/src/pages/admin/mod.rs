pub mod broadcast;
pub mod extras_admin;
pub mod history;
pub mod home;
pub mod home_admin;
pub mod login;
pub mod menu_admin;
pub mod orders;
pub mod settings_admin;
pub mod shell;

/// Server-side helper to verify the admin cookie. Returns Ok(()) when allowed.
#[cfg(feature = "ssr")]
pub async fn require_admin() -> Result<(), leptos::prelude::ServerFnError> {
    use leptos::prelude::ServerFnError;
    use leptos_axum::extract;
    use tower_cookies::Cookies;

    let cookies: Cookies = extract().await?;
    match cookies.get("admin_session") {
        Some(c) if c.value() == "ok" => Ok(()),
        _ => Err(ServerFnError::new("nicht angemeldet")),
    }
}
