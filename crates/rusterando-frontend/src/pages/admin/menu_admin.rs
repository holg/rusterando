use leptos::prelude::*;
use rusterando_shared::models::{format_eur, MenuPayload};

use crate::pages::admin::shell::AdminShell;
use crate::pages::menu::list_admin_menu;

/// Full-field update for an existing menu item. All editable fields go through
/// here in a single round-trip.
///
/// `#[allow(clippy::too_many_arguments)]`: server-fn args define the
/// URL-encoded wire schema that `ActionForm` posts; bundling them into a
/// struct would break the form's flat `<input name="…">` shape with no
/// caller benefit.
#[server(
    name = UpdateMenuItem,
    prefix = "/api",
    endpoint = "update_menu_item"
)]
#[allow(clippy::too_many_arguments)]
pub async fn update_menu_item(
    id: String,
    category_id: String,
    menu_number: Option<String>,
    name: String,
    description: Option<String>,
    price_small_cents: i64,
    price_large_cents: Option<i64>,
    allergen_codes: Option<String>,
    additive_codes: Option<String>,
    is_available: bool,
    is_listed: bool,
    is_spicy: bool,
    included_extras_count: i64,
    flat_extra_price_cents: Option<i64>,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(ServerFnError::new("Name darf nicht leer sein"));
    }
    if price_small_cents < 0 || price_large_cents.unwrap_or(0) < 0 {
        return Err(ServerFnError::new("Preis darf nicht negativ sein"));
    }
    if included_extras_count < 0 {
        return Err(ServerFnError::new("Inkl. Extras darf nicht negativ sein"));
    }
    if flat_extra_price_cents.unwrap_or(0) < 0 {
        return Err(ServerFnError::new(
            "Pauschalpreis pro Extra darf nicht negativ sein",
        ));
    }
    let menu_number = menu_number
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let description = description
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let allergen_codes = normalize_codes(allergen_codes);
    let additive_codes = normalize_codes(additive_codes);

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    sqlx::query(
        "UPDATE menu_items
         SET category_id = ?1,
             menu_number = ?2,
             name = ?3,
             description = ?4,
             price_small_cents = ?5,
             price_large_cents = ?6,
             allergen_codes = ?7,
             additive_codes = ?8,
             is_available = ?9,
             is_listed = ?10,
             is_spicy = ?11,
             included_extras_count = ?12,
             flat_extra_price_cents = ?13,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?14",
    )
    .bind(&category_id)
    .bind(&menu_number)
    .bind(&name)
    .bind(&description)
    .bind(price_small_cents)
    .bind(price_large_cents)
    .bind(&allergen_codes)
    .bind(&additive_codes)
    .bind(if is_available { 1_i64 } else { 0_i64 })
    .bind(if is_listed { 1_i64 } else { 0_i64 })
    .bind(if is_spicy { 1_i64 } else { 0_i64 })
    .bind(included_extras_count)
    .bind(flat_extra_price_cents)
    .bind(&id)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("update item: {e}")))?;

    Ok(())
}

