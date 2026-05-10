use leptos::prelude::*;
use rusterando_shared::models::{
    format_eur, LegendEntry, MenuCategory, MenuItem, MenuPayload, PizzaExtra,
};

use crate::components::cart_drawer::use_cart_ctx;
use crate::pages::admin::extras_admin::list_pizza_extras;
use crate::pages::cart::AddToCart;

#[server(
    name = ListMenu,
    prefix = "/api",
    endpoint = "list_menu"
)]
pub async fn list_menu() -> Result<MenuPayload, ServerFnError> {
    use sqlx::SqlitePool;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let cat_rows = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT id, name, sort_order FROM menu_categories WHERE is_active = 1 ORDER BY sort_order",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load categories: {e}")))?;

    let categories = cat_rows
        .into_iter()
        .map(|(id, name, sort_order)| MenuCategory {
            id,
            name,
            sort_order,
        })
        .collect::<Vec<_>>();

    let items = load_items(&db, /* admin */ false).await?;

    let allergens = sqlx::query_as::<_, (String, String)>(
        "SELECT code, name_de FROM allergen_legend ORDER BY code",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load allergens: {e}")))?
    .into_iter()
    .map(|(code, name_de)| LegendEntry { code, name_de })
    .collect();

    let additives = sqlx::query_as::<_, (String, String)>(
        "SELECT code, name_de FROM additive_legend ORDER BY
         CASE WHEN length(code) = 1 AND code GLOB '[0-9]' THEN 0 ELSE 1 END, code",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load additives: {e}")))?
    .into_iter()
    .map(|(code, name_de)| LegendEntry { code, name_de })
    .collect();

    Ok(MenuPayload {
        categories,
        items,
        allergens,
        additives,
    })
}

#[server(
    name = ListAdminMenu,
    prefix = "/api",
    endpoint = "list_admin_menu"
)]
pub async fn list_admin_menu() -> Result<MenuPayload, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let cat_rows = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT id, name, sort_order FROM menu_categories WHERE is_active = 1 ORDER BY sort_order",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load categories: {e}")))?;
    let categories = cat_rows
        .into_iter()
        .map(|(id, name, sort_order)| MenuCategory {
            id,
            name,
            sort_order,
        })
        .collect::<Vec<_>>();

    let items = load_items(&db, /* admin */ true).await?;

    let allergens = sqlx::query_as::<_, (String, String)>(
        "SELECT code, name_de FROM allergen_legend ORDER BY code",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load allergens: {e}")))?
    .into_iter()
    .map(|(code, name_de)| LegendEntry { code, name_de })
    .collect();

    let additives = sqlx::query_as::<_, (String, String)>(
        "SELECT code, name_de FROM additive_legend ORDER BY
         CASE WHEN length(code) = 1 AND code GLOB '[0-9]' THEN 0 ELSE 1 END, code",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load additives: {e}")))?
    .into_iter()
    .map(|(code, name_de)| LegendEntry { code, name_de })
    .collect();

    Ok(MenuPayload {
        categories,
        items,
        allergens,
        additives,
    })
}

/// Shared item loader. `admin=true` returns every row; `admin=false` filters
/// to listed items only (`is_listed = 1`).
#[cfg(feature = "ssr")]
async fn load_items(db: &sqlx::SqlitePool, admin: bool) -> Result<Vec<MenuItem>, ServerFnError> {
    let where_clause = if admin { "" } else { "WHERE is_listed = 1" };
    let sql = format!(
        "SELECT id, category_id, menu_number, name, description, item_type,
                price_small_cents, price_large_cents, size_small_label, size_large_label,
                allergen_codes, additive_codes, is_spicy, is_available, is_listed, sort_order
         FROM menu_items
         {where_clause}
         ORDER BY sort_order,
                  CAST(COALESCE(menu_number, '') AS INTEGER),
                  menu_number"
    );
    let item_rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            String,
            i64,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
            i64,
            i64,
            i64,
        ),
    >(&sql)
    .fetch_all(db)
    .await
    .map_err(|e| ServerFnError::new(format!("load items: {e}")))?;

    Ok(item_rows
        .into_iter()
        .map(
            |(
                id,
                category_id,
                menu_number,
                name,
                description,
                item_type,
                price_small_cents,
                price_large_cents,
                size_small_label,
                size_large_label,
                allergen_codes,
                additive_codes,
                is_spicy,
                is_available,
                is_listed,
                sort_order,
            )| MenuItem {
                id,
                category_id,
                menu_number,
                name,
                description,
                item_type,
                price_small_cents,
                price_large_cents,
                size_small_label,
                size_large_label,
                allergen_codes,
                additive_codes,
                is_spicy: is_spicy != 0,
                is_available: is_available != 0,
                is_listed: is_listed != 0,
                sort_order,
            },
        )
        .collect())
}

