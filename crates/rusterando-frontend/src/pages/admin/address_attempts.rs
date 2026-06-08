//! /admin/address-attempts — forensic log of rejected delivery addresses.
//!
//! Every time `validate_address` returns `ok=false` (no Nominatim hit, HTTP
//! error, out-of-radius, wrong-city, no-zone), a row lands in the
//! `address_attempts` table. This page is the read-only window into that
//! table — what David opens after a customer complains "ich konnte nicht
//! bestellen". It shows the customer's typed inputs, what Nominatim said,
//! and the precise reason the classifier rejected.
//!
//! No action buttons in v1: this is pure visibility. Once we have real
//! production data we'll learn which override shape is worth adding (city
//! alias from this UI? per-address manual override? zone re-tag?).

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;

#[cfg_attr(feature = "ssr", derive(sqlx::FromRow))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AddressAttemptRow {
    pub id: String,
    pub attempted_at: String,
    pub input_street: String,
    pub input_house_number: String,
    pub input_postcode: String,
    pub input_city: String,
    pub resolved_lat: Option<f64>,
    pub resolved_lon: Option<f64>,
    pub resolved_city: Option<String>,
    pub resolved_suburb: Option<String>,
    pub distance_km: Option<f64>,
    pub rejection_reason: String,
    pub rejection_detail: Option<String>,
    pub raw_nominatim_json: Option<String>,
    pub customer_phone: Option<String>,
    pub customer_email: Option<String>,
}

#[server(
    name = ListAddressAttempts,
    prefix = "/api",
    endpoint = "list_address_attempts"
)]
pub async fn list_address_attempts() -> Result<Vec<AddressAttemptRow>, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows: Vec<AddressAttemptRow> = sqlx::query_as(
        "SELECT id, attempted_at,
                input_street, input_house_number, input_postcode, input_city,
                resolved_lat, resolved_lon, resolved_city, resolved_suburb,
                distance_km, rejection_reason, rejection_detail,
                raw_nominatim_json, customer_phone, customer_email
         FROM address_attempts
         ORDER BY attempted_at DESC
         LIMIT 500",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("list address_attempts: {e}")))?;
    Ok(rows)
}

fn reason_label(slug: &str) -> &'static str {
    match slug {
        "no_results" => "Adresse nicht gefunden",
        "http_error" => "Geocoder-Fehler",
        "out_of_radius" => "Zu weit weg",
        "wrong_city" => "Falsche Stadt",
        "no_zone" => "Keine Zone",
        _ => "Unbekannt",
    }
}

fn reason_chip_class(slug: &str) -> &'static str {
    match slug {
        "no_results" | "http_error" => "chip-warn",
        "out_of_radius" => "chip-info",
        "wrong_city" | "no_zone" => "chip-err",
        _ => "chip-info",
    }
}

#[component]
pub fn AddressAttemptsPage() -> impl IntoView {
    let data = Resource::new(|| (), |_| async move { list_address_attempts().await });
    let expanded: RwSignal<Option<String>> = RwSignal::new(None);

    view! {
        <AdminShell>
            <section class="admin-address-attempts">
                <header class="admin-bar">
                    <h1>"Abgelehnte Adressen"</h1>
                </header>
                <p class="hint">
                    "Jede Adresse, die nicht in ein Liefergebiet eingeordnet werden konnte, \
                     landet hier — mit dem Grund und der Antwort des Geocoders. So lässt sich \
                     prüfen, warum ein:e Kund:in nicht bestellen konnte. Einträge älter als \
                     60 Tage werden automatisch gelöscht."
                </p>
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || data.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(rows) if rows.is_empty() => view! {
                            <p class="empty">"Noch keine abgelehnten Adressen."</p>
                        }.into_any(),
                        Ok(rows) => view! { <AttemptsTable rows expanded/> }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn AttemptsTable(
    rows: Vec<AddressAttemptRow>,
    expanded: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <table class="admin-table">
            <thead>
                <tr>
                    <th>"Zeit"</th>
                    <th>"Eingabe"</th>
                    <th>"Grund"</th>
                    <th>"Auflösung"</th>
                </tr>
            </thead>
            <tbody>
                {rows.into_iter().map(|r| {
                    let id_for_toggle = r.id.clone();
                    let id_for_check = r.id.clone();
                    let is_open = Memo::new(move |_| {
                        expanded.with(|e| e.as_deref() == Some(id_for_check.as_str()))
                    });
                    let row_class = move || if is_open.get() { "expandable open" } else { "expandable" };
                    let chip_cls = format!("chip {}", reason_chip_class(&r.rejection_reason));
                    let resolved_summary = match (&r.resolved_city, &r.resolved_suburb, r.distance_km) {
                        (Some(c), Some(s), Some(km)) => format!("{s} / {c} — {km:.1} km"),
                        (Some(c), Some(s), None) => format!("{s} / {c}"),
                        (Some(c), None, Some(km)) => format!("{c} — {km:.1} km"),
                        (Some(c), None, None) => c.clone(),
                        (None, _, Some(km)) => format!("{km:.1} km"),
                        _ => "—".to_string(),
                    };
                    let r_clone = r.clone();
                    view! {
                        <tr class=row_class on:click=move |_| {
                            expanded.update(|e| {
                                if e.as_deref() == Some(id_for_toggle.as_str()) {
                                    *e = None;
                                } else {
                                    *e = Some(id_for_toggle.clone());
                                }
                            });
                        }>
                            <td>{r.attempted_at.clone()}</td>
                            <td>
                                {format!("{} {}, {} {}",
                                    r.input_street, r.input_house_number,
                                    r.input_postcode, r.input_city)}
                            </td>
                            <td>
                                <span class=chip_cls>{reason_label(&r.rejection_reason)}</span>
                            </td>
                            <td>{resolved_summary}</td>
                        </tr>
                        {move || is_open.get().then(|| view! {
                            <tr class="expansion">
                                <td colspan="4">
                                    <AttemptDetail r=r_clone.clone()/>
                                </td>
                            </tr>
                        })}
                    }
                }).collect_view()}
            </tbody>
        </table>
    }
}

#[component]
fn AttemptDetail(r: AddressAttemptRow) -> impl IntoView {
    let detail = r.rejection_detail.clone().unwrap_or_default();
    let raw = r.raw_nominatim_json.clone();
    let contact = match (&r.customer_phone, &r.customer_email) {
        (Some(p), Some(e)) => format!("Kontakt: {p} / {e}"),
        (Some(p), None) => format!("Kontakt: {p}"),
        (None, Some(e)) => format!("Kontakt: {e}"),
        _ => "Kein bekannter Kontakt".to_string(),
    };
    view! {
        <div class="attempt-detail">
            <p><strong>"Grund-Details: "</strong>{detail}</p>
            <p class="muted">{contact}</p>
            {raw.map(|j| view! {
                <details>
                    <summary>"Nominatim-Antwort"</summary>
                    <pre>{j}</pre>
                </details>
            })}
        </div>
    }
}