/// `#[allow(clippy::too_many_arguments)]`: same rationale as
/// `update_menu_item` — flat args mirror the form schema.
#[server(
    name = CreateMenuItem,
    prefix = "/api",
    endpoint = "create_menu_item"
)]
#[allow(clippy::too_many_arguments)]
pub async fn create_menu_item(
    category_id: String,
    menu_number: Option<String>,
    name: String,
    description: Option<String>,
    item_type: String,
    price_small_cents: i64,
    price_large_cents: Option<i64>,
    size_small_label: Option<String>,
    size_large_label: Option<String>,
    allergen_codes: Option<String>,
    additive_codes: Option<String>,
    is_spicy: bool,
) -> Result<String, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(ServerFnError::new("Name darf nicht leer sein"));
    }
    if price_small_cents < 0 {
        return Err(ServerFnError::new("Preis darf nicht negativ sein"));
    }
    let menu_number = menu_number
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let description = description
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let allergen_codes = normalize_codes(allergen_codes);
    let additive_codes = normalize_codes(additive_codes);
    let size_small_label = size_small_label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let size_large_label = size_large_label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let item_type = item_type.trim().to_string();
    if item_type.is_empty() {
        return Err(ServerFnError::new("Typ darf nicht leer sein"));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let id = format!("mi-{}", uuid::Uuid::new_v4().simple());

    // sort_order: max within category + 10 so it lands at the end of the list.
    let next_sort: (Option<i64>,) =
        sqlx::query_as("SELECT MAX(sort_order) FROM menu_items WHERE category_id = ?1")
            .bind(&category_id)
            .fetch_one(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("sort lookup: {e}")))?;
    let sort_order = next_sort.0.unwrap_or(0) + 10;

    sqlx::query(
        "INSERT INTO menu_items
            (id, category_id, menu_number, name, description, item_type,
             price_small_cents, price_large_cents, size_small_label, size_large_label,
             allergen_codes, additive_codes, is_spicy, is_available, is_listed, sort_order)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 1, 1, ?14)",
    )
    .bind(&id)
    .bind(&category_id)
    .bind(&menu_number)
    .bind(&name)
    .bind(&description)
    .bind(&item_type)
    .bind(price_small_cents)
    .bind(price_large_cents)
    .bind(&size_small_label)
    .bind(&size_large_label)
    .bind(&allergen_codes)
    .bind(&additive_codes)
    .bind(if is_spicy { 1_i64 } else { 0_i64 })
    .bind(sort_order)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("create item: {e}")))?;

    Ok(id)
}

/// Normalize a comma-separated code list: trim, lowercase letter codes,
/// drop empties, dedup. Returns None for an empty result.
#[cfg(feature = "ssr")]
fn normalize_codes(s: Option<String>) -> Option<String> {
    let raw = s?;
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for piece in raw.split(',') {
        let t = piece.trim();
        if t.is_empty() {
            continue;
        }
        if seen.insert(t.to_string()) {
            out.push(t.to_string());
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join(","))
    }
}

use rusterando_shared::models::{LegendEntry, MenuCategory, MenuItem};

#[component]
pub fn AdminMenuPage() -> impl IntoView {
    let saver = ServerAction::<UpdateMenuItem>::new();
    let creator = ServerAction::<CreateMenuItem>::new();

    let menu = Resource::new(
        move || (saver.version().get(), creator.version().get()),
        |_| async move { list_admin_menu().await },
    );

    view! {
        <AdminShell>
            <section class="admin-menu">
                <div class="page-bar">
                    <h1>"Speisekarte verwalten"</h1>
                    <a href="/menu" class="link">"→ öffentliche Ansicht"</a>
                </div>

                <Suspense fallback=|| view! { <p>"Lädt…"</p> }>
                    {move || menu.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(payload) => view! { <Editor payload saver creator/> }.into_any(),
                    })}
                </Suspense>

                {move || match saver.value().get() {
                    Some(Ok(()))  => Some(view! { <p class="toast ok">"Gespeichert."</p> }.into_any()),
                    Some(Err(e)) => Some(view! { <p class="toast error">{format!("Fehler: {e}")}</p> }.into_any()),
                    None => None,
                }}
                {move || match creator.value().get() {
                    Some(Ok(_))  => Some(view! { <p class="toast ok">"Neuer Artikel angelegt."</p> }.into_any()),
                    Some(Err(e)) => Some(view! { <p class="toast error">{format!("Anlegen fehlgeschlagen: {e}")}</p> }.into_any()),
                    None => None,
                }}
            </section>
        </AdminShell>
    }
}

#[component]
fn Editor(
    payload: MenuPayload,
    saver: ServerAction<UpdateMenuItem>,
    creator: ServerAction<CreateMenuItem>,
) -> impl IntoView {
    let MenuPayload {
        categories,
        items,
        allergens,
        additives,
    } = payload;
    let cats_for_select: Vec<MenuCategory> = categories.clone();

    view! {
        <div class="admin-menu-editor">
            {categories.into_iter().map(|cat| {
                let cat_items = items.iter()
                    .filter(|it| it.category_id == cat.id)
                    .cloned()
                    .collect::<Vec<_>>();
                let allergens = allergens.clone();
                let additives = additives.clone();
                let cats_for_select = cats_for_select.clone();
                view! { <CategoryBlock cat cat_items allergens additives cats_for_select saver creator/> }
            }).collect_view()}
        </div>
    }
}

