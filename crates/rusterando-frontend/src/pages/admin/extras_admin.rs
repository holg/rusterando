//! Admin CRUD for the pizza-extras catalog.
//!
//! David maintains the list of extras (Tabasco, Extra Käse, Krabben, …) plus
//! their per-piece price and availability. The menu page's extras picker is
//! driven entirely by this table — flipping `is_available = 0` on Tabasco
//! removes it from the picker the moment the page next loads.

use leptos::prelude::*;
use rusterando_shared::models::{format_eur, PizzaExtra};
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtraAdminRow {
    pub id: String,
    pub label: String,
    pub price_cents: i64,
    pub is_available: bool,
    pub sort_order: i64,
}

#[server(
    name = ListExtrasAdmin,
    prefix = "/api",
    endpoint = "list_extras_admin"
)]
pub async fn list_extras_admin() -> Result<Vec<ExtraAdminRow>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, label, price_cents, is_available, sort_order
         FROM pizza_extras ORDER BY sort_order, label",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load extras: {e}")))?;

    Ok(rows
        .into_iter()
        .map(|(id, label, price, avail, sort_order)| ExtraAdminRow {
            id,
            label,
            price_cents: price,
            is_available: avail != 0,
            sort_order,
        })
        .collect())
}

/// Public read used by the menu page's extras picker. Only available rows.
#[server(
    name = ListPizzaExtras,
    prefix = "/api",
    endpoint = "list_pizza_extras"
)]
pub async fn list_pizza_extras() -> Result<Vec<PizzaExtra>, ServerFnError> {
    use sqlx::SqlitePool;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, label, price_cents, sort_order
         FROM pizza_extras
         WHERE is_available = 1
         ORDER BY sort_order, label",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load extras: {e}")))?;

    Ok(rows
        .into_iter()
        .map(|(id, label, price_cents, sort_order)| PizzaExtra {
            id,
            label,
            price_cents,
            sort_order,
        })
        .collect())
}

#[server(
    name = UpdateExtra,
    prefix = "/api",
    endpoint = "update_extra"
)]
pub async fn update_extra(
    id: String,
    label: String,
    price_cents: i64,
    is_available: bool,
    sort_order: i64,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    if label.trim().is_empty() {
        return Err(ServerFnError::new("Label darf nicht leer sein."));
    }
    if price_cents < 0 {
        return Err(ServerFnError::new("Preis darf nicht negativ sein."));
    }
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    sqlx::query(
        "UPDATE pizza_extras
         SET label = ?2, price_cents = ?3, is_available = ?4, sort_order = ?5,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
    )
    .bind(&id)
    .bind(label.trim())
    .bind(price_cents)
    .bind(if is_available { 1 } else { 0 })
    .bind(sort_order)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("update extra: {e}")))?;
    Ok(())
}

#[server(
    name = CreateExtra,
    prefix = "/api",
    endpoint = "create_extra"
)]
pub async fn create_extra(label: String, price_cents: i64) -> Result<String, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    if label.trim().is_empty() {
        return Err(ServerFnError::new("Label darf nicht leer sein."));
    }
    if price_cents < 0 {
        return Err(ServerFnError::new("Preis darf nicht negativ sein."));
    }
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Slug from label so the id stays human-readable when David inspects the
    // DB. Collisions are vanishingly rare (he picks unique extra names);
    // fall back to a uuid suffix on conflict.
    let base = slugify(label.trim());
    let id = format!("ex-{base}");
    let next_sort: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(sort_order), 0) + 10 FROM pizza_extras")
            .fetch_one(&db)
            .await
            .unwrap_or(1000);

    let res = sqlx::query(
        "INSERT INTO pizza_extras (id, label, price_cents, sort_order)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&id)
    .bind(label.trim())
    .bind(price_cents)
    .bind(next_sort)
    .execute(&db)
    .await;

    match res {
        Ok(_) => Ok(id),
        Err(_) => {
            // Conflict on slug — disambiguate.
            let id2 = format!("{id}-{}", uuid::Uuid::new_v4().simple());
            sqlx::query(
                "INSERT INTO pizza_extras (id, label, price_cents, sort_order)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(&id2)
            .bind(label.trim())
            .bind(price_cents)
            .bind(next_sort)
            .execute(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("insert extra: {e}")))?;
            Ok(id2)
        }
    }
}

