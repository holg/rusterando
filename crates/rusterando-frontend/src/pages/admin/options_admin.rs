//! /admin/options — CRUD for required-choice option groups.
//!
//! Each group is a set of mutually-exclusive (or capped multi-select)
//! picks the customer must make when ordering certain items: "Dressing"
//! on a salad, "Schärfegrad" on a dish, "Beilage" picks, etc.
//!
//! Groups and their options are admin-edited here once; per-item
//! assignment ("attach this dressing group to all 14 salad items")
//! is done in /admin/menu via the new option-groups multi-select.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptionGroupAdminRow {
    pub id: String,
    pub label: String,
    pub min_select: i64,
    pub max_select: i64,
    pub sort_order: i64,
    pub is_active: bool,
    /// Suppress the "gratis" label on 0 € options (variant pickers).
    #[serde(default)]
    pub hide_zero_price: bool,
    pub options: Vec<OptionAdminRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptionAdminRow {
    pub id: String,
    pub group_id: String,
    pub label: String,
    pub price_cents: i64,
    pub sort_order: i64,
    pub is_active: bool,
}

// ---------------------------------------------------------------------------
// Server fns
// ---------------------------------------------------------------------------

#[server(
    name = ListOptionGroupsAdmin,
    prefix = "/api",
    endpoint = "list_option_groups_admin"
)]
pub async fn list_option_groups_admin() -> Result<Vec<OptionGroupAdminRow>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let groups: Vec<(String, String, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, label, min_select, max_select, sort_order, is_active, hide_zero_price
         FROM item_option_groups
         ORDER BY sort_order, label",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load groups: {e}")))?;

    let options: Vec<(String, String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, group_id, label, price_cents, sort_order, is_active
         FROM item_options
         ORDER BY group_id, sort_order, label",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load options: {e}")))?;

    use std::collections::HashMap;
    let mut by_group: HashMap<String, Vec<OptionAdminRow>> = HashMap::new();
    for (id, gid, label, price_cents, sort_order, is_active) in options {
        by_group
            .entry(gid.clone())
            .or_default()
            .push(OptionAdminRow {
                id,
                group_id: gid,
                label,
                price_cents,
                sort_order,
                is_active: is_active != 0,
            });
    }

    Ok(groups
        .into_iter()
        .map(
            |(id, label, min_s, max_s, sort_order, is_active, hide_zero)| {
                let opts = by_group.remove(&id).unwrap_or_default();
                OptionGroupAdminRow {
                    id,
                    label,
                    min_select: min_s,
                    max_select: max_s,
                    sort_order,
                    is_active: is_active != 0,
                    hide_zero_price: hide_zero != 0,
                    options: opts,
                }
            },
        )
        .collect())
}

#[server(
    name = CreateOptionGroup,
    prefix = "/api",
    endpoint = "create_option_group"
)]
pub async fn create_option_group(
    label: String,
    min_select: i64,
    max_select: i64,
    #[server(default)] hide_zero_price: bool,
) -> Result<String, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    if label.trim().is_empty() {
        return Err(ServerFnError::new("Bezeichnung darf nicht leer sein."));
    }
    if min_select < 0 || max_select < 1 || min_select > max_select {
        return Err(ServerFnError::new(
            "Ungültige Grenzen: 0 ≤ min ≤ max, max ≥ 1.",
        ));
    }
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let id = format!("og-{}", uuid::Uuid::new_v4().simple());
    let next_sort: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(sort_order), 0) + 10 FROM item_option_groups")
            .fetch_one(&db)
            .await
            .unwrap_or(10);

    sqlx::query(
        "INSERT INTO item_option_groups (id, label, min_select, max_select, sort_order, hide_zero_price)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&id)
    .bind(label.trim())
    .bind(min_select)
    .bind(max_select)
    .bind(next_sort)
    .bind(if hide_zero_price { 1_i64 } else { 0_i64 })
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("insert group: {e}")))?;
    Ok(id)
}