#[component]
fn CategoryBlock(
    cat: MenuCategory,
    cat_items: Vec<MenuItem>,
    allergens: Vec<LegendEntry>,
    additives: Vec<LegendEntry>,
    cats_for_select: Vec<MenuCategory>,
    saver: ServerAction<UpdateMenuItem>,
    creator: ServerAction<CreateMenuItem>,
) -> impl IntoView {
    let count = cat_items.len();
    let cat_id = cat.id.clone();
    let cat_name = cat.name.clone();

    view! {
        <details class="cat-block" open>
            <summary>
                <strong>{cat_name}</strong>
                <span class="muted">" (" {count} ")"</span>
            </summary>
            <div class="cat-rows">
                {cat_items.into_iter().map(|it| {
                    let allergens = allergens.clone();
                    let additives = additives.clone();
                    let cats_for_select = cats_for_select.clone();
                    view! { <ItemCard it allergens additives cats_for_select saver/> }
                }).collect_view()}
                <NewItemRow
                    cat_id=cat_id.clone()
                    allergens=allergens.clone()
                    additives=additives.clone()
                    creator/>
            </div>
        </details>
    }
}

#[component]
fn ItemCard(
    it: MenuItem,
    allergens: Vec<LegendEntry>,
    additives: Vec<LegendEntry>,
    cats_for_select: Vec<MenuCategory>,
    saver: ServerAction<UpdateMenuItem>,
) -> impl IntoView {
    let id = it.id.clone();
    let category_id = RwSignal::new(it.category_id.clone());
    let menu_number = RwSignal::new(it.menu_number.clone().unwrap_or_default());
    let name = RwSignal::new(it.name.clone());
    let description = RwSignal::new(it.description.clone().unwrap_or_default());
    let small_price = RwSignal::new(it.price_small_cents);
    let large_price = RwSignal::new(it.price_large_cents.unwrap_or(0));
    let has_large = it.price_large_cents.is_some();
    let allergen_set = RwSignal::new(parse_codes(it.allergen_codes.clone()));
    let additive_set = RwSignal::new(parse_codes(it.additive_codes.clone()));
    let is_available = RwSignal::new(it.is_available);
    let is_listed = RwSignal::new(it.is_listed);
    let is_spicy = RwSignal::new(it.is_spicy);
    // Per-item flat extra pricing — see migration 20260510000006.
    // `included_extras_count`: how many extras are free.
    // `flat_extra_price`: per-extra flat charge (cents); 0 means
    // "fall back to pizza_extras catalog" (encoded as None on the wire).
    let included_extras = RwSignal::new(it.included_extras_count);
    let flat_extra_price = RwSignal::new(it.flat_extra_price_cents.unwrap_or(0));

    let on_save = {
        let id = id.clone();
        move |_| {
            saver.dispatch(UpdateMenuItem {
                id: id.clone(),
                category_id: category_id.get(),
                menu_number: opt_string(menu_number.get()),
                name: name.get(),
                description: opt_string(description.get()),
                price_small_cents: small_price.get(),
                price_large_cents: if has_large {
                    Some(large_price.get())
                } else {
                    None
                },
                allergen_codes: opt_string(join_codes(&allergen_set.get())),
                additive_codes: opt_string(join_codes(&additive_set.get())),
                is_available: is_available.get(),
                is_listed: is_listed.get(),
                is_spicy: is_spicy.get(),
                included_extras_count: included_extras.get(),
                // 0 cents → no flat rule (fall back to catalog).
                flat_extra_price_cents: {
                    let p = flat_extra_price.get();
                    if p > 0 {
                        Some(p)
                    } else {
                        None
                    }
                },
            });
        }
    };

    let toggle_listed = move |_| is_listed.update(|v| *v = !*v);

    view! {
        <article class:item-card=true class:unlisted=move || !is_listed.get()>
            <div class="row1">
                <label class="num">
                    <span>"Nr."</span>
                    <input type="text" maxlength="8" placeholder="z.B. 22a"
                        prop:value=move || menu_number.get()
                        on:input=move |ev| menu_number.set(event_target_value(&ev))/>
                </label>
                <label class="name">
                    <span>"Name"</span>
                    <input type="text"
                        prop:value=move || name.get()
                        on:input=move |ev| name.set(event_target_value(&ev))/>
                </label>
                <label class="cat">
                    <span>"Kategorie"</span>
                    <select on:change=move |ev| category_id.set(event_target_value(&ev))>
                        {cats_for_select.into_iter().map(|c| {
                            let selected = c.id == it.category_id;
                            view! { <option value=c.id.clone() selected=selected>{c.name}</option> }
                        }).collect_view()}
                    </select>
                </label>
            </div>
            <label class="desc">
                <span>"Beschreibung (Zutaten)"</span>
                <input type="text"
                    placeholder="mit Tomaten und Goudakäse"
                    prop:value=move || description.get()
                    on:input=move |ev| description.set(event_target_value(&ev))/>
            </label>
            <div class="prices">
                <label>
                    <span>"Preis klein (Cent)"</span>
                    <input type="number" min="0" step="10"
                        prop:value=move || small_price.get()
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<i64>() {
                                small_price.set(v);
                            }
                        }/>
                    <small class="hint">{move || format_eur(small_price.get())}</small>
                </label>
                <label>
                    <span>"Preis groß (Cent)"</span>
                    {if has_large {
                        view! {
                            <span>
                                <input type="number" min="0" step="10"
                                    prop:value=move || large_price.get()
                                    on:input=move |ev| {
                                        if let Ok(v) = event_target_value(&ev).parse::<i64>() {
                                            large_price.set(v);
                                        }
                                    }/>
                                <small class="hint">{move || format_eur(large_price.get())}</small>
                            </span>
                        }.into_any()
                    } else {
                        view! { <span class="muted">"– (nur eine Größe)"</span> }.into_any()
                    }}
                </label>
            </div>
            <CodePicker label="Allergene".to_string() entries=allergens selected=allergen_set/>
            <CodePicker label="Zusatzstoffe".to_string() entries=additives selected=additive_set/>

            // Per-item flat extra pricing. When `Pauschalpreis` is 0,
            // the item falls back to the global pizza_extras catalog
            // prices (Krabben €1, Lachs €2, sonstige €0.70). When > 0,
            // every selected extra costs that flat amount, except for
            // the first `Inkl. Extras` selections which are free.
            // Pizzablech: 3 inkl., €3 pauschal. Pizza 36 cm: 0 inkl., €1 pauschal.
            <div class="row1">
                <label>
                    <span>"Inkl. Extras"</span>
                    <input type="number" min="0" step="1"
                        prop:value=move || included_extras.get()
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<i64>() {
                                included_extras.set(v.max(0));
                            }
                        }/>
                    <small class="hint">"Wieviele Extras sind inklusive (gratis)?"</small>
                </label>
                <label>
                    <span>"Pauschalpreis pro Extra (Cent)"</span>
                    <input type="number" min="0" step="10"
                        prop:value=move || flat_extra_price.get()
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<i64>() {
                                flat_extra_price.set(v.max(0));
                            }
                        }/>
                    <small class="hint">
                        {move || {
                            let p = flat_extra_price.get();
                            if p == 0 {
                                "0 = Katalogpreise verwenden".to_string()
                            } else {
                                format!("{} pro Extra", format_eur(p))
                            }
                        }}
                    </small>
                </label>
            </div>

            <div class="flags">
                <label class="flag">
                    <input type="checkbox"
                        prop:checked=move || is_available.get()
                        on:change=move |ev| is_available.set(event_target_checked(&ev))/>
                    <span>"verfügbar"</span>
                </label>
                <label class="flag">
                    <input type="checkbox"
                        prop:checked=move || is_spicy.get()
                        on:change=move |ev| is_spicy.set(event_target_checked(&ev))/>
                    <span>"scharf"</span>
                </label>
                <button class="btn ghost danger" type="button" on:click=toggle_listed>
                    {move || if is_listed.get() { "✕ Vom Menü entfernen" } else { "↺ Wieder ins Menü aufnehmen" }}
                </button>
                <button class="btn primary" type="button" on:click=on_save>"Speichern"</button>
            </div>
            <Show when=move || !is_listed.get() fallback=|| ()>
                <p class="hint warn">
                    "Dieser Artikel ist auf der öffentlichen Speisekarte ausgeblendet. Bestehende Bestellungen bleiben erhalten."
                </p>
            </Show>
        </article>
    }
}

