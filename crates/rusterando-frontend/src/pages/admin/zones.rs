//! /admin/zones — single delivery-area admin page.
//!
//! All delivery configuration lives here, stacked top-to-bottom with an
//! anchor-link jump bar at the top:
//!
//!   1. Zonen                — delivery_zones CRUD (fee / min order / ETA).
//!   2. Belieferte Orte      — served_municipalities (city → suburb routing).
//!   3. Liefer-Inseln        — extra_circles (lat/lon/radius/zone outliers).
//!   4. Bezahlte Bestellungen außerhalb — paid-bypass threshold + zone.
//!   5. PLZ ↔ Ort Vorschläge — cosmetic cart-drawer autofill hints.
//!
//! Previously sections 2-5 lived on a separate /admin/localities page;
//! merged here because they're all "how delivery works" and splitting
//! into two URLs was admin sprawl. The forensic /admin/address-attempts
//! log stays a separate page — different mental mode (debugging, not
//! configuration).
//!
//! delivery_zones schema reminder: (id, postcode UNIQUE, name,
//! fee_cents, min_order_cents, eta_minutes, is_active).

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;
#[cfg(feature = "ssr")]
use crate::pages::locality::{get_delivery_areas, get_postcode_hints};
use crate::pages::locality::{
    DeliveryAreas, ExtraCircle, PostcodeHint, ServedMunicipality, SetDeliveryAreas,
    SetPostcodeHints, SuburbRoute, SUBURB_WILDCARD,
};

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
// Locality / delivery-area sections — loaded together with the zone list
// in a single Resource so the page does one round-trip.
// ---------------------------------------------------------------------------

/// Snapshot consumed by the lower (locality) sections. Bundled with the
/// zones data so the page makes one Resource fetch.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LocalitiesAdmin {
    pub areas: DeliveryAreas,
    pub hints: Vec<PostcodeHint>,
    pub bypass_min_cents: i64,
    pub bypass_zone_id: String,
    /// Available zones for the <select> in each routing row. (id, name).
    pub zones: Vec<(String, String)>,
}

#[server(
    name = LoadLocalitiesAdmin,
    prefix = "/api",
    endpoint = "load_localities_admin"
)]
pub async fn load_localities_admin() -> Result<LocalitiesAdmin, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let areas = get_delivery_areas().await?;
    let hints = get_postcode_hints().await?;

    let branding = use_context::<crate::branding::BrandingHandle>().map(|h| h.get());
    let bypass_min_cents = branding
        .as_ref()
        .map(|b| b.bypass_paid_min_cents)
        .unwrap_or(0);
    let bypass_zone_id = branding
        .as_ref()
        .map(|b| b.bypass_zone_id.clone())
        .unwrap_or_default();

    let zones: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT id, name FROM delivery_zones WHERE is_active = 1 ORDER BY name")
            .fetch_all(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("load zones: {e}")))?;
    let zones: Vec<(String, String)> = zones
        .into_iter()
        .map(|(id, name)| (id.clone(), name.unwrap_or(id)))
        .collect();

    Ok(LocalitiesAdmin {
        areas,
        hints,
        bypass_min_cents,
        bypass_zone_id,
        zones,
    })
}

