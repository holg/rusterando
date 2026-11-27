//! Tagesangebot: one menu item — a size, extras on it — under its own name
//! at its own price, on the homepage at the top of the hero with a button
//! that puts it in the cart. It runs until the admin switches it off
//! (/admin/tagesangebot).
//!
//! Kept as one JSON value in `app_settings` (`daily_offer`), no table of its
//! own. The price is the server's: [`AddDailyOffer`] writes the cart line
//! (item + extras at the offer price, the extras at 0), marked in its
//! options with the offer's name and version (`rev`, new on every save);
//! `place_order` takes the line only while that same version is on
//! ([`ssr::check_cart`]) — switched off or changed, the line goes and the
//! customer is told.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// What the admin set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DailyOffer {
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub item_id: String,
    /// "small" | "large" | "single", as the cart has it.
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub extras_ids: Vec<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub price_cents: i64,
    /// New on every save: a cart line of an older version isn't the offer.
    #[serde(default)]
    pub rev: String,
    /// Customers may add toppings on top (at the item's extra price). Off by
    /// default: an offer is usually made to use up what there is too much
    /// of, as it is.
    #[serde(default)]
    pub customer_extras: bool,
}

/// What the homepage shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyOfferView {
    pub name: String,
    /// "Pizza Salami · 30cm · mit Peperoni, Zwiebeln".
    pub detail: String,
    pub price_cents: i64,
    /// The menu price of the same (item + extras), when higher.
    pub regular_cents: Option<i64>,
    /// Toppings the customer may add on top, at the item's extra price.
    #[serde(default)]
    pub extras: Vec<OfferExtra>,
    /// The item's drawing (public/img/motif/<motif>.svg), as on the menu.
    #[serde(default)]
    pub motif: String,
}

/// A topping to add to the offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferExtra {
    pub id: String,
    pub label: String,
    pub price_cents: i64,
}

#[cfg(feature = "ssr")]
pub mod ssr {
    use super::*;
    use rusterando_shared::models::{CartExtra, SizeChoice};
    use sqlx::SqlitePool;

    pub const SETTING_KEY: &str = "daily_offer";

    pub async fn load(db: &SqlitePool) -> DailyOffer {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = ?1")
                .bind(SETTING_KEY)
                .fetch_optional(db)
                .await
                .ok()
                .flatten();
        row.and_then(|(v,)| serde_json::from_str(&v).ok())
            .unwrap_or_default()
    }

    /// Bumped on every save: the kitchen board feed pushes the e-ink
    /// picture's new tag at once (rusterando-server kitchen_board.rs).
    static CHANGED: std::sync::LazyLock<tokio::sync::watch::Sender<u64>> =
        std::sync::LazyLock::new(|| tokio::sync::watch::channel(0).0);

    /// Wakes on every save of the Tagesangebot — and on [`picture_changed`].
    pub fn changes() -> tokio::sync::watch::Receiver<u64> {
        CHANGED.subscribe()
    }

    /// Something else about the e-ink picture changed (which device drives
    /// the frame, /admin/drucker): the board feeds look again at once.
    pub fn picture_changed() {
        CHANGED.send_modify(|n| *n = n.wrapping_add(1));
    }