#[component]
pub fn MenuPage() -> impl IntoView {
    // Bundle both fetches into one resource so SSR and hydrate see exactly the
    // same await-point. With two separate resources read inside a single
    // <Suspense>, tachys occasionally trips on the reactive shape and panics
    // during hydration with `entered unreachable code` — collapsing them into
    // one tuple-returning future avoids that entirely.
    let combined = OnceResource::new(async move {
        let m = list_menu().await;
        let e = list_pizza_extras().await.unwrap_or_default();
        (m, e)
    });

    view! {
        <Suspense fallback=|| view! { <p class="loading">"Speisekarte lädt…"</p> }>
            {move || combined.get().map(|(menu_res, extras)| match menu_res {
                Err(e) => view! {
                    <p class="error">{format!("Fehler beim Laden der Speisekarte: {e}")}</p>
                }.into_any(),
                Ok(payload) => view! {
                    <MenuView payload extras=extras.clone()/>
                }.into_any(),
            })}
        </Suspense>
    }
}

#[component]
fn MenuView(payload: MenuPayload, extras: Vec<PizzaExtra>) -> impl IntoView {
    let MenuPayload {
        categories,
        items,
        allergens,
        additives,
    } = payload;

    // Provide the resolved extras catalog to every Card via context. The
    // value is captured at MenuView construction time so SSR and hydrate see
    // an identical immutable Vec.
    let extras_sv: StoredValue<Vec<PizzaExtra>> = StoredValue::new(extras);
    provide_context(extras_sv);

    let cats_for_tabs = categories.clone();
    let cats_for_sections = categories.clone();

    // Pull shop phone from BrandingHandle (SSR context) so the "call us"
    // line on the menu reflects the configured shop, not a hardcoded
    // number. Empty when the admin hasn't set one yet.
    #[cfg(feature = "ssr")]
    let phone = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get().shop_phone)
        .unwrap_or_default();
    #[cfg(not(feature = "ssr"))]
    let phone = String::new();

    view! {
        <div class="menu">
            <header class="menu-header">
                <h1>"Speisekarte"</h1>
                {(!phone.is_empty()).then(|| view! {
                    <p>{format!("Telefonisch bestellen: {phone}")}</p>
                })}
                <p>
                    <a class="pdf-link" href="/menu.pdf" target="_blank" rel="noopener">
                        "📄 Speisekarte als PDF"
                    </a>
                </p>
            </header>

            <nav class="category-tabs">
                {cats_for_tabs.into_iter().map(|c| {
                    let href = format!("#cat-{}", c.id);
                    view! { <a href=href>{c.name.clone()}</a> }
                }).collect_view()}
            </nav>

            <div class="categories">
                {cats_for_sections.into_iter().map(|cat| {
                    let cat_items: Vec<MenuItem> = items.iter()
                        .filter(|it| it.category_id == cat.id)
                        .cloned()
                        .collect();
                    view! { <CategorySection cat items=cat_items/> }
                }).collect_view()}
            </div>

            <Legend allergens additives/>
        </div>
    }
}

