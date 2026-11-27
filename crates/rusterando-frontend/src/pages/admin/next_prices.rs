//! /admin/next-prices — stage a new price list while the old prices stay live.
//!
//! Why
//! ---
//! A price increase has a hard ordering constraint: the printed menu has to
//! be back from the print shop BEFORE the online shop switches, otherwise
//! walk-in customers hold a menu with prices the till no longer charges.
//! So the admin needs to (1) enter the new prices without touching the live
//! ones, (2) render the print-shop PDF from those staged prices, and only
//! then (3) flip everything in one go — with a way back.
//!
//! How
//! ---
//! Staged prices live in nullable `next_price_*` columns beside the live
//! ones (migration 20260914000001). Nothing customer-facing reads them; the
//! only consumer is `/menu.pdf?prices=next`. "Aktivieren" copies every
//! staged value over its live column inside one transaction and journals
//! the old/new pair in `price_activations` + `price_activation_rows`, so
//! "Rückgängig" can restore the previous list. The static offline menu.pdf
//! cache is rebuilt after either.
//!
//! Working out a price list takes a while, so the staged prices can be kept
//! as named templates ("Vorlagen", migration 20261111000001) and loaded back
//! later. Each item also shows the ingredients of its description that are
//! in the extras catalog, with their extra prices, as a cost hint.
//!
//! Every pizza has tomatoes and cheese, so a Margherita with the right extras
//! is the same pizza. When that comes out cheaper than the listed one,
//! customers build it themselves — so each pizza shows "Margherita + Extras"
//! (new prices where set) and is flagged when its price is higher; one button
//! (`cap_pizza_prices`) lowers all such prices to it. A pizza with something
//! that isn't an extra (Sardellen on the Portofino) can't be built that way:
//! it gets a note, never the advice to lower its price.

use leptos::prelude::*;
use rusterando_shared::models::{format_eur, parse_eur_cents};
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NextPriceItem {
    pub id: String,
    pub menu_number: Option<String>,
    pub name: String,
    pub price_small_cents: i64,
    pub price_large_cents: Option<i64>,
    pub next_small_cents: Option<i64>,
    pub next_large_cents: Option<i64>,
    pub is_listed: bool,
    /// Ingredients of the description found in the extras catalog.
    #[serde(default)]
    pub ingredients: Vec<ItemIngredient>,
    /// Pizzas: a Margherita with this pizza's extras, per size (new prices
    /// where set): its price shouldn't be higher.
    #[serde(default)]
    pub build: Option<PizzaBuild>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PizzaBuild {
    /// The extras on top of the Margherita.
    pub extras: Vec<String>,
    pub small_cents: i64,
    pub large_cents: Option<i64>,
    /// What the description names that can't be ordered as an extra: then
    /// nobody can build this pizza, and the comparison is a note only.
    #[serde(default)]
    pub not_extras: Vec<String>,
}

impl PizzaBuild {
    /// Can this pizza be ordered as Margherita + extras at all?
    pub fn comparable(&self) -> bool {
        self.not_extras.is_empty()
    }
}

/// An item's ingredient that is also an extra, with the extra's price (the
/// new one where set): a cost hint while setting the new price.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ItemIngredient {
    pub label: String,
    pub price_cents: i64,
}

/// A saved template: the staged prices under a name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PriceTemplate {
    pub id: i64,
    pub name: String,
    /// Local time, "11.11. 10:40".
    pub saved_at: String,
    pub row_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NextPriceCategory {
    pub id: String,
    pub name: String,
    pub items: Vec<NextPriceItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NextPriceExtra {
    pub id: String,
    pub label: String,
    pub price_cents: i64,
    pub next_price_cents: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivationInfo {
    pub id: i64,
    pub activated_at: String,
    pub row_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NextPricesView {
    pub categories: Vec<NextPriceCategory>,
    /// What comparable shops on this platform charge per category. Pure
    /// decision support: item names never match across shops, but category
    /// names do, so this answers "is 7,50 € for a pizza in line?" with
    /// evidence instead of a guess. Empty in single-tenant deployments.
    #[serde(default)]
    pub benchmarks: Vec<crate::pages::sysadmin::CategoryBenchmark>,
    pub extras: Vec<NextPriceExtra>,
    /// Items with at least one staged price.
    pub pending_items: i64,
    pub pending_extras: i64,
    /// Most recent activation that has not been reverted (revert target).
    pub last_activation: Option<ActivationInfo>,
    /// Saved templates, newest first.
    #[serde(default)]
    pub templates: Vec<PriceTemplate>,
}

#[server(name = ListNextPrices, prefix = "/api", endpoint = "list_next_prices")]
pub async fn list_next_prices() -> Result<NextPricesView, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let cat_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, name FROM menu_categories WHERE is_active = 1 ORDER BY sort_order",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load categories: {e}")))?;

    type ItemRow = (
        String,
        String,
        Option<String>,
        String,
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        i64,
        Option<String>,
    );
    let item_rows: Vec<ItemRow> = sqlx::query_as(
        "SELECT id, category_id, menu_number, name,
                price_small_cents, price_large_cents,
                next_price_small_cents, next_price_large_cents, is_listed,
                description
         FROM menu_items
         ORDER BY sort_order, CAST(COALESCE(menu_number, '') AS INTEGER), menu_number",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load items: {e}")))?;

    let mut categories: Vec<NextPriceCategory> = cat_rows
        .into_iter()
        .map(|(id, name)| NextPriceCategory {
            id,
            name,
            items: Vec::new(),
        })
        .collect();
    // The same matcher the "ohne …" removals use (description words vs.
    // the extras catalog and its aliases), with the extras' new prices.
    let catalog = ssr::catalog_new_prices(&db).await;
    let builds = ssr::pizza_builds(
        item_rows.iter().map(|r| ssr::PriceItem {
            id: &r.0,
            name: &r.3,
            description: r.9.as_deref().unwrap_or(""),
            small: r.6.unwrap_or(r.4),
            large: r.7.or(r.5),
        }),
        &catalog,
    );
    let mut pending_items = 0;
    for r in item_rows {
        if r.6.is_some() || r.7.is_some() {
            pending_items += 1;
        }
        let ingredients = rusterando_shared::models::match_removable_ingredients(
            r.9.as_deref().unwrap_or(""),
            &catalog,
        )
        .into_iter()
        .filter_map(|m| {
            let extra = catalog.iter().find(|e| e.id == m.id)?;
            Some(ItemIngredient {
                label: m.bare_label,
                price_cents: extra.price_cents,
            })
        })
        .collect();
        let build = builds.get(&r.0).cloned();
        if let Some(cat) = categories.iter_mut().find(|c| c.id == r.1) {
            cat.items.push(NextPriceItem {
                id: r.0,
                menu_number: r.2,
                name: r.3,
                price_small_cents: r.4,
                price_large_cents: r.5,
                next_small_cents: r.6,
                next_large_cents: r.7,
                is_listed: r.8 != 0,
                ingredients,
                build,
            });
        }
    }
    categories.retain(|c| !c.items.is_empty());

    let extra_rows: Vec<(String, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, label, price_cents, next_price_cents
         FROM pizza_extras WHERE is_available = 1 ORDER BY sort_order, label",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load extras: {e}")))?;
    let pending_extras = extra_rows.iter().filter(|r| r.3.is_some()).count() as i64;
    let extras = extra_rows
        .into_iter()
        .map(
            |(id, label, price_cents, next_price_cents)| NextPriceExtra {
                id,
                label,
                price_cents,
                next_price_cents,
            },
        )
        .collect();

    let last: Option<(i64, String, i64)> = sqlx::query_as(
        "SELECT id, activated_at, row_count FROM price_activations
         WHERE reverted_at IS NULL ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load activations: {e}")))?;

    // The server runs in the shop's time zone (TZ), so 'localtime' is shop
    // time.
    let templates: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT id, name, strftime('%d.%m. %H:%M', created_at, 'localtime'), row_count
         FROM price_templates ORDER BY id DESC",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load templates: {e}")))?;

    Ok(NextPricesView {
        categories,
        // Loaded separately by the page (see `price_benchmarks`) so a slow
        // sibling tenant can't stall the price list.
        benchmarks: Vec::new(),
        extras,
        pending_items,
        pending_extras,
        last_activation: last.map(|(id, activated_at, row_count)| ActivationInfo {
            id,
            activated_at,
            row_count,
        }),
        templates: templates
            .into_iter()
            .map(|(id, name, saved_at, row_count)| PriceTemplate {
                id,
                name,
                saved_at,
                row_count,
            })
            .collect(),
    })
}