#[component]
fn NewItemRow(
    cat_id: String,
    allergens: Vec<LegendEntry>,
    additives: Vec<LegendEntry>,
    creator: ServerAction<CreateMenuItem>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let menu_number = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let description = RwSignal::new(String::new());
    let item_type = RwSignal::new("pizza".to_string());
    let price_small = RwSignal::new(0_i64);
    let price_large = RwSignal::new(0_i64);
    let has_large = RwSignal::new(true);
    let size_small = RwSignal::new("22cm".to_string());
    let size_large = RwSignal::new("30cm".to_string());
    let allergen_set = RwSignal::new(std::collections::BTreeSet::<String>::new());
    let additive_set = RwSignal::new(std::collections::BTreeSet::<String>::new());
    let is_spicy = RwSignal::new(false);

    let cat_id_sv = StoredValue::new(cat_id);
    let on_create = move |_| {
        creator.dispatch(CreateMenuItem {
            category_id: cat_id_sv.get_value(),
            menu_number: opt_string(menu_number.get()),
            name: name.get(),
            description: opt_string(description.get()),
            item_type: item_type.get(),
            price_small_cents: price_small.get(),
            price_large_cents: if has_large.get() {
                Some(price_large.get())
            } else {
                None
            },
            size_small_label: if has_large.get() {
                opt_string(size_small.get())
            } else {
                None
            },
            size_large_label: if has_large.get() {
                opt_string(size_large.get())
            } else {
                None
            },
            allergen_codes: opt_string(join_codes(&allergen_set.get())),
            additive_codes: opt_string(join_codes(&additive_set.get())),
            is_spicy: is_spicy.get(),
        });
        // Reset for the next add — collapses on success via the resource refetch.
        menu_number.set(String::new());
        name.set(String::new());
        description.set(String::new());
        price_small.set(0);
        price_large.set(0);
        allergen_set.set(Default::default());
        additive_set.set(Default::default());
        is_spicy.set(false);
        open.set(false);
    };

    // The <Show> children block is invoked on every re-render, so we need
    // closure-captured clones inside the move-once body. Wrap in StoredValue
    // so we can clone cheaply from inside the children.
    let allergens_sv = StoredValue::new(allergens);
    let additives_sv = StoredValue::new(additives);

    view! {
        <Show
            when=move || open.get()
            fallback=move || view! {
                <button class="btn ghost add-item" on:click=move |_| open.set(true)>
                    "+ Neuer Artikel"
                </button>
            }>
            <article class="item-card new">
                <div class="row1">
                    <label class="num">
                        <span>"Nr."</span>
                        <input type="text" maxlength="8"
                            prop:value=move || menu_number.get()
                            on:input=move |ev| menu_number.set(event_target_value(&ev))/>
                    </label>
                    <label class="name">
                        <span>"Name *"</span>
                        <input type="text" required
                            prop:value=move || name.get()
                            on:input=move |ev| name.set(event_target_value(&ev))/>
                    </label>
                    <label class="cat">
                        <span>"Typ"</span>
                        <select on:change=move |ev| item_type.set(event_target_value(&ev))>
                            <option value="pizza" selected=true>"pizza"</option>
                            <option value="calzone">"calzone"</option>
                            <option value="pasta">"pasta"</option>
                            <option value="oven">"oven"</option>
                            <option value="meat">"meat"</option>
                            <option value="salad">"salad"</option>
                            <option value="side">"side"</option>
                            <option value="drink">"drink"</option>
                        </select>
                    </label>
                </div>
                <label class="desc">
                    <span>"Beschreibung"</span>
                    <input type="text"
                        prop:value=move || description.get()
                        on:input=move |ev| description.set(event_target_value(&ev))/>
                </label>
                <div class="flags">
                    <label class="flag">
                        <input type="checkbox"
                            prop:checked=move || has_large.get()
                            on:change=move |ev| has_large.set(event_target_checked(&ev))/>
                        <span>"Zwei Größen (z.B. Pizza)"</span>
                    </label>
                </div>
                <div class="prices">
                    <label>
                        <span>"Preis klein (Cent) *"</span>
                        <input type="number" min="0" step="10"
                            prop:value=move || price_small.get()
                            on:input=move |ev| {
                                if let Ok(v) = event_target_value(&ev).parse::<i64>() {
                                    price_small.set(v);
                                }
                            }/>
                        <small class="hint">{move || format_eur(price_small.get())}</small>
                        <input type="text" placeholder="Größe (z.B. 22cm)"
                            prop:value=move || size_small.get()
                            on:input=move |ev| size_small.set(event_target_value(&ev))/>
                    </label>
                    <label>
                        <span>"Preis groß (Cent)"</span>
                        <Show
                            when=move || has_large.get()
                            fallback=move || view! { <span class="muted">"–"</span> }>
                            <input type="number" min="0" step="10"
                                prop:value=move || price_large.get()
                                on:input=move |ev| {
                                    if let Ok(v) = event_target_value(&ev).parse::<i64>() {
                                        price_large.set(v);
                                    }
                                }/>
                            <small class="hint">{move || format_eur(price_large.get())}</small>
                            <input type="text" placeholder="Größe (z.B. 30cm)"
                                prop:value=move || size_large.get()
                                on:input=move |ev| size_large.set(event_target_value(&ev))/>
                        </Show>
                    </label>
                </div>
                <CodePicker label="Allergene".to_string() entries=allergens_sv.get_value() selected=allergen_set/>
                <CodePicker label="Zusatzstoffe".to_string() entries=additives_sv.get_value() selected=additive_set/>
                <div class="flags">
                    <label class="flag">
                        <input type="checkbox"
                            prop:checked=move || is_spicy.get()
                            on:change=move |ev| is_spicy.set(event_target_checked(&ev))/>
                        <span>"scharf"</span>
                    </label>
                    <button class="btn ghost" type="button" on:click=move |_| open.set(false)>"Abbrechen"</button>
                    <button class="btn primary" type="button" on:click=on_create>"Anlegen"</button>
                </div>
            </article>
        </Show>
    }
}