#[component]
fn Legend(allergens: Vec<LegendEntry>, additives: Vec<LegendEntry>) -> impl IntoView {
    view! {
        <section class="legend" id="legend">
            <h2>"Allergene & Zusatzstoffe"</h2>
            <p class="legend-intro">
                "Gemäß EU-Verordnung 1169/2011. Die Buchstaben hinter den Speisen kennzeichnen \
                 Allergene, die Zahlen Zusatzstoffe."
            </p>

            <div class="legend-grid">
                <div>
                    <h3>"Allergene"</h3>
                    <dl>
                        {allergens.into_iter().map(|e| {
                            let id = format!("allergen-{}", e.code);
                            view! {
                                <div class="legend-row" id=id>
                                    <dt>{e.code}</dt>
                                    <dd>{e.name_de}</dd>
                                </div>
                            }
                        }).collect_view()}
                    </dl>
                </div>
                <div>
                    <h3>"Zusatzstoffe"</h3>
                    <dl>
                        {additives.into_iter().map(|e| {
                            let id = format!("additive-{}", code_to_anchor(&e.code));
                            view! {
                                <div class="legend-row" id=id>
                                    <dt>{e.code}</dt>
                                    <dd>{e.name_de}</dd>
                                </div>
                            }
                        }).collect_view()}
                    </dl>
                </div>
            </div>
        </section>
    }
}

/// `*` and `**` aren't valid in URL fragments / id attributes; map them to readable slugs.
fn code_to_anchor(code: &str) -> String {
    match code {
        "*" => "schinken".into(),
        "**" => "salami".into(),
        other => other.into(),
    }
}

#[component]
fn CategorySection(cat: MenuCategory, items: Vec<MenuItem>) -> impl IntoView {
    let anchor = format!("cat-{}", cat.id);
    view! {
        <section class="category" id=anchor>
            <h2>{cat.name}</h2>
            <div class="grid">
                {items.into_iter().map(|it| view! { <Card it/> }).collect_view()}
            </div>
        </section>
    }
}

#[component]
fn Card(it: MenuItem) -> impl IntoView {
    let number = it.menu_number.clone().map(|n| format!("Nr. {n}"));
    let allergens = it.allergen_codes.clone().filter(|s| !s.is_empty());
    let additives = it.additive_codes.clone().filter(|s| !s.is_empty());
    let unavailable = !it.is_available;
    let spicy = it.is_spicy;
    let item_id = it.id.clone();
    let has_large = it.price_large_cents.is_some();
    let small_label = it.size_small_label.clone().unwrap_or_default();
    let large_label = it.size_large_label.clone().unwrap_or_default();
    let small_price = it.price_small_cents;
    let large_price = it.price_large_cents.unwrap_or(0);

    // Extras picker is shown only for pizza-type items. Other categories
    // (drinks, salads, pasta) skip it; the schema would let us extend later.
    let allows_extras = matches!(it.item_type.as_str(), "pizza" | "calzone");
    let selected_extras: RwSignal<Vec<String>> = RwSignal::new(Vec::new());
    let extras_catalog: StoredValue<Vec<PizzaExtra>> =
        use_context().unwrap_or_else(|| StoredValue::new(Vec::new()));

    // Live sum of ticked extras so the size buttons show the real price the
    // customer is about to pay (base + extras) instead of just the base.
    let extras_sum_cents = Memo::new(move |_| -> i64 {
        let picked = selected_extras.get();
        if picked.is_empty() {
            return 0;
        }
        extras_catalog.with_value(|cat| {
            cat.iter()
                .filter(|e| picked.iter().any(|p| p == &e.id))
                .map(|e| e.price_cents)
                .sum()
        })
    });

    let ctx = use_cart_ctx();

    let add_small = {
        let id = item_id.clone();
        move |_| {
            let size = if has_large { "small" } else { "single" };
            ctx.add.dispatch(AddToCart {
                menu_item_id: id.clone(),
                size: size.to_string(),
                quantity: 1,
                extras_ids: selected_extras.get_untracked(),
            });
            selected_extras.set(Vec::new());
            ctx.open.set(true);
        }
    };
    let add_large = {
        let id = item_id.clone();
        move |_| {
            ctx.add.dispatch(AddToCart {
                menu_item_id: id.clone(),
                size: "large".to_string(),
                quantity: 1,
                extras_ids: selected_extras.get_untracked(),
            });
            selected_extras.set(Vec::new());
            ctx.open.set(true);
        }
    };

    view! {
        <article class:card=true class:unavailable=unavailable>
            <div class="card-head">
                {number.map(|n| view! { <span class="num">{n}</span> })}
                <h3>{it.name.clone()}</h3>
                {spicy.then(|| view! { <span class="spicy" title="scharf">"🌶"</span> })}
            </div>
            {it.description.clone().map(|d| view! { <p class="desc">{d}</p> })}
            <div class="codes">
                {allergens.map(|s| view! {
                    <span class="codeset allergens">
                        <span class="label">"Allergene:"</span>
                        {split_codes(&s).into_iter().map(|c| {
                            let href = format!("#allergen-{c}");
                            view! { <a class="code" href=href>{c}</a> }
                        }).collect_view()}
                    </span>
                })}
                {additives.map(|s| view! {
                    <span class="codeset additives">
                        <span class="label">"Zusatzstoffe:"</span>
                        {split_codes(&s).into_iter().map(|c| {
                            let href = format!("#additive-{}", code_to_anchor(&c));
                            view! { <a class="code" href=href>{c}</a> }
                        }).collect_view()}
                    </span>
                })}
            </div>
            {allows_extras.then(|| view! {
                <ExtrasPicker selected=selected_extras catalog=extras_catalog/>
            })}
            <div class="add-row">
                {if has_large {
                    view! {
                        <button class="add" disabled=unavailable on:click=add_small>
                            <span class="add-size">{small_label.clone()}</span>
                            <span class="add-price">
                                {move || format_eur(small_price + extras_sum_cents.get())}
                            </span>
                        </button>
                        <button class="add" disabled=unavailable on:click=add_large>
                            <span class="add-size">{large_label.clone()}</span>
                            <span class="add-price">
                                {move || format_eur(large_price + extras_sum_cents.get())}
                            </span>
                        </button>
                    }.into_any()
                } else {
                    view! {
                        <button class="add wide" disabled=unavailable on:click=add_small>
                            <span class="add-size">"Hinzufügen"</span>
                            <span class="add-price">
                                {move || format_eur(small_price + extras_sum_cents.get())}
                            </span>
                        </button>
                    }.into_any()
                }}
            </div>
            {unavailable.then(|| view! { <p class="oos">"derzeit nicht verfügbar"</p> })}
        </article>
    }
}

