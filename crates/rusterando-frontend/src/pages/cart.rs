//! Server-side cart bound to a `dp_cart` cookie. The cookie holds the cart's
//! row id; everything else lives in the `carts` and `cart_items` tables.

use leptos::prelude::*;
use rusterando_shared::models::CartView;

#[cfg(feature = "ssr")]
use rusterando_shared::models::{CartExtra, CartLine, CartSelectedOption, SizeChoice};

#[cfg(feature = "ssr")]
const CART_COOKIE: &str = "dp_cart";

/// The free-giveaway item: "Pizzabrötchen (6 Stück)" (seeded as mi-400) and
/// its required Sauce option group (Knoblauchsauce / Kräuterbutter, both 0 €),
/// both created by migration 20260529000002. Referenced when offering /
/// validating the giveaway line.
pub const GIVEAWAY_ITEM_ID: &str = "mi-400";
pub const GIVEAWAY_SAUCE_GROUP_ID: &str = "og-sauce-giveaway";

#[cfg(feature = "ssr")]
pub mod ssr {
    use super::*;
    use leptos_axum::extract;
    use sqlx::SqlitePool;
    use tower_cookies::{cookie::time::Duration, cookie::SameSite, Cookie, Cookies};

    /// Resolve (or create) the cart for the current request. On creation a new
    /// `dp_cart` cookie is set so subsequent requests reuse the same cart.
    pub async fn current_cart_id() -> Result<String, ServerFnError> {
        let cookies: Cookies = extract().await?;
        let db = use_context::<SqlitePool>()
            .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

        if let Some(c) = cookies.get(CART_COOKIE) {
            let id = c.value().to_string();
            if !id.is_empty() {
                // Verify the cart row still exists. After a DB wipe / reseed the
                // browser cookie can outlive its cart row, which then trips a
                // FOREIGN KEY violation on the next cart_items insert.
                let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM carts WHERE id = ?1")
                    .bind(&id)
                    .fetch_optional(&db)
                    .await
                    .map_err(|e| ServerFnError::new(format!("check cart: {e}")))?;
                if exists.is_some() {
                    return Ok(id);
                }
            }
        }

        let id = uuid::Uuid::new_v4().to_string();
        let session = id.clone(); // we have no real session yet; use cart id as session id
        sqlx::query("INSERT INTO carts (id, session_id) VALUES (?1, ?2)")
            .bind(&id)
            .bind(&session)
            .execute(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("create cart: {e}")))?;