/// Save the staged prices as a template. Returns how many prices it holds.
#[server(name = SavePriceTemplate, prefix = "/api", endpoint = "save_price_template")]
pub async fn save_price_template(name: String) -> Result<i64, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let name = name.trim();
    if name.is_empty() {
        return Err(ServerFnError::new(
            "Bitte einen Namen für die Vorlage eingeben",
        ));
    }
    let err = |e: sqlx::Error| ServerFnError::new(format!("Vorlage speichern: {e}"));
    let mut tx = db.begin().await.map_err(err)?;
    let id = sqlx::query("INSERT INTO price_templates (name) VALUES (?1)")
        .bind(name)
        .execute(&mut *tx)
        .await
        .map_err(err)?
        .last_insert_rowid();
    let items = sqlx::query(
        "INSERT INTO price_template_rows (template_id, target_kind, target_id, small_cents, large_cents)
         SELECT ?1, 'item', id, next_price_small_cents, next_price_large_cents FROM menu_items
         WHERE next_price_small_cents IS NOT NULL OR next_price_large_cents IS NOT NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(err)?
    .rows_affected();
    let extras = sqlx::query(
        "INSERT INTO price_template_rows (template_id, target_kind, target_id, small_cents, large_cents)
         SELECT ?1, 'extra', id, next_price_cents, NULL FROM pizza_extras
         WHERE next_price_cents IS NOT NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(err)?
    .rows_affected();
    let count = (items + extras) as i64;
    if count == 0 {
        // Dropping the transaction rolls the empty template back.
        return Err(ServerFnError::new(
            "Keine vorgemerkten Preise zum Speichern",
        ));
    }
    sqlx::query("UPDATE price_templates SET row_count = ?1 WHERE id = ?2")
        .bind(count)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    tx.commit().await.map_err(err)?;
    Ok(count)
}

/// Put a template's prices back as the staged prices, replacing the current
/// ones. Items or extras deleted since are skipped. Returns how many
/// prices were loaded.
#[server(name = LoadPriceTemplate, prefix = "/api", endpoint = "load_price_template")]
pub async fn load_price_template(id: i64) -> Result<i64, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let err = |e: sqlx::Error| ServerFnError::new(format!("Vorlage laden: {e}"));
    let mut tx = db.begin().await.map_err(err)?;
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM price_templates WHERE id = ?1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(err)?;
    if exists.is_none() {
        return Err(ServerFnError::new("Vorlage nicht gefunden"));
    }
    for sql in [
        "UPDATE menu_items SET next_price_small_cents = NULL, next_price_large_cents = NULL",
        "UPDATE pizza_extras SET next_price_cents = NULL",
    ] {
        sqlx::query(sql).execute(&mut *tx).await.map_err(err)?;
    }
    let items = sqlx::query(
        "UPDATE menu_items SET next_price_small_cents = r.small_cents,
                               next_price_large_cents = r.large_cents
         FROM price_template_rows r
         WHERE r.template_id = ?1 AND r.target_kind = 'item' AND r.target_id = menu_items.id",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(err)?
    .rows_affected();
    let extras = sqlx::query(
        "UPDATE pizza_extras SET next_price_cents = r.small_cents
         FROM price_template_rows r
         WHERE r.template_id = ?1 AND r.target_kind = 'extra' AND r.target_id = pizza_extras.id",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(err)?
    .rows_affected();
    tx.commit().await.map_err(err)?;
    Ok((items + extras) as i64)
}

/// Delete a template (the staged prices stay as they are).
#[server(name = DeletePriceTemplate, prefix = "/api", endpoint = "delete_price_template")]
pub async fn delete_price_template(id: i64) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let err = |e: sqlx::Error| ServerFnError::new(format!("Vorlage löschen: {e}"));
    let mut tx = db.begin().await.map_err(err)?;
    // Rows first: foreign keys may be off on this connection.
    sqlx::query("DELETE FROM price_template_rows WHERE template_id = ?1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    sqlx::query("DELETE FROM price_templates WHERE id = ?1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    tx.commit().await.map_err(err)?;
    Ok(())
}

/// Stage (or clear, with both `None`) the new prices of one menu item.
/// Benchmarks for the categories this shop actually has. Split from
/// `list_next_prices` so a slow/absent sibling tenant can never delay or
/// break the price list itself.
#[server(name = PriceBenchmarks, prefix = "/api", endpoint = "price_benchmarks")]
pub async fn price_benchmarks(
) -> Result<Vec<crate::pages::sysadmin::CategoryBenchmark>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    // Multi-tenant only: without the handle there are no sibling shops.
    let Some(ops) = use_context::<crate::pages::sysadmin::SysAdminHandle>().flatten() else {
        return Ok(Vec::new());
    };
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let cats: Vec<String> =
        sqlx::query_scalar("SELECT name FROM menu_categories WHERE is_active = 1 ORDER BY name")
            .fetch_all(&db)
            .await
            .unwrap_or_default();
    let me = use_context::<crate::pages::settings::ResolvedTenant>()
        .map(|t| t.slug)
        .unwrap_or_default();
    Ok(ops.price_benchmarks(&me, &cats).await)
}

#[server(name = SetNextItemPrice, prefix = "/api", endpoint = "set_next_item_price")]
pub async fn set_next_item_price(
    id: String,
    next_small_cents: Option<i64>,
    next_large_cents: Option<i64>,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    if next_small_cents.is_some_and(|c| c < 0) || next_large_cents.is_some_and(|c| c < 0) {
        return Err(ServerFnError::new("Preis darf nicht negativ sein"));
    }
    // Items without a large size never get a staged large price — the PDF
    // and the activation would otherwise invent a second size.
    let has_large: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT price_large_cents FROM menu_items WHERE id = ?1")
            .bind(&id)
            .fetch_optional(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("load item: {e}")))?;
    let Some((live_large,)) = has_large else {
        return Err(ServerFnError::new("Artikel nicht gefunden"));
    };
    let next_large_cents = if live_large.is_some() {
        next_large_cents
    } else {
        None
    };
    sqlx::query(
        "UPDATE menu_items
         SET next_price_small_cents = ?2, next_price_large_cents = ?3
         WHERE id = ?1",
    )
    .bind(&id)
    .bind(next_small_cents)
    .bind(next_large_cents)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("save: {e}")))?;
    Ok(())
}

/// Stage (or clear) the new per-piece price of one extra.
#[server(name = SetNextExtraPrice, prefix = "/api", endpoint = "set_next_extra_price")]
pub async fn set_next_extra_price(
    id: String,
    next_price_cents: Option<i64>,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    if next_price_cents.is_some_and(|c| c < 0) {
        return Err(ServerFnError::new("Preis darf nicht negativ sein"));
    }
    let res = sqlx::query("UPDATE pizza_extras SET next_price_cents = ?2 WHERE id = ?1")
        .bind(&id)
        .bind(next_price_cents)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("save: {e}")))?;
    if res.rows_affected() == 0 {
        return Err(ServerFnError::new("Extra nicht gefunden"));
    }
    Ok(())
}

