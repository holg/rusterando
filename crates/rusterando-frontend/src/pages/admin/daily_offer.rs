//! /admin/tagesangebot — the Tagesangebot (pages/daily_offer.rs): a menu
//! item with its size and extras, a name of its own, a price of its own;
//! on the homepage and orderable until switched off.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;
use crate::pages::daily_offer::DailyOffer;

/// A menu item to choose from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OfferItem {
    pub id: String,
    /// "12 · Pizza Salami (Pizza Spezialitäten …)".
    pub label: String,
    pub name: String,
    pub small_label: String,
    pub small_cents: i64,
    pub large_label: Option<String>,
    pub large_cents: Option<i64>,
    pub allow_extras: bool,
    pub included_extras: i64,
    pub flat_extra_cents: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OfferExtra {
    pub id: String,
    pub label: String,
    pub price_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyOfferAdmin {
    pub offer: DailyOffer,
    pub items: Vec<OfferItem>,
    pub extras: Vec<OfferExtra>,
    /// Why the saved offer isn't shown right now (sold out, …).
    pub problem: Option<String>,
}

#[server(
    name = GetDailyOfferAdmin,
    prefix = "/api",
    endpoint = "get_daily_offer_admin"
)]
pub async fn get_daily_offer_admin() -> Result<DailyOfferAdmin, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    use crate::pages::daily_offer::ssr;
    type Row = (
        String,
        Option<String>,
        String,
        String,
        Option<String>,
        i64,
        Option<String>,
        Option<i64>,
        i64,
        i64,
        Option<i64>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT i.id, i.menu_number, i.name, c.name, i.size_small_label, i.price_small_cents,
                i.size_large_label, i.price_large_cents, i.allow_extras,
                i.included_extras_count, i.flat_extra_price_cents
         FROM menu_items i JOIN menu_categories c ON c.id = i.category_id
         WHERE i.is_available = 1 AND i.is_listed = 1 AND c.is_active = 1
         ORDER BY c.sort_order, i.sort_order,
                  CAST(COALESCE(i.menu_number, '') AS INTEGER), i.menu_number",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("Speisekarte: {e}")))?;
    let items = rows
        .into_iter()
        .map(
            |(id, nr, name, cat, small_label, small, large_label, large, allow, incl, flat)| {
                OfferItem {
                    label: match nr.filter(|n| !n.is_empty()) {
                        Some(nr) => format!("{nr} · {name} ({cat})"),
                        None => format!("{name} ({cat})"),
                    },
                    id,
                    name,
                    small_label: small_label.unwrap_or_default(),
                    small_cents: small,
                    large_label: large.and(large_label),
                    large_cents: large,
                    allow_extras: allow != 0,
                    included_extras: incl,
                    flat_extra_cents: flat,
                }
            },
        )
        .collect();
    let extras: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT id, label, price_cents FROM pizza_extras WHERE is_available = 1 ORDER BY sort_order, label",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("Extras: {e}")))?;
    let offer = ssr::load(&db).await;
    let problem = if offer.item_id.is_empty() {
        None
    } else {
        ssr::resolve(&db, &offer).await.err()
    };
    Ok(DailyOfferAdmin {
        offer,
        items,
        extras: extras
            .into_iter()
            .map(|(id, label, price_cents)| OfferExtra {
                id,
                label,
                price_cents,
            })
            .collect(),
        problem,
    })
}

#[server(
    name = SaveDailyOffer,
    prefix = "/api",
    endpoint = "save_daily_offer"
)]
pub async fn save_daily_offer(
    active: bool,
    item_id: String,
    size: String,
    #[server(default)] extras_ids: Vec<String>,
    name: String,
    price: String,
    /// Customers may add toppings on top.
    #[server(default)]
    customer_extras: bool,
) -> Result<(), ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    use crate::pages::daily_offer::ssr;
    let price_cents = match rusterando_shared::models::parse_eur_cents(&price) {
        Ok(Some(c)) if c > 0 && c <= 100_000 => c,
        Ok(_) => return Err(ServerFnError::new("Bitte einen Preis angeben")),
        Err(e) => return Err(ServerFnError::new(e)),
    };
    let mut offer = DailyOffer {
        active,
        item_id,
        size,
        extras_ids,
        name: name.trim().to_string(),
        price_cents,
        rev: String::new(),
        customer_extras,
    };
    let resolved = ssr::resolve(&db, &offer).await;
    let old = ssr::load(&db).await;
    // Another dish keeps no name of the previous one: still the old offer's
    // name (the form wasn't updated) counts as "none given".
    if offer.item_id != old.item_id && offer.name == old.name {
        offer.name.clear();
    }
    if offer.name.is_empty() {
        if let Ok(r) = &resolved {
            offer.name = r.item_name.clone();
        }
    }
    if offer.active {
        if let Err(why) = resolved {
            return Err(ServerFnError::new(why));
        }
    }
    // A new version: cart lines of the old one no longer count.
    let same = DailyOffer {
        rev: old.rev.clone(),
        ..offer.clone()
    } == old;
    offer.rev = if same && !old.rev.is_empty() {
        old.rev
    } else {
        uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
    };
    ssr::store(&db, &offer).await.map_err(ServerFnError::new)?;
    Ok(())
}