        let mut c = Cookie::new(CART_COOKIE, id.clone());
        c.set_path("/");
        c.set_http_only(true);
        c.set_same_site(SameSite::Lax);
        c.set_max_age(Duration::days(30));
        cookies.add(c);
        Ok(id)
    }

    pub async fn load_cart(db: &SqlitePool, cart_id: &str) -> Result<CartView, ServerFnError> {
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<String>,
                String,
                String,
                i64,
                i64,
                Option<String>,
                Option<String>,
                Option<String>,
                i64,
            ),
        >(
            // Locale-aware item name: COALESCE(mi.name_<lang>, mi.name)
            // so the cart line shows the customer's chosen language.
            // The cart_items row itself doesn't snapshot the name; it
            // references menu_item_id, so live JOIN here is correct.
            &{
                let loc = crate::i18n::current_locale();
                let name_col = if loc == crate::i18n::Locale::DEFAULT {
                    "mi.name".to_string()
                } else {
                    format!("COALESCE(mi.name_{lang}, mi.name)", lang = loc.code())
                };
                format!(
                    "SELECT ci.id,
                            ci.menu_item_id,
                            mi.menu_number,
                            {name_col} AS name,
                            ci.options_json,
                            ci.quantity,
                            ci.unit_price_cents,
                            ci.extras_json,
                            ci.selected_options_json,
                            ci.removals_json,
                            ci.is_giveaway
                     FROM cart_items ci
                     JOIN menu_items mi ON mi.id = ci.menu_item_id
                     WHERE ci.cart_id = ?1
                     ORDER BY ci.created_at"
                )
            },
        )
        .bind(cart_id)
        .fetch_all(db)
        .await
        .map_err(|e| ServerFnError::new(format!("load cart: {e}")))?;

        let mut lines = Vec::with_capacity(rows.len());
        let mut subtotal = 0_i64;
        // Subtotal of PAID lines only — the giveaway line (0 €) must not count
        // toward its own qualifying threshold.
        let mut paid_subtotal = 0_i64;
        let mut item_count = 0_i64;
        let mut has_giveaway_line = false;

        for (
            id,
            menu_item_id,
            menu_number,
            name,
            options_json,
            quantity,
            unit_price_cents,
            extras_json,
            selected_options_json,
            removals_json,
            is_giveaway_i,
        ) in rows
        {
            let opts: Options = serde_json::from_str(&options_json).unwrap_or_default();
            let extras: Vec<CartExtra> = extras_json
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_default();
            let selected_options: Vec<CartSelectedOption> = selected_options_json
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_default();
            let removals: Vec<rusterando_shared::models::CartRemoval> = removals_json
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_default();
            let is_giveaway = is_giveaway_i != 0;
            let extras_unit_cents: i64 = extras.iter().map(|e| e.price_cents).sum();
            let options_unit_cents: i64 = selected_options.iter().map(|o| o.price_cents).sum();
            // A giveaway line is free regardless of the item's catalog price.
            let line_total = if is_giveaway {
                0
            } else {
                (unit_price_cents + extras_unit_cents + options_unit_cents) * quantity
            };
            subtotal += line_total;
            if is_giveaway {
                has_giveaway_line = true;
            } else {
                paid_subtotal += line_total;
            }
            item_count += quantity;
            lines.push(CartLine {
                id,
                menu_item_id,
                menu_number,
                name,
                size: opts.size,
                size_label: opts.size_label,
                quantity,
                unit_price_cents: if is_giveaway { 0 } else { unit_price_cents },
                extras_unit_cents,
                extras,
                selected_options,
                removals,
                line_total_cents: line_total,
                is_giveaway,
            });
        }

        let free_delivery_threshold_cents =
            crate::pages::settings::ssr::free_delivery_threshold_cents(db).await;

        // Closed state via the canonical helper (pause/snooze/Ruhetag/hours).
        let (orders_closed, _) = crate::pages::order::ssr::shop_closed_state(db).await;

        // Giveaway offer state. The cart can't know the order type yet (chosen
        // at checkout), so we gate only on enabled + paid subtotal here; the
        // order-type restriction is enforced authoritatively in place_order.
        let gcfg = crate::pages::settings::ssr::giveaway_config(db).await;
        let giveaway = if gcfg.enabled {
            Some(rusterando_shared::models::GiveawayOffer {
                min_order_cents: gcfg.min_order_cents,
                qualifies: paid_subtotal >= gcfg.min_order_cents,
                claimed: has_giveaway_line,
                item_id: gcfg.item_id.clone(),
            })
        } else {
            None
        };

        Ok(CartView {
            lines,
            subtotal_cents: subtotal,
            item_count,
            free_delivery_threshold_cents,
            orders_closed,
            giveaway,
        })
    }

    /// Persisted shape of `cart_items.options_json`. Kept private to the
    /// server module so the wire format stays opaque to the frontend.
    #[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
    pub struct Options {
        #[serde(default)]
        pub size: SizeChoice,
        pub size_label: Option<String>,
    }
}

#[server(
    name = GetCart,
    prefix = "/api",
    endpoint = "get_cart"
)]
pub async fn get_cart() -> Result<CartView, ServerFnError> {
    use sqlx::SqlitePool;
    let cart_id = ssr::current_cart_id().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    ssr::load_cart(&db, &cart_id).await
}