    pub async fn store(db: &SqlitePool, offer: &DailyOffer) -> Result<(), String> {
        let value = serde_json::to_string(offer).map_err(|e| e.to_string())?;
        sqlx::query(
            "INSERT INTO app_settings (key, value, label_de, hint_de)
             VALUES (?1, ?2, 'Tagesangebot', 'Wird unter /admin/tagesangebot bearbeitet.')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(SETTING_KEY)
        .bind(value)
        .execute(db)
        .await
        .map_err(|e| format!("speichern: {e}"))?;
        CHANGED.send_modify(|n| *n = n.wrapping_add(1));
        Ok(())
    }

    /// The offer against today's menu.
    pub struct Resolved {
        pub item_name: String,
        pub size: SizeChoice,
        pub size_label: Option<String>,
        /// The extras with their menu prices (for the regular price).
        pub extras: Vec<CartExtra>,
        pub regular_cents: i64,
        /// The item takes extras, and what one more costs on it (`None`:
        /// the catalog price).
        pub allow_extras: bool,
        pub flat_extra_cents: Option<i64>,
        /// The item's drawing (menu.rs `item_motif`).
        pub motif: &'static str,
    }

    /// Checks the offer against the menu: the item there and orderable,
    /// the size it has, the extras available. `Err`: why not, for the admin.
    pub async fn resolve(db: &SqlitePool, offer: &DailyOffer) -> Result<Resolved, String> {
        type ItemRow = (
            String,
            i64,
            Option<i64>,
            Option<String>,
            Option<String>,
            i64,
            i64,
            i64,
            i64,
            i64,
            Option<i64>,
            String,
        );
        let row: Option<ItemRow> = sqlx::query_as(
            "SELECT i.name, i.price_small_cents, i.price_large_cents,
                    i.size_small_label, i.size_large_label,
                    i.is_available, i.is_listed, c.is_active, i.allow_extras,
                    i.included_extras_count, i.flat_extra_price_cents, c.name
             FROM menu_items i JOIN menu_categories c ON c.id = i.category_id
             WHERE i.id = ?1",
        )
        .bind(&offer.item_id)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("Artikel: {e}"))?;
        let Some((
            name,
            small,
            large,
            small_label,
            large_label,
            available,
            listed,
            cat_active,
            allow_extras,
            included,
            flat,
            category,
        )) = row
        else {
            return Err("Kein Artikel gewählt".into());
        };
        if available == 0 || listed == 0 || cat_active == 0 {
            return Err(format!(
                "{name} ist gerade nicht bestellbar (ausverkauft oder nicht gelistet)"
            ));
        }
        // Required choices (a salad's dressing) the offer can't make.
        let required: Option<(String,)> = sqlx::query_as(
            "SELECT g.label FROM menu_item_option_groups m
             JOIN item_option_groups g ON g.id = m.group_id
             WHERE m.menu_item_id = ?1 AND g.is_active = 1 AND g.min_select > 0 LIMIT 1",
        )
        .bind(&offer.item_id)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("Auswahl-Gruppen: {e}"))?;
        if let Some((group,)) = required {
            return Err(format!(
                "{name} verlangt eine Auswahl ({group}) — als Tagesangebot nicht möglich"
            ));
        }
        let size = SizeChoice::parse(&offer.size);
        let (unit, size_label) = match (size, large) {
            (SizeChoice::Large, Some(p)) => (p, large_label),
            (SizeChoice::Large, None) => return Err(format!("{name} gibt es nicht groß")),
            _ => (
                small,
                if large.is_some() {
                    small_label
                } else {
                    small_label.filter(|l| !l.is_empty())
                },
            ),
        };
        // Small and large are both "small"/"large"; a one-size item is "single".
        let size = match (size, large) {
            (SizeChoice::Large, _) => SizeChoice::Large,
            (_, Some(_)) => SizeChoice::Small,
            (_, None) => SizeChoice::Single,
        };
        let mut extras = Vec::new();
        if !offer.extras_ids.is_empty() {
            if allow_extras == 0 {
                return Err(format!("{name} nimmt keine Extras"));
            }
            for id in &offer.extras_ids {
                let e: Option<(String, i64, i64)> = sqlx::query_as(
                    "SELECT label, price_cents, is_available FROM pizza_extras WHERE id = ?1",
                )
                .bind(id)
                .fetch_optional(db)
                .await
                .map_err(|e| format!("Extras: {e}"))?;
                match e {
                    Some((label, price, 1)) => extras.push(CartExtra {
                        id: id.clone(),
                        label,
                        price_cents: price,
                    }),
                    Some((label, _, _)) => return Err(format!("Extra {label} ist ausverkauft")),
                    None => return Err("Ein gewähltes Extra gibt es nicht mehr".into()),
                }
            }
            // The item's own extras rule, as add_to_cart applies it.
            if let Some(flat) = flat {
                for (i, e) in extras.iter_mut().enumerate() {
                    e.price_cents = if i < included.max(0) as usize {
                        0
                    } else {
                        flat
                    };
                }
            }
        }
        let regular_cents = unit + extras.iter().map(|e| e.price_cents).sum::<i64>();
        let motif = crate::pages::menu::item_motif(&name, &category);
        Ok(Resolved {
            item_name: name,
            size,
            size_label,
            extras,
            regular_cents,
            allow_extras: allow_extras != 0,
            flat_extra_cents: flat,
            motif,
        })
    }

    /// Toppings the customer may add: the available extras not already on
    /// the offer, at the item's flat extra price or the catalog price.
    pub async fn addable(db: &SqlitePool, offer: &DailyOffer, r: &Resolved) -> Vec<OfferExtra> {
        if !offer.customer_extras || !r.allow_extras {
            return Vec::new();
        }
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT id, label, price_cents FROM pizza_extras WHERE is_available = 1 ORDER BY sort_order, label",
        )
        .fetch_all(db)
        .await
        .unwrap_or_default();
        rows.into_iter()
            .filter(|(id, _, _)| !offer.extras_ids.contains(id))
            .map(|(id, label, price)| OfferExtra {
                id,
                label,
                price_cents: r.flat_extra_cents.unwrap_or(price),
            })
            .collect()
    }

    /// The offer, when it is on and orderable.
    pub async fn current(db: &SqlitePool) -> Option<(DailyOffer, Resolved)> {
        let offer = load(db).await;
        if !offer.active || offer.price_cents <= 0 {
            return None;
        }
        let resolved = resolve(db, &offer).await.ok()?;
        Some((offer, resolved))
    }

    pub fn detail(r: &Resolved) -> String {
        let mut parts = vec![r.item_name.clone()];
        if let Some(size) = r.size_label.as_ref().filter(|s| !s.is_empty()) {
            parts.push(size.clone());
        }
        if !r.extras.is_empty() {
            let labels: Vec<&str> = r.extras.iter().map(|e| e.label.as_str()).collect();
            parts.push(format!("mit {}", labels.join(", ")));
        }
        parts.join(" · ")
    }

    pub async fn view(db: &SqlitePool) -> Option<DailyOfferView> {
        let (offer, r) = current(db).await?;
        let extras = addable(db, &offer, &r).await;
        Some(DailyOfferView {
            name: offer.name.clone(),
            detail: detail(&r),
            price_cents: offer.price_cents,
            regular_cents: (r.regular_cents > offer.price_cents).then_some(r.regular_cents),
            extras,
            motif: r.motif.to_string(),
        })
    }

    /// Before an order: offer lines of a version no longer on leave the
    /// cart. `Err`: some went — the customer should look again.
    pub async fn check_cart(db: &SqlitePool, cart_id: &str) -> Result<(), String> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, options_json FROM cart_items WHERE cart_id = ?1")
                .bind(cart_id)
                .fetch_all(db)
                .await
                .map_err(|e| format!("Warenkorb: {e}"))?;
        let current = current(db).await.map(|(o, _)| o.rev);
        let mut stale = Vec::new();
        for (id, options_json) in rows {
            let opts: crate::pages::cart::ssr::Options =
                serde_json::from_str(&options_json).unwrap_or_default();
            if let Some(rev) = opts.offer_rev {
                if current.as_deref() != Some(rev.as_str()) {
                    stale.push(id);
                }
            }
        }
        if stale.is_empty() {
            return Ok(());
        }
        for id in &stale {
            sqlx::query("DELETE FROM cart_items WHERE id = ?1")
                .bind(id)
                .execute(db)
                .await
                .map_err(|e| format!("Warenkorb: {e}"))?;
        }
        Err("Das Tagesangebot ist beendet oder hat sich geändert und wurde aus dem Warenkorb genommen — bitte noch einmal prüfen.".into())
    }
}

