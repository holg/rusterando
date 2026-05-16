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

    let cat_name = crate::i18n::coalesce_col("name", "");
    let cat_rows = sqlx::query_as::<_, (String, String, i64)>(
        &format!(
            "SELECT id, {cat_name} AS name, sort_order FROM menu_categories WHERE is_active = 1 ORDER BY sort_order"
        ),
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

    let shop_phone = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get().shop_phone)
        .unwrap_or_default();

    // Per-shop toggle for the always-visible category sidebar. Stored
    // as '0'/'1' (or 'true'/'false') in the generic app_settings table.
    let category_overlay = sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_settings WHERE key = 'menu_category_overlay'",
    )
    .fetch_optional(&db)
    .await
    .ok()
    .flatten()
    .map(|v| matches!(v.as_str(), "1" | "true"))
    .unwrap_or(false);

    Ok(MenuPayload {
        categories,
        items,
        allergens,
        additives,
        shop_phone,
        category_overlay,
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

    // Admin payload doesn't render the public phone hint, but the field
    // is shared with the public payload so we populate it from
    // BrandingHandle to keep both branches in sync.
    let shop_phone = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get().shop_phone)
        .unwrap_or_default();

    // Same toggle the public payload carries — admin UI doesn't show
    // an overlay, but the shape stays in sync.
    let category_overlay = sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_settings WHERE key = 'menu_category_overlay'",
    )
    .fetch_optional(&db)
    .await
    .ok()
    .flatten()
    .map(|v| matches!(v.as_str(), "1" | "true"))
    .unwrap_or(false);

    Ok(MenuPayload {
        categories,
        items,
        allergens,
        additives,
        shop_phone,
        category_overlay,
    })
}

