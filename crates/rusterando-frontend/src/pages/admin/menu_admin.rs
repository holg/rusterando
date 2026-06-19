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
    allow_extras: bool,
    /// Replace the option-group attachments for this item. Empty
    /// vec = no required-choice groups attached (item has no
    /// dressings / sides / etc. picker).
    #[server(default)]
    option_group_ids: Vec<String>,
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

    // One transaction: item update + option-group attachment swap.
    let mut tx = db
        .begin()
        .await
        .map_err(|e| ServerFnError::new(format!("begin tx: {e}")))?;
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
             allow_extras = ?14,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?15",
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
    .bind(if allow_extras { 1_i64 } else { 0_i64 })
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("update item: {e}")))?;

    // Replace-all on attachments. With a few groups per item this is
    // cheap and avoids set-diff logic. Skip rows where group_id is
    // empty (defensive — caller shouldn't send those anyway).
    sqlx::query("DELETE FROM menu_item_option_groups WHERE menu_item_id = ?1")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("clear attachments: {e}")))?;
    for (idx, gid) in option_group_ids
        .iter()
        .filter(|s| !s.trim().is_empty())
        .enumerate()
    {
        sqlx::query(
            "INSERT INTO menu_item_option_groups (menu_item_id, group_id, sort_order)
             VALUES (?1, ?2, ?3)",
        )
        .bind(&id)
        .bind(gid)
        .bind(idx as i64 * 10)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("attach group: {e}")))?;
    }
    tx.commit()
        .await
        .map_err(|e| ServerFnError::new(format!("commit: {e}")))?;

    // Menu changed → refresh the cached Restaurant JSON-LD (prices/names).
    crate::pages::seo::rebuild_jsonld_cache().await;
    crate::pages::push::rebuild_menu_pdf_cache().await;
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

    // New item → refresh the cached Restaurant JSON-LD.
    crate::pages::seo::rebuild_jsonld_cache().await;
    crate::pages::push::rebuild_menu_pdf_cache().await;
    Ok(id)
}

// ---------------------------------------------------------------------------
// Category CRUD — rename / reorder / activate-toggle / create / delete.
//
// `menu_categories` rows are the section headers on /menu + /admin/menu.
// Renaming "Pute" → "Hähnchen" is a single UPDATE of `name`; items keep
// their `category_id` FK untouched. We also recompute the SEO `slug` so
// /menu/<slug> follows the new name (with a -N collision suffix), mirroring
// the boot-time backfill in main.rs::backfill_category_slugs.
// ---------------------------------------------------------------------------

/// Build a unique slug for `name`, avoiding any slug already used by a
/// DIFFERENT category. Mirrors the boot backfill's collision handling so
/// admin renames and the backfill never diverge.
#[cfg(feature = "ssr")]
async fn unique_category_slug(
    db: &sqlx::SqlitePool,
    name: &str,
    exclude_id: &str,
) -> Result<String, ServerFnError> {
    use rusterando_shared::models::seo_slug;
    let base = seo_slug(name);
    let base = if base.is_empty() {
        "kategorie".to_string()
    } else {
        base
    };
    let taken: std::collections::HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT slug FROM menu_categories
         WHERE slug IS NOT NULL AND slug <> '' AND id <> ?1",
    )
    .bind(exclude_id)
    .fetch_all(db)
    .await
    .map_err(|e| ServerFnError::new(format!("load slugs: {e}")))?
    .into_iter()
    .collect();
    if !taken.contains(&base) {
        return Ok(base);
    }
    let mut n = 2;
    loop {
        let cand = format!("{base}-{n}");
        if !taken.contains(&cand) {
            return Ok(cand);
        }
        n += 1;
    }
}