#[component]
pub fn DailyOfferAdminPage() -> impl IntoView {
    let saver = ServerAction::<SaveDailyOffer>::new();
    let data = Resource::new(
        move || saver.version().get(),
        |_| async move { get_daily_offer_admin().await },
    );
    view! {
        <AdminShell>
            <section class="admin-daily-offer">
                <header class="admin-bar"><h1>"Tagesangebot"</h1></header>
                <p class="hint">
                    "Ein Artikel der Speisekarte — mit Größe und Extras — unter eigenem Namen zum eigenen Preis. "
                    "Steht ganz oben auf der Startseite mit „Tagesangebot in den Warenkorb“ und gilt, bis es "
                    "ausgeschaltet wird. Der Preis gilt nur für genau diese Zusammenstellung; wird das Angebot "
                    "geändert oder ausgeschaltet, nimmt die Kasse es aus offenen Warenkörben."
                </p>
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || data.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(d) => view! { <OfferForm d saver/> }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn OfferForm(d: DailyOfferAdmin, saver: ServerAction<SaveDailyOffer>) -> impl IntoView {
    use rusterando_shared::models::format_eur;
    let DailyOfferAdmin {
        offer,
        items,
        extras,
        problem,
    } = d;
    // A new offer starts switched on: making one means showing it.
    let active = RwSignal::new(offer.active || offer.item_id.is_empty());
    // Whether the last save switched it on (for the message).
    let saved_on = RwSignal::new(offer.active);
    let customer_extras = RwSignal::new(offer.customer_extras);
    let item_id = RwSignal::new(offer.item_id.clone());
    let size = RwSignal::new(if offer.size.is_empty() {
        "small".to_string()
    } else {
        offer.size.clone()
    });
    let chosen = RwSignal::new(offer.extras_ids.clone());
    let name = RwSignal::new(offer.name.clone());
    let price = RwSignal::new(if offer.price_cents > 0 {
        format_eur(offer.price_cents)
            .trim_end_matches(" €")
            .to_string()
    } else {
        String::new()
    });
    let items = StoredValue::new(items);
    let extras = StoredValue::new(extras);
    let item = move || items.with_value(|all| all.iter().find(|i| i.id == item_id.get()).cloned());

    // What the same costs from the menu.
    let regular = move || {
        let it = item()?;
        let unit = match (size.get().as_str(), it.large_cents) {
            ("large", Some(p)) => p,
            _ => it.small_cents,
        };
        let ids = chosen.get();
        let extra_sum: i64 = if !it.allow_extras {
            0
        } else {
            extras.with_value(|all| {
                ids.iter()
                    .enumerate()
                    .map(|(i, id)| match it.flat_extra_cents {
                        Some(flat) => {
                            if (i as i64) < it.included_extras {
                                0
                            } else {
                                flat
                            }
                        }
                        None => all
                            .iter()
                            .find(|e| &e.id == id)
                            .map(|e| e.price_cents)
                            .unwrap_or(0),
                    })
                    .sum()
            })
        };
        Some(unit + extra_sum)
    };

    let save = move |_| {
        let it = item();
        let extras_ids = if it.as_ref().is_some_and(|i| i.allow_extras) {
            chosen.get_untracked()
        } else {
            Vec::new()
        };
        let size_v = match it.as_ref().and_then(|i| i.large_cents) {
            Some(_) => size.get_untracked(),
            None => "single".to_string(),
        };
        saved_on.set(active.get_untracked());
        saver.dispatch(SaveDailyOffer {
            active: active.get_untracked(),
            item_id: item_id.get_untracked(),
            size: size_v,
            extras_ids,
            name: name.get_untracked(),
            price: price.get_untracked(),
            customer_extras: customer_extras.get_untracked(),
        });
    };

    view! {
        <div class="daily-offer-form">
            {problem.map(|p| view! { <p class="error">{format!("Wird gerade nicht angezeigt: {p}")}</p> })}
            <label class="switch-row">
                <input type="checkbox" prop:checked=move || active.get()
                    on:change=move |ev| active.set(event_target_checked(&ev))/>
                <span>{move || if active.get() { "Eingeschaltet — auf der Startseite und bestellbar" } else { "Ausgeschaltet" }}</span>
            </label>

            <label>
                <span>"Artikel"</span>
                <select prop:value=move || item_id.get()
                    on:change=move |ev| {
                        let id = event_target_value(&ev);
                        // Another dish: its own name as the offer's name
                        // (editable) — never the previous dish's.
                        name.set(items.with_value(|all| {
                            all.iter().find(|i| i.id == id).map(|i| i.name.clone()).unwrap_or_default()
                        }));
                        item_id.set(id);
                    }>
                    <option value="">"— Artikel wählen —"</option>
                    {items.with_value(|all| all.iter().map(|i| view! {
                        <option value=i.id.clone()>{i.label.clone()}</option>
                    }).collect_view())}
                </select>
            </label>

            {move || item().and_then(|it| it.large_cents.map(|large| {
                let small_label = if it.small_label.is_empty() { "klein".to_string() } else { it.small_label.clone() };
                let large_label = it.large_label.clone().unwrap_or_else(|| "groß".into());
                view! {
                    <label>
                        <span>"Größe"</span>
                        <select prop:value=move || size.get()
                            on:change=move |ev| size.set(event_target_value(&ev))>
                            <option value="small">{format!("{small_label} ({})", format_eur(it.small_cents))}</option>
                            <option value="large">{format!("{large_label} ({})", format_eur(large))}</option>
                        </select>
                    </label>
                }
            }))}

            {move || item().filter(|it| it.allow_extras).map(|it| view! {
                <fieldset class="daily-offer-extras">
                    <legend>"Extras darauf"
                        {it.flat_extra_cents.map(|f| format!(" — {} frei, dann je {}", it.included_extras, format_eur(f)))}
                    </legend>
                    {extras.with_value(|all| all.iter().map(|e| {
                        let id = e.id.clone();
                        let id2 = e.id.clone();
                        view! {
                            <label class="extra-check">
                                <input type="checkbox"
                                    prop:checked=move || chosen.get().contains(&id)
                                    on:change=move |ev| {
                                        let on = event_target_checked(&ev);
                                        chosen.update(|c| {
                                            c.retain(|x| x != &id2);
                                            if on { c.push(id2.clone()); }
                                        });
                                    }/>
                                <span>{format!("{} ({})", e.label, format_eur(e.price_cents))}</span>
                            </label>
                        }
                    }).collect_view())}
                </fieldset>
            })}

            <label class="switch-row">
                <input type="checkbox" prop:checked=move || customer_extras.get()
                    on:change=move |ev| customer_extras.set(event_target_checked(&ev))/>
                <span>"Kunden dürfen Extras dazu wählen (zum normalen Extra-Preis)"</span>
            </label>

            <label>
                <span>"Name des Angebots"</span>
                <input type="text" maxlength="80"
                    placeholder=move || item().map(|i| i.name).unwrap_or_default()
                    prop:value=move || name.get()
                    on:input=move |ev| name.set(event_target_value(&ev))/>
            </label>

            <label>
                <span>"Angebotspreis (€)"</span>
                <input type="text" inputmode="decimal" placeholder="z. B. 9,90"
                    prop:value=move || price.get()
                    on:input=move |ev| price.set(event_target_value(&ev))/>
            </label>
            <p class="hint">{move || regular().map(|r| format!("Normalpreis laut Speisekarte: {}", format_eur(r)))}</p>

            <p>
                <button class="btn primary" type="button" on:click=save
                    disabled=move || saver.pending().get()>"Speichern"</button>
                {move || match saver.value().get() {
                    Some(Ok(())) if saved_on.get() => Some(view! {
                        <span class="ok">" Gespeichert — eingeschaltet, steht auf der Startseite."</span>
                    }.into_any()),
                    Some(Ok(())) => Some(view! {
                        <span class="error">" Gespeichert — aber AUSGESCHALTET: nicht auf der Startseite. Oben einschalten und noch einmal speichern."</span>
                    }.into_any()),
                    Some(Err(e)) => Some(view! { <span class="error">{format!(" {e}")}</span> }.into_any()),
                    None => None,
                }}
            </p>
        </div>
    }
}