#[server(
    name = UpdateOptionGroup,
    prefix = "/api",
    endpoint = "update_option_group"
)]
pub async fn update_option_group(
    id: String,
    label: String,
    min_select: i64,
    max_select: i64,
    sort_order: i64,
    is_active: bool,
    #[server(default)] hide_zero_price: bool,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    if label.trim().is_empty() {
        return Err(ServerFnError::new("Bezeichnung darf nicht leer sein."));
    }
    if min_select < 0 || max_select < 1 || min_select > max_select {
        return Err(ServerFnError::new(
            "Ungültige Grenzen: 0 ≤ min ≤ max, max ≥ 1.",
        ));
    }
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query(
        "UPDATE item_option_groups
         SET label = ?2, min_select = ?3, max_select = ?4, sort_order = ?5,
             is_active = ?6, hide_zero_price = ?7, updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
    )
    .bind(&id)
    .bind(label.trim())
    .bind(min_select)
    .bind(max_select)
    .bind(sort_order)
    .bind(if is_active { 1 } else { 0 })
    .bind(if hide_zero_price { 1_i64 } else { 0_i64 })
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("update group: {e}")))?;
    Ok(())
}

#[server(
    name = DeleteOptionGroup,
    prefix = "/api",
    endpoint = "delete_option_group"
)]
pub async fn delete_option_group(id: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    // ON DELETE CASCADE on item_options and menu_item_option_groups
    // means options + attachments go too.
    sqlx::query("DELETE FROM item_option_groups WHERE id = ?1")
        .bind(&id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("delete group: {e}")))?;
    Ok(())
}

#[server(
    name = CreateOption,
    prefix = "/api",
    endpoint = "create_option"
)]
pub async fn create_option(
    group_id: String,
    label: String,
    price_cents: i64,
) -> Result<String, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    if label.trim().is_empty() {
        return Err(ServerFnError::new("Bezeichnung darf nicht leer sein."));
    }
    if price_cents < 0 {
        return Err(ServerFnError::new("Preis darf nicht negativ sein."));
    }
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let id = format!("opt-{}", uuid::Uuid::new_v4().simple());
    let next_sort: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sort_order), 0) + 10 FROM item_options WHERE group_id = ?1",
    )
    .bind(&group_id)
    .fetch_one(&db)
    .await
    .unwrap_or(10);
    sqlx::query(
        "INSERT INTO item_options (id, group_id, label, price_cents, sort_order)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(&id)
    .bind(&group_id)
    .bind(label.trim())
    .bind(price_cents)
    .bind(next_sort)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("insert option: {e}")))?;
    Ok(id)
}

#[server(
    name = UpdateOption,
    prefix = "/api",
    endpoint = "update_option"
)]
pub async fn update_option(
    id: String,
    label: String,
    price_cents: i64,
    sort_order: i64,
    is_active: bool,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    if label.trim().is_empty() {
        return Err(ServerFnError::new("Bezeichnung darf nicht leer sein."));
    }
    if price_cents < 0 {
        return Err(ServerFnError::new("Preis darf nicht negativ sein."));
    }
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query(
        "UPDATE item_options
         SET label = ?2, price_cents = ?3, sort_order = ?4, is_active = ?5,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
    )
    .bind(&id)
    .bind(label.trim())
    .bind(price_cents)
    .bind(sort_order)
    .bind(if is_active { 1 } else { 0 })
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("update option: {e}")))?;
    Ok(())
}

#[server(
    name = DeleteOption,
    prefix = "/api",
    endpoint = "delete_option"
)]
pub async fn delete_option(id: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query("DELETE FROM item_options WHERE id = ?1")
        .bind(&id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("delete option: {e}")))?;
    Ok(())
}

// --- Bulk attach by category ------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CategoryRow {
    pub id: String,
    pub name: String,
}

#[server(
    name = ListCategoriesForAttach,
    prefix = "/api",
    endpoint = "list_categories_for_attach"
)]
pub async fn list_categories_for_attach() -> Result<Vec<CategoryRow>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, name FROM menu_categories
         WHERE is_active = 1 ORDER BY sort_order, name",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load categories: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|(id, name)| CategoryRow { id, name })
        .collect())
}

/// For each group, the list of categories it's currently attached to
/// (deduplicated; one entry per (group, category) pair regardless of
/// how many items in that category are linked). Used to render an
/// "Angewendet auf: …" line under each group card so admins can see
/// at a glance where a group is in play.
#[server(
    name = ListGroupAttachments,
    prefix = "/api",
    endpoint = "list_group_attachments"
)]
pub async fn list_group_attachments() -> Result<Vec<(String, String, i64)>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    // (group_id, category_name, item_count)
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT mg.group_id, c.name, COUNT(*)
         FROM menu_item_option_groups mg
         JOIN menu_items mi ON mi.id = mg.menu_item_id
         JOIN menu_categories c ON c.id = mi.category_id
         GROUP BY mg.group_id, c.id
         ORDER BY mg.group_id, c.sort_order, c.name",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("list attachments: {e}")))?;
    Ok(rows)
}