/// The offer for the homepage; `None` when off.
#[server(
    name = GetDailyOffer,
    prefix = "/api",
    endpoint = "get_daily_offer"
)]
pub async fn get_daily_offer() -> Result<Option<DailyOfferView>, ServerFnError> {
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    Ok(ssr::view(&db).await)
}

/// "Tagesangebot in den Warenkorb": one more of it in the cart, at the
/// offer price.
#[server(
    name = AddDailyOffer,
    prefix = "/api",
    endpoint = "add_daily_offer"
)]
pub async fn add_daily_offer(
    /// Toppings the customer added, each at its price.
    #[server(default)]
    extras_ids: Vec<String>,
) -> Result<(), ServerFnError> {
    let db = use_context::<sqlx::SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let Some((offer, r)) = ssr::current(&db).await else {
        return Err(ServerFnError::new(
            "Das Tagesangebot ist gerade nicht verfügbar.",
        ));
    };
    let cart_id = crate::pages::cart::ssr::current_cart_id().await?;
    let opts = crate::pages::cart::ssr::Options {
        size: r.size,
        size_label: r.size_label.clone(),
        offer_name: Some(offer.name.clone()),
        offer_rev: Some(offer.rev.clone()),
    };
    let opts_json = serde_json::to_string(&opts).map_err(|e| ServerFnError::new(e.to_string()))?;
    // The extras on the line for the kitchen: the offer's (in its price),
    // then the customer's own, each at its price.
    let mut extras: Vec<rusterando_shared::models::CartExtra> = r
        .extras
        .iter()
        .map(|e| rusterando_shared::models::CartExtra {
            price_cents: 0,
            ..e.clone()
        })
        .collect();
    let addable = ssr::addable(&db, &offer, &r).await;
    for id in &extras_ids {
        let Some(e) = addable.iter().find(|e| &e.id == id) else {
            return Err(ServerFnError::new("Dieses Extra gibt es gerade nicht."));
        };
        if !extras.iter().any(|x| x.id == e.id) {
            extras.push(rusterando_shared::models::CartExtra {
                id: e.id.clone(),
                label: e.label.clone(),
                price_cents: e.price_cents,
            });
        }
    }
    let extras_json = if extras.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&extras).map_err(|e| ServerFnError::new(e.to_string()))?)
    };
    // Already in the cart: one more.
    let bumped = sqlx::query(
        "UPDATE cart_items SET quantity = MIN(quantity + 1, 50)
         WHERE cart_id = ?1 AND menu_item_id = ?2 AND options_json = ?3 AND is_giveaway = 0
           AND COALESCE(extras_json, '') = COALESCE(?4, '')",
    )
    .bind(&cart_id)
    .bind(&offer.item_id)
    .bind(&opts_json)
    .bind(extras_json.as_deref())
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("Warenkorb: {e}")))?
    .rows_affected();
    if bumped == 0 {
        sqlx::query(
            "INSERT INTO cart_items
                (id, cart_id, menu_item_id, quantity, options_json, unit_price_cents, extras_json)
             VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&cart_id)
        .bind(&offer.item_id)
        .bind(&opts_json)
        .bind(offer.price_cents)
        .bind(extras_json.as_deref())
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("Warenkorb: {e}")))?;
    }
    Ok(())
}