/// `(id, price_small, price_large, next_small, next_large)` of a menu item.
#[cfg(feature = "ssr")]
type StagedItemRow = (String, i64, Option<i64>, Option<i64>, Option<i64>);

/// Lower every pizza's new price (per size) that is higher than a
/// Margherita with its extras ([`PizzaBuild`]). Returns how many prices
/// changed.
#[server(name = CapPizzaPrices, prefix = "/api", endpoint = "cap_pizza_prices")]
pub async fn cap_pizza_prices() -> Result<i64, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    type Row = (
        String,
        String,
        Option<String>,
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, name, description, price_small_cents, price_large_cents,
                next_price_small_cents, next_price_large_cents
         FROM menu_items",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load items: {e}")))?;
    let catalog = ssr::catalog_new_prices(&db).await;
    let builds = ssr::pizza_builds(
        rows.iter().map(|r| ssr::PriceItem {
            id: &r.0,
            name: &r.1,
            description: r.2.as_deref().unwrap_or(""),
            small: r.5.unwrap_or(r.3),
            large: r.6.or(r.4),
        }),
        &catalog,
    );
    let mut changed = 0;
    for (id, _, _, live_small, live_large, next_small, next_large) in rows {
        let Some(b) = builds.get(&id).filter(|b| b.comparable()) else {
            continue;
        };
        // The new price, lowered to the build where it's above; equal to
        // the live price = nothing staged.
        let cap = |live: i64, next: Option<i64>, build: i64| -> (Option<i64>, bool) {
            let price = next.unwrap_or(live);
            if price <= build {
                return (next, false);
            }
            ((build != live).then_some(build), true)
        };
        let (small, small_changed) = cap(live_small, next_small, b.small_cents);
        let (large, large_changed) = match (live_large, b.large_cents) {
            (Some(live), Some(build)) => cap(live, next_large, build),
            _ => (next_large, false),
        };
        if !small_changed && !large_changed {
            continue;
        }
        sqlx::query(
            "UPDATE menu_items SET next_price_small_cents = ?1, next_price_large_cents = ?2
             WHERE id = ?3",
        )
        .bind(small)
        .bind(large)
        .bind(&id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("update {id}: {e}")))?;
        changed += i64::from(small_changed) + i64::from(large_changed);
    }
    Ok(changed)
}

#[cfg(feature = "ssr")]
pub(crate) mod ssr {
    use std::collections::{HashMap, HashSet};

    use rusterando_shared::models::{match_removable_ingredients, PizzaExtra};
    use sqlx::SqlitePool;

    use super::PizzaBuild;

    /// The extras catalog (with aliases) carrying each extra's new price
    /// where one is staged.
    pub async fn catalog_new_prices(db: &SqlitePool) -> Vec<PizzaExtra> {
        let mut catalog =
            crate::pages::admin::extras_admin::ssr::load_catalog_with_aliases(db, "label").await;
        let staged: Vec<(String, i64)> = sqlx::query_as(
            "SELECT id, next_price_cents FROM pizza_extras WHERE next_price_cents IS NOT NULL",
        )
        .fetch_all(db)
        .await
        .unwrap_or_default();
        for (id, price) in staged {
            if let Some(e) = catalog.iter_mut().find(|e| e.id == id) {
                e.price_cents = price;
            }
        }
        catalog
    }

    /// A menu item with its prices as they'll be (new where staged).
    pub struct PriceItem<'a> {
        pub id: &'a str,
        pub name: &'a str,
        pub description: &'a str,
        pub small: i64,
        pub large: Option<i64>,
    }

    /// A pizza: "Pizza …" (not "Pizzabrötchen").
    pub fn is_pizza(name: &str) -> bool {
        name.trim().to_lowercase().starts_with("pizza ")
    }

    /// The ingredients a description lists: "mit Schinken, Pilzen und
    /// Tabasco" → Schinken, Pilzen, Tabasco.
    pub fn listed_ingredients(description: &str) -> Vec<String> {
        let d = description.trim();
        let d = d
            .strip_prefix("mit ")
            .or_else(|| d.strip_prefix("Mit "))
            .unwrap_or(d);
        d.split([',', '&', '/', ';', '+'])
            .flat_map(|part| part.split(" und "))
            .map(|t| {
                t.trim()
                    .trim_matches(|c: char| c == '.' || c == '(' || c == ')')
                    .trim()
            })
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Per pizza: the Margherita (the first "Pizza … Margherita") plus the
    /// extras its description names that the Margherita's doesn't — every
    /// pizza has its tomatoes and cheese. Pizzas without such an extra get
    /// none (nothing to compare).
    pub fn pizza_builds<'a>(
        items: impl Iterator<Item = PriceItem<'a>>,
        catalog: &[PizzaExtra],
    ) -> HashMap<String, PizzaBuild> {
        let items: Vec<PriceItem> = items.collect();
        let Some(margherita) = items
            .iter()
            .find(|i| is_pizza(i.name) && i.name.to_lowercase().contains("margherita"))
        else {
            return HashMap::new();
        };
        let base: HashSet<String> = match_removable_ingredients(margherita.description, catalog)
            .into_iter()
            .map(|m| m.id)
            .collect();
        let mut out = HashMap::new();
        for item in items
            .iter()
            .filter(|i| is_pizza(i.name) && i.id != margherita.id)
        {
            let extras: Vec<&PizzaExtra> = match_removable_ingredients(item.description, catalog)
                .into_iter()
                .filter(|m| !base.contains(&m.id))
                .filter_map(|m| catalog.iter().find(|e| e.id == m.id))
                .collect();
            if extras.is_empty() {
                continue;
            }
            let not_extras: Vec<String> = listed_ingredients(item.description)
                .into_iter()
                .filter(|t| match_removable_ingredients(t, catalog).is_empty())
                .collect();
            let sum: i64 = extras.iter().map(|e| e.price_cents).sum();
            out.insert(
                item.id.to_string(),
                PizzaBuild {
                    extras: extras
                        .iter()
                        .map(|e| {
                            rusterando_shared::models::bare_ingredient_label(&e.label).to_string()
                        })
                        .collect(),
                    small_cents: margherita.small + sum,
                    large_cents: item.large.and(margherita.large).map(|l| l + sum),
                    not_extras,
                },
            );
        }
        out
    }
}

/// Round `cents` to a multiple of `step` (0 / 1 = no rounding). `up` = always
/// round up (the usual choice for a price increase), else nearest.
#[cfg(feature = "ssr")]
fn round_cents(cents: i64, step: i64, up: bool) -> i64 {
    if step <= 1 {
        return cents;
    }
    let rem = cents.rem_euclid(step);
    if rem == 0 {
        cents
    } else if up || rem * 2 >= step {
        cents - rem + step
    } else {
        cents - rem
    }
}

#[cfg(feature = "ssr")]
fn bulk_new_price(live: i64, percent: f64, add_cents: i64, step: i64, up: bool) -> i64 {
    let raised = (live as f64 * (1.0 + percent / 100.0)).round() as i64 + add_cents;
    round_cents(raised.max(0), step, up)
}

