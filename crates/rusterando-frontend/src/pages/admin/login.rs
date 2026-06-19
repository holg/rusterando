use leptos::prelude::*;

#[server(
    name = AdminLogin,
    prefix = "/api",
    endpoint = "admin_login"
)]
pub async fn admin_login(password: String) -> Result<bool, ServerFnError> {
    use leptos_axum::extract;
    use std::sync::Arc;
    use tower_cookies::{Cookie, Cookies};

    // Per-tenant admin password (from the current tenant's `.env`) wins; fall
    // back to the boot-global `Arc<String>` (Model A / unset). Without the
    // tenant check, every tenant accepted the parent showroom's password.
    let expected = use_context::<crate::pages::session::TenantAuth>()
        .and_then(|t| t.admin_password)
        .or_else(|| use_context::<Arc<String>>().map(|a| (*a).clone()))
        .ok_or_else(|| ServerFnError::new("admin password not configured"))?;
    let cookies: Cookies = extract().await?;

    if password != expected {
        return Ok(false);
    }

    let mut c = Cookie::new("admin_session", "ok");
    c.set_path("/");
    c.set_http_only(true);
    c.set_same_site(tower_cookies::cookie::SameSite::Lax);
    c.set_max_age(tower_cookies::cookie::time::Duration::hours(8));
    cookies.add(c);

    leptos_axum::redirect("/admin");
    Ok(true)
}

#[server(
    name = AdminLogout,
    prefix = "/api",
    endpoint = "admin_logout"
)]
pub async fn admin_logout() -> Result<(), ServerFnError> {
    use leptos_axum::extract;
    use tower_cookies::{Cookie, Cookies};
    let cookies: Cookies = extract().await?;
    let mut c = Cookie::new("admin_session", "");
    c.set_path("/");
    c.set_max_age(tower_cookies::cookie::time::Duration::ZERO);
    cookies.add(c);
    leptos_axum::redirect("/");
    Ok(())
}

#[component]
pub fn AdminLoginPage() -> impl IntoView {
    let action = ServerAction::<AdminLogin>::new();
    let pending = action.pending();
    let value = action.value();

    view! {
        <section class="admin-login">
            <h1>"Admin-Anmeldung"</h1>
            <ActionForm action=action>
                <label>
                    <span>"Passwort"</span>
                    <input type="password" name="password" required autofocus/>
                </label>
                <button type="submit" disabled=move || pending.get()>
                    {move || if pending.get() { "Bitte warten…" } else { "Anmelden" }}
                </button>
            </ActionForm>
            {move || match value.get() {
                Some(Ok(true))  => Some(view! { <p class="ok">"Erfolg, leite weiter…"</p> }.into_any()),
                Some(Ok(false)) => Some(view! { <p class="error">"Falsches Passwort."</p> }.into_any()),
                Some(Err(e))    => Some(view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any()),
                None            => None,
            }}
        </section>
    }
}
