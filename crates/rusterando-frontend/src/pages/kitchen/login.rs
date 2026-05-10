use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::pages::session::Role;

#[server(
    name = KitchenLogin,
    prefix = "/api",
    endpoint = "kitchen_login"
)]
pub async fn kitchen_login(password: String) -> Result<bool, ServerFnError> {
    use crate::pages::session::ssr::try_login;
    let ok = try_login(Role::Kitchen, &password).await?;
    if ok {
        leptos_axum::redirect("/kitchen");
    }
    Ok(ok)
}

#[component]
pub fn KitchenLoginPage() -> impl IntoView {
    let action = ServerAction::<KitchenLogin>::new();
    let pending = action.pending();
    let value = action.value();

    view! {
        <section class="admin-login">
            <h1>"Küche — Anmeldung"</h1>
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