/// Fill the staged prices from the live ones by formula:
/// `next = round(live × (1 + percent/100) + add_cents)`.
///
/// `scope`: "all" | "items" | "extras". `overwrite = false` only fills rows
/// that have no staged price yet, so hand-edited rows survive a re-run.
#[server(name = BulkNextPrices, prefix = "/api", endpoint = "bulk_next_prices")]
pub async fn bulk_next_prices(
    percent: f64,
    add_cents: i64,
    round_step_cents: i64,
    round_up: bool,
    scope: String,
    overwrite: bool,
) -> Result<i64, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    if !percent.is_finite() || percent < -100.0 || percent > 500.0 {
        return Err(ServerFnError::new("Prozentsatz außerhalb von -100 … 500"));
    }
    if !matches!(round_step_cents, 0 | 1 | 5 | 10 | 50 | 100) {
        return Err(ServerFnError::new("Rundungsschritt ungültig"));
    }
    let do_items = matches!(scope.as_str(), "all" | "items");
    let do_extras = matches!(scope.as_str(), "all" | "extras");
    if !do_items && !do_extras {
        return Err(ServerFnError::new("Bereich ungültig"));
    }

    let mut tx = db
        .begin()
        .await
        .map_err(|e| ServerFnError::new(format!("tx: {e}")))?;
    let mut touched = 0i64;

    if do_items {
        let rows: Vec<StagedItemRow> = sqlx::query_as(
            "SELECT id, price_small_cents, price_large_cents,
                    next_price_small_cents, next_price_large_cents
             FROM menu_items",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("load items: {e}")))?;
        for (id, small, large, next_small, next_large) in rows {
            if !overwrite && (next_small.is_some() || next_large.is_some()) {
                continue;
            }
            let ns = bulk_new_price(small, percent, add_cents, round_step_cents, round_up);
            let nl =
                large.map(|l| bulk_new_price(l, percent, add_cents, round_step_cents, round_up));
            sqlx::query(
                "UPDATE menu_items
                 SET next_price_small_cents = ?2, next_price_large_cents = ?3
                 WHERE id = ?1",
            )
            .bind(&id)
            .bind(ns)
            .bind(nl)
            .execute(&mut *tx)
            .await
            .map_err(|e| ServerFnError::new(format!("update item: {e}")))?;
            touched += 1;
        }
    }
    if do_extras {
        let rows: Vec<(String, i64, Option<i64>)> =
            sqlx::query_as("SELECT id, price_cents, next_price_cents FROM pizza_extras")
                .fetch_all(&mut *tx)
                .await
                .map_err(|e| ServerFnError::new(format!("load extras: {e}")))?;
        for (id, price, next) in rows {
            if !overwrite && next.is_some() {
                continue;
            }
            let np = bulk_new_price(price, percent, add_cents, round_step_cents, round_up);
            sqlx::query("UPDATE pizza_extras SET next_price_cents = ?2 WHERE id = ?1")
                .bind(&id)
                .bind(np)
                .execute(&mut *tx)
                .await
                .map_err(|e| ServerFnError::new(format!("update extra: {e}")))?;
            touched += 1;
        }
    }
    tx.commit()
        .await
        .map_err(|e| ServerFnError::new(format!("commit: {e}")))?;
    Ok(touched)
}

/// Drop every staged price (live prices untouched).
#[server(name = ClearNextPrices, prefix = "/api", endpoint = "clear_next_prices")]
pub async fn clear_next_prices() -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query(
        "UPDATE menu_items SET next_price_small_cents = NULL, next_price_large_cents = NULL",
    )
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("clear items: {e}")))?;
    sqlx::query("UPDATE pizza_extras SET next_price_cents = NULL")
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("clear extras: {e}")))?;
    Ok(())
}

/// Make the staged prices live — one transaction, journaled for revert.
/// Returns the number of prices that changed.
#[server(name = ActivateNextPrices, prefix = "/api", endpoint = "activate_next_prices")]
pub async fn activate_next_prices() -> Result<i64, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let mut tx = db
        .begin()
        .await
        .map_err(|e| ServerFnError::new(format!("tx: {e}")))?;

    let items: Vec<StagedItemRow> = sqlx::query_as(
        "SELECT id, price_small_cents, price_large_cents,
                next_price_small_cents, next_price_large_cents
         FROM menu_items
         WHERE next_price_small_cents IS NOT NULL OR next_price_large_cents IS NOT NULL",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("load items: {e}")))?;
    let extras: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT id, price_cents, next_price_cents FROM pizza_extras
         WHERE next_price_cents IS NOT NULL",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("load extras: {e}")))?;

    if items.is_empty() && extras.is_empty() {
        return Err(ServerFnError::new("Keine neuen Preise eingetragen"));
    }

    let activation_id: i64 =
        sqlx::query_scalar("INSERT INTO price_activations (row_count) VALUES (?1) RETURNING id")
            .bind((items.len() + extras.len()) as i64)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ServerFnError::new(format!("journal: {e}")))?;

    for (id, small, large, next_small, next_large) in &items {
        // A partially staged row keeps its other size as-is.
        let new_small = next_small.unwrap_or(*small);
        let new_large = match (large, next_large) {
            (Some(_), Some(n)) => Some(*n),
            (Some(l), None) => Some(*l),
            (None, _) => None,
        };
        sqlx::query(
            "INSERT INTO price_activation_rows
                (activation_id, target_kind, target_id, old_small, old_large, new_small, new_large)
             VALUES (?1, 'item', ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(activation_id)
        .bind(id)
        .bind(small)
        .bind(large)
        .bind(new_small)
        .bind(new_large)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("journal item: {e}")))?;
        sqlx::query(
            "UPDATE menu_items
             SET price_small_cents = ?2, price_large_cents = ?3,
                 next_price_small_cents = NULL, next_price_large_cents = NULL,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
        )
        .bind(id)
        .bind(new_small)
        .bind(new_large)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("activate item: {e}")))?;
    }
    for (id, price, next) in &extras {
        sqlx::query(
            "INSERT INTO price_activation_rows
                (activation_id, target_kind, target_id, old_small, new_small)
             VALUES (?1, 'extra', ?2, ?3, ?4)",
        )
        .bind(activation_id)
        .bind(id)
        .bind(price)
        .bind(next)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("journal extra: {e}")))?;
        sqlx::query(
            "UPDATE pizza_extras
             SET price_cents = ?2, next_price_cents = NULL, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
        )
        .bind(id)
        .bind(next)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("activate extra: {e}")))?;
    }
    tx.commit()
        .await
        .map_err(|e| ServerFnError::new(format!("commit: {e}")))?;

    // Live prices changed → the offline-fallback PDF must follow.
    crate::pages::push::rebuild_menu_pdf_cache().await;
    Ok((items.len() + extras.len()) as i64)
}

