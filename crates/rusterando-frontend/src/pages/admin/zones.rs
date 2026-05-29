//! /admin/zones — CRUD for delivery zones.
//!
//! `delivery_zones` is the table that drives the geocoder fee + ETA +
//! min-order classification at checkout. Names like "Mitte" or
//! "Vorort 1" are admin-visible labels — they go into the receipt and
//! the checkout summary, so they must be editable without a code
//! change.
//!
//! Schema reminder: (id, postcode UNIQUE, name, fee_cents, min_order_cents,
//! eta_minutes, is_active).

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneRow {
    pub id: String,
    pub postcode: String,
    pub name: String,
    pub fee_cents: i64,
    pub min_order_cents: i64,
    pub eta_minutes: i64,
    pub is_active: bool,
    /// Optional admin-written marketing copy for the /lieferservice/<slug>
    /// SEO landing page. Empty → the page renders a generated fallback.
    #[serde(default)]
    pub seo_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateZoneForm {
    pub postcode: String,
    pub name: String,
    pub fee_cents: i64,
    pub min_order_cents: i64,
    pub eta_minutes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateZoneForm {
    pub id: String,
    pub postcode: String,
    pub name: String,
    pub fee_cents: i64,
    pub min_order_cents: i64,
    pub eta_minutes: i64,
    pub is_active: bool,
    #[serde(default)]
    pub seo_text: String,
}

// ---------------------------------------------------------------------------
// Server fns
// ---------------------------------------------------------------------------

#[server(
    name = ListAdminZones,
    prefix = "/api",
    endpoint = "list_admin_zones"
)]
pub async fn list_admin_zones() -> Result<Vec<ZoneRow>, ServerFnError> {
    use sqlx::{Row, SqlitePool};

    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows = sqlx::query(
        "SELECT id, postcode, COALESCE(name, '') AS name,
                fee_cents, min_order_cents, eta_minutes, is_active,
                COALESCE(seo_text, '') AS seo_text
         FROM delivery_zones
         ORDER BY is_active DESC, postcode ASC",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load zones: {e}")))?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(ZoneRow {
            id: r.get("id"),
            postcode: r.get("postcode"),
            name: r.get("name"),
            fee_cents: r.get("fee_cents"),
            min_order_cents: r.get("min_order_cents"),
            eta_minutes: r.get("eta_minutes"),
            is_active: r.get::<i64, _>("is_active") != 0,
            seo_text: r.get("seo_text"),
        });
    }
    Ok(out)
}

#[server(
    name = CreateZone,
    prefix = "/api",
    endpoint = "create_zone"
)]
pub async fn create_zone(form: CreateZoneForm) -> Result<String, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let postcode = form.postcode.trim().to_string();
    let name = form.name.trim().to_string();
    if postcode.is_empty() {
        return Err(ServerFnError::new("PLZ darf nicht leer sein"));
    }
    if name.is_empty() {
        return Err(ServerFnError::new("Name darf nicht leer sein"));
    }
    if form.eta_minutes < 0 {
        return Err(ServerFnError::new("ETA muss >= 0 sein"));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let id = uuid::Uuid::new_v4().to_string();
    let res = sqlx::query(
        "INSERT INTO delivery_zones
           (id, postcode, name, fee_cents, min_order_cents, eta_minutes, is_active)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
    )
    .bind(&id)
    .bind(&postcode)
    .bind(&name)
    .bind(form.fee_cents.max(0))
    .bind(form.min_order_cents.max(0))
    .bind(form.eta_minutes.max(0))
    .execute(&db)
    .await;

    match res {
        Ok(_) => Ok(id),
        Err(sqlx::Error::Database(e)) if e.message().contains("UNIQUE") => Err(ServerFnError::new(
            format!("PLZ '{postcode}' existiert bereits."),
        )),
        Err(e) => Err(ServerFnError::new(format!("INSERT zone: {e}"))),
    }
}

#[server(
    name = UpdateZone,
    prefix = "/api",
    endpoint = "update_zone"
)]
pub async fn update_zone(form: UpdateZoneForm) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let postcode = form.postcode.trim().to_string();
    let name = form.name.trim().to_string();
    if postcode.is_empty() {
        return Err(ServerFnError::new("PLZ darf nicht leer sein"));
    }
    if name.is_empty() {
        return Err(ServerFnError::new("Name darf nicht leer sein"));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let seo_text = form.seo_text.trim();
    let seo_opt: Option<&str> = (!seo_text.is_empty()).then_some(seo_text);
    let res = sqlx::query(
        "UPDATE delivery_zones
           SET postcode = ?2, name = ?3,
               fee_cents = ?4, min_order_cents = ?5,
               eta_minutes = ?6, is_active = ?7, seo_text = ?8
         WHERE id = ?1",
    )
    .bind(&form.id)
    .bind(&postcode)
    .bind(&name)
    .bind(form.fee_cents.max(0))
    .bind(form.min_order_cents.max(0))
    .bind(form.eta_minutes.max(0))
    .bind(if form.is_active { 1 } else { 0 })
    .bind(seo_opt)
    .execute(&db)
    .await;

    match res {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(e)) if e.message().contains("UNIQUE") => Err(ServerFnError::new(
            format!("PLZ '{postcode}' wird bereits genutzt."),
        )),
        Err(e) => Err(ServerFnError::new(format!("UPDATE zone: {e}"))),
    }
}