/// At the top of the homepage hero.
#[component]
pub fn DailyOfferHero(offer: DailyOfferView) -> impl IntoView {
    use rusterando_shared::models::format_eur;
    let cart = crate::components::cart_drawer::use_cart_ctx();
    let add = ServerAction::<AddDailyOffer>::new();
    let chosen: RwSignal<Vec<String>> = RwSignal::new(Vec::new());
    Effect::new(move |_| {
        if matches!(add.value().get(), Some(Ok(()))) {
            chosen.set(Vec::new());
            cart.refresh();
            cart.open.set(true);
        }
    });
    let error = move || match add.value().get() {
        Some(Err(e)) => Some(view! {
            <p class="daily-offer-error">{e.to_string().trim_start_matches("error running server function: ").to_string()}</p>
        }),
        _ => None,
    };
    let base = offer.price_cents;
    let extras = StoredValue::new(offer.extras.clone());
    let total = move || {
        let ids = chosen.get();
        base + extras.with_value(|all| {
            all.iter()
                .filter(|e| ids.contains(&e.id))
                .map(|e| e.price_cents)
                .sum::<i64>()
        })
    };
    let has_extras = !offer.extras.is_empty();
    view! {
        <div class="daily-offer">
            // The item's chalk drawing and the tricolour: shown by the
            // chalkboard theme only.
            <span class="chalk-motif" aria-hidden="true"
                style=format!("--motif: url('/img/motif/{}.svg');", offer.motif)></span>
            <span class="daily-offer-tag">"Tagesangebot"</span>
            <strong class="daily-offer-name">{offer.name}</strong>
            <span class="chalk-flag" aria-hidden="true"></span>
            <span class="daily-offer-detail">{offer.detail}</span>
            <span class="daily-offer-price">
                {move || format_eur(total())}
                {offer.regular_cents.map(|r| view! { <s>{format_eur(r)}</s> })}
            </span>
            {has_extras.then(|| view! {
                <details class="daily-offer-extras">
                    <summary>"+ Extras dazu"</summary>
                    <div class="daily-offer-extras-list">
                        {extras.with_value(|all| all.iter().map(|e| {
                            let id = e.id.clone();
                            let id2 = e.id.clone();
                            view! {
                                <label>
                                    <input type="checkbox"
                                        prop:checked=move || chosen.get().contains(&id)
                                        on:change=move |ev| {
                                            let on = event_target_checked(&ev);
                                            chosen.update(|c| {
                                                c.retain(|x| x != &id2);
                                                if on { c.push(id2.clone()); }
                                            });
                                        }/>
                                    <span>{format!("{} +{}", e.label, format_eur(e.price_cents))}</span>
                                </label>
                            }
                        }).collect_view())}
                    </div>
                </details>
            })}
            <button class="btn pill daily-offer-add" type="button"
                disabled=move || add.pending().get()
                on:click=move |_| { add.dispatch(AddDailyOffer { extras_ids: chosen.get_untracked() }); }>
                "Tagesangebot in den Warenkorb"
            </button>
            {error}
        </div>
    }
}