#[server(
    name = AttachGroupToCategory,
    prefix = "/api",
    endpoint = "attach_group_to_category"
)]
pub async fn attach_group_to_category(
    group_id: String,
    category_id: String,
) -> Result<i64, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    // INSERT OR IGNORE so re-running is idempotent. sort_order falls
    // after any existing attachments for that item.
    let res = sqlx::query(
        "INSERT OR IGNORE INTO menu_item_option_groups (menu_item_id, group_id, sort_order)
         SELECT mi.id, ?1,
                COALESCE((SELECT MAX(sort_order) FROM menu_item_option_groups
                          WHERE menu_item_id = mi.id), 0) + 10
         FROM menu_items mi
         WHERE mi.category_id = ?2 AND mi.is_listed = 1",
    )
    .bind(&group_id)
    .bind(&category_id)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("attach by category: {e}")))?;
    Ok(res.rows_affected() as i64)
}

#[server(
    name = DetachGroupFromCategory,
    prefix = "/api",
    endpoint = "detach_group_from_category"
)]
pub async fn detach_group_from_category(
    group_id: String,
    category_id: String,
) -> Result<i64, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let res = sqlx::query(
        "DELETE FROM menu_item_option_groups
         WHERE group_id = ?1
           AND menu_item_id IN (SELECT id FROM menu_items WHERE category_id = ?2)",
    )
    .bind(&group_id)
    .bind(&category_id)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("detach by category: {e}")))?;
    Ok(res.rows_affected() as i64)
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Body of the Auswahl-Gruppen page — no AdminShell wrapper so it can be
/// embedded as a tab inside ExtrasAdminPage.
#[component]
pub fn OptionsAdminBody() -> impl IntoView {
    let create_group = ServerAction::<CreateOptionGroup>::new();
    let update_group = ServerAction::<UpdateOptionGroup>::new();
    let delete_group = ServerAction::<DeleteOptionGroup>::new();
    let create_option = ServerAction::<CreateOption>::new();
    let update_option = ServerAction::<UpdateOption>::new();
    let delete_option = ServerAction::<DeleteOption>::new();
    let attach_cat = ServerAction::<AttachGroupToCategory>::new();
    let detach_cat = ServerAction::<DetachGroupFromCategory>::new();

    let groups = Resource::new(
        move || {
            (
                create_group.version().get(),
                update_group.version().get(),
                delete_group.version().get(),
                create_option.version().get(),
                update_option.version().get(),
                delete_option.version().get(),
                attach_cat.version().get(),
                detach_cat.version().get(),
            )
        },
        |_| async move { list_option_groups_admin().await },
    );
    let categories = Resource::new(|| (), |_| async move { list_categories_for_attach().await });
    // Attachments refresh on every attach/detach so the "Angewendet auf …"
    // line under each card stays in sync.
    let attachments = Resource::new(
        move || (attach_cat.version().get(), detach_cat.version().get()),
        |_| async move { list_group_attachments().await },
    );

    view! {
        <section class="options-admin">
            <p class="hint">
                "Eine "<b>"Gruppe"</b>" ist die Frage an den Kunden (z.B. \"Dressing\"). "
                "Die einzelnen "<b>"Optionen"</b>" (Italian, Joghurt, Caesar …) fügst du "
                "danach "<b>"in der Gruppe"</b>" hinzu — nicht als eigene Gruppen!"
            </p>
            <p class="hint muted">
                "Min/Max steuern wie viele Optionen der Kunde wählen muss: "
                "1/1 = genau eine (Radio-Buttons). 0/3 = bis zu 3, optional (Checkboxen)."
                " Einmal definiert, dann per Kategorie auf alle passenden Artikel anwenden "
                "(oder einzeln in "<a href="/admin/menu">"/admin/menu"</a>")."
            </p>

            <h2>"Neue Gruppe (= eine Frage an den Kunden)"</h2>
            <ActionForm action=create_group attr:class="options-create">
                <label>
                    <span>"Bezeichnung der Frage"</span>
                    <input type="text" name="label" required placeholder="z.B. Dressing wählen"/>
                </label>
                <label>
                    <span>"Min."</span>
                    <input type="number" name="min_select" min="0" value="1" required/>
                </label>
                <label>
                    <span>"Max."</span>
                    <input type="number" name="max_select" min="1" value="1" required/>
                </label>
                <button type="submit" class="btn primary">"Gruppe anlegen"</button>
            </ActionForm>
            {move || create_group.value().get().and_then(|res| match res {
                Err(e) => Some(view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any()),
                Ok(_) => None,
            })}

            <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                {move || {
                    let g = groups.get();
                    let c = categories.get();
                    let a = attachments.get();
                    match (g, c, a) {
                        (Some(Err(e)), _, _) | (_, Some(Err(e)), _) | (_, _, Some(Err(e))) => {
                            view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any()
                        }
                        (Some(Ok(rows)), Some(Ok(cats)), Some(Ok(att_rows))) => {
                            use std::collections::HashMap;
                            let mut by_group: HashMap<String, Vec<(String, i64)>> = HashMap::new();
                            for (gid, cname, n) in att_rows {
                                by_group.entry(gid).or_default().push((cname, n));
                            }
                            view! {
                                <GroupList
                                    rows
                                    cats
                                    attachments=by_group
                                    update_group
                                    delete_group
                                    create_option
                                    update_option
                                    delete_option
                                    attach_cat
                                    detach_cat
                                />
                            }.into_any()
                        }
                        _ => view! { <p class="loading">"Lädt…"</p> }.into_any(),
                    }
                }}
            </Suspense>
            {move || attach_cat.value().get().map(|res| match res {
                Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                Ok(n) => view! { <p class="ok">{format!("{n} Artikel verknüpft.")}</p> }.into_any(),
            })}
            {move || detach_cat.value().get().map(|res| match res {
                Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                Ok(n) => view! { <p class="ok">{format!("{n} Verknüpfung(en) entfernt.")}</p> }.into_any(),
            })}
        </section>
    }
}