#[server(
    name = AddToCart,
    prefix = "/api",
    endpoint = "add_to_cart"
)]
pub async fn add_to_cart(
    menu_item_id: String,
    size: String, // "small" | "large" | "single"
    quantity: i64,
    /// Catalog ids of extras (Tabasco, Extra Käse, …) the customer ticked.
    /// Server resolves them to current price + label and snapshots; price
    /// changes later don't rewrite history. Empty for non-pizza items.
    #[server(default)]
    extras_ids: Vec<String>,
    /// Catalog ids of options the customer picked from required-choice
    /// groups (e.g. a salad's dressing). Server resolves to labels +
    /// prices, validates min/max per group, snapshots. Empty when the
    /// item has no option groups attached.
    #[server(default)]
    selected_option_ids: Vec<String>,
    /// Catalog ids of ingredients the customer wants left OFF ("ohne X").
    /// Server validates each against THIS item's removable set (derived
    /// from its description × the extras catalog) and snapshots the bare
    /// label at price 0. Empty for items with no removable ingredients.
    #[server(default)]
    removal_ids: Vec<String>,
) -> Result<CartView, ServerFnError> {
    use sqlx::SqlitePool;

    if !(1..=50).contains(&quantity) {
        return Err(ServerFnError::new("ungültige Menge"));
    }

    let size = SizeChoice::parse(&size);
    let cart_id = ssr::current_cart_id().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Look up the item and decide which price applies.
    // Tail two columns drive the per-item flat-extras model:
    //   * `included_extras_count` — first N extras free.
    //   * `flat_extra_price_cents` — flat per-extra charge when set.
    //                                NULL = use pizza_extras catalog.
    type MenuItemRow = (
        i64,
        Option<i64>,
        Option<String>,
        Option<String>,
        i64,
        String,
        i64,
        Option<i64>,
    );
    let row: MenuItemRow = sqlx::query_as(
        "SELECT price_small_cents, price_large_cents,
                size_small_label, size_large_label,
                is_available, name,
                included_extras_count, flat_extra_price_cents
         FROM menu_items WHERE id = ?1",
    )
    .bind(&menu_item_id)
    .fetch_one(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("Artikel nicht gefunden: {e}")))?;

    let (
        small,
        large,
        small_label,
        large_label,
        available,
        _name,
        included_extras_count,
        flat_extra_price_cents,
    ) = row;
    if available == 0 {
        return Err(ServerFnError::new("Artikel derzeit nicht verfügbar"));
    }

    let (unit_price, size_label) = match size {
        SizeChoice::Large => match large {
            Some(p) => (p, large_label.clone()),
            None => return Err(ServerFnError::new("Große Größe nicht verfügbar")),
        },
        SizeChoice::Small | SizeChoice::Single => (small, small_label.clone()),
    };

    let opts = ssr::Options { size, size_label };
    let opts_json = serde_json::to_string(&opts)
        .map_err(|e| ServerFnError::new(format!("serialise options: {e}")))?;

    // Resolve picked extras against the current catalog. Snapshot the label
    // and price into the cart line so subsequent admin price edits don't
    // rewrite the customer's order. Unknown / unavailable ids are silently
    // dropped — better than a 500 if the catalog has changed mid-session.
    //
    // The DB returns rows in catalog sort_order, but we want to preserve
    // the customer's TICK order so the per-item "first N free" rule
    // applies to what they actually picked first. Re-sort by index in
    // `extras_ids` after the fetch.
    let extras_snapshot: Vec<CartExtra> = if extras_ids.is_empty() {
        Vec::new()
    } else {
        let placeholders = extras_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        // Snapshot the extras label in the customer's active locale so
        // the cart line + receipt + kitchen print show consistent text
        // regardless of when the cart is later viewed. The snapshot is
        // intentional: a later DB rename shouldn't rewrite an open
        // customer's order. v1 also doesn't refresh on locale switch
        // (the customer would need to re-add to pick up a new locale).
        let label_col = crate::i18n::coalesce_col("label", "");
        let q = format!(
            "SELECT id, {label_col} AS label, price_cents
             FROM pizza_extras
             WHERE is_available = 1 AND id IN ({placeholders})"
        );
        let mut query = sqlx::query_as::<_, (String, String, i64)>(&q);
        for id in &extras_ids {
            query = query.bind(id);
        }
        let mut catalog: Vec<CartExtra> = query
            .fetch_all(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("resolve extras: {e}")))?
            .into_iter()
            .map(|(id, label, price_cents)| CartExtra {
                id,
                label,
                price_cents,
            })
            .collect();

        // Re-sort to match the customer's selection order.
        let order_index: std::collections::HashMap<&str, usize> = extras_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        catalog.sort_by_key(|e| {
            order_index
                .get(e.id.as_str())
                .copied()
                .unwrap_or(usize::MAX)
        });

        // Apply the per-item flat-pricing rule when set:
        //   * first `included_extras_count` get price_cents = 0 (free).
        //   * the rest get price_cents = flat_extra_price_cents.
        // When `flat_extra_price_cents` is NULL we keep the catalog
        // prices (existing behaviour for normal pizzas).
        if let Some(flat) = flat_extra_price_cents {
            let included = included_extras_count.max(0) as usize;
            for (idx, e) in catalog.iter_mut().enumerate() {
                e.price_cents = if idx < included { 0 } else { flat };
            }
        }
        catalog
    };
    let extras_json = if extras_snapshot.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&extras_snapshot)
                .map_err(|e| ServerFnError::new(format!("serialise extras: {e}")))?,
        )
    };

    // Resolve "ohne X" removals. We re-derive the item's removable set
    // server-side (description × extras catalog) and accept only ids in it,
    // so a client can't remove an ingredient the pizza doesn't have. The
    // snapshot stores the BARE label ("Käse"), always free.
    let removals_json: Option<String> = if removal_ids.is_empty() {
        None
    } else {
        use rusterando_shared::models::{match_removable_ingredients, CartRemoval};
        // Item description (locale-aware, same as the name above).
        let desc_col = crate::i18n::coalesce_col("description", "");
        let description: String = sqlx::query_scalar(&format!(
            "SELECT COALESCE({desc_col}, '') FROM menu_items WHERE id = ?1"
        ))
        .bind(&menu_item_id)
        .fetch_optional(&db)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        // Full available catalog (with aliases) for the match.
        let label_col = crate::i18n::coalesce_col("label", "");
        let catalog =
            crate::pages::admin::extras_admin::ssr::load_catalog_with_aliases(&db, &label_col)
                .await;
        let removable = match_removable_ingredients(&description, &catalog);
        // Keep only requested ids that are genuinely removable, in the
        // removable set's (catalog) order.
        let snapshot: Vec<CartRemoval> = removable
            .into_iter()
            .filter(|r| removal_ids.iter().any(|id| id == &r.id))
            .map(|r| CartRemoval {
                id: r.id,
                label: r.bare_label,
            })
            .collect();
        if snapshot.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&snapshot)
                    .map_err(|e| ServerFnError::new(format!("serialise removals: {e}")))?,
            )
        }
    };

    // Resolve picked options against item_option_groups + item_options.
    // We need to know:
    //   1. which groups this item has (for min/max validation)
    //   2. which group each chosen option_id belongs to (to count
    //      picks per group)
    //   3. the snapshot label + price for the receipt
    type GroupRow = (String, String, i64, i64);
    let group_rows: Vec<GroupRow> = sqlx::query_as(
        "SELECT g.id, g.label, g.min_select, g.max_select
         FROM item_option_groups g
         JOIN menu_item_option_groups mg ON mg.group_id = g.id
         WHERE mg.menu_item_id = ?1 AND g.is_active = 1
         ORDER BY g.sort_order, g.id",
    )
    .bind(&menu_item_id)
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load option groups: {e}")))?;

    let mut selected_options: Vec<CartSelectedOption> = Vec::new();
    if !selected_option_ids.is_empty() {
        let placeholders = selected_option_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        // Locale-aware snapshot of option + group labels (same
        // rationale as extras above).
        let loc = crate::i18n::current_locale();
        let (opt_label, grp_label) = if loc == crate::i18n::Locale::DEFAULT {
            ("o.label".to_string(), "g.label".to_string())
        } else {
            let lang = loc.code();
            (
                format!("COALESCE(o.label_{lang}, o.label)"),
                format!("COALESCE(g.label_{lang}, g.label)"),
            )
        };
        let q = format!(
            "SELECT o.id, {opt_label} AS label, o.price_cents, o.group_id, {grp_label} AS group_label
             FROM item_options o
             JOIN item_option_groups g ON g.id = o.group_id
             WHERE o.is_active = 1 AND g.is_active = 1
               AND o.id IN ({placeholders})"
        );
        let mut query = sqlx::query_as::<_, (String, String, i64, String, String)>(&q);
        for id in &selected_option_ids {
            query = query.bind(id);
        }
        let resolved = query
            .fetch_all(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("resolve options: {e}")))?;

        // Reject picks from groups not attached to this item — protects
        // against clients submitting arbitrary option ids by URL hack.
        let allowed_group_ids: std::collections::HashSet<&str> =
            group_rows.iter().map(|(id, _, _, _)| id.as_str()).collect();
        for (oid, olabel, price_cents, gid, glabel) in resolved {
            if !allowed_group_ids.contains(gid.as_str()) {
                return Err(ServerFnError::new(format!(
                    "Option {oid} gehört nicht zu einer für diesen Artikel verfügbaren Gruppe"
                )));
            }
            selected_options.push(CartSelectedOption {
                group_id: gid,
                group_label: glabel,
                option_id: oid,
                option_label: olabel,
                price_cents,
            });
        }
    }

    // Validate min/max per attached group against the picks we got.
    for (gid, glabel, min_sel, max_sel) in &group_rows {
        let picked_count = selected_options
            .iter()
            .filter(|o| &o.group_id == gid)
            .count() as i64;
        if picked_count < *min_sel {
            return Err(ServerFnError::new(format!(
                "Bitte mindestens {min_sel} Auswahl bei \"{glabel}\""
            )));
        }
        if picked_count > *max_sel {
            return Err(ServerFnError::new(format!(
                "Höchstens {max_sel} Auswahl bei \"{glabel}\" möglich"
            )));
        }
    }

    let selected_options_json = if selected_options.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&selected_options)
                .map_err(|e| ServerFnError::new(format!("serialise options: {e}")))?,
        )
    };

    // Merge with an existing identical line (same item + same options + same
    // extras + same option picks). Different extras → new line, even if the
    // base item matches.
    //
    // `is_giveaway = 0`: NEVER merge a paid add into the free giveaway line.
    // The giveaway adds mi-400 (Pizzabrötchen) at qty 1 / price 0 with a
    // chosen sauce; a customer ordering the SAME item with the SAME sauce as a
    // PAID line matches every other column, so without this guard the paid
    // quantity was summed onto the 0€ giveaway line — handing out 3 free
    // instead of 1 (the DP-2506-0002 bug). Keep the two lines distinct.
    let existing: Option<(String, i64)> = sqlx::query_as(
        "SELECT id, quantity FROM cart_items
         WHERE cart_id = ?1 AND menu_item_id = ?2 AND options_json = ?3
               AND COALESCE(extras_json, '') = COALESCE(?4, '')
               AND COALESCE(selected_options_json, '') = COALESCE(?5, '')
               AND COALESCE(removals_json, '') = COALESCE(?6, '')
               AND is_giveaway = 0",
    )
    .bind(&cart_id)
    .bind(&menu_item_id)
    .bind(&opts_json)
    .bind(extras_json.as_deref())
    .bind(selected_options_json.as_deref())
    .bind(removals_json.as_deref())
    .fetch_optional(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("lookup existing: {e}")))?;

    if let Some((line_id, existing_qty)) = existing {
        sqlx::query("UPDATE cart_items SET quantity = ?1 WHERE id = ?2")
            .bind(existing_qty + quantity)
            .bind(&line_id)
            .execute(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("merge line: {e}")))?;
    } else {
        let line_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO cart_items
                (id, cart_id, menu_item_id, quantity, options_json,
                 unit_price_cents, extras_json, selected_options_json, removals_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(&line_id)
        .bind(&cart_id)
        .bind(&menu_item_id)
        .bind(quantity)
        .bind(&opts_json)
        .bind(unit_price)
        .bind(extras_json.as_deref())
        .bind(selected_options_json.as_deref())
        .bind(removals_json.as_deref())
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("insert line: {e}")))?;
    }

    sqlx::query("UPDATE carts SET updated_at = CURRENT_TIMESTAMP WHERE id = ?1")
        .bind(&cart_id)
        .execute(&db)
        .await
        .ok();

    ssr::load_cart(&db, &cart_id).await
}

