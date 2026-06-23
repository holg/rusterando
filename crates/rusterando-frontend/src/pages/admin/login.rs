use leptos::prelude::*;

#[server(
    name = AdminLogin,
    prefix = "/api",
    endpoint = "admin_login"
)]
pub async fn admin_login(password: String) -> Result<bool, ServerFnError> {
    use std::sync::Arc;

    // Per-tenant admin password (from the current tenant's `.env`) wins; fall
    // back to the boot-global `Arc<String>` (Model A / unset). Without the
    // tenant check, every tenant accepted the parent showroom's password.
    let expected = use_context::<crate::pages::session::TenantAuth>()
        .and_then(|t| t.admin_password)
        .or_else(|| use_context::<Arc<String>>().map(|a| (*a).clone()))
        .ok_or_else(|| ServerFnError::new("admin password not configured"))?;

    if password != expected {
        return Ok(false);
    }

    // Write the MODERN session cookie (`dp_session=admin`, 365 days) — the
    // same long-lived cookie kitchen/driver use. The old `admin_session=ok`
    // cookie expired after 8h, so admins were silently logged out mid-shift
    // and the client kept trusting a cookie the server had already dropped.
    // current_role() still honors the legacy cookie for sessions in flight.
    crate::pages::session::ssr::set_session(crate::pages::session::Role::Admin).await?;

    leptos_axum::redirect("/admin");
    Ok(true)
}

#[server(
    name = AdminLogout,
    prefix = "/api",
    endpoint = "admin_logout"
)]
pub async fn admin_logout() -> Result<(), ServerFnError> {
    // Clear BOTH the modern `dp_session` cookie (admin now uses it) and the
    // legacy `admin_session=ok`. The shared logout sets each to Max-Age=0 with
    // matching attributes (Path=/, HttpOnly, SameSite=Lax) — a browser only
    // deletes a cookie whose key attributes line up, so the attributes must
    // match the ones set at login or the original lingers.
    crate::pages::session::ssr::logout().await?;
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