/// Rename a category + optionally change its sort_order / active flag.
/// The rename recomputes the SEO slug. Items are untouched (they FK to
/// the category id, not the name).
#[server(name = UpdateCategory, prefix = "/api", endpoint = "update_category")]
pub async fn update_category(
    id: String,
    name: String,
    sort_order: i64,
    is_active: bool,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(ServerFnError::new("Name darf nicht leer sein"));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let slug = unique_category_slug(&db, &name, &id).await?;

    let res = sqlx::query(
        "UPDATE menu_categories
         SET name = ?1, sort_order = ?2, is_active = ?3, slug = ?4
         WHERE id = ?5",
    )
    .bind(&name)
    .bind(sort_order)
    .bind(if is_active { 1_i64 } else { 0_i64 })
    .bind(&slug)
    .bind(&id)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("update category: {e}")))?;

    if res.rows_affected() == 0 {
        return Err(ServerFnError::new("Kategorie nicht gefunden"));
    }

    // Category name/slug feeds the public menu, the sitemap, and the
    // JSON-LD — refresh the cache so the change is visible immediately.
    crate::pages::seo::rebuild_jsonld_cache().await;
    crate::pages::push::rebuild_menu_pdf_cache().await;
    Ok(())
}

/// Create a new (empty) category. Lands at the end of the list
/// (max sort_order + 10) so it doesn't reshuffle existing sections.
#[server(name = CreateCategory, prefix = "/api", endpoint = "create_category")]
pub async fn create_category(name: String) -> Result<String, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(ServerFnError::new("Name darf nicht leer sein"));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let next_sort: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(sort_order), 0) + 10 FROM menu_categories")
            .fetch_one(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("next sort: {e}")))?;

    let id = uuid::Uuid::new_v4().to_string();
    let slug = unique_category_slug(&db, &name, &id).await?;

    sqlx::query(
        "INSERT INTO menu_categories (id, name, sort_order, is_active, slug)
         VALUES (?1, ?2, ?3, 1, ?4)",
    )
    .bind(&id)
    .bind(&name)
    .bind(next_sort)
    .bind(&slug)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("create category: {e}")))?;

    crate::pages::seo::rebuild_jsonld_cache().await;
    crate::pages::push::rebuild_menu_pdf_cache().await;
    Ok(id)
}

/// Delete a category. Refused if it still holds any items — the admin
/// must move or delete those first, so an FK orphan can never happen.
#[server(name = DeleteCategory, prefix = "/api", endpoint = "delete_category")]
pub async fn delete_category(id: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let item_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM menu_items WHERE category_id = ?1")
            .bind(&id)
            .fetch_one(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("count items: {e}")))?;
    if item_count > 0 {
        return Err(ServerFnError::new(format!(
            "Kategorie enthält noch {item_count} Artikel — bitte erst verschieben oder löschen."
        )));
    }

    sqlx::query("DELETE FROM menu_categories WHERE id = ?1")
        .bind(&id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("delete category: {e}")))?;

    crate::pages::seo::rebuild_jsonld_cache().await;
    crate::pages::push::rebuild_menu_pdf_cache().await;
    Ok(())
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
    // Category CRUD actions. The menu Resource re-reads whenever any of
    // these fire so a rename / new category / delete shows immediately.
    let cat_saver = ServerAction::<UpdateCategory>::new();
    let cat_creator = ServerAction::<CreateCategory>::new();
    let cat_deleter = ServerAction::<DeleteCategory>::new();

    // `new_blocking`: render the menu table during SSR (against the correct
    // per-tenant pool) and serialise it to the client, so it survives a
    // fragile admin hydration instead of going empty on a client refetch.
    // Edits still refetch (the action versions change). See extras_admin.rs.
    let menu = Resource::new_blocking(
        move || {
            (
                saver.version().get(),
                creator.version().get(),
                cat_saver.version().get(),
                cat_creator.version().get(),
                cat_deleter.version().get(),
            )
        },
        |_| async move { list_admin_menu().await },
    );
    // All option-groups for the per-item multi-select. Stable enough
    // that we refetch only on save (so a new group created via the
    // Auswahl-Gruppen tab on /admin/extras shows up after the next
    // per-item save).
    let groups = Resource::new_blocking(
        move || saver.version().get(),
        |_| async move { crate::pages::admin::options_admin::list_option_groups_admin().await },
    );

    view! {
        <AdminShell>
            <section class="admin-menu">
                <div class="page-bar">
                    <h1>"Speisekarte verwalten"</h1>
                    <a href="/menu" class="link">"→ öffentliche Ansicht"</a>
                </div>

                <Suspense fallback=|| view! { <p>"Lädt…"</p> }>
                    {move || {
                        let m = menu.get();
                        let g = groups.get();
                        match (m, g) {
                            (Some(Err(e)), _) => view! {
                                <p class="error">{format!("Fehler: {e}")}</p>
                            }.into_any(),
                            (_, Some(Err(e))) => view! {
                                <p class="error">{format!("Auswahl-Gruppen: {e}")}</p>
                            }.into_any(),
                            (Some(Ok(payload)), Some(Ok(group_list))) => view! {
                                <Editor payload group_list saver creator
                                    cat_saver cat_creator cat_deleter/>
                            }.into_any(),
                            _ => view! { <p>"Lädt…"</p> }.into_any(),
                        }
                    }}
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
                {move || match cat_saver.value().get() {
                    Some(Ok(()))  => Some(view! { <p class="toast ok">"Kategorie gespeichert."</p> }.into_any()),
                    Some(Err(e)) => Some(view! { <p class="toast error">{format!("Kategorie: {e}")}</p> }.into_any()),
                    None => None,
                }}
                {move || match cat_creator.value().get() {
                    Some(Ok(_))  => Some(view! { <p class="toast ok">"Neue Kategorie angelegt."</p> }.into_any()),
                    Some(Err(e)) => Some(view! { <p class="toast error">{format!("Kategorie anlegen: {e}")}</p> }.into_any()),
                    None => None,
                }}
                {move || match cat_deleter.value().get() {
                    Some(Ok(()))  => Some(view! { <p class="toast ok">"Kategorie gelöscht."</p> }.into_any()),
                    Some(Err(e)) => Some(view! { <p class="toast error">{format!("Kategorie löschen: {e}")}</p> }.into_any()),
                    None => None,
                }}
            </section>
        </AdminShell>
    }
}