#[server(
    name = DeleteZone,
    prefix = "/api",
    endpoint = "delete_zone"
)]
pub async fn delete_zone(id: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Reject delete if any address or order still references the zone —
    // deactivate-instead is the safer path. Customer recall + history
    // would otherwise lose their zone snapshot.
    let used_addr: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM customer_addresses WHERE delivery_zone_id = ?1")
            .bind(&id)
            .fetch_one(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("address ref count: {e}")))?;
    if used_addr.0 > 0 {
        return Err(ServerFnError::new(
            "Zone wird von gespeicherten Adressen referenziert — bitte deaktivieren statt löschen.",
        ));
    }

    sqlx::query("DELETE FROM delivery_zones WHERE id = ?1")
        .bind(&id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("delete zone: {e}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

#[component]
pub fn ZonesAdminPage() -> impl IntoView {
    let creator = ServerAction::<CreateZone>::new();
    let updater = ServerAction::<UpdateZone>::new();
    let deleter = ServerAction::<DeleteZone>::new();

    let list = Resource::new(
        move || {
            (
                creator.version().get(),
                updater.version().get(),
                deleter.version().get(),
            )
        },
        |_| async move { list_admin_zones().await },
    );

    view! {
        <AdminShell>
            <section class="admin-zones">
                <header class="admin-bar">
                    <h1>"Liefergebiete"</h1>
                </header>
                <p class="hint">
                    "Name + PLZ + Liefergebühr + Mindestbestellwert + ETA. "
                    "Der Geocoder klassifiziert eingegebene Adressen anhand der PLZ; "
                    "der Name erscheint auf dem Beleg und im Checkout."
                </p>

                <CreateZoneCard creator/>

                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || list.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(rows) => view! { <ZoneTable rows updater deleter/> }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn CreateZoneCard(creator: ServerAction<CreateZone>) -> impl IntoView {
    let postcode = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let fee_eur = RwSignal::new("0".to_string());
    let min_eur = RwSignal::new("0".to_string());
    let eta = RwSignal::new("30".to_string());

    let parse_eur = |s: String| -> i64 {
        let trimmed = s.trim().replace(',', ".");
        trimmed
            .parse::<f64>()
            .map(|f| (f * 100.0).round() as i64)
            .unwrap_or(0)
    };
    let parse_int = |s: String| s.trim().parse::<i64>().unwrap_or(0);

    let on_submit = move |_| {
        creator.dispatch(CreateZone {
            form: CreateZoneForm {
                postcode: postcode.get(),
                name: name.get(),
                fee_cents: parse_eur(fee_eur.get()),
                min_order_cents: parse_eur(min_eur.get()),
                eta_minutes: parse_int(eta.get()),
            },
        });
        postcode.set(String::new());
        name.set(String::new());
    };

    view! {
        <details class="card create-zone" open=false>
            <summary><strong>"Neues Liefergebiet anlegen"</strong></summary>
            <div class="grid">
                <label>
                    <span>"PLZ"</span>
                    <input type="text" placeholder="12345"
                        prop:value=move || postcode.get()
                        on:input=move |ev| postcode.set(event_target_value(&ev))/>
                </label>
                <label>
                    <span>"Name"</span>
                    <input type="text" placeholder="z.B. Mitte / Vorort 1"
                        prop:value=move || name.get()
                        on:input=move |ev| name.set(event_target_value(&ev))/>
                </label>
                <label>
                    <span>"Liefergebühr in €"</span>
                    <input type="text" inputmode="decimal" placeholder="2.50"
                        prop:value=move || fee_eur.get()
                        on:input=move |ev| fee_eur.set(event_target_value(&ev))/>
                </label>
                <label>
                    <span>"Mindestbestellwert in €"</span>
                    <input type="text" inputmode="decimal" placeholder="15"
                        prop:value=move || min_eur.get()
                        on:input=move |ev| min_eur.set(event_target_value(&ev))/>
                </label>
                <label>
                    <span>"ETA in Minuten"</span>
                    <input type="number" min="0" placeholder="30"
                        prop:value=move || eta.get()
                        on:input=move |ev| eta.set(event_target_value(&ev))/>
                </label>
            </div>
            <div class="actions">
                <button class="btn primary" on:click=on_submit>"Anlegen"</button>
                {move || creator.value().get().map(|res| match res {
                    Ok(_) => view! { <span class="ok">"✓ Gespeichert"</span> }.into_any(),
                    Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                })}
            </div>
        </details>
    }
}

#[component]
fn ZoneTable(
    rows: Vec<ZoneRow>,
    updater: ServerAction<UpdateZone>,
    deleter: ServerAction<DeleteZone>,
) -> impl IntoView {
    if rows.is_empty() {
        return view! { <p class="empty">"Keine Liefergebiete angelegt."</p> }.into_any();
    }
    view! {
        <table class="admin-zones-table">
            <thead>
                <tr>
                    <th>"PLZ"</th>
                    <th>"Name"</th>
                    <th class="num">"Gebühr"</th>
                    <th class="num">"Mindest"</th>
                    <th class="num">"ETA"</th>
                    <th>"Aktiv"</th>
                    <th></th>
                </tr>
            </thead>
            <tbody>
                {rows.into_iter().map(|r| view! { <ZoneRowView r updater deleter/> }).collect_view()}
            </tbody>
        </table>
    }
    .into_any()
}

#[component]
fn ZoneRowView(
    r: ZoneRow,
    updater: ServerAction<UpdateZone>,
    deleter: ServerAction<DeleteZone>,
) -> impl IntoView {
    let postcode = RwSignal::new(r.postcode.clone());
    let name = RwSignal::new(r.name.clone());
    let fee_eur = RwSignal::new(format!("{:.2}", r.fee_cents as f64 / 100.0));
    let min_eur = RwSignal::new(format!("{:.2}", r.min_order_cents as f64 / 100.0));
    let eta = RwSignal::new(r.eta_minutes.to_string());
    let active = RwSignal::new(r.is_active);
    let seo_text = RwSignal::new(r.seo_text.clone());

    let id_save = r.id.clone();
    let id_delete = r.id.clone();

    let parse_eur = |s: String| -> i64 {
        let trimmed = s.trim().replace(',', ".");
        trimmed
            .parse::<f64>()
            .map(|f| (f * 100.0).round() as i64)
            .unwrap_or(0)
    };
    let parse_int = |s: String| s.trim().parse::<i64>().unwrap_or(0);

    let on_save = move |_| {
        updater.dispatch(UpdateZone {
            form: UpdateZoneForm {
                id: id_save.clone(),
                postcode: postcode.get(),
                name: name.get(),
                fee_cents: parse_eur(fee_eur.get()),
                min_order_cents: parse_eur(min_eur.get()),
                eta_minutes: parse_int(eta.get()),
                is_active: active.get(),
                seo_text: seo_text.get(),
            },
        });
    };
    let on_delete = move |_| {
        deleter.dispatch(DeleteZone {
            id: id_delete.clone(),
        });
    };

    let row_class = if r.is_active { "active" } else { "inactive" };
    view! {
        <tr class=row_class>
            <td>
                <input type="text" class="cell-input"
                    prop:value=move || postcode.get()
                    on:input=move |ev| postcode.set(event_target_value(&ev))/>
            </td>
            <td>
                <input type="text" class="cell-input"
                    prop:value=move || name.get()
                    on:input=move |ev| name.set(event_target_value(&ev))/>
            </td>
            <td class="num">
                <input type="text" class="cell-input small" inputmode="decimal"
                    prop:value=move || fee_eur.get()
                    on:input=move |ev| fee_eur.set(event_target_value(&ev))/>
                <p class="muted">{format_eur(r.fee_cents)}</p>
            </td>
            <td class="num">
                <input type="text" class="cell-input small" inputmode="decimal"
                    prop:value=move || min_eur.get()
                    on:input=move |ev| min_eur.set(event_target_value(&ev))/>
                <p class="muted">{format_eur(r.min_order_cents)}</p>
            </td>
            <td class="num">
                <input type="number" class="cell-input small" min="0"
                    prop:value=move || eta.get()
                    on:input=move |ev| eta.set(event_target_value(&ev))/>
            </td>
            <td>
                <input type="checkbox"
                    prop:checked=move || active.get()
                    on:change=move |ev| active.set(event_target_checked(&ev))/>
            </td>
            <td class="actions">
                <button class="btn small primary" on:click=on_save>"Speichern"</button>
                <button class="btn small danger" on:click=on_delete>"Löschen"</button>
                <details class="zone-seo">
                    <summary>"SEO-Text (Lieferservice-Seite)"</summary>
                    <textarea rows="3" placeholder="Optionaler Werbetext für /lieferservice/… — leer = automatischer Text."
                        prop:value=move || seo_text.get()
                        on:input=move |ev| seo_text.set(event_target_value(&ev))></textarea>
                </details>
            </td>
        </tr>
    }
}