/// Claim the free Pizzabrötchen giveaway. Adds ONE giveaway line (mi-400,
/// price 0, is_giveaway=1) with the chosen sauce. Fully server-validated:
/// the promo must be enabled, the cart's PAID subtotal must meet the
/// threshold, the sauce must belong to the giveaway Sauce group, and only one
/// giveaway line is allowed. A client cannot fabricate a free item by any
/// other route — this is the only path that sets is_giveaway. (Order-type is
/// only known at checkout, so it's enforced again in place_order.)
#[server(
    name = ClaimGiveaway,
    prefix = "/api",
    endpoint = "claim_giveaway"
)]
pub async fn claim_giveaway(sauce_option_id: String) -> Result<CartView, ServerFnError> {
    use sqlx::SqlitePool;
    let cart_id = ssr::current_cart_id().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // 1) Promo enabled?
    let gcfg = crate::pages::settings::ssr::giveaway_config(&db).await;
    if !gcfg.enabled {
        return Err(ServerFnError::new(
            "Die Gratis-Beigabe ist derzeit nicht verfügbar.",
        ));
    }

    // 2) Recompute the PAID subtotal from the DB (authoritative — never trust
    //    a client number) and check the threshold. Also detect an existing
    //    giveaway line so we never add a second.
    #[allow(clippy::type_complexity)]
    let rows: Vec<(i64, i64, Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT ci.quantity, ci.unit_price_cents, ci.extras_json,
                ci.selected_options_json, ci.is_giveaway
         FROM cart_items ci WHERE ci.cart_id = ?1",
    )
    .bind(&cart_id)
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load cart: {e}")))?;

    let mut paid_subtotal = 0_i64;
    let mut already_claimed = false;
    for (qty, unit, extras_json, opts_json, is_give) in &rows {
        if *is_give != 0 {
            already_claimed = true;
            continue;
        }
        let extras_unit: i64 = extras_json
            .as_deref()
            .and_then(|j| serde_json::from_str::<Vec<CartExtra>>(j).ok())
            .map(|v| v.iter().map(|e| e.price_cents).sum())
            .unwrap_or(0);
        let opts_unit: i64 = opts_json
            .as_deref()
            .and_then(|j| serde_json::from_str::<Vec<CartSelectedOption>>(j).ok())
            .map(|v| v.iter().map(|o| o.price_cents).sum())
            .unwrap_or(0);
        paid_subtotal += (unit + extras_unit + opts_unit) * qty;
    }

    if already_claimed {
        // Idempotent: just return the current cart.
        return ssr::load_cart(&db, &cart_id).await;
    }
    if paid_subtotal < gcfg.min_order_cents {
        return Err(ServerFnError::new(
            "Der Mindestbestellwert für die Gratis-Beigabe ist noch nicht erreicht.",
        ));
    }

    // 3) Validate the chosen sauce belongs to the giveaway Sauce group and
    //    snapshot its (locale-aware) label. Reject anything else.
    let loc = crate::i18n::current_locale();
    let (opt_label, grp_label) = if loc == crate::i18n::Locale::DEFAULT {
        ("o.label".to_string(), "g.label".to_string())
    } else {
        let lang = loc.code();
        (
            format!("COALESCE(o.label_{lang}, o.label)"),
            format!("COALESCE(g.label_{lang}, g.label)"),
        )
    };
    let sauce: Option<(String, String, i64, String)> = sqlx::query_as(&format!(
        "SELECT o.id, {opt_label} AS label, o.price_cents, {grp_label} AS group_label
         FROM item_options o
         JOIN item_option_groups g ON g.id = o.group_id
         WHERE o.id = ?1 AND o.group_id = ?2 AND o.is_active = 1 AND g.is_active = 1",
    ))
    .bind(&sauce_option_id)
    .bind(GIVEAWAY_SAUCE_GROUP_ID)
    .fetch_optional(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("resolve sauce: {e}")))?;

    let (oid, olabel, oprice, glabel) =
        sauce.ok_or_else(|| ServerFnError::new("Bitte eine gültige Sauce wählen."))?;

    let selected = vec![CartSelectedOption {
        group_id: GIVEAWAY_SAUCE_GROUP_ID.to_string(),
        group_label: glabel,
        option_id: oid,
        option_label: olabel,
        price_cents: oprice, // 0 € — sauce never adds cost
    }];
    let selected_json = serde_json::to_string(&selected)
        .map_err(|e| ServerFnError::new(format!("serialise sauce: {e}")))?;
    let opts_json = serde_json::to_string(&ssr::Options {
        size: SizeChoice::Single,
        size_label: None,
    })
    .map_err(|e| ServerFnError::new(format!("serialise opts: {e}")))?;

    // 4) Insert the single free line. unit_price 0, is_giveaway 1.
    let line_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO cart_items
            (id, cart_id, menu_item_id, quantity, options_json,
             unit_price_cents, extras_json, selected_options_json, is_giveaway)
         VALUES (?1, ?2, ?3, 1, ?4, 0, NULL, ?5, 1)",
    )
    .bind(&line_id)
    .bind(&cart_id)
    .bind(&gcfg.item_id)
    .bind(&opts_json)
    .bind(&selected_json)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("insert giveaway: {e}")))?;

    sqlx::query("UPDATE carts SET updated_at = CURRENT_TIMESTAMP WHERE id = ?1")
        .bind(&cart_id)
        .execute(&db)
        .await
        .ok();

    ssr::load_cart(&db, &cart_id).await
}