#[component]
fn Editor(
    payload: MenuPayload,
    /// All option-groups in the system, used by per-item multi-select.
    group_list: Vec<crate::pages::admin::options_admin::OptionGroupAdminRow>,
    saver: ServerAction<UpdateMenuItem>,
    creator: ServerAction<CreateMenuItem>,
    cat_saver: ServerAction<UpdateCategory>,
    cat_creator: ServerAction<CreateCategory>,
    cat_deleter: ServerAction<DeleteCategory>,
) -> impl IntoView {
    let MenuPayload {
        categories,
        items,
        allergens,
        additives,
        shop_phone: _,
        category_overlay: _,
    } = payload;
    let cats_for_select: Vec<MenuCategory> = categories.clone();
    let cats_for_nav: Vec<MenuCategory> = categories.clone();

    // Hydrate-side: track which category section is closest to the
    // viewport top and mark the matching pill .active. Mirrors the
    // public /menu scroll-listener (menu.rs:387). When you click a
    // pill, the anchor jump triggers a scroll which then triggers
    // recompute → the pill highlights itself; no extra wiring needed.
    #[cfg(feature = "hydrate")]
    {
        let cat_ids: Vec<String> = categories.iter().map(|c| c.id.clone()).collect();
        Effect::new(move |_| {
            use leptos::wasm_bindgen::closure::Closure;
            use leptos::wasm_bindgen::JsCast;
            let Some(win) = web_sys::window() else { return };
            let ids = std::rc::Rc::new(cat_ids.clone());
            let ids_for_recompute = ids.clone();
            let recompute = move || {
                let Some(win) = web_sys::window() else { return };
                let Some(doc) = win.document() else { return };
                // Probe line below the sticky admin shell bar + admin
                // page-bar + admin-menu-nav (~150px on desktop).
                let probe: f64 = 160.0;
                let mut best: Option<(String, f64)> = None;
                for id in ids_for_recompute.iter() {
                    let sel = format!("#admin-cat-{id}");
                    let Some(el) = doc.query_selector(&sel).ok().flatten() else {
                        continue;
                    };
                    let rect = el.get_bounding_client_rect();
                    let top = rect.top();
                    if top <= probe && best.as_ref().map(|(_, t)| top > *t).unwrap_or(true) {
                        best = Some((id.clone(), top));
                    }
                }
                let active_cat = best
                    .map(|(id, _)| id)
                    .or_else(|| ids_for_recompute.first().cloned());
                let Some(active_cat) = active_cat else { return };
                let Some(strip) = doc
                    .query_selector(".admin-menu-nav .nav-pills")
                    .ok()
                    .flatten()
                else {
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
            recompute();
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
            win.add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref())
                .ok();
            cb.forget();
        });
    }

    // Bulk-open / bulk-close handlers. Walk every <details.cat-block>
    // and flip its `open` attribute. Pills that ALSO need to open
    // their target use this same machinery through the anchor href.
    let on_open_all = move |_| {
        #[cfg(feature = "hydrate")]
        {
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                if let Ok(nodes) = doc.query_selector_all("details.cat-block") {
                    for i in 0..nodes.length() {
                        if let Some(node) = nodes.item(i) {
                            use leptos::wasm_bindgen::JsCast;
                            if let Ok(el) = node.dyn_into::<web_sys::Element>() {
                                el.set_attribute("open", "").ok();
                            }
                        }
                    }
                }
            }
        }
    };
    let on_close_all = move |_| {
        #[cfg(feature = "hydrate")]
        {
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                if let Ok(nodes) = doc.query_selector_all("details.cat-block") {
                    for i in 0..nodes.length() {
                        if let Some(node) = nodes.item(i) {
                            use leptos::wasm_bindgen::JsCast;
                            if let Ok(el) = node.dyn_into::<web_sys::Element>() {
                                el.remove_attribute("open").ok();
                            }
                        }
                    }
                }
            }
        }
    };

    // Pill click: open the target <details> before the anchor jump
    // scrolls the page so we land on an expanded section, not a
    // collapsed header.
    #[cfg(feature = "hydrate")]
    let on_pill_click = move |ev: leptos::ev::MouseEvent| {
        use leptos::wasm_bindgen::JsCast;
        let Some(target) = ev.target() else { return };
        let Ok(anchor) = target.dyn_into::<web_sys::Element>() else {
            return;
        };
        let Some(cat_id) = anchor.get_attribute("data-cat") else {
            return;
        };
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let sel = format!("#admin-cat-{cat_id}");
        if let Some(node) = doc.query_selector(&sel).ok().flatten() {
            node.set_attribute("open", "").ok();
        }
    };
    #[cfg(not(feature = "hydrate"))]
    let on_pill_click = move |_: leptos::ev::MouseEvent| {};

    view! {
        <div class="admin-menu-editor">
            <nav class="admin-menu-nav">
                <div class="nav-actions">
                    <button class="btn ghost" type="button" on:click=on_open_all>
                        "▾ Alle ausklappen"
                    </button>
                    <button class="btn ghost" type="button" on:click=on_close_all>
                        "▴ Alle einklappen"
                    </button>
                </div>
                <div class="nav-pills">
                    {cats_for_nav.into_iter().map(|c| {
                        let href = format!("#admin-cat-{}", c.id);
                        let cat_id_attr = c.id.clone();
                        view! {
                            <a class="pill" href=href data-cat=cat_id_attr on:click=on_pill_click>
                                {c.name}
                            </a>
                        }
                    }).collect_view()}
                </div>
            </nav>

            {categories.into_iter().map(|cat| {
                let cat_items = items.iter()
                    .filter(|it| it.category_id == cat.id)
                    .cloned()
                    .collect::<Vec<_>>();
                let allergens = allergens.clone();
                let additives = additives.clone();
                let cats_for_select = cats_for_select.clone();
                let group_list = group_list.clone();
                view! { <CategoryBlock cat cat_items allergens additives cats_for_select group_list saver creator cat_saver cat_deleter/> }
            }).collect_view()}

            <NewCategoryRow cat_creator/>
        </div>
    }
}