#[cfg(feature = "ssr")]
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.extend(c.to_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("extra");
    }
    out
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

#[component]
pub fn ExtrasAdminPage() -> impl IntoView {
    let updater = ServerAction::<UpdateExtra>::new();
    let creator = ServerAction::<CreateExtra>::new();
    let extras = Resource::new(
        move || (updater.version().get(), creator.version().get()),
        |_| async move { list_extras_admin().await },
    );

    view! {
        <AdminShell>
            <section class="extras-admin">
                <header class="admin-bar">
                    <h1>"Pizza-Extras"</h1>
                </header>

                <p class="hint">
                    "Diese Liste erscheint im Online-Shop unter jeder Pizza/Calzone als Auswahl. "
                    "Nicht aktive Extras werden dem Kunden nicht angezeigt."
                </p>

                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || extras.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(rows) => view! { <ExtrasTable rows updater/> }.into_any(),
                    })}
                </Suspense>

                <h2>"Neuen Extra hinzufügen"</h2>
                <ActionForm action=creator attr:class="extras-create">
                    <label>
                        <span>"Bezeichnung"</span>
                        <input type="text" name="label" required placeholder="z.B. Rucola"/>
                    </label>
                    <label>
                        <span>"Preis (Cent)"</span>
                        <input type="number" name="price_cents" min="0" value="100" required/>
                    </label>
                    <button type="submit" class="btn primary">"Hinzufügen"</button>
                </ActionForm>
                {move || creator.value().get().and_then(|res| match res {
                    Err(e) => Some(view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any()),
                    Ok(_) => None,
                })}
            </section>
        </AdminShell>
    }
}

#[component]
fn ExtrasTable(rows: Vec<ExtraAdminRow>, updater: ServerAction<UpdateExtra>) -> impl IntoView {
    if rows.is_empty() {
        return view! { <p class="empty">"Noch keine Extras."</p> }.into_any();
    }
    view! {
        <table class="extras-table">
            <thead>
                <tr>
                    <th>"Bezeichnung"</th>
                    <th>"Preis"</th>
                    <th>"Aktiv"</th>
                    <th>"Sortierung"</th>
                    <th></th>
                </tr>
            </thead>
            <tbody>
                {rows.into_iter().map(|r| view! {
                    <ExtraRow r updater/>
                }).collect_view()}
            </tbody>
        </table>
    }
    .into_any()
}

#[component]
fn ExtraRow(r: ExtraAdminRow, updater: ServerAction<UpdateExtra>) -> impl IntoView {
    let id = r.id.clone();
    let label = RwSignal::new(r.label.clone());
    let price = RwSignal::new(r.price_cents.to_string());
    let avail = RwSignal::new(r.is_available);
    let sort = RwSignal::new(r.sort_order.to_string());

    let id_for_save = id.clone();
    let on_save = move |_| {
        let p = price.get().parse::<i64>().unwrap_or(0);
        let s = sort.get().parse::<i64>().unwrap_or(0);
        updater.dispatch(UpdateExtra {
            id: id_for_save.clone(),
            label: label.get(),
            price_cents: p,
            is_available: avail.get(),
            sort_order: s,
        });
    };

    let display_price = Memo::new(move |_| {
        price
            .get()
            .parse::<i64>()
            .map(format_eur)
            .unwrap_or_default()
    });

    view! {
        <tr>
            <td>
                <input type="text" class="cell-input"
                    prop:value=move || label.get()
                    on:input=move |ev| label.set(event_target_value(&ev))/>
            </td>
            <td>
                <input type="number" class="cell-input narrow" min="0"
                    prop:value=move || price.get()
                    on:input=move |ev| price.set(event_target_value(&ev))/>
                <span class="muted">{move || display_price.get()}</span>
            </td>
            <td>
                <input type="checkbox"
                    prop:checked=move || avail.get()
                    on:change=move |ev| avail.set(event_target_checked(&ev))/>
            </td>
            <td>
                <input type="number" class="cell-input narrow"
                    prop:value=move || sort.get()
                    on:input=move |ev| sort.set(event_target_value(&ev))/>
            </td>
            <td>
                <button class="btn ghost small" on:click=on_save>"Speichern"</button>
            </td>
        </tr>
    }
}