#[component]
fn GroupList(
    rows: Vec<OptionGroupAdminRow>,
    cats: Vec<CategoryRow>,
    attachments: std::collections::HashMap<String, Vec<(String, i64)>>,
    update_group: ServerAction<UpdateOptionGroup>,
    delete_group: ServerAction<DeleteOptionGroup>,
    create_option: ServerAction<CreateOption>,
    update_option: ServerAction<UpdateOption>,
    delete_option: ServerAction<DeleteOption>,
    attach_cat: ServerAction<AttachGroupToCategory>,
    detach_cat: ServerAction<DetachGroupFromCategory>,
) -> impl IntoView {
    if rows.is_empty() {
        return view! { <p class="empty">"Noch keine Gruppen."</p> }.into_any();
    }
    view! {
        <div class="option-groups-admin">
            {rows.into_iter().map(|g| {
                let cats_for_card = cats.clone();
                let attached = attachments.get(&g.id).cloned().unwrap_or_default();
                view! {
                    <GroupCard
                        g
                        cats=cats_for_card
                        attached
                        update_group
                        delete_group
                        create_option
                        update_option
                        delete_option
                        attach_cat
                        detach_cat
                    />
                }
            }).collect_view()}
        </div>
    }
    .into_any()
}

#[component]
fn GroupCard(
    g: OptionGroupAdminRow,
    cats: Vec<CategoryRow>,
    attached: Vec<(String, i64)>,
    update_group: ServerAction<UpdateOptionGroup>,
    delete_group: ServerAction<DeleteOptionGroup>,
    create_option: ServerAction<CreateOption>,
    update_option: ServerAction<UpdateOption>,
    delete_option: ServerAction<DeleteOption>,
    attach_cat: ServerAction<AttachGroupToCategory>,
    detach_cat: ServerAction<DetachGroupFromCategory>,
) -> impl IntoView {
    let id = g.id.clone();
    let id_for_save = id.clone();
    let id_for_delete = id.clone();
    let id_for_create = id.clone();
    let label = RwSignal::new(g.label.clone());
    let min_sel = RwSignal::new(g.min_select.to_string());
    let max_sel = RwSignal::new(g.max_select.to_string());
    let sort_order = RwSignal::new(g.sort_order.to_string());
    let is_active = RwSignal::new(g.is_active);
    let hide_zero_price = RwSignal::new(g.hide_zero_price);

    let new_opt_label = RwSignal::new(String::new());
    let new_opt_price = RwSignal::new("0".to_string());
    let is_empty_group = g.options.is_empty();

    let initial_cat = cats.first().map(|c| c.id.clone()).unwrap_or_default();
    let bulk_cat = RwSignal::new(initial_cat);
    let id_for_attach = id.clone();
    let id_for_detach = id.clone();
    let on_attach_cat = move |_| {
        let cid = bulk_cat.get();
        if cid.trim().is_empty() {
            return;
        }
        attach_cat.dispatch(AttachGroupToCategory {
            group_id: id_for_attach.clone(),
            category_id: cid,
        });
    };
    let on_detach_cat = move |_| {
        let cid = bulk_cat.get();
        if cid.trim().is_empty() {
            return;
        }
        detach_cat.dispatch(DetachGroupFromCategory {
            group_id: id_for_detach.clone(),
            category_id: cid,
        });
    };

    let on_save_group = move |_| {
        let mn = min_sel.get().parse::<i64>().unwrap_or(1);
        let mx = max_sel.get().parse::<i64>().unwrap_or(1);
        let so = sort_order.get().parse::<i64>().unwrap_or(0);
        update_group.dispatch(UpdateOptionGroup {
            id: id_for_save.clone(),
            label: label.get(),
            min_select: mn,
            max_select: mx,
            sort_order: so,
            is_active: is_active.get(),
            hide_zero_price: hide_zero_price.get(),
        });
    };
    let on_delete_group = move |_| {
        // No JS-confirm dialog here — keeps the SSR/hydrate paths
        // identical and avoids depending on web_sys::window inside
        // a #[component]. If accidental delete becomes a problem
        // we can wire a small inline-confirm UI later.
        delete_group.dispatch(DeleteOptionGroup {
            id: id_for_delete.clone(),
        });
    };
    let on_add_option = move |_| {
        let p = new_opt_price.get().parse::<i64>().unwrap_or(0);
        let label_now = new_opt_label.get();
        if label_now.trim().is_empty() {
            return;
        }
        create_option.dispatch(CreateOption {
            group_id: id_for_create.clone(),
            label: label_now,
            price_cents: p,
        });
        new_opt_label.set(String::new());
        new_opt_price.set("0".to_string());
    };

    view! {
        <article class="option-group-card">
            <div class="og-head">
                <div class="og-grid">
                    <label>
                        <span>"Bezeichnung"</span>
                        <input type="text"
                            prop:value=move || label.get()
                            on:input=move |ev| label.set(event_target_value(&ev))/>
                    </label>
                    <label>
                        <span>"Min."</span>
                        <input type="number" min="0"
                            prop:value=move || min_sel.get()
                            on:input=move |ev| min_sel.set(event_target_value(&ev))/>
                    </label>
                    <label>
                        <span>"Max."</span>
                        <input type="number" min="1"
                            prop:value=move || max_sel.get()
                            on:input=move |ev| max_sel.set(event_target_value(&ev))/>
                    </label>
                    <label>
                        <span>"Sortierung"</span>
                        <input type="number"
                            prop:value=move || sort_order.get()
                            on:input=move |ev| sort_order.set(event_target_value(&ev))/>
                    </label>
                    <label class="checkbox">
                        <input type="checkbox"
                            prop:checked=move || is_active.get()
                            on:change=move |ev| is_active.set(event_target_checked(&ev))/>
                        <span>"Aktiv"</span>
                    </label>
                    <label class="checkbox" title="Für reine Varianten-Auswahl (z.B. Getränke-Sorten): blendet \"gratis\" bei 0-€-Optionen aus, damit es nicht so aussieht, als wäre das Produkt kostenlos.">
                        <input type="checkbox"
                            prop:checked=move || hide_zero_price.get()
                            on:change=move |ev| hide_zero_price.set(event_target_checked(&ev))/>
                        <span>"\"gratis\" ausblenden"</span>
                    </label>
                </div>
                <div class="og-actions">
                    <button class="btn primary small" on:click=on_save_group>"Speichern"</button>
                    <button class="btn ghost small danger" on:click=on_delete_group>"Löschen"</button>
                </div>
            </div>
            {is_empty_group.then(|| view! {
                <p class="warn">
                    "⚠ Diese Gruppe hat noch "<b>"keine Optionen"</b>". "
                    "Füge unten Auswahl-Möglichkeiten hinzu (z.B. \"Italian\", \"Joghurt\"), "
                    "sonst sieht der Kunde nichts zum Anklicken."
                </p>
            })}
            <h4 class="opts-h">"Optionen in dieser Gruppe"</h4>
            <table class="options-table">
                <thead>
                    <tr><th>"Option"</th><th>"Preis (Cent)"</th><th>"Aktiv"</th><th>"Sort"</th><th></th></tr>
                </thead>
                <tbody>
                    {g.options.into_iter().map(|o| view! {
                        <OptionRow o update_option delete_option/>
                    }).collect_view()}
                    <tr class="new-row">
                        <td>
                            <input type="text" class="cell-input"
                                placeholder="Neue Option, z.B. Joghurt-Dressing"
                                prop:value=move || new_opt_label.get()
                                on:input=move |ev| new_opt_label.set(event_target_value(&ev))/>
                        </td>
                        <td>
                            <input type="number" class="cell-input narrow" min="0"
                                prop:value=move || new_opt_price.get()
                                on:input=move |ev| new_opt_price.set(event_target_value(&ev))/>
                        </td>
                        <td></td>
                        <td></td>
                        <td>
                            <button class="btn primary small" on:click=on_add_option>"+ Option"</button>
                        </td>
                    </tr>
                </tbody>
            </table>
            <div class="og-applied">
                <span class="label">"Angewendet auf: "</span>
                {if attached.is_empty() {
                    view! { <em class="muted">"– (noch keine Kategorie)"</em> }.into_any()
                } else {
                    view! {
                        <span>
                        {attached.into_iter().map(|(name, n)| view! {
                            <span class="chip">{format!("{name} ({n})")}</span>
                        }).collect_view()}
                        </span>
                    }.into_any()
                }}
            </div>
            <div class="og-bulk">
                <label>
                    <span>"Auf Kategorie anwenden:"</span>
                    <select on:change=move |ev| bulk_cat.set(event_target_value(&ev))>
                        {cats.into_iter().map(|c| {
                            let cid = c.id.clone();
                            let selected = Memo::new({
                                let cid = cid.clone();
                                move |_| bulk_cat.get() == cid
                            });
                            view! {
                                <option value=cid.clone() selected=move || selected.get()>
                                    {c.name}
                                </option>
                            }
                        }).collect_view()}
                    </select>
                </label>
                <button class="btn primary small" on:click=on_attach_cat>"Anwenden"</button>
                <button class="btn ghost small" on:click=on_detach_cat>"Entfernen"</button>
            </div>
        </article>
    }
}