/// "+ neue Kategorie" — create an empty category. It lands at the end of
/// the list; the admin then adds items to it via each item's category
/// <select>, or via the per-category "+ neuer Artikel" row.
#[component]
fn NewCategoryRow(cat_creator: ServerAction<CreateCategory>) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let on_create = move |_| {
        let n = name.get().trim().to_string();
        if n.is_empty() {
            return;
        }
        cat_creator.dispatch(CreateCategory { name: n });
        name.set(String::new());
    };
    view! {
        <div class="new-category-row">
            <input
                type="text"
                placeholder="Neue Kategorie (z. B. Hähnchen)"
                prop:value=move || name.get()
                on:input=move |ev| name.set(event_target_value(&ev))/>
            <button class="btn primary" type="button" on:click=on_create>
                "+ Kategorie anlegen"
            </button>
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
    group_list: Vec<crate::pages::admin::options_admin::OptionGroupAdminRow>,
    saver: ServerAction<UpdateMenuItem>,
    creator: ServerAction<CreateMenuItem>,
    cat_saver: ServerAction<UpdateCategory>,
    cat_deleter: ServerAction<DeleteCategory>,
) -> impl IntoView {
    let count = cat_items.len();
    let cat_id = cat.id.clone();
    let cat_name = cat.name.clone();
    let cat_sort = cat.sort_order;
    // Anchor target for the sticky-nav pills. <details> starts
    // collapsed; on pill click we set the `open` attribute imperatively
    // (see Editor::on_pill_click) so the anchor scroll lands on an
    // expanded section.
    let details_id = format!("admin-cat-{cat_id}");

    // Inline category-edit state. Starts collapsed (just the name + count
    // in the summary); the ✏️ toggle reveals a rename/sort/delete row.
    let editing = RwSignal::new(false);
    let name_sig = RwSignal::new(cat_name.clone());
    let sort_sig = RwSignal::new(cat_sort);

    let on_save_cat = {
        let cat_id = cat_id.clone();
        move |_| {
            let n = name_sig.get().trim().to_string();
            if n.is_empty() {
                return;
            }
            cat_saver.dispatch(UpdateCategory {
                id: cat_id.clone(),
                name: n,
                sort_order: sort_sig.get(),
                is_active: true,
            });
            editing.set(false);
        }
    };
    let on_delete_cat = {
        let cat_id = cat_id.clone();
        move |_| {
            cat_deleter.dispatch(DeleteCategory { id: cat_id.clone() });
        }
    };
    let can_delete = count == 0;

    let details_id_for_toggle = details_id.clone();

    view! {
        <details class="cat-block" id=details_id>
            <summary>
                <strong>{move || name_sig.get()}</strong>
                <span class="muted">" (" {count} ")"</span>
                // The ✏️ button toggles the edit row. It's inside <summary>,
                // so we stop the click from also toggling the <details>
                // open/closed state. Because the edit row lives inside the
                // <details> body (which the browser hides while collapsed),
                // we imperatively force the <details> open when turning
                // editing ON — otherwise the revealed row would stay
                // invisible behind the collapsed section.
                <button class="btn ghost cat-edit-toggle" type="button"
                    on:click={
                        let details_id = details_id_for_toggle.clone();
                        move |ev| {
                            ev.prevent_default();
                            ev.stop_propagation();
                            let turning_on = !editing.get();
                            editing.set(turning_on);
                            #[cfg(feature = "hydrate")]
                            if turning_on {
                                if let Some(doc) =
                                    web_sys::window().and_then(|w| w.document())
                                {
                                    if let Some(el) =
                                        doc.get_element_by_id(&details_id)
                                    {
                                        el.set_attribute("open", "").ok();
                                    }
                                }
                            }
                            #[cfg(not(feature = "hydrate"))]
                            let _ = &details_id;
                        }
                    }>"✏️ Kategorie"</button>
            </summary>

            {move || editing.get().then(|| view! {
                <div class="cat-edit-row">
                    <label>
                        <span>"Name"</span>
                        <input type="text"
                            prop:value=move || name_sig.get()
                            on:input=move |ev| name_sig.set(event_target_value(&ev))/>
                    </label>
                    <label>
                        <span>"Reihenfolge"</span>
                        <input type="number" step="1"
                            prop:value=move || sort_sig.get().to_string()
                            on:input=move |ev| {
                                let v: i64 = event_target_value(&ev).parse().unwrap_or(0);
                                sort_sig.set(v);
                            }/>
                    </label>
                    <div class="cat-edit-actions">
                        <button class="btn primary" type="button" on:click=on_save_cat.clone()>
                            "Speichern"
                        </button>
                        {can_delete.then(|| view! {
                            <button class="btn ghost danger" type="button" on:click=on_delete_cat.clone()>
                                "Kategorie löschen"
                            </button>
                        })}
                        {(!can_delete).then(|| view! {
                            <span class="muted small">
                                "Zum Löschen erst alle Artikel verschieben/entfernen."
                            </span>
                        })}
                    </div>
                </div>
            })}

            <div class="cat-rows">
                {cat_items.into_iter().map(|it| {
                    let allergens = allergens.clone();
                    let additives = additives.clone();
                    let cats_for_select = cats_for_select.clone();
                    let group_list = group_list.clone();
                    view! { <ItemCard it allergens additives cats_for_select group_list saver/> }
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
    group_list: Vec<crate::pages::admin::options_admin::OptionGroupAdminRow>,
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
    let allow_extras = RwSignal::new(it.allow_extras);
    // Currently-attached option-group ids. Loaded from the item's
    // own payload (menu loader already joins menu_item_option_groups).
    let attached_groups: RwSignal<std::collections::HashSet<String>> =
        RwSignal::new(it.option_groups.iter().map(|g| g.id.clone()).collect());

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
                allow_extras: allow_extras.get(),
                option_group_ids: attached_groups.get().into_iter().collect(),
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
                <label class="flag" title="Globale Extras-Liste (Tabasco, extra Käse …) für diesen Artikel zeigen.">
                    <input type="checkbox"
                        prop:checked=move || allow_extras.get()
                        on:change=move |ev| allow_extras.set(event_target_checked(&ev))/>
                    <span>"Extras erlaubt"</span>
                </label>
                <button class="btn ghost danger" type="button" on:click=toggle_listed>
                    {move || if is_listed.get() { "✕ Vom Menü entfernen" } else { "↺ Wieder ins Menü aufnehmen" }}
                </button>
                <button class="btn primary" type="button" on:click=on_save>"Speichern"</button>
            </div>
            // Option-group attachment row — one checkbox per available group.
            // Empty if no groups are configured yet.
            {if group_list.is_empty() {
                view! {
                    <p class="hint muted">
                        "Auswahl-Gruppen (z.B. Dressing) erst unter "
                        <a href="/admin/extras">"/admin/extras → Auswahl-Gruppen"</a>
                        " anlegen, dann erscheinen sie hier."
                    </p>
                }.into_any()
            } else {
                view! {
                    <div class="option-group-attach">
                        <span class="label">"Auswahl-Gruppen:"</span>
                        {group_list.into_iter().map(|g| {
                            let gid_check = g.id.clone();
                            let gid_change = g.id.clone();
                            let label = format!("{} ({}–{})", g.label, g.min_select, g.max_select);
                            let checked = Memo::new(move |_| {
                                attached_groups.with(|s| s.contains(&gid_check))
                            });
                            view! {
                                <label class="flag">
                                    <input type="checkbox"
                                        prop:checked=move || checked.get()
                                        on:change=move |ev| {
                                            let on = event_target_checked(&ev);
                                            attached_groups.update(|s| {
                                                if on {
                                                    s.insert(gid_change.clone());
                                                } else {
                                                    s.remove(&gid_change);
                                                }
                                            });
                                        }/>
                                    <span>{label}</span>
                                </label>
                            }
                        }).collect_view()}
                    </div>
                }.into_any()
            }}
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