fn split_codes(s: &str) -> Vec<String> {
    s.split(',')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

#[component]
fn ExtrasPicker(
    selected: RwSignal<Vec<String>>,
    catalog: StoredValue<Vec<PizzaExtra>>,
) -> impl IntoView {
    // Render statically (no enclosing `move ||`) so SSR and hydrate produce
    // an identical DOM tree. The catalog is captured by value once per
    // MenuView mount; toggling is handled per-checkbox via reactive
    // `is_checked` memos.
    let list = catalog.with_value(|v| v.clone());
    if list.is_empty() {
        return ().into_any();
    }
    view! {
        <details class="extras-picker">
            <summary>"Extras hinzufügen"</summary>
            <ul class="extras-list">
                {list.into_iter().map(|ex| {
                    let id = ex.id.clone();
                    let id_for_check = id.clone();
                    let id_for_track = id.clone();
                    let is_checked = Memo::new(move |_| {
                        selected.with(|v| v.contains(&id_for_track))
                    });
                    let label = ex.label.clone();
                    let price_label = if ex.price_cents == 0 {
                        "gratis".to_string()
                    } else {
                        format!("+{}", format_eur(ex.price_cents))
                    };
                    view! {
                        <li>
                            <label class="extra-row">
                                <input type="checkbox"
                                    prop:checked=move || is_checked.get()
                                    on:change=move |ev| {
                                        let on = event_target_checked(&ev);
                                        selected.update(|v| {
                                            v.retain(|x| x != &id_for_check);
                                            if on { v.push(id_for_check.clone()); }
                                        });
                                    }/>
                                <span class="extra-label">{label}</span>
                                <span class="extra-price">{price_label}</span>
                            </label>
                        </li>
                    }
                }).collect_view()}
            </ul>
        </details>
    }
    .into_any()
}