/// Shared item loader. `admin=true` returns every row; `admin=false` filters
/// to listed items only (`is_listed = 1`).
#[cfg(feature = "ssr")]
async fn load_items(db: &sqlx::SqlitePool, admin: bool) -> Result<Vec<MenuItem>, ServerFnError> {
    #[derive(sqlx::FromRow)]
    struct ItemRow {
        id: String,
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
        is_spicy: i64,
        is_available: i64,
        is_listed: i64,
        sort_order: i64,
        included_extras_count: i64,
        flat_extra_price_cents: Option<i64>,
        allow_extras: i64,
    }

    let where_clause = if admin { "" } else { "WHERE is_listed = 1" };
    // Admin loader keeps the canonical DE columns so the edit form shows
    // the source text. Customer-facing loader picks the active locale.
    let name_col = if admin {
        "name".to_string()
    } else {
        crate::i18n::coalesce_col("name", "")
    };
    let desc_col = if admin {
        "description".to_string()
    } else {
        crate::i18n::coalesce_col("description", "")
    };
    let sql = format!(
        "SELECT id, category_id, menu_number, {name_col} AS name, {desc_col} AS description, item_type,
                price_small_cents, price_large_cents, size_small_label, size_large_label,
                allergen_codes, additive_codes, is_spicy, is_available, is_listed, sort_order,
                included_extras_count, flat_extra_price_cents, allow_extras
         FROM menu_items
         {where_clause}
         ORDER BY sort_order,
                  CAST(COALESCE(menu_number, '') AS INTEGER),
                  menu_number"
    );
    let item_rows = sqlx::query_as::<_, ItemRow>(&sql)
        .fetch_all(db)
        .await
        .map_err(|e| ServerFnError::new(format!("load items: {e}")))?;

    // Per-item option groups + their options. Load in two batched
    // queries so we don't N+1 even with 100+ items on the menu.
    use rusterando_shared::models::{OptionGroup, OptionItem};
    use std::collections::HashMap;

    let group_label = if admin {
        "g.label".to_string()
    } else {
        // Manual COALESCE here because coalesce_col() doesn't yet handle
        // the table-alias prefix `g.` — we only have one consumer of this
        // exact pattern so the inline version is OK.
        let loc = crate::i18n::current_locale();
        if loc == crate::i18n::Locale::DEFAULT {
            "g.label".to_string()
        } else {
            format!("COALESCE(g.label_{lang}, g.label)", lang = loc.code())
        }
    };
    let group_sql = format!(
        "SELECT mg.menu_item_id, g.id, {group_label} AS label, g.min_select, g.max_select, g.sort_order
         FROM menu_item_option_groups mg
         JOIN item_option_groups g ON g.id = mg.group_id
         WHERE g.is_active = 1
         ORDER BY mg.menu_item_id, g.sort_order, g.id"
    );
    let group_rows = sqlx::query_as::<_, (String, String, String, i64, i64, i64)>(&group_sql)
        .fetch_all(db)
        .await
        .map_err(|e| ServerFnError::new(format!("load option groups: {e}")))?;

    // group_id → Vec<OptionItem>
    let option_label = if admin {
        "label".to_string()
    } else {
        crate::i18n::coalesce_col("label", "")
    };
    let option_sql = format!(
        "SELECT group_id, id, {option_label} AS label, price_cents, sort_order
         FROM item_options
         WHERE is_active = 1
         ORDER BY group_id, sort_order, id"
    );
    let option_rows = sqlx::query_as::<_, (String, String, String, i64, i64)>(&option_sql)
        .fetch_all(db)
        .await
        .map_err(|e| ServerFnError::new(format!("load options: {e}")))?;

    let mut options_by_group: HashMap<String, Vec<OptionItem>> = HashMap::new();
    for (gid, oid, label, price_cents, sort_order) in option_rows {
        options_by_group.entry(gid).or_default().push(OptionItem {
            id: oid,
            label,
            price_cents,
            sort_order,
        });
    }

    let mut groups_by_item: HashMap<String, Vec<OptionGroup>> = HashMap::new();
    for (mi_id, gid, glabel, min_sel, max_sel, sort_order) in group_rows {
        let options = options_by_group.get(&gid).cloned().unwrap_or_default();
        groups_by_item.entry(mi_id).or_default().push(OptionGroup {
            id: gid,
            label: glabel,
            min_select: min_sel,
            max_select: max_sel,
            sort_order,
            options,
        });
    }

    Ok(item_rows
        .into_iter()
        .map(|r| {
            let option_groups = groups_by_item.remove(&r.id).unwrap_or_default();
            MenuItem {
                id: r.id,
                category_id: r.category_id,
                menu_number: r.menu_number,
                name: r.name,
                description: r.description,
                item_type: r.item_type,
                price_small_cents: r.price_small_cents,
                price_large_cents: r.price_large_cents,
                size_small_label: r.size_small_label,
                size_large_label: r.size_large_label,
                allergen_codes: r.allergen_codes,
                additive_codes: r.additive_codes,
                is_spicy: r.is_spicy != 0,
                is_available: r.is_available != 0,
                is_listed: r.is_listed != 0,
                sort_order: r.sort_order,
                included_extras_count: r.included_extras_count,
                flat_extra_price_cents: r.flat_extra_price_cents,
                allow_extras: r.allow_extras != 0,
                option_groups,
            }
        })
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
        <Suspense fallback=|| view! { <p class="loading">{crate::t!("menu.loading")}</p> }>
            {move || combined.get().map(|(menu_res, extras)| match menu_res {
                Err(e) => view! {
                    <p class="error">{crate::t!("menu.load_error").replace("{err}", &e.to_string())}</p>
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
        shop_phone: phone,
        category_overlay,
    } = payload;

    // Provide the resolved extras catalog to every Card via context. The
    // value is captured at MenuView construction time so SSR and hydrate see
    // an identical immutable Vec.
    let extras_sv: StoredValue<Vec<PizzaExtra>> = StoredValue::new(extras);
    provide_context(extras_sv);

    let cats_for_tabs = categories.clone();
    let cats_for_sheet = categories.clone();
    let cats_for_sections = categories.clone();

    // Sheet open state — toggled by the floating "Kategorien" FAB.
    let sheet_open: RwSignal<bool> = RwSignal::new(false);

    // Hydrate-side: track which category section is closest to the
    // top of the viewport (just under the sticky nav) and mark the
    // matching pill `.active`. Auto-scroll that pill into the centre
    // of the strip so the customer always sees where they are.
    //
    // Replaces the v2 IntersectionObserver approach which had two
    // bugs: rootMargin doesn't accept rem units (silently rejected
    // → observer never fired), and the "no section intersecting"
    // case (between two sections) left no pill marked. A plain
    // "find the last section whose top ≤ probe-y" computation on
    // every scroll returns exactly one answer and always works.
    #[cfg(feature = "hydrate")]
    {
        let cat_ids: Vec<String> = categories.iter().map(|c| c.id.clone()).collect();
        Effect::new(move |_| {
            use leptos::wasm_bindgen::closure::Closure;
            use leptos::wasm_bindgen::JsCast;
            let Some(win) = web_sys::window() else { return };

            // The recompute fn: for each .category section, find
            // the one whose top is closest to (but not below) the
            // probe line (140px from the viewport top — under the
            // sticky header + nav). Mark it active; clear the rest.
            // Clone cat_ids per-tick so the outer Effect closure
            // stays FnMut (the Rc move would make it FnOnce).
            let ids = std::rc::Rc::new(cat_ids.clone());
            let ids_for_recompute = ids.clone();
            let recompute = move || {
                let Some(win) = web_sys::window() else { return };
                let Some(doc) = win.document() else { return };
                // Probe line: 140px from the top of the viewport,
                // just under the sticky chain (header ~40px + nav
                // ~50px + breathing). Tuned empirically.
                let probe: f64 = 140.0;
                let mut best: Option<(String, f64)> = None;
                for id in ids_for_recompute.iter() {
                    let sel = format!("#cat-{id}");
                    let Some(el) = doc.query_selector(&sel).ok().flatten() else {
                        continue;
                    };
                    let rect = el.get_bounding_client_rect();
                    let top = rect.top();
                    if top <= probe && best.as_ref().map(|(_, t)| top > *t).unwrap_or(true) {
                        best = Some((id.clone(), top));
                    }
                }
                // If nothing is above the probe (page just loaded,
                // user hasn't scrolled), pick the first category.
                let active_cat = best
                    .map(|(id, _)| id)
                    .or_else(|| ids_for_recompute.first().cloned());
                let Some(active_cat) = active_cat else { return };

                let Some(strip) = doc.query_selector(".category-tabs").ok().flatten() else {
                    return;
                };
                let Ok(pills) = strip.query_selector_all("a[data-cat]") else {
                    return;
                };
                for i in 0..pills.length() {
                    let Some(node) = pills.item(i) else { continue };
                    let Ok(el) = node.dyn_into::<web_sys::Element>() else {
                        continue;
                    };
                    let is_match = el
                        .get_attribute("data-cat")
                        .map(|v| v == active_cat)
                        .unwrap_or(false);
                    if is_match {
                        if !el.class_list().contains("active") {
                            el.class_list().add_1("active").ok();
                            // Only centre when it wasn't already
                            // active — avoids re-triggering smooth
                            // scroll on every scroll tick.
                            let opts = web_sys::ScrollIntoViewOptions::new();
                            opts.set_behavior(web_sys::ScrollBehavior::Smooth);
                            opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
                            opts.set_inline(web_sys::ScrollLogicalPosition::Center);
                            el.scroll_into_view_with_scroll_into_view_options(&opts);
                        }
                    } else {
                        el.class_list().remove_1("active").ok();
                    }
                }
            };

            // Initial mark on hydrate so the first pill is highlighted
            // before any scroll happens.
            recompute();

            // Throttled scroll handler via requestAnimationFrame —
            // recompute at most once per frame so very long scroll
            // sessions stay smooth on phones.
            let pending = std::rc::Rc::new(std::cell::Cell::new(false));
            let recompute_rc = std::rc::Rc::new(recompute);
            let pending_for_cb = pending.clone();
            let recompute_for_cb = recompute_rc.clone();
            let cb = Closure::<dyn Fn()>::new(move || {
                if pending_for_cb.get() {
                    return;
                }
                pending_for_cb.set(true);
                let pending_inner = pending_for_cb.clone();
                let recompute_inner = recompute_for_cb.clone();
                let raf_cb = Closure::once_into_js(move || {
                    pending_inner.set(false);
                    recompute_inner();
                });
                if let Some(win) = web_sys::window() {
                    let _ = win.request_animation_frame(raf_cb.as_ref().unchecked_ref());
                }
            });
            win.add_event_listener_with_callback("scroll", cb.as_ref().unchecked_ref())
                .ok();
            // Also recompute on resize because viewport-height
            // changes shift our probe line relative to section tops.
            win.add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref())
                .ok();
            cb.forget();
        });
    }

    view! {
        <div class="menu">
            <header class="menu-header">
                <h1>{crate::t!("menu.title")}</h1>
                {(!phone.is_empty()).then(|| view! {
                    <p>{crate::t!("menu.phone_hint").replace("{phone}", &phone)}</p>
                })}
                <p>
                    <a class="pdf-link" href="/menu.pdf" target="_blank" rel="noopener">
                        {format!("📄 {}", crate::t!("home.qr_pdf_title"))}
                    </a>
                </p>
            </header>

            // Sticky .category-nav: ‹ arrow, pill strip, › arrow,
            // and (when the per-shop overlay setting is on) a
            // bullet-list button that opens the category sheet.
            // Everything travels together while scrolling.
            <CategoryNav
                cats=cats_for_tabs
                show_list_button=category_overlay
                sheet_open
            />

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

            // Side / bottom sheet listing every category. Opened by
            // the bullet-list button in the sticky nav above. Same
            // overlay-click-closes pattern as the cart drawer.
            {category_overlay.then(|| view! {
                <CategorySheet cats=cats_for_sheet open=sheet_open/>
            })}
        </div>
    }
}

/// Sticky horizontal pill strip wrapped in a row of left/right scroll
/// arrows + an optional list-overlay button. The list button sits to
/// the right of the › arrow when the per-shop category-overlay setting
/// is on; otherwise it's hidden. All controls travel with the sticky
/// container so they're available everywhere the customer scrolls.
///
/// The arrow + auto-scroll logic depends on `web_sys` so it only fires
/// on hydrate; SSR renders the controls visible but inert.
#[component]
fn CategoryNav(
    cats: Vec<MenuCategory>,
    show_list_button: bool,
    sheet_open: RwSignal<bool>,
) -> impl IntoView {
    let scroll_strip = move |dir: i32| {
        #[cfg(feature = "hydrate")]
        {
            use leptos::wasm_bindgen::JsCast;
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                if let Some(el) = doc.query_selector(".category-tabs").ok().flatten() {
                    if let Ok(el) = el.dyn_into::<web_sys::Element>() {
                        let cur = el.scroll_left();
                        // Scroll ~70% of the visible strip per click — far
                        // enough to feel like a real jump, short enough to
                        // keep the previous pills as anchor context.
                        let step = (el.client_width() as f64 * 0.7) as i32;
                        el.set_scroll_left(cur + dir * step);
                    }
                }
            }
        }
        #[cfg(not(feature = "hydrate"))]
        {
            let _ = dir;
        }
    };
    view! {
        <div class="category-nav">
            <button class="cat-arrow left"
                aria-label=crate::t!("menu.scroll_left_aria")
                on:click=move |_| scroll_strip(-1)>
                "‹"
            </button>
            <nav class="category-tabs">
                {cats.into_iter().map(|c| {
                    let href = format!("#cat-{}", c.id);
                    let id = format!("tab-{}", c.id);
                    let data_cat = c.id.clone();
                    view! {
                        <a href=href id=id data-cat=data_cat>{c.name.clone()}</a>
                    }
                }).collect_view()}
            </nav>
            <button class="cat-arrow right"
                aria-label=crate::t!("menu.scroll_right_aria")
                on:click=move |_| scroll_strip(1)>
                "›"
            </button>
            // List-overlay button next to ›. SVG bullet-list icon
            // (three horizontal lines + dots) so the meaning is
            // visually unambiguous. Only renders when the per-shop
            // `menu_category_overlay` setting is on.
            {show_list_button.then(|| view! {
                <button class="cat-list-btn"
                    aria-label=crate::t!("menu.category_fab_aria")
                    on:click=move |_| sheet_open.set(true)>
                    <svg viewBox="0 0 16 16" width="20" height="20"
                         aria-hidden="true" focusable="false">
                        <circle cx="2.5" cy="3.5" r="1.3" fill="currentColor"/>
                        <circle cx="2.5" cy="8" r="1.3" fill="currentColor"/>
                        <circle cx="2.5" cy="12.5" r="1.3" fill="currentColor"/>
                        <rect x="5.5" y="2.7" width="9" height="1.6" rx="0.4" fill="currentColor"/>
                        <rect x="5.5" y="7.2" width="9" height="1.6" rx="0.4" fill="currentColor"/>
                        <rect x="5.5" y="11.7" width="9" height="1.6" rx="0.4" fill="currentColor"/>
                    </svg>
                </button>
            })}
        </div>
    }
}

/// Bottom-sheet (mobile) / side-sheet (desktop) listing every
/// category. One tap scrolls to the section and closes the sheet.
/// Uses the same overlay-click-to-close pattern as the cart drawer.
#[component]
fn CategorySheet(cats: Vec<MenuCategory>, open: RwSignal<bool>) -> impl IntoView {
    let close = move |_| open.set(false);
    // Use `class=move || ...` instead of `class:open` so SSR + hydrate
    // emit identical class strings and the reactive update always
    // re-runs (some `class:` directives skip the initial diff after
    // hydration if the SSR didn't pre-emit the class, which would
    // leave the sheet stuck closed).
    let bg_class = move || {
        if open.get() {
            "category-overlay-bg open"
        } else {
            "category-overlay-bg"
        }
    };
    let sheet_class = move || {
        if open.get() {
            "category-sheet open"
        } else {
            "category-sheet"
        }
    };
    view! {
        <div class=bg_class on:click=close></div>
        <aside class=sheet_class
            aria-hidden=move || (!open.get()).to_string()
            aria-label=crate::t!("menu.category_sheet_aria")>
            <header class="sheet-head">
                <h2>{crate::t!("menu.category_sheet_title")}</h2>
                <button class="close"
                    aria-label=crate::t!("common.close")
                    on:click=close>
                    "×"
                </button>
            </header>
            <nav class="sheet-list">
                {cats.into_iter().map(|c| {
                    let href = format!("#cat-{}", c.id);
                    view! {
                        <a href=href on:click=close>{c.name.clone()}</a>
                    }
                }).collect_view()}
            </nav>
        </aside>
    }
}

#[component]
fn Legend(allergens: Vec<LegendEntry>, additives: Vec<LegendEntry>) -> impl IntoView {
    view! {
        <section class="legend" id="legend">
            <h2>{crate::t!("menu.legend_title")}</h2>
            <p class="legend-intro">
                "Gemäß EU-Verordnung 1169/2011. Die Buchstaben hinter den Speisen kennzeichnen \
                 Allergene, die Zahlen Zusatzstoffe."
            </p>

            <div class="legend-grid">
                <div>
                    <h3>{crate::t!("menu.allergens_heading")}</h3>
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
                    <h3>{crate::t!("menu.additives_heading")}</h3>
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
    let number = it
        .menu_number
        .clone()
        .map(|n| crate::t!("menu.menu_number_prefix").replace("{n}", &n));
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

    // Whether the extras (Tabasco, extra cheese, …) checkboxes are
    // shown for this item. Driven by the per-item `allow_extras`
    // column; the legacy "pizza | calzone" heuristic was wrong for
    // pasta items that DO allow extras and salads that don't.
    let allows_extras = it.allow_extras;
    let selected_extras: RwSignal<Vec<String>> = RwSignal::new(Vec::new());
    let extras_catalog: StoredValue<Vec<PizzaExtra>> =
        use_context().unwrap_or_else(|| StoredValue::new(Vec::new()));

    // Pre-fill extras when the URL hash names this item, e.g.
    // /menu#mi-007?extras=ex-pilze,ex-zwiebeln. Used by the cart
    // drawer's "Bearbeiten" button: remove the line, navigate here
    // with the prior extras encoded, the customer adjusts and re-adds.
    // Hydrate-only — SSR has no Location.
    #[cfg(feature = "hydrate")]
    {
        let item_id_clone = item_id.clone();
        Effect::new(move |_| {
            if let Some(win) = web_sys::window() {
                if let Ok(hash) = win.location().hash() {
                    // hash looks like "#mi-007?extras=a,b" (or just "#mi-007")
                    let stripped = hash.trim_start_matches('#');
                    let (anchor, query) = match stripped.split_once('?') {
                        Some((a, q)) => (a, Some(q)),
                        None => (stripped, None),
                    };
                    if anchor == item_id_clone {
                        if let Some(q) = query {
                            // Parse "extras=a,b,c"
                            if let Some(rest) = q.strip_prefix("extras=") {
                                let ids: Vec<String> = rest
                                    .split(',')
                                    .filter(|s| !s.is_empty())
                                    .map(|s| s.to_string())
                                    .collect();
                                if !ids.is_empty() {
                                    selected_extras.set(ids);
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    // Required-choice option groups (e.g. salad dressing). Tracked as
    // a parallel signal `(group_id → Vec<option_id>)` so a single
    // dressing pick is a 1-element Vec, and "pick up to 2 sides"
    // groups also work.
    let option_groups: StoredValue<Vec<rusterando_shared::models::OptionGroup>> =
        StoredValue::new(it.option_groups.clone());
    let selected_options: RwSignal<std::collections::HashMap<String, Vec<String>>> =
        RwSignal::new(std::collections::HashMap::new());

    // Flatten all picked option_ids for the AddToCart dispatch.
    let collect_option_ids = move || -> Vec<String> {
        selected_options
            .get_untracked()
            .into_values()
            .flatten()
            .collect()
    };

    // True iff every group with `min_select > 0` has at least
    // min_select picks AND no group exceeds max_select. Used to gate
    // the size buttons.
    let options_valid = Memo::new(move |_| -> bool {
        let picks = selected_options.get();
        option_groups.with_value(|groups| {
            groups.iter().all(|g| {
                let n = picks.get(&g.id).map(|v| v.len() as i64).unwrap_or(0);
                n >= g.min_select && n <= g.max_select
            })
        })
    });
    // Sum of picked option prices, for the live total price on the
    // add buttons. v1 dressings are all 0 €, but the wiring is here
    // for premium-options later.
    let options_sum_cents = Memo::new(move |_| -> i64 {
        selected_options.with(|m| {
            let ids: std::collections::HashSet<&str> =
                m.values().flatten().map(String::as_str).collect();
            if ids.is_empty() {
                return 0;
            }
            option_groups.with_value(|gs| {
                gs.iter()
                    .flat_map(|g| g.options.iter())
                    .filter(|o| ids.contains(o.id.as_str()))
                    .map(|o| o.price_cents)
                    .sum()
            })
        })
    });

    // Per-item flat-pricing rule (mirrors `add_to_cart` in cart.rs):
    //   * `included` extras are free.
    //   * Beyond that, every additional extra costs `flat_override`
    //     when set; otherwise we fall back to the catalog price.
    let included = it.included_extras_count.max(0) as usize;
    let flat_override = it.flat_extra_price_cents;

    // Live sum of ticked extras so the size buttons show the real price the
    // customer is about to pay (base + extras) instead of just the base.
    // Honours selection order so the "first N free" rule matches the server.
    let extras_sum_cents = Memo::new(move |_| -> i64 {
        let picked = selected_extras.get();
        if picked.is_empty() {
            return 0;
        }
        extras_catalog.with_value(|cat| {
            picked
                .iter()
                .enumerate()
                .map(|(idx, id)| {
                    if idx < included {
                        0
                    } else if let Some(flat) = flat_override {
                        flat
                    } else {
                        cat.iter()
                            .find(|e| &e.id == id)
                            .map(|e| e.price_cents)
                            .unwrap_or(0)
                    }
                })
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
                selected_option_ids: collect_option_ids(),
            });
            selected_extras.set(Vec::new());
            selected_options.set(std::collections::HashMap::new());
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
                selected_option_ids: collect_option_ids(),
            });
            selected_extras.set(Vec::new());
            selected_options.set(std::collections::HashMap::new());
            ctx.open.set(true);
        }
    };

    let card_id = item_id.clone();
    view! {
        <article id=card_id class:card=true class:unavailable=unavailable>
            <div class="card-head">
                {number.map(|n| view! { <span class="num">{n}</span> })}
                <h3>{it.name.clone()}</h3>
                {spicy.then(|| view! { <span class="spicy" title="scharf">"🌶"</span> })}
            </div>
            {it.description.clone().map(|d| view! { <p class="desc">{d}</p> })}
            <div class="codes">
                {allergens.map(|s| view! {
                    <span class="codeset allergens">
                        <span class="label">{crate::t!("menu.allergens_label")}</span>
                        {split_codes(&s).into_iter().map(|c| {
                            let href = format!("#allergen-{c}");
                            view! { <a class="code" href=href>{c}</a> }
                        }).collect_view()}
                    </span>
                })}
                {additives.map(|s| view! {
                    <span class="codeset additives">
                        <span class="label">{crate::t!("menu.additives_label")}</span>
                        {split_codes(&s).into_iter().map(|c| {
                            let href = format!("#additive-{}", code_to_anchor(&c));
                            view! { <a class="code" href=href>{c}</a> }
                        }).collect_view()}
                    </span>
                })}
            </div>
            <OptionGroupsPicker
                groups=option_groups
                selected=selected_options
            />
            {allows_extras.then(|| view! {
                <ExtrasPicker
                    selected=selected_extras
                    catalog=extras_catalog
                    included=included
                    flat_override=flat_override
                />
            })}
            <div class="add-row">
                {if has_large {
                    view! {
                        <button class="add"
                            disabled=move || unavailable || !options_valid.get()
                            on:click=add_small>
                            <span class="add-size">{small_label.clone()}</span>
                            <span class="add-price">
                                {move || format_eur(small_price + extras_sum_cents.get() + options_sum_cents.get())}
                            </span>
                        </button>
                        <button class="add"
                            disabled=move || unavailable || !options_valid.get()
                            on:click=add_large>
                            <span class="add-size">{large_label.clone()}</span>
                            <span class="add-price">
                                {move || format_eur(large_price + extras_sum_cents.get() + options_sum_cents.get())}
                            </span>
                        </button>
                    }.into_any()
                } else {
                    view! {
                        <button class="add wide"
                            disabled=move || unavailable || !options_valid.get()
                            on:click=add_small>
                            <span class="add-size">{crate::t!("menu.add_to_size")}</span>
                            <span class="add-price">
                                {move || format_eur(small_price + extras_sum_cents.get() + options_sum_cents.get())}
                            </span>
                        </button>
                    }.into_any()
                }}
            </div>
            {unavailable.then(|| view! { <p class="oos">{crate::t!("menu.unavailable")}</p> })}
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
    /// First `included` ticks are free for this item.
    included: usize,
    /// When `Some`, every tick beyond `included` costs this flat rate
    /// (catalog prices are ignored). `None` keeps catalog pricing.
    flat_override: Option<i64>,
) -> impl IntoView {
    // Render statically (no enclosing `move ||`) so SSR and hydrate produce
    // an identical DOM tree. The catalog is captured by value once per
    // MenuView mount; toggling is handled per-checkbox via reactive
    // `is_checked` memos.
    let list = catalog.with_value(|v| v.clone());
    if list.is_empty() {
        return ().into_any();
    }
    let hint = match (included, flat_override) {
        (0, None) => None,
        (0, Some(flat)) => {
            Some(crate::t!("menu.extras_flat_each").replace("{amount}", &format_eur(flat)))
        }
        (n, None) => Some(crate::t!("menu.extras_first_n_free").replace("{n}", &n.to_string())),
        (n, Some(flat)) => Some(
            crate::t!("menu.extras_first_n_then_flat")
                .replace("{n}", &n.to_string())
                .replace("{amount}", &format_eur(flat)),
        ),
    };
    view! {
        <details class="extras-picker">
            <summary>{crate::t!("menu.expand_extras")}</summary>
            {hint.map(|h| view! { <p class="extras-hint">{h}</p> })}
            <ul class="extras-list">
                {list.into_iter().map(|ex| {
                    let id = ex.id.clone();
                    let id_for_check = id.clone();
                    let id_for_track = id.clone();
                    let id_for_price = id.clone();
                    let is_checked = Memo::new(move |_| {
                        selected.with(|v| v.contains(&id_for_track))
                    });
                    let label = ex.label.clone();
                    let catalog_price = ex.price_cents;
                    // Reactive per-row price label: depends on whether this
                    // extra is selected and at what position in the
                    // selection list. Mirrors the server-side rule in
                    // `add_to_cart`.
                    let price_label = move || {
                        let pos = selected.with(|v| {
                            v.iter().position(|x| x == &id_for_price)
                        });
                        let effective = match pos {
                            Some(idx) if idx < included => 0,
                            Some(_) => flat_override.unwrap_or(catalog_price),
                            None => {
                                // Not selected yet — show what it WOULD
                                // cost as the next tick.
                                let already = selected.with(|v| v.len());
                                if already < included {
                                    0
                                } else {
                                    flat_override.unwrap_or(catalog_price)
                                }
                            }
                        };
                        if effective == 0 {
                            "gratis".to_string()
                        } else {
                            format!("+{}", format_eur(effective))
                        }
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

/// Required-choice option groups (Dressing, Beilage, …). Renders one
/// fieldset per group. max_select==1 → radio buttons; otherwise
/// checkboxes capped at max_select picks.
#[component]
fn OptionGroupsPicker(
    groups: StoredValue<Vec<rusterando_shared::models::OptionGroup>>,
    selected: RwSignal<std::collections::HashMap<String, Vec<String>>>,
) -> impl IntoView {
    let list = groups.with_value(|v| v.clone());
    if list.is_empty() {
        return ().into_any();
    }
    view! {
        <div class="option-groups">
            {list.into_iter().map(|g| {
                let group_id = g.id.clone();
                let label = g.label.clone();
                let required = g.min_select > 0;
                let single = g.max_select == 1;
                let max_select = g.max_select;
                let min_select = g.min_select;
                view! {
                    <fieldset class="option-group">
                        <legend>
                            {label}
                            {required.then(|| view! { <span class="req">" *"</span> })}
                        </legend>
                        <ul class="option-list">
                            {g.options.into_iter().map(|o| {
                                let opt_id = o.id.clone();
                                let opt_label = o.label.clone();
                                let opt_price = o.price_cents;
                                let gid_check = group_id.clone();
                                let oid_check = opt_id.clone();
                                let gid_change = group_id.clone();
                                let oid_change = opt_id.clone();
                                let is_picked = Memo::new(move |_| {
                                    selected.with(|m| {
                                        m.get(&gid_check).map(|v| v.contains(&oid_check)).unwrap_or(false)
                                    })
                                });
                                let price_label = if opt_price == 0 {
                                    "gratis".to_string()
                                } else {
                                    format!("+{}", format_eur(opt_price))
                                };
                                let input_type = if single { "radio" } else { "checkbox" };
                                view! {
                                    <li>
                                        <label class="option-row">
                                            <input
                                                type=input_type
                                                name=group_id.clone()
                                                prop:checked=move || is_picked.get()
                                                on:change=move |ev| {
                                                    let on = event_target_checked(&ev);
                                                    selected.update(|m| {
                                                        let entry = m.entry(gid_change.clone()).or_default();
                                                        if single {
                                                            // Radio: clear group + set this one if on.
                                                            entry.clear();
                                                            if on {
                                                                entry.push(oid_change.clone());
                                                            }
                                                        } else {
                                                            entry.retain(|x| x != &oid_change);
                                                            if on && (entry.len() as i64) < max_select {
                                                                entry.push(oid_change.clone());
                                                            }
                                                        }
                                                    });
                                                }/>
                                            <span class="option-label">{opt_label}</span>
                                            <span class="option-price">{price_label}</span>
                                        </label>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                        {(!single && min_select > 0).then(|| view! {
                            <p class="option-hint">
                                {crate::t!("menu.options_choose_n")
                                    .replace("{min}", &min_select.to_string())
                                    .replace("{max}", &max_select.to_string())}
                            </p>
                        })}
                    </fieldset>
                }
            }).collect_view()}
        </div>
    }
    .into_any()
}