#[component]
fn CodePicker(
    label: String,
    entries: Vec<LegendEntry>,
    selected: RwSignal<std::collections::BTreeSet<String>>,
) -> impl IntoView {
    view! {
        <fieldset class="codes-fs">
            <legend>{label}</legend>
            <div class="code-grid">
                {entries.into_iter().map(|e| {
                    let code = e.code.clone();
                    let name_de = e.name_de.clone();
                    let code_for_check = code.clone();
                    let code_for_toggle = code.clone();
                    view! {
                        <label class="code-chip">
                            <input type="checkbox"
                                prop:checked=move || selected.get().contains(&code_for_check)
                                on:change=move |ev| {
                                    let on = event_target_checked(&ev);
                                    selected.update(|s| {
                                        if on { s.insert(code_for_toggle.clone()); }
                                        else  { s.remove(&code_for_toggle); }
                                    });
                                }/>
                            <span class="code">{code}</span>
                            <span class="name">{name_de}</span>
                        </label>
                    }
                }).collect_view()}
            </div>
        </fieldset>
    }
}

fn parse_codes(raw: Option<String>) -> std::collections::BTreeSet<String> {
    let mut s = std::collections::BTreeSet::new();
    if let Some(r) = raw {
        for piece in r.split(',') {
            let t = piece.trim();
            if !t.is_empty() {
                s.insert(t.to_string());
            }
        }
    }
    s
}

fn join_codes(set: &std::collections::BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join(",")
}

fn opt_string(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}
