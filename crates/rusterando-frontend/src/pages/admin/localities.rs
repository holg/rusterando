//! /admin/localities — edit the two-layer delivery-area config.
//!
//! Section A "Belieferte Orte" — `served_municipalities`. Each entry
//! has a list of match-names (variants Nominatim might return) and a
//! zone_routing list (specific suburbs → zone, plus a `*` wildcard for
//! the rest).
//!
//! Section B "Zusätzliche Liefer-Inseln" — `extra_circles`. Each entry
//! is a circle (centre lat/lon + radius) with a zone. Used for outlier
//! addresses that Nominatim doesn't put in a served municipality but
//! David chooses to serve anyway.
//!
//! Section C "Bezahlte Bestellungen außerhalb" — the paid-bypass
//! threshold + zone. When set, online-paid orders over the threshold
//! bypass the wrong_municipality / wrong_suburb rejections.
//!
//! Also: PLZ → Ort autofill hints (cosmetic only). Separate sub-section.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;
#[cfg(feature = "ssr")]
use crate::pages::locality::{get_delivery_areas, get_postcode_hints};
use crate::pages::locality::{
    DeliveryAreas, ExtraCircle, PostcodeHint, ServedMunicipality, SetDeliveryAreas,
    SetPostcodeHints, SuburbRoute, SUBURB_WILDCARD,
};

/// Snapshot of what /admin/localities edits. Bundled so the page does
/// one Resource fetch and can save sections independently.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LocalitiesAdmin {
    pub areas: DeliveryAreas,
    pub hints: Vec<PostcodeHint>,
    pub bypass_min_cents: i64,
    pub bypass_zone_id: String,
    /// Available zones for the <select> in each row.
    pub zones: Vec<(String, String)>, // (id, name)
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

#[component]
pub fn LocalitiesAdminPage() -> impl IntoView {
    let areas_saver = ServerAction::<SetDeliveryAreas>::new();
    let hints_saver = ServerAction::<SetPostcodeHints>::new();
    let bypass_saver = ServerAction::<SaveBypassSettings>::new();
    let data = Resource::new(
        move || {
            (
                areas_saver.version().get(),
                hints_saver.version().get(),
                bypass_saver.version().get(),
            )
        },
        |_| async move { load_localities_admin().await },
    );

    view! {
        <AdminShell>
            <section class="admin-localities">
                <header class="admin-bar">
                    <h1>"Liefergebiet"</h1>
                </header>
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || data.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(d) => view! {
                            <LocalitiesEditor d areas_saver hints_saver bypass_saver/>
                        }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn LocalitiesEditor(
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
        <p class="hint">
            "Liefer-Konfiguration in zwei Schichten: (A) bekannte Orte mit ihren \
             Stadtteil-Routen — hält das Versprechen \"wir liefern in <Ort>\". \
             (B) zusätzliche Liefer-Inseln per Umkreis-Kreis — für ausgewählte \
             Bauerschaften, die nicht in einem belieferten Ort liegen, aber \
             trotzdem beliefert werden sollen. Optional (C): ab welchem online \
             bezahlten Betrag wir auch außerhalb liefern."
        </p>

        <ServedMunicipalitiesSection areas zones=zones areas_saver/>
        <ExtraCirclesSection areas zones=zones areas_saver/>
        <BypassSection bypass_min bypass_zone zones=zones bypass_saver/>
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
        <h2>"Belieferte Orte"</h2>
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
                                    <input type="text" value=suburb_v
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
    view! {
        <select on:change=move |ev| on_change(event_target_value(&ev))>
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
        <h2>"Zusätzliche Liefer-Inseln (Umkreis)"</h2>
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
                                <input type="text" value=label_v
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
                                <input type="number" step="0.000001" value=lat_v.to_string()
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
                                <input type="number" step="0.000001" value=lon_v.to_string()
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
                                <input type="number" step="0.1" min="0.1" value=rad_v.to_string()
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
        <h2>"Bezahlte Bestellungen außerhalb"</h2>
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
        <h2>"PLZ ↔ Ort Vorschläge"</h2>
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
                                <input type="text" value=plz_v maxlength="5" inputmode="numeric"
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        hints.update(|list| {
                                            if let Some(r) = list.get_mut(idx) { r.plz = v; }
                                        });
                                    }/>
                            </td>
                            <td>
                                <input type="text" value=city_v
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
