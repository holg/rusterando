//! /admin/settings — generic key/value editor for the `app_settings`
//! table. v1 has just one row (free-delivery threshold) but the page
//! is built generically so adding a new tunable is a one-line INSERT
//! migration plus a per-key validation rule in
//! `pages::settings::update_setting`.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;

use crate::pages::admin::shell::AdminShell;
use crate::pages::settings::{list_settings, SettingRow, UpdateSetting};

#[component]
pub fn SettingsAdminPage() -> impl IntoView {
    let updater = ServerAction::<UpdateSetting>::new();
    let settings = Resource::new(
        move || updater.version().get(),
        |_| async move { list_settings().await },
    );

    // Track the key of the most recent save. The action's `input()`
    // signal carries the last dispatched payload; when the matching
    // `value()` resolves Ok, we know exactly which setting just
    // landed and can react to specific ones (e.g. stripe_mode needs
    // a full page reload to refresh Banner + Chip).
    let last_input = updater.input();
    let last_value = updater.value();

    // Hard-reload after a successful stripe_mode flip. The Test/Live
    // banner and the admin chip are both built on OnceResource which
    // doesn't refetch, so the only way to make them reflect the new
    // mode without per-component refresh signals is to reload the
    // page. Mode flips are rare; a reload is fine UX.
    //
    // Wrapped in #[cfg(feature = "hydrate")] because window is a
    // browser API — SSR has no window object.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let key = last_input.get().map(|i| i.key);
        let ok = matches!(last_value.get(), Some(Ok(_)));
        if ok && key.as_deref() == Some("stripe_mode") {
            if let Some(w) = web_sys::window() {
                let _ = w.location().reload();
            }
        }
    });
    // Reference the locals on the SSR side to silence the
    // unused-variable warning the cfg-gated effect would otherwise
    // produce.
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = (last_input, last_value);
    }

    view! {
        <AdminShell>
            <section class="settings-admin">
                <header class="admin-bar">
                    <h1>"Einstellungen"</h1>
                </header>

                <p class="hint">
                    "Globale Tunables. Änderungen wirken sofort, ohne Deploy."
                </p>

                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || settings.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(rows) => view! { <SettingsTable rows updater/> }.into_any(),
                    })}
                </Suspense>

                {move || updater.value().get().and_then(|res| match res {
                    Err(e) => Some(view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any()),
                    Ok(_) => None,
                })}
            </section>
        </AdminShell>
    }
}

#[component]
fn SettingsTable(rows: Vec<SettingRow>, updater: ServerAction<UpdateSetting>) -> impl IntoView {
    if rows.is_empty() {
        return view! { <p class="empty">"Keine Einstellungen."</p> }.into_any();
    }
    view! {
        <table class="settings-table">
            <thead>
                <tr>
                    <th>"Bezeichnung"</th>
                    <th>"Wert"</th>
                    <th></th>
                </tr>
            </thead>
            <tbody>
                {rows.into_iter().map(|r| view! { <SettingRowView r updater/> }).collect_view()}
            </tbody>
        </table>
    }
    .into_any()
}

#[component]
fn SettingRowView(r: SettingRow, updater: ServerAction<UpdateSetting>) -> impl IntoView {
    let key = r.key.clone();
    let label = r.label_de.clone();
    let hint = r.hint_de.clone();
    let value = RwSignal::new(r.value.clone());

    // For cents-typed settings (currently just the threshold) show a
    // live euro preview. Recognised by the key suffix.
    let is_cents = key.ends_with("_cents");
    let is_theme = key == "theme";
    let is_stripe_mode = key == "stripe_mode";
    let preview_eur = Memo::new(move |_| {
        value
            .get()
            .trim()
            .parse::<i64>()
            .map(format_eur)
            .unwrap_or_else(|_| "—".to_string())
    });

    let key_for_save = key.clone();
    let on_save = move |_| {
        updater.dispatch(UpdateSetting {
            key: key_for_save.clone(),
            value: value.get(),
        });
    };

    let editor = if is_theme {
        view! {
            <select class="cell-input"
                prop:value=move || value.get()
                on:change=move |ev| value.set(event_target_value(&ev))>
                <option value="warm">"warm — beige/rot (Standard)"</option>
                <option value="dark">"dark — dunkel mit roten Akzenten"</option>
            </select>
        }
        .into_any()
    } else if is_stripe_mode {
        view! {
            <select class="cell-input"
                prop:value=move || value.get()
                on:change=move |ev| value.set(event_target_value(&ev))>
                <option value="sandbox">"Sandbox (Test) — keine echten Zahlungen"</option>
                <option value="live">"Live (Echtbetrieb) — Karten werden belastet"</option>
            </select>
        }
        .into_any()
    } else {
        view! {
            <input type="text" class="cell-input"
                prop:value=move || value.get()
                on:input=move |ev| value.set(event_target_value(&ev))/>
        }
        .into_any()
    };

    view! {
        <tr>
            <td>
                <strong>{label}</strong>
                {hint.map(|h| view! { <p class="muted">{h}</p> })}
            </td>
            <td>
                {editor}
                {is_cents.then(|| view! {
                    <span class="muted">" = " {move || preview_eur.get()}</span>
                })}
            </td>
            <td>
                <button class="btn primary small" on:click=on_save>"Speichern"</button>
            </td>
        </tr>
    }
}