#[server(
    name = UpdateCartLine,
    prefix = "/api",
    endpoint = "update_cart_line"
)]
pub async fn update_cart_line(line_id: String, quantity: i64) -> Result<CartView, ServerFnError> {
    use sqlx::SqlitePool;
    if !(0..=50).contains(&quantity) {
        return Err(ServerFnError::new("ungültige Menge"));
    }
    let cart_id = ssr::current_cart_id().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    if quantity == 0 {
        sqlx::query("DELETE FROM cart_items WHERE id = ?1 AND cart_id = ?2")
            .bind(&line_id)
            .bind(&cart_id)
            .execute(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("delete line: {e}")))?;
    } else {
        sqlx::query("UPDATE cart_items SET quantity = ?1 WHERE id = ?2 AND cart_id = ?3")
            .bind(quantity)
            .bind(&line_id)
            .bind(&cart_id)
            .execute(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("update line: {e}")))?;
    }

    ssr::load_cart(&db, &cart_id).await
}

#[server(
    name = ClearCart,
    prefix = "/api",
    endpoint = "clear_cart"
)]
pub async fn clear_cart() -> Result<CartView, ServerFnError> {
    use sqlx::SqlitePool;
    let cart_id = ssr::current_cart_id().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query("DELETE FROM cart_items WHERE cart_id = ?1")
        .bind(&cart_id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("clear cart: {e}")))?;
    Ok(CartView::default())
}