#[server(
    name = SaveBypassSettings,
    prefix = "/api",
    endpoint = "save_bypass_settings"
)]
pub async fn save_bypass_settings(min_cents: i64, zone_id: String) -> Result<(), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    if min_cents < 0 {
        return Err(ServerFnError::new("Schwelle darf nicht negativ sein."));
    }
    crate::pages::settings::update_setting(
        "shop_bypass_paid_min_cents".to_string(),
        min_cents.to_string(),
    )
    .await?;
    crate::pages::settings::update_setting(
        "shop_bypass_zone_id".to_string(),
        zone_id.trim().to_string(),
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

#[component]
pub fn ZonesAdminPage() -> impl IntoView {
    // Zone CRUD server actions.
    let creator = ServerAction::<CreateZone>::new();
    let updater = ServerAction::<UpdateZone>::new();
    let deleter = ServerAction::<DeleteZone>::new();

    // Locality/area server actions — share the same Resource so saving
    // anywhere on this page refreshes the whole view.
    let areas_saver = ServerAction::<SetDeliveryAreas>::new();
    let hints_saver = ServerAction::<SetPostcodeHints>::new();
    let bypass_saver = ServerAction::<SaveBypassSettings>::new();

    let zones_list = Resource::new(
        move || {
            (
                creator.version().get(),
                updater.version().get(),
                deleter.version().get(),
            )
        },
        |_| async move { list_admin_zones().await },
    );

    let locality_data = Resource::new(
        move || {
            (
                areas_saver.version().get(),
                hints_saver.version().get(),
                bypass_saver.version().get(),
                // Also refresh when zones change (the <select> options
                // come from delivery_zones).
                creator.version().get(),
                updater.version().get(),
                deleter.version().get(),
            )
        },
        |_| async move { load_localities_admin().await },
    );

    view! {
        <AdminShell>
            <section class="admin-zones">
                <header class="admin-bar">
                    <h1>"Liefergebiete"</h1>
                </header>
                // Anchor jump bar — quick navigation between the five
                // stacked sections without having to scroll.
                <nav class="admin-anchors">
                    <a href="#zonen">"Zonen"</a>
                    " · "
                    <a href="#belieferte-orte">"Belieferte Orte"</a>
                    " · "
                    <a href="#liefer-inseln">"Liefer-Inseln"</a>
                    " · "
                    <a href="#bypass">"Bezahlte Bestellungen außerhalb"</a>
                    " · "
                    <a href="#plz-hints">"PLZ ↔ Ort Vorschläge"</a>
                </nav>

                // ----- Section 1: Zonen (delivery_zones CRUD) -----
                <h2 id="zonen">"Zonen"</h2>
                <p class="hint">
                    "Name + PLZ + Liefergebühr + Mindestbestellwert + ETA. "
                    "Die Zonen werden in den Abschnitten unten referenziert "
                    "(Stadtteil-Routen, Liefer-Inseln, Bypass)."
                </p>
                <CreateZoneCard creator/>
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || zones_list.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(rows) => view! { <ZoneTable rows updater deleter/> }.into_any(),
                    })}
                </Suspense>

                // ----- Sections 2-5: locality / circle / bypass / hints -----
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || locality_data.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(d) => view! {
                            <LocalitySections d areas_saver hints_saver bypass_saver/>
                        }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

// ---------------------------------------------------------------------------
// Locality sections — Belieferte Orte, Liefer-Inseln, Bypass, PLZ-Hints.
// Each section has its own save button + its own server action; saving in
// one section doesn't blow away unsaved edits in another.
// ---------------------------------------------------------------------------

#[component]
fn LocalitySections(
    d: LocalitiesAdmin,
    areas_saver: ServerAction<SetDeliveryAreas>,
    hints_saver: ServerAction<SetPostcodeHints>,
    bypass_saver: ServerAction<SaveBypassSettings>,
) -> impl IntoView {
    let zones = StoredValue::new(d.zones.clone());
    let areas = RwSignal::new(d.areas.clone());
    let hints = RwSignal::new(d.hints.clone());
    let bypass_min = RwSignal::new(d.bypass_min_cents);
    let bypass_zone = RwSignal::new(d.bypass_zone_id.clone());

    view! {
        <ServedMunicipalitiesSection areas zones areas_saver/>
        <ExtraCirclesSection areas zones areas_saver/>
        <BypassSection bypass_min bypass_zone zones bypass_saver/>
        <PostcodeHintsSection hints hints_saver/>
    }
}

#[component]
fn ServedMunicipalitiesSection(
    areas: RwSignal<DeliveryAreas>,
    zones: StoredValue<Vec<(String, String)>>,
    areas_saver: ServerAction<SetDeliveryAreas>,
) -> impl IntoView {
    let add_municipality = move |_| {
        areas.update(|a| {
            a.served_municipalities.push(ServedMunicipality::default());
        });
    };
    let save_all = move |_| {
        areas_saver.dispatch(SetDeliveryAreas { areas: areas.get() });
    };

    view! {
        <h2 id="belieferte-orte">"Belieferte Orte"</h2>
        <p class="hint">
            "Name(n), die Nominatim für den Ort zurückgibt. Für jede:n bekannte:n \
             Stadtteil eine Zone festlegen. Der Eintrag mit \"*\" greift für \
             alle anderen Stadtteile (z. B. Bauerschaften)."
        </p>
        {move || areas.get().served_municipalities.iter().enumerate().map(|(idx, _m)| {
            view! { <MunicipalityRow idx areas zones=zones/> }
        }).collect_view()}
        <div class="admin-actions">
            <button type="button" class="btn ghost" on:click=add_municipality>"+ Ort hinzufügen"</button>
            <button type="button" class="btn primary" on:click=save_all>"Speichern"</button>
        </div>
        {move || match areas_saver.value().get() {
            Some(Ok(())) => Some(view! { <p class="ok">"✓ Gespeichert."</p> }.into_any()),
            Some(Err(e)) => Some(view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any()),
            None => None,
        }}
    }
}