/// Restore the prices from before the most recent activation. The
/// activation stays in the journal, marked reverted. Returns the number of
/// prices restored.
#[server(name = RevertLastActivation, prefix = "/api", endpoint = "revert_last_price_activation")]
pub async fn revert_last_activation() -> Result<i64, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let mut tx = db
        .begin()
        .await
        .map_err(|e| ServerFnError::new(format!("tx: {e}")))?;
    let last: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM price_activations WHERE reverted_at IS NULL ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("load activation: {e}")))?;
    let Some((activation_id,)) = last else {
        return Err(ServerFnError::new("Keine Aktivierung zum Zurücknehmen"));
    };
    let rows: Vec<(String, String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT target_kind, target_id, old_small, old_large
         FROM price_activation_rows WHERE activation_id = ?1",
    )
    .bind(activation_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| ServerFnError::new(format!("load rows: {e}")))?;
    let mut restored = 0i64;
    for (kind, id, old_small, old_large) in rows {
        let Some(old_small) = old_small else { continue };
        let n = match kind.as_str() {
            "item" => {
                sqlx::query(
                    "UPDATE menu_items
                 SET price_small_cents = ?2, price_large_cents = ?3,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                )
                .bind(&id)
                .bind(old_small)
                .bind(old_large)
                .execute(&mut *tx)
                .await
            }
            "extra" => {
                sqlx::query(
                    "UPDATE pizza_extras
                 SET price_cents = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                )
                .bind(&id)
                .bind(old_small)
                .execute(&mut *tx)
                .await
            }
            _ => continue,
        }
        .map_err(|e| ServerFnError::new(format!("restore {kind}: {e}")))?;
        restored += n.rows_affected() as i64;
    }
    sqlx::query("UPDATE price_activations SET reverted_at = CURRENT_TIMESTAMP WHERE id = ?1")
        .bind(activation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerFnError::new(format!("mark reverted: {e}")))?;
    tx.commit()
        .await
        .map_err(|e| ServerFnError::new(format!("commit: {e}")))?;

    crate::pages::push::rebuild_menu_pdf_cache().await;
    Ok(restored)
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::{bulk_new_price, round_cents};

    #[test]
    fn rounding() {
        assert_eq!(round_cents(934, 10, true), 940);
        assert_eq!(round_cents(934, 10, false), 930);
        assert_eq!(round_cents(935, 10, false), 940);
        assert_eq!(round_cents(940, 10, true), 940);
        assert_eq!(round_cents(934, 0, true), 934);
        assert_eq!(round_cents(1201, 50, true), 1250);
    }

    #[test]
    fn formula() {
        // 8,90 € + 5 % = 9,345 → ceil to 10 ct = 9,40
        assert_eq!(bulk_new_price(890, 5.0, 0, 10, true), 940);
        // flat +0,50 €, no rounding
        assert_eq!(bulk_new_price(890, 0.0, 50, 0, true), 940);
        // never below zero
        assert_eq!(bulk_new_price(30, -100.0, -10, 0, true), 0);
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

fn eur_input(c: Option<i64>) -> String {
    c.map(|c| format!("{},{:02}", c / 100, (c % 100).abs()))
        .unwrap_or_default()
}

fn diff_view(live: i64, next: Option<i64>) -> impl IntoView {
    next.filter(|n| *n != live).map(|n| {
        let d = n - live;
        let cls = if d > 0 { "diff up" } else { "diff down" };
        let sign = if d > 0 { "+" } else { "−" };
        view! { <span class=cls>{format!("{sign}{}", format_eur(d.abs()))}</span> }
    })
}

#[component]
pub fn NextPricesAdminPage() -> impl IntoView {
    let bulk = ServerAction::<BulkNextPrices>::new();
    let clear = ServerAction::<ClearNextPrices>::new();
    let activate = ServerAction::<ActivateNextPrices>::new();
    let revert = ServerAction::<RevertLastActivation>::new();
    let tpl_save = ServerAction::<SavePriceTemplate>::new();
    let tpl_load = ServerAction::<LoadPriceTemplate>::new();
    let tpl_delete = ServerAction::<DeletePriceTemplate>::new();
    let cap = ServerAction::<CapPizzaPrices>::new();

    // `new_blocking` like the other admin tables: rendered on the server
    // against the per-tenant pool, not re-fetched on hydration. Per-row
    // edits deliberately do NOT bump this (see `ItemRow`) so typing in one
    // field never re-renders the whole table under the cursor.
    // Benchmarks load in their own (non-blocking) resource: they query every
    // sibling tenant's DB, so a slow one must never hold up the price list.
    // `unwrap_or_default` means "no comparison shown", never an error page.
    let bench = Resource::new(
        || (),
        |_| async move { price_benchmarks().await.unwrap_or_default() },
    );

    let data = Resource::new_blocking(
        move || {
            (
                bulk.version().get(),
                clear.version().get(),
                activate.version().get(),
                revert.version().get(),
                tpl_save.version().get(),
                tpl_load.version().get(),
                tpl_delete.version().get(),
                cap.version().get(),
            )
        },
        |_| async move { list_next_prices().await },
    );

    // Bulk-formula inputs.
    let percent = RwSignal::new("5".to_string());
    let add = RwSignal::new("0,00".to_string());
    let step = RwSignal::new("10".to_string());
    let mode = RwSignal::new("up".to_string());
    let scope = RwSignal::new("all".to_string());
    let overwrite = RwSignal::new(true);
    let bulk_error = RwSignal::new(String::new());
    let run_bulk = move |_| {
        let pct: f64 = match percent.get().trim().replace(',', ".").parse() {
            Ok(v) => v,
            Err(_) => {
                bulk_error.set("Prozentsatz ungültig".into());
                return;
            }
        };
        let add_cents = match parse_eur_cents(&add.get()) {
            Ok(v) => v.unwrap_or(0),
            Err(e) => {
                bulk_error.set(e);
                return;
            }
        };
        bulk_error.set(String::new());
        bulk.dispatch(BulkNextPrices {
            percent: pct,
            add_cents,
            round_step_cents: step.get().parse().unwrap_or(0),
            round_up: mode.get() == "up",
            scope: scope.get(),
            overwrite: overwrite.get(),
        });
    };

    // Two-step confirms for the irreversible-feeling buttons (inline, no
    // browser dialog).
    let confirm_activate = RwSignal::new(false);
    let confirm_revert = RwSignal::new(false);
    let confirm_clear = RwSignal::new(false);
    // Templates: the name to save under, and which template's "Laden" /
    // "Löschen" is armed (id, true = delete) for its second tap.
    let tpl_name = RwSignal::new(String::new());
    let tpl_armed = RwSignal::new(None::<(i64, bool)>);

    view! {
        <AdminShell>
            <section class="next-prices-admin">
                <header class="admin-bar">
                    <h1>"Neue Preise vorbereiten"</h1>
                </header>
                <p class="hint">
                    "Hier trägst du die neuen Preise ein, ohne die aktuellen zu verändern. "
                    "Der Shop rechnet weiter mit den alten Preisen. Lade die PDF „mit neuen Preisen“ "
                    "für die Druckerei herunter — und erst wenn die gedruckte Karte da ist, "
                    "klickst du auf „Neue Preise aktivieren“. Die alten Preise bleiben gespeichert "
                    "und lassen sich mit einem Klick wiederherstellen."
                </p>

                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || data.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(v) => {
                            let pending = v.pending_items + v.pending_extras;
                            // Pizzas whose new price is above Margherita + Extras.
                            let over_pizzas = v
                                .categories
                                .iter()
                                .flat_map(|c| c.items.iter())
                                .filter(|i| {
                                    i.build.as_ref().filter(|b| b.comparable()).is_some_and(|b| {
                                        i.next_small_cents.unwrap_or(i.price_small_cents) > b.small_cents
                                            || i.price_large_cents.zip(b.large_cents).is_some_and(|(live, build)| {
                                                i.next_large_cents.unwrap_or(live) > build
                                            })
                                    })
                                })
                                .count();
                            let last = v.last_activation.clone();
                            view! {
                                <div class="status-bar">
                                    <span>
                                        <span class="count">{pending}</span>
                                        {format!(" Preise vorgemerkt ({} Artikel, {} Extras)", v.pending_items, v.pending_extras)}
                                    </span>
                                    <span class="pdf-links">
                                        <span class="hint">"PDF mit neuen Preisen:"</span>
                                        <a class="btn ghost small" href="/menu.pdf?prices=next" target="_blank" rel="noopener">"Faltblatt"</a>
                                        <a class="btn ghost small" href="/menu.pdf?prices=next&condensed=1" target="_blank" rel="noopener">"Kompakt"</a>
                                        <a class="btn ghost small" href="/menu.pdf?prices=next&format=a5-zickzack" target="_blank" rel="noopener">"A5 Zickzack"</a>
                                        <a class="btn ghost small" href="/menu.pdf?prices=next&format=a5-zickzack&marks=1" target="_blank" rel="noopener"
                                            title="Mit Schnitt- und Falzmarken außerhalb des Anschnitts">"A5 Zickzack + Schnittmarken"</a>
                                    </span>
                                </div>

                                <div class="templates">
                                    <h3>"Vorlagen"</h3>
                                    <p class="hint">
                                        "Speichert die vorgemerkten Preise unter einem Namen. "
                                        "„Laden“ setzt sie später wieder als vorgemerkte Preise ein."
                                    </p>
                                    <div class="save">
                                        <input type="text" placeholder="Name, z. B. Preise 2027 – Entwurf"
                                            prop:value=move || tpl_name.get()
                                            on:input=move |ev| tpl_name.set(event_target_value(&ev))/>
                                        <button class="btn primary small" disabled=pending == 0
                                            on:click=move |_| {
                                                tpl_save.dispatch(SavePriceTemplate { name: tpl_name.get_untracked() });
                                            }>
                                            "Als Vorlage speichern"
                                        </button>
                                        {move || tpl_save.value().get().map(|r| match r {
                                            Ok(n) => view! { <span class="ok">{format!("{n} Preise gespeichert")}</span> }.into_any(),
                                            Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                                        })}
                                        {move || tpl_load.value().get().map(|r| match r {
                                            Ok(n) => view! { <span class="ok">{format!("{n} Preise geladen")}</span> }.into_any(),
                                            Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                                        })}
                                    </div>
                                    {if v.templates.is_empty() {
                                        view! { <p class="hint">"Noch keine Vorlagen."</p> }.into_any()
                                    } else {
                                        view! {
                                            <ul class="template-list">
                                                {v.templates.clone().into_iter().map(|t| {
                                                    let id = t.id;
                                                    let armed = move |delete: bool| tpl_armed.get() == Some((id, delete));
                                                    view! {
                                                        <li>
                                                            <span class="name">{t.name}</span>
                                                            <span class="meta">{format!("{} · {} Preise", t.saved_at, t.row_count)}</span>
                                                            {move || if armed(false) {
                                                                view! {
                                                                    <span class="confirm">
                                                                        <span>{format!("Ersetzt die {pending} vorgemerkten Preise?")}</span>
                                                                        <button class="btn primary small" on:click=move |_| {
                                                                            tpl_armed.set(None);
                                                                            tpl_load.dispatch(LoadPriceTemplate { id });
                                                                        }>"Ja, laden"</button>
                                                                        <button class="btn ghost small" on:click=move |_| tpl_armed.set(None)>"Abbrechen"</button>
                                                                    </span>
                                                                }.into_any()
                                                            } else if armed(true) {
                                                                view! {
                                                                    <span class="confirm">
                                                                        <span>"Vorlage löschen?"</span>
                                                                        <button class="btn ghost small" on:click=move |_| {
                                                                            tpl_armed.set(None);
                                                                            tpl_delete.dispatch(DeletePriceTemplate { id });
                                                                        }>"Ja, löschen"</button>
                                                                        <button class="btn ghost small" on:click=move |_| tpl_armed.set(None)>"Abbrechen"</button>
                                                                    </span>
                                                                }.into_any()
                                                            } else {
                                                                view! {
                                                                    <span class="confirm">
                                                                        // Nothing staged: nothing to lose, load right away.
                                                                        <button class="btn ghost small" on:click=move |_| {
                                                                            if pending == 0 {
                                                                                tpl_load.dispatch(LoadPriceTemplate { id });
                                                                            } else {
                                                                                tpl_armed.set(Some((id, false)));
                                                                            }
                                                                        }>"Laden"</button>
                                                                        <button class="btn ghost small" on:click=move |_| tpl_armed.set(Some((id, true)))>"Löschen"</button>
                                                                    </span>
                                                                }.into_any()
                                                            }}
                                                        </li>
                                                    }
                                                }).collect_view()}
                                            </ul>
                                        }.into_any()
                                    }}
                                </div>

                                <div class="bulk">
                                    <label>
                                        <span>"Aufschlag in %"</span>
                                        <input type="text" inputmode="decimal"
                                            prop:value=move || percent.get()
                                            on:input=move |ev| percent.set(event_target_value(&ev))/>
                                    </label>
                                    <label>
                                        <span>"Zusätzlich in € (fest)"</span>
                                        <input type="text" inputmode="decimal"
                                            prop:value=move || add.get()
                                            on:input=move |ev| add.set(event_target_value(&ev))/>
                                    </label>
                                    <label>
                                        <span>"Runden auf"</span>
                                        <select on:change=move |ev| step.set(event_target_value(&ev))>
                                            <option value="0" selected=move || step.get() == "0">"nicht runden"</option>
                                            <option value="5" selected=move || step.get() == "5">"5 Cent"</option>
                                            <option value="10" selected=move || step.get() == "10">"10 Cent"</option>
                                            <option value="50" selected=move || step.get() == "50">"50 Cent"</option>
                                            <option value="100" selected=move || step.get() == "100">"1 Euro"</option>
                                        </select>
                                    </label>
                                    <label>
                                        <span>"Rundung"</span>
                                        <select on:change=move |ev| mode.set(event_target_value(&ev))>
                                            <option value="up" selected=move || mode.get() == "up">"immer aufrunden"</option>
                                            <option value="nearest" selected=move || mode.get() == "nearest">"kaufmännisch"</option>
                                        </select>
                                    </label>
                                    <label>
                                        <span>"Bereich"</span>
                                        <select on:change=move |ev| scope.set(event_target_value(&ev))>
                                            <option value="all" selected=move || scope.get() == "all">"Artikel + Extras"</option>
                                            <option value="items" selected=move || scope.get() == "items">"nur Artikel"</option>
                                            <option value="extras" selected=move || scope.get() == "extras">"nur Extras"</option>
                                        </select>
                                    </label>
                                    <label>
                                        <span>"Bereits eingetragene überschreiben"</span>
                                        <input type="checkbox"
                                            prop:checked=move || overwrite.get()
                                            on:change=move |ev| overwrite.set(event_target_checked(&ev))/>
                                    </label>
                                    <div class="actions">
                                        <button class="btn primary small" on:click=run_bulk>"Neue Preise berechnen"</button>
                                        {move || (!bulk_error.get().is_empty()).then(|| view! { <span class="error">{bulk_error.get()}</span> })}
                                        {move || bulk.value().get().map(|r| match r {
                                            Ok(n) => view! { <span class="ok">{format!("{n} Preise berechnet")}</span> }.into_any(),
                                            Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                                        })}
                                    </div>
                                    <div class="actions">
                                        <button class="btn ghost small"
                                            title="Jede Pizza höchstens so teuer wie eine Margherita mit ihren Extras"
                                            on:click=move |_| { cap.dispatch(CapPizzaPrices {}); }>
                                            {format!("Pizzen ≤ Margherita + Extras ({over_pizzas} zu teuer)")}
                                        </button>
                                        {move || cap.value().get().map(|r| match r {
                                            Ok(n) => view! { <span class="ok">{format!("{n} Preise gesenkt")}</span> }.into_any(),
                                            Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                                        })}
                                    </div>
                                </div>

                                {v.categories.into_iter().map(|cat| {
                                    // Benchmark line: what comparable shops
                                    // charge in this category. Shown next to
                                    // the heading so the admin sees it while
                                    // typing the new price, not on a separate
                                    // page they'd have to go look up.
                                    let marks: Vec<_> = bench
                                        .get()
                                        .unwrap_or_default()
                                        .into_iter()
                                        .filter(|b| b.category == cat.name)
                                        .collect();
                                    let bench_line = (!marks.is_empty()).then(|| {
                                        let parts: Vec<String> = marks
                                            .iter()
                                            .map(|b| {
                                                format!(
                                                    "{}: {}–{} ⌀{} (n={})",
                                                    b.shop,
                                                    format_eur(b.min_cents),
                                                    format_eur(b.max_cents),
                                                    format_eur(b.median_cents),
                                                    b.item_count
                                                )
                                            })
                                            .collect();
                                        view! {
                                            <p class="bench-line">
                                                <strong>"Vergleich: "</strong>
                                                {parts.join(" · ")}
                                            </p>
                                        }
                                    });
                                    view! {
                                    <h2>{cat.name}</h2>
                                    {bench_line}
                                    <table class="next-prices-table">
                                        <thead>
                                            <tr>
                                                <th>"Nr."</th>
                                                <th>"Artikel"</th>
                                                <th>"Preis klein / einzeln"</th>
                                                <th>"Neu"</th>
                                                <th>"Preis groß"</th>
                                                <th>"Neu"</th>
                                                <th></th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {cat.items.into_iter().map(|it| view! { <ItemRow it/> }).collect_view()}
                                        </tbody>
                                    </table>
                                }}).collect_view()}

                                <h2>"Extras (pro Stück)"</h2>
                                <table class="next-prices-table">
                                    <thead>
                                        <tr>
                                            <th>"Extra"</th>
                                            <th>"Preis"</th>
                                            <th>"Neu"</th>
                                            <th></th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {v.extras.into_iter().map(|ex| view! { <ExtraRow ex/> }).collect_view()}
                                    </tbody>
                                </table>
                                <p class="hint">
                                    "Die Extras-Zeilen auf der gedruckten Karte sind Freitext "
                                    "(Einstellungen → PDF-Extras) und müssen dort separat angepasst werden."
                                </p>

                                <div class="danger-zone">
                                    {move || if confirm_activate.get() {
                                        view! {
                                            <span class="confirm">
                                                <span>{format!("{pending} Preise jetzt live schalten?")}</span>
                                                <button class="btn primary small" on:click=move |_| {
                                                    confirm_activate.set(false);
                                                    activate.dispatch(ActivateNextPrices {});
                                                }>"Ja, aktivieren"</button>
                                                <button class="btn ghost small" on:click=move |_| confirm_activate.set(false)>"Abbrechen"</button>
                                            </span>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <button class="btn primary" disabled=pending == 0
                                                on:click=move |_| confirm_activate.set(true)>
                                                "Neue Preise aktivieren"
                                            </button>
                                        }.into_any()
                                    }}
                                    {move || if confirm_clear.get() {
                                        view! {
                                            <span class="confirm">
                                                <span>"Alle vorgemerkten Preise verwerfen?"</span>
                                                <button class="btn ghost small" on:click=move |_| {
                                                    confirm_clear.set(false);
                                                    clear.dispatch(ClearNextPrices {});
                                                }>"Ja, verwerfen"</button>
                                                <button class="btn ghost small" on:click=move |_| confirm_clear.set(false)>"Abbrechen"</button>
                                            </span>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <button class="btn ghost" disabled=pending == 0
                                                on:click=move |_| confirm_clear.set(true)>
                                                "Vorgemerkte verwerfen"
                                            </button>
                                        }.into_any()
                                    }}
                                    {move || activate.value().get().map(|r| match r {
                                        Ok(n) => view! { <span class="ok">{format!("{n} Preise aktiviert")}</span> }.into_any(),
                                        Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                                    })}
                                </div>

                                {last.map(|a| view! {
                                    <div class="danger-zone">
                                        <span>
                                            {format!("Letzte Aktivierung: {} ({} Preise)", a.activated_at, a.row_count)}
                                        </span>
                                        {move || if confirm_revert.get() {
                                            view! {
                                                <span class="confirm">
                                                    <span>"Alte Preise wiederherstellen?"</span>
                                                    <button class="btn primary small" on:click=move |_| {
                                                        confirm_revert.set(false);
                                                        revert.dispatch(RevertLastActivation {});
                                                    }>"Ja, zurücksetzen"</button>
                                                    <button class="btn ghost small" on:click=move |_| confirm_revert.set(false)>"Abbrechen"</button>
                                                </span>
                                            }.into_any()
                                        } else {
                                            view! {
                                                <button class="btn ghost" on:click=move |_| confirm_revert.set(true)>
                                                    "Rückgängig (alte Preise zurück)"
                                                </button>
                                            }.into_any()
                                        }}
                                        {move || revert.value().get().map(|r| match r {
                                            Ok(n) => view! { <span class="ok">{format!("{n} Preise wiederhergestellt")}</span> }.into_any(),
                                            Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                                        })}
                                    </div>
                                })}
                            }.into_any()
                        }
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

/// One menu item: live prices read-only, staged prices editable. Saves on
/// change (blur / Enter) through a row-local action so the table isn't
/// re-rendered by other rows' saves.
#[component]
fn ItemRow(it: NextPriceItem) -> impl IntoView {
    let id = it.id.clone();
    let live_small = it.price_small_cents;
    let live_large = it.price_large_cents;
    let small = RwSignal::new(eur_input(it.next_small_cents));
    let large = RwSignal::new(eur_input(it.next_large_cents));
    let saved_small = RwSignal::new(it.next_small_cents);
    let saved_large = RwSignal::new(it.next_large_cents);
    let status = RwSignal::new(String::new());
    let status_err = RwSignal::new(false);

    let save = Action::new(move |(s, l): &(Option<i64>, Option<i64>)| {
        let id = id.clone();
        let (s, l) = (*s, *l);
        async move { set_next_item_price(id, s, l).await }
    });
    let commit = move || {
        let s = parse_eur_cents(&small.get());
        let l = parse_eur_cents(&large.get());
        match (s, l) {
            (Ok(s), Ok(l)) => {
                // A staged value identical to the live one is noise: treat
                // it as "not staged" so the counters stay meaningful.
                let s = s.filter(|v| *v != live_small);
                let l = l.filter(|v| Some(*v) != live_large);
                saved_small.set(s);
                saved_large.set(l);
                status_err.set(false);
                status.set("…".into());
                save.dispatch((s, l));
            }
            (Err(e), _) | (_, Err(e)) => {
                status_err.set(true);
                status.set(e);
            }
        }
    };
    let on_change = move |_| commit();
    // Pizzas: higher than a Margherita with its extras?
    let build = it.build.clone();
    // Only a pizza one can really order as Margherita + extras gets the
    // "too expensive" check; the others a note.
    let comparable = build.as_ref().is_some_and(PizzaBuild::comparable);
    let (build_small, build_large) = if comparable {
        (
            build.as_ref().map(|b| b.small_cents),
            build.as_ref().and_then(|b| b.large_cents),
        )
    } else {
        (None, None)
    };
    let over_small =
        move || build_small.is_some_and(|b| saved_small.get().unwrap_or(live_small) > b);
    let over_large = move || {
        build_large
            .is_some_and(|b| live_large.is_some_and(|ll| saved_large.get().unwrap_or(ll) > b))
    };
    let lower = move || {
        if over_small() {
            small.set(eur_input(build_small));
        }
        if over_large() {
            large.set(eur_input(build_large));
        }
        commit();
    };
    let build_line = build.map(|b| {
        let mut line = format!(
            "Margherita + {}: {}{}",
            b.extras.join(", "),
            format_eur(b.small_cents),
            b.large_cents
                .map(|l| format!(" / {}", format_eur(l)))
                .unwrap_or_default()
        );
        if !b.not_extras.is_empty() {
            line.push_str(&format!(
                " · {} gibt es nicht als Extra – kein Vergleich",
                b.not_extras.join(", ")
            ));
        }
        line
    });
    Effect::new(move |_| {
        if let Some(r) = save.value().get() {
            match r {
                Ok(()) => {
                    status_err.set(false);
                    status.set("gespeichert".into());
                }
                Err(e) => {
                    status_err.set(true);
                    status.set(format!("Fehler: {e}"));
                }
            }
        }
    });

    let row_cls = if it.is_listed { "" } else { "unlisted" };
    let name = if it.is_listed {
        it.name.clone()
    } else {
        format!("{} (nicht gelistet)", it.name)
    };
    // "Schinken 1,00 € · Pilze 0,80 €": what the extras would cost.
    let ingredients = (!it.ingredients.is_empty()).then(|| {
        it.ingredients
            .iter()
            .map(|i| format!("{} {}", i.label, format_eur(i.price_cents)))
            .collect::<Vec<_>>()
            .join(" · ")
    });
    view! {
        <tr class=row_cls>
            <td class="num">{it.menu_number.clone().unwrap_or_default()}</td>
            <td>
                {name}
                {ingredients.map(|i| view! { <div class="ingredients">{i}</div> })}
                {build_line.map(|text| view! {
                    <div class="build" class:note=!comparable class:over=move || over_small() || over_large()>
                        {text}
                        {move || (over_small() || over_large()).then(|| view! {
                            " — teurer! "
                            <button class="btn ghost small" on:click=move |_| lower()>"Preis senken"</button>
                        })}
                    </div>
                })}
            </td>
            <td class="num">{format_eur(live_small)}</td>
            <td class="num">
                <input type="text" inputmode="decimal" placeholder="–"
                    class=move || if saved_small.get().is_some() { "cell-input changed" } else { "cell-input" }
                    prop:value=move || small.get()
                    on:input=move |ev| small.set(event_target_value(&ev))
                    on:change=on_change/>
                {move || diff_view(live_small, saved_small.get())}
            </td>
            <td class="num">{live_large.map(format_eur).unwrap_or_default()}</td>
            <td class="num">
                {live_large.map(|ll| view! {
                    <input type="text" inputmode="decimal" placeholder="–"
                        class=move || if saved_large.get().is_some() { "cell-input changed" } else { "cell-input" }
                        prop:value=move || large.get()
                        on:input=move |ev| large.set(event_target_value(&ev))
                        on:change=on_change/>
                    {move || diff_view(ll, saved_large.get())}
                })}
            </td>
            <td>
                <span class=move || if status_err.get() { "row-status error" } else { "row-status" }>
                    {move || status.get()}
                </span>
            </td>
        </tr>
    }
}

#[component]
fn ExtraRow(ex: NextPriceExtra) -> impl IntoView {
    let id = ex.id.clone();
    let live = ex.price_cents;
    let next = RwSignal::new(eur_input(ex.next_price_cents));
    let saved = RwSignal::new(ex.next_price_cents);
    let status = RwSignal::new(String::new());
    let status_err = RwSignal::new(false);

    let save = Action::new(move |c: &Option<i64>| {
        let id = id.clone();
        let c = *c;
        async move { set_next_extra_price(id, c).await }
    });
    let on_change = move |_| match parse_eur_cents(&next.get()) {
        Ok(c) => {
            let c = c.filter(|v| *v != live);
            saved.set(c);
            status_err.set(false);
            status.set("…".into());
            save.dispatch(c);
        }
        Err(e) => {
            status_err.set(true);
            status.set(e);
        }
    };
    Effect::new(move |_| {
        if let Some(r) = save.value().get() {
            match r {
                Ok(()) => {
                    status_err.set(false);
                    status.set("gespeichert".into());
                }
                Err(e) => {
                    status_err.set(true);
                    status.set(format!("Fehler: {e}"));
                }
            }
        }
    });

    view! {
        <tr>
            <td>{ex.label.clone()}</td>
            <td class="num">{format_eur(live)}</td>
            <td class="num">
                <input type="text" inputmode="decimal" placeholder="–"
                    class=move || if saved.get().is_some() { "cell-input changed" } else { "cell-input" }
                    prop:value=move || next.get()
                    on:input=move |ev| next.set(event_target_value(&ev))
                    on:change=on_change/>
                {move || diff_view(live, saved.get())}
            </td>
            <td>
                <span class=move || if status_err.get() { "row-status error" } else { "row-status" }>
                    {move || status.get()}
                </span>
            </td>
        </tr>
    }
}

#[cfg(all(test, feature = "ssr"))]
mod pizza_build_tests {
    use super::ssr::{pizza_builds, PriceItem};
    use rusterando_shared::models::PizzaExtra;

    fn extra(id: &str, label: &str, price: i64, aliases: &[&str]) -> PizzaExtra {
        PizzaExtra {
            id: id.into(),
            label: label.into(),
            price_cents: price,
            sort_order: 0,
            aliases: aliases.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn margherita_plus_extras_per_pizza() {
        let catalog = vec![
            extra("kaese", "Extra Käse", 80, &["Goudakäse"]),
            extra("tomate", "Tomaten", 80, &[]),
            extra("pilze", "Pilze", 80, &["Pilzen"]),
            extra("salami", "Salami", 80, &[]),
        ];
        let item =
            |id: &'static str, name: &'static str, desc: &'static str, s: i64, l: Option<i64>| {
                PriceItem {
                    id,
                    name,
                    description: desc,
                    small: s,
                    large: l,
                }
            };
        let items = vec![
            item(
                "m",
                "Pizza Margherita",
                "mit Tomaten und Goudakäse",
                530,
                Some(840),
            ),
            item("f", "Pizza Funghi", "mit Pilzen", 630, Some(900)),
            item(
                "s",
                "Pizza Salami",
                "mit Tomaten, Käse und Salami",
                610,
                Some(950),
            ),
            item("b", "Pizzabrötchen", "mit Käse und Salami", 400, None),
            item(
                "z",
                "Pizza Mozzarella",
                "mit Tomaten und Mozzarella",
                600,
                Some(850),
            ),
            item(
                "p",
                "Pizza Portofino",
                "mit Salami, Sardellen und Kapern",
                700,
                Some(990),
            ),
        ];
        let builds = pizza_builds(items.into_iter(), &catalog);
        let funghi = &builds["f"];
        assert_eq!((funghi.small_cents, funghi.large_cents), (610, Some(920)));
        assert_eq!(funghi.extras, vec!["Pilze".to_string()]);
        let salami = &builds["s"];
        assert_eq!((salami.small_cents, salami.large_cents), (610, Some(920)));
        assert_eq!(salami.extras, vec!["Salami".to_string()]);
        assert!(!builds.contains_key("m"), "the Margherita itself");
        assert!(!builds.contains_key("b"), "not a pizza");
        assert!(!builds.contains_key("z"), "no extra beyond the Margherita");
        assert!(funghi.comparable() && salami.comparable());
        let portofino = &builds["p"];
        assert_eq!(
            portofino.not_extras,
            vec!["Sardellen".to_string(), "Kapern".to_string()]
        );
        assert!(
            !portofino.comparable(),
            "Sardellen aren't an extra: no advice"
        );
    }

    #[test]
    fn listed_ingredients_from_descriptions() {
        use super::ssr::listed_ingredients;
        assert_eq!(
            listed_ingredients("mit Schinken, Pilzen, Peperoni, Artischocken und Tabasco"),
            vec!["Schinken", "Pilzen", "Peperoni", "Artischocken", "Tabasco"]
        );
        assert_eq!(listed_ingredients("mit Pilzen"), vec!["Pilzen"]);
        assert_eq!(listed_ingredients("Spinat & Feta"), vec!["Spinat", "Feta"]);
    }
}