#[component]
fn OptionRow(
    o: OptionAdminRow,
    update_option: ServerAction<UpdateOption>,
    delete_option: ServerAction<DeleteOption>,
) -> impl IntoView {
    let id = o.id.clone();
    let id_for_save = id.clone();
    let id_for_delete = id.clone();
    let label = RwSignal::new(o.label.clone());
    let price = RwSignal::new(o.price_cents.to_string());
    let active = RwSignal::new(o.is_active);
    let sort = RwSignal::new(o.sort_order.to_string());

    let on_save = move |_| {
        let p = price.get().parse::<i64>().unwrap_or(0);
        let s = sort.get().parse::<i64>().unwrap_or(0);
        update_option.dispatch(UpdateOption {
            id: id_for_save.clone(),
            label: label.get(),
            price_cents: p,
            sort_order: s,
            is_active: active.get(),
        });
    };
    let on_delete = move |_| {
        delete_option.dispatch(DeleteOption {
            id: id_for_delete.clone(),
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
                    prop:checked=move || active.get()
                    on:change=move |ev| active.set(event_target_checked(&ev))/>
            </td>
            <td>
                <input type="number" class="cell-input narrow"
                    prop:value=move || sort.get()
                    on:input=move |ev| sort.set(event_target_value(&ev))/>
            </td>
            <td>
                <button class="btn ghost small" on:click=on_save>"Speichern"</button>
                <button class="btn ghost small danger" on:click=on_delete>"Löschen"</button>
            </td>
        </tr>
    }
}