#[component]
fn MunicipalityRow(
    idx: usize,
    areas: RwSignal<DeliveryAreas>,
    zones: StoredValue<Vec<(String, String)>>,
) -> impl IntoView {
    let names = Memo::new(move |_| {
        areas
            .with(|a| {
                a.served_municipalities
                    .get(idx)
                    .map(|m| m.match_names.join(", "))
            })
            .unwrap_or_default()
    });
    let routes = Memo::new(move |_| {
        areas
            .with(|a| {
                a.served_municipalities
                    .get(idx)
                    .map(|m| m.zone_routing.clone())
            })
            .unwrap_or_default()
    });

    let remove = move |_| {
        areas.update(|a| {
            if idx < a.served_municipalities.len() {
                a.served_municipalities.remove(idx);
            }
        });
    };
    let add_route = move |_| {
        areas.update(|a| {
            if let Some(m) = a.served_municipalities.get_mut(idx) {
                m.zone_routing.push(SuburbRoute::default());
            }
        });
    };

    view! {
        <fieldset class="municipality">
            <legend>{move || {
                let n = names.get();
                if n.is_empty() { "Neuer Ort".to_string() } else { n }
            }}</legend>
            <label>
                <span>"Match-Namen (Komma-getrennt)"</span>
                <input type="text" prop:value=names placeholder="Ort, alternative Schreibweise"
                    on:input=move |ev| {
                        let v = event_target_value(&ev);
                        let parsed: Vec<String> = v
                            .split(',').map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty()).collect();
                        areas.update(|a| {
                            if let Some(m) = a.served_municipalities.get_mut(idx) {
                                m.match_names = parsed;
                            }
                        });
                    }/>
            </label>
            <table class="admin-table">
                <thead><tr>
                    <th>"Stadtteil (oder \"*\")"</th>
                    <th>"Zone"</th>
                    <th></th>
                </tr></thead>
                <tbody>
                    {move || routes.get().into_iter().enumerate().map(|(r_idx, route)| {
                        let suburb_v = route.match_suburb.clone();
                        let zone_v = route.zone_id.clone();
                        view! {
                            <tr>
                                <td>
                                    // prop:value (not value=) so the input
                                    // hydrates with the existing value
                                    // instead of starting empty on the
                                    // client side. Same fix repeated below
                                    // in ExtraCircles / PostcodeHints.
                                    <input type="text" prop:value=suburb_v
                                        placeholder=SUBURB_WILDCARD
                                        on:input=move |ev| {
                                            let v = event_target_value(&ev);
                                            areas.update(|a| {
                                                if let Some(m) = a.served_municipalities.get_mut(idx) {
                                                    if let Some(r) = m.zone_routing.get_mut(r_idx) {
                                                        r.match_suburb = v;
                                                    }
                                                }
                                            });
                                        }/>
                                </td>
                                <td>
                                    <ZoneSelect
                                        selected=zone_v
                                        zones=zones
                                        on_change=Box::new(move |new_zid: String| {
                                            areas.update(|a| {
                                                if let Some(m) = a.served_municipalities.get_mut(idx) {
                                                    if let Some(r) = m.zone_routing.get_mut(r_idx) {
                                                        r.zone_id = new_zid;
                                                    }
                                                }
                                            });
                                        })/>
                                </td>
                                <td>
                                    <button type="button" class="btn ghost"
                                        on:click=move |_| {
                                            areas.update(|a| {
                                                if let Some(m) = a.served_municipalities.get_mut(idx) {
                                                    if r_idx < m.zone_routing.len() {
                                                        m.zone_routing.remove(r_idx);
                                                    }
                                                }
                                            });
                                        }>"Entfernen"</button>
                                </td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
            <div class="admin-actions">
                <button type="button" class="btn ghost" on:click=add_route>"+ Stadtteil-Route"</button>
                <button type="button" class="btn ghost danger" on:click=remove>"Ort entfernen"</button>
            </div>
        </fieldset>
    }
}

#[component]
fn ZoneSelect(
    selected: String,
    zones: StoredValue<Vec<(String, String)>>,
    on_change: Box<dyn Fn(String) + 'static>,
) -> impl IntoView {
    // `prop:value` on the <select> ensures the rendered DOM picks the
    // right option after hydrate (the `selected` attribute on <option>
    // is documentation only; browsers prefer the live property).
    let selected_for_prop = selected.clone();
    view! {
        <select prop:value=selected_for_prop
                on:change=move |ev| on_change(event_target_value(&ev))>
            <option value="" selected=selected.is_empty()>"— wählen —"</option>
            {zones.get_value().into_iter().map(|(zid, zname)| {
                let is_sel = zid == selected;
                view! { <option value=zid.clone() selected=is_sel>{format!("{zname} ({zid})")}</option> }
            }).collect_view()}
        </select>
    }
}

#[component]
fn ExtraCirclesSection(
    areas: RwSignal<DeliveryAreas>,
    zones: StoredValue<Vec<(String, String)>>,
    areas_saver: ServerAction<SetDeliveryAreas>,
) -> impl IntoView {
    let add_circle = move |_| {
        areas.update(|a| {
            a.extra_circles.push(ExtraCircle {
                radius_km: 1.0,
                ..Default::default()
            });
        });
    };
    let save_all = move |_| {
        areas_saver.dispatch(SetDeliveryAreas { areas: areas.get() });
    };

    view! {
        <h2 id="liefer-inseln">"Zusätzliche Liefer-Inseln (Umkreis)"</h2>
        <p class="hint">
            "Einzelne Punkte außerhalb der belieferten Orte, die trotzdem \
             beliefert werden sollen (z. B. einzelne Bauerschaften, die \
             organisatorisch nicht zum belieferten Ort gehören, aber von \
             Ihnen versorgt werden). Jeder Kreis: Mittelpunkt + Radius + Zone."
        </p>
        <table class="admin-table">
            <thead><tr>
                <th>"Label"</th>
                <th>"Lat"</th>
                <th>"Lon"</th>
                <th>"Radius (km)"</th>
                <th>"Zone"</th>
                <th></th>
            </tr></thead>
            <tbody>
                {move || areas.get().extra_circles.into_iter().enumerate().map(|(idx, c)| {
                    let label_v = c.label.clone();
                    let lat_v = c.center_lat;
                    let lon_v = c.center_lon;
                    let rad_v = c.radius_km;
                    let zone_v = c.zone_id.clone();
                    view! {
                        <tr>
                            <td>
                                <input type="text" prop:value=label_v
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        areas.update(|a| {
                                            if let Some(r) = a.extra_circles.get_mut(idx) {
                                                r.label = v;
                                            }
                                        });
                                    }/>
                            </td>
                            <td>
                                <input type="number" step="0.000001"
                                    prop:value=lat_v.to_string()
                                    on:input=move |ev| {
                                        let v: f64 = event_target_value(&ev).parse().unwrap_or(0.0);
                                        areas.update(|a| {
                                            if let Some(r) = a.extra_circles.get_mut(idx) {
                                                r.center_lat = v;
                                            }
                                        });
                                    }/>
                            </td>
                            <td>
                                <input type="number" step="0.000001"
                                    prop:value=lon_v.to_string()
                                    on:input=move |ev| {
                                        let v: f64 = event_target_value(&ev).parse().unwrap_or(0.0);
                                        areas.update(|a| {
                                            if let Some(r) = a.extra_circles.get_mut(idx) {
                                                r.center_lon = v;
                                            }
                                        });
                                    }/>
                            </td>
                            <td>
                                <input type="number" step="0.1" min="0.1"
                                    prop:value=rad_v.to_string()
                                    on:input=move |ev| {
                                        let v: f64 = event_target_value(&ev).parse().unwrap_or(1.0);
                                        areas.update(|a| {
                                            if let Some(r) = a.extra_circles.get_mut(idx) {
                                                r.radius_km = v;
                                            }
                                        });
                                    }/>
                            </td>
                            <td>
                                <ZoneSelect
                                    selected=zone_v
                                    zones=zones
                                    on_change=Box::new(move |new_zid: String| {
                                        areas.update(|a| {
                                            if let Some(r) = a.extra_circles.get_mut(idx) {
                                                r.zone_id = new_zid;
                                            }
                                        });
                                    })/>
                            </td>
                            <td>
                                <button type="button" class="btn ghost"
                                    on:click=move |_| {
                                        areas.update(|a| {
                                            if idx < a.extra_circles.len() {
                                                a.extra_circles.remove(idx);
                                            }
                                        });
                                    }>"Entfernen"</button>
                            </td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
        <div class="admin-actions">
            <button type="button" class="btn ghost" on:click=add_circle>"+ Kreis hinzufügen"</button>
            <button type="button" class="btn primary" on:click=save_all>"Speichern"</button>
        </div>
    }
}

#[component]
fn BypassSection(
    bypass_min: RwSignal<i64>,
    bypass_zone: RwSignal<String>,
    zones: StoredValue<Vec<(String, String)>>,
    bypass_saver: ServerAction<SaveBypassSettings>,
) -> impl IntoView {
    let on_save = move |_| {
        bypass_saver.dispatch(SaveBypassSettings {
            min_cents: bypass_min.get(),
            zone_id: bypass_zone.get(),
        });
    };
    let euros_str = move || {
        let c = bypass_min.get();
        if c <= 0 {
            String::new()
        } else {
            format!("{:.2}", c as f64 / 100.0)
        }
    };

    view! {
        <h2 id="bypass">"Bezahlte Bestellungen außerhalb"</h2>
        <p class="hint">
            "Optional: ab welchem online-bezahlten Bestellwert (z. B. Kartenzahlung \
             oder Voucher auf 0 €) wir Adressen ausserhalb des Standardgebiets \
             trotzdem akzeptieren. 0 oder leere Zone = aus."
        </p>
        <label>
            <span>"Schwelle (Euro)"</span>
            <input type="number" step="0.01" min="0" prop:value=euros_str
                placeholder="z. B. 25.00"
                on:input=move |ev| {
                    let v: f64 = event_target_value(&ev).parse().unwrap_or(0.0);
                    bypass_min.set((v * 100.0).round() as i64);
                }/>
        </label>
        <label>
            <span>"Bypass-Zone"</span>
            <ZoneSelect
                selected=bypass_zone.get()
                zones=zones
                on_change=Box::new(move |new_zid: String| { bypass_zone.set(new_zid); })/>
        </label>
        <div class="admin-actions">
            <button type="button" class="btn primary" on:click=on_save>"Speichern"</button>
        </div>
        {move || match bypass_saver.value().get() {
            Some(Ok(())) => Some(view! { <p class="ok">"✓ Gespeichert."</p> }.into_any()),
            Some(Err(e)) => Some(view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any()),
            None => None,
        }}
    }
}

#[component]
fn PostcodeHintsSection(
    hints: RwSignal<Vec<PostcodeHint>>,
    hints_saver: ServerAction<SetPostcodeHints>,
) -> impl IntoView {
    let add_hint = move |_| {
        hints.update(|h| h.push(PostcodeHint::default()));
    };
    let on_save = move |_| {
        hints_saver.dispatch(SetPostcodeHints { hints: hints.get() });
    };

    view! {
        <h2 id="plz-hints">"PLZ ↔ Ort Vorschläge"</h2>
        <p class="hint">
            "Reine UI-Hilfe für den Warenkorb: wenn der Kunde eine PLZ tippt, wird \
             der Ort vorgeschlagen (und umgekehrt). Beeinflusst NICHT die \
             Liefergebiets-Prüfung — das macht die Konfiguration oben."
        </p>
        <table class="admin-table">
            <thead><tr><th>"PLZ"</th><th>"Ort"</th><th></th></tr></thead>
            <tbody>
                {move || hints.get().into_iter().enumerate().map(|(idx, h)| {
                    let plz_v = h.plz.clone();
                    let city_v = h.city.clone();
                    view! {
                        <tr>
                            <td>
                                <input type="text" prop:value=plz_v maxlength="5" inputmode="numeric"
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        hints.update(|list| {
                                            if let Some(r) = list.get_mut(idx) { r.plz = v; }
                                        });
                                    }/>
                            </td>
                            <td>
                                <input type="text" prop:value=city_v
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        hints.update(|list| {
                                            if let Some(r) = list.get_mut(idx) { r.city = v; }
                                        });
                                    }/>
                            </td>
                            <td>
                                <button type="button" class="btn ghost" on:click=move |_| {
                                    hints.update(|list| { list.remove(idx); });
                                }>"Entfernen"</button>
                            </td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
        <div class="admin-actions">
            <button type="button" class="btn ghost" on:click=add_hint>"+ Zeile"</button>
            <button type="button" class="btn primary" on:click=on_save>"Speichern"</button>
        </div>
        {move || match hints_saver.value().get() {
            Some(Ok(())) => Some(view! { <p class="ok">"✓ Gespeichert."</p> }.into_any()),
            Some(Err(e)) => Some(view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any()),
            None => None,
        }}
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
