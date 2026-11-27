//! Editable home-page content (hero, offers, gallery) backed by
//! `home_hero`/`home_offers`/`home_gallery`. Public read fn used by
//! the home page; admin-gated mutations used by /admin/home.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hero {
    pub title: String,
    pub subtitle: String,
    /// Hero background. Either a path to an image (`/img/foo.jpg`,
    /// `/img/uploads/<hash>.jpg`) OR a `gradient:<key>` string where
    /// `<key>` is one of `HERO_PRESETS[*].key`. Empty / `None` falls
    /// back to the warm theme's `--hero-bg` CSS variable.
    pub image_path: Option<String>,
}

/// Built-in hero background presets the admin can pick from instead of
/// uploading a photo. The CSS string lands directly in a `background:`
/// rule on `.hero-bg`. Keep the list short and visually distinct; the admin
/// can always override with a photo.
pub struct HeroPreset {
    pub key: &'static str,
    pub label_de: &'static str,
    pub css: &'static str,
}

pub const HERO_PRESETS: &[HeroPreset] = &[
    HeroPreset {
        key: "black",
        label_de: "Tiefschwarz",
        css: "#0a0a0a",
    },
    HeroPreset {
        key: "white",
        label_de: "Weiß",
        css: "#ffffff",
    },
    HeroPreset {
        key: "white-to-black",
        label_de: "Weiß → Schwarz",
        css: "linear-gradient(180deg, #ffffff 0%, #0a0a0a 100%)",
    },
    HeroPreset {
        key: "black-to-white",
        label_de: "Schwarz → Weiß",
        css: "linear-gradient(180deg, #0a0a0a 0%, #ffffff 100%)",
    },
    HeroPreset {
        key: "italian",
        label_de: "Italien (Grün/Weiß/Rot)",
        css: "linear-gradient(90deg, #008c45 0%, #ffffff 50%, #cd212a 100%)",
    },
    HeroPreset {
        key: "warm-fade",
        label_de: "Warmes Beige",
        css: "linear-gradient(180deg, #fff8f0 0%, #faead4 100%)",
    },
    HeroPreset {
        key: "dark-pizza",
        label_de: "Dunkler Ofen",
        css: "radial-gradient(ellipse at 30% 30%, #2a1014 0%, #0d0709 70%)",
    },
];

/// Resolve a hero background spec to:
///   * `(Some(image_url), None)`        — render as `<img src=...>`
///   * `(None, Some(css_background))`   — render as `background: ...`
///   * `(None, None)`                   — fall back to theme default
///
/// Treats `image_path` strings starting with `gradient:` as preset
/// references; everything else (including absolute URLs) is an image
/// path.
pub fn resolve_hero_background(image_path: &Option<String>) -> (Option<String>, Option<String>) {
    let Some(value) = image_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return (None, None);
    };
    if let Some(key) = value.strip_prefix("gradient:") {
        let preset = HERO_PRESETS.iter().find(|p| p.key == key);
        return (None, preset.map(|p| p.css.to_string()));
    }
    (Some(value.to_string()), None)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Offer {
    pub id: String,
    pub title: String,
    pub price_label: String,
    pub blurb: String,
    pub add_on: Option<String>,
    pub image_path: Option<String>,
    pub active: bool,
    pub position: i64,
    /// The menu item this card is, as the admin linked it (`None`: a card
    /// with fixed texts only).
    #[serde(default)]
    pub menu_item_id: Option<String>,
    /// The orderable item behind the card (the linked one, or one of the
    /// same name): its "In den Warenkorb" opens the item's card.
    #[serde(default)]
    pub item_id: Option<String>,
    /// "mit 3 Zutaten nach Wahl", live from the linked item.
    #[serde(default)]
    pub included_line: Option<String>,
    /// The item's drawing (public/img/motif/<motif>.svg), as on the menu.
    #[serde(default)]
    pub motif: Option<String>,
}

/// A menu item to link an offer card to (admin).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OfferItemChoice {
    pub id: String,
    /// "240 · Pizzablech 60x40cm (Pizza Spezialitäten …)".
    pub label: String,
    /// What the card shows from it: "28 € · mit 3 Zutaten nach Wahl · …".
    pub preview: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GalleryItem {
    pub id: String,
    pub image_path: String,
    pub caption: String,
    pub position: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HomeContent {
    pub hero: Hero,
    /// Active offers only — rendered on /. Admin endpoints return all
    /// offers including inactive ones via `list_offers_admin`.
    pub offers: Vec<Offer>,
    pub gallery: Vec<GalleryItem>,
    /// The PDF menu's front cover as a picture, shown beside the hero title.
    /// The server writes it with the menu's static PDF cache (a poster cover
    /// with today's prices, `home_cover_file`); `None` when there is none or
    /// `home_show_cover` is off.
    #[serde(default)]
    pub cover_image: Option<String>,
    /// The Tagesangebot, when on (daily_offer.rs): at the top of the hero.
    #[serde(default)]
    pub daily_offer: Option<crate::pages::daily_offer::DailyOfferView>,
    /// The menu items the offer cards link to, and the extras catalog: the
    /// cards' "In den Warenkorb" opens the menu's item card in a dialog.
    #[serde(default)]
    pub offer_items: Vec<rusterando_shared::models::MenuItem>,
    #[serde(default)]
    pub extras: Vec<rusterando_shared::models::PizzaExtra>,
}

/// Single read used by the public home page and the admin editor seed.
/// Returns active offers only; the admin page hits `list_offers_admin`
/// for the full set including inactive rows.
#[server(
    name = GetHomeContent,
    prefix = "/api",
    endpoint = "get_home_content"
)]
pub async fn get_home_content() -> Result<HomeContent, ServerFnError> {
    use sqlx::SqlitePool;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let hero: (String, String, Option<String>) =
        sqlx::query_as("SELECT title, subtitle, image_path FROM home_hero WHERE id = 1")
            .fetch_one(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("hero: {e}")))?;

    let mut offers = ssr::offers_active(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("offers: {e}")))?;
    ssr::link_offers(&db, &mut offers).await;
    let wanted: Vec<&str> = offers.iter().filter_map(|o| o.item_id.as_deref()).collect();
    let (offer_items, extras) = if wanted.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let items = crate::pages::menu::load_items(&db, false)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|it| wanted.contains(&it.id.as_str()))
            .collect();
        let extras = crate::pages::admin::extras_admin::list_pizza_extras()
            .await
            .unwrap_or_default();
        (items, extras)
    };
    let gallery = ssr::gallery(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("gallery: {e}")))?;

    let cover: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM app_settings WHERE key IN ('home_cover_file', 'home_show_cover')",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("cover: {e}")))?;
    let setting = |k: &str| {
        cover
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default()
    };
    let file = setting("home_cover_file");
    let shown = setting("home_show_cover") != "0";
    let cover_image = (shown && !file.is_empty() && !file.contains(['/', '\\']))
        .then(|| format!("/img/uploads/covers/{file}"));

    Ok(HomeContent {
        hero: Hero {
            title: hero.0,
            subtitle: hero.1,
            image_path: hero.2,
        },
        offers,
        gallery,
        cover_image,
        daily_offer: crate::pages::daily_offer::ssr::view(&db).await,
        offer_items,
        extras,
    })
}

/// Admin-only: every offer including inactive ones, used by the editor.
#[server(
    name = ListOffersAdmin,
    prefix = "/api",
    endpoint = "list_offers_admin"
)]
pub async fn list_offers_admin() -> Result<Vec<Offer>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    ssr::offers_all(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("offers: {e}")))
}

#[server(
    name = UpdateHero,
    prefix = "/api",
    endpoint = "update_hero"
)]
pub async fn update_hero(
    title: String,
    subtitle: String,
    image_path: String,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;

    let title = title.trim();
    let subtitle = subtitle.trim();
    if title.is_empty() {
        return Err(ServerFnError::new("Titel darf nicht leer sein."));
    }
    if subtitle.is_empty() {
        return Err(ServerFnError::new("Untertitel darf nicht leer sein."));
    }
    let image = image_path.trim();
    let image_opt = if image.is_empty() { None } else { Some(image) };

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query(
        "UPDATE home_hero
         SET title = ?1, subtitle = ?2, image_path = ?3,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = 1",
    )
    .bind(title)
    .bind(subtitle)
    .bind(image_opt)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("update hero: {e}")))?;
    Ok(())
}

/// Admin: the orderable menu items an offer card can link to, with what
/// the card would show from each.
#[server(
    name = ListOfferItemChoices,
    prefix = "/api",
    endpoint = "list_offer_item_choices"
)]
pub async fn list_offer_item_choices() -> Result<Vec<OfferItemChoice>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    type Row = (
        String,
        Option<String>,
        String,
        String,
        i64,
        Option<i64>,
        i64,
        Option<i64>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT i.id, i.menu_number, i.name, c.name, i.price_small_cents, i.price_large_cents,
                i.included_extras_count, i.flat_extra_price_cents
         FROM menu_items i JOIN menu_categories c ON c.id = i.category_id
         WHERE i.is_available = 1 AND i.is_listed = 1 AND c.is_active = 1
         ORDER BY c.sort_order, i.sort_order,
                  CAST(COALESCE(i.menu_number, '') AS INTEGER), i.menu_number",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("Speisekarte: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|(id, nr, name, cat, small, large, incl, flat)| {
            let (price, included, extra) = ssr::item_lines(small, large, incl, flat);
            let preview = [Some(price), included, extra]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            let label = match nr.filter(|n| !n.is_empty()) {
                Some(nr) => format!("{nr} · {name} ({cat})"),
                None => format!("{name} ({cat})"),
            };
            OfferItemChoice { id, label, preview }
        })
        .collect())
}

#[server(
    name = UpsertOffer,
    prefix = "/api",
    endpoint = "upsert_offer"
)]
#[allow(clippy::too_many_arguments)]
pub async fn upsert_offer(
    id: String,
    title: String,
    price_label: String,
    blurb: String,
    add_on: String,
    image_path: String,
    active: bool,
    position: i64,
    /// The menu item the card is (empty: a card with fixed texts). Linked,
    /// its price and topping lines come live from the item.
    #[server(default)]
    menu_item_id: String,
) -> Result<String, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;

    let title = title.trim();
    let price_label = price_label.trim();
    let blurb = blurb.trim();
    let menu_item_id = menu_item_id.trim();
    let linked = (!menu_item_id.is_empty()).then_some(menu_item_id);
    if linked.is_none() && (title.is_empty() || price_label.is_empty()) {
        return Err(ServerFnError::new(
            "Ohne Menü-Artikel brauchen Titel und Preis einen Text.",
        ));
    }
    let add_on = add_on.trim();
    let add_on_opt = if add_on.is_empty() {
        None
    } else {
        Some(add_on)
    };
    let image = image_path.trim();
    let image_opt = if image.is_empty() { None } else { Some(image) };

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let id_trim = id.trim().to_string();
    let final_id = if id_trim.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        id_trim
    };

    sqlx::query(
        "INSERT INTO home_offers
            (id, title, price_label, blurb, add_on, image_path, active, position, menu_item_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
            menu_item_id = excluded.menu_item_id,
            title       = excluded.title,
            price_label = excluded.price_label,
            blurb       = excluded.blurb,
            add_on      = excluded.add_on,
            image_path  = excluded.image_path,
            active      = excluded.active,
            position    = excluded.position,
            updated_at  = CURRENT_TIMESTAMP",
    )
    .bind(&final_id)
    .bind(title)
    .bind(price_label)
    .bind(blurb)
    .bind(add_on_opt)
    .bind(image_opt)
    .bind(if active { 1 } else { 0 })
    .bind(position)
    .bind(linked)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("upsert offer: {e}")))?;

    Ok(final_id)
}

#[server(
    name = DeleteOffer,
    prefix = "/api",
    endpoint = "delete_offer"
)]
pub async fn delete_offer(id: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query("DELETE FROM home_offers WHERE id = ?1")
        .bind(id.trim())
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("delete offer: {e}")))?;
    Ok(())
}

#[server(
    name = UpsertGalleryItem,
    prefix = "/api",
    endpoint = "upsert_gallery_item"
)]
pub async fn upsert_gallery_item(
    id: String,
    image_path: String,
    caption: String,
    position: i64,
) -> Result<String, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;

    let image_path = image_path.trim();
    let caption = caption.trim();
    if image_path.is_empty() {
        return Err(ServerFnError::new("Bildpfad fehlt."));
    }
    if caption.is_empty() {
        return Err(ServerFnError::new("Bildunterschrift darf nicht leer sein."));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let id_trim = id.trim().to_string();
    let final_id = if id_trim.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        id_trim
    };

    sqlx::query(
        "INSERT INTO home_gallery (id, image_path, caption, position)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            image_path = excluded.image_path,
            caption    = excluded.caption,
            position   = excluded.position,
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(&final_id)
    .bind(image_path)
    .bind(caption)
    .bind(position)
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("upsert gallery: {e}")))?;
    Ok(final_id)
}

#[server(
    name = DeleteGalleryItem,
    prefix = "/api",
    endpoint = "delete_gallery_item"
)]
pub async fn delete_gallery_item(id: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query("DELETE FROM home_gallery WHERE id = ?1")
        .bind(id.trim())
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("delete gallery: {e}")))?;
    Ok(())
}

#[cfg(feature = "ssr")]
pub mod ssr {
    use super::*;
    use sqlx::SqlitePool;

    /// Tuple shape returned by both offer queries:
    /// (id, title, price_label, blurb, add_on, image_path, active, position,
    /// menu_item_id).
    type OfferRow = (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        i64,
        Option<String>,
    );

    pub async fn offers_active(db: &SqlitePool) -> Result<Vec<Offer>, sqlx::Error> {
        let rows: Vec<OfferRow> = sqlx::query_as(
            "SELECT id, title, price_label, blurb, add_on, image_path, active, position,
                    menu_item_id
             FROM home_offers
             WHERE active = 1
             ORDER BY position, id",
        )
        .fetch_all(db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Offer {
                id: r.0,
                title: r.1,
                price_label: r.2,
                blurb: r.3,
                add_on: r.4,
                image_path: r.5,
                active: r.6 != 0,
                position: r.7,
                menu_item_id: r.8,
                item_id: None,
                included_line: None,
                motif: None,
            })
            .collect())
    }

    /// "28 €", "14,70 €": whole euros without cents, as on the cards.
    pub fn eur_short(cents: i64) -> String {
        if cents % 100 == 0 {
            format!("{} €", cents / 100)
        } else {
            format!("{},{:02} €", cents / 100, (cents % 100).abs())
        }
    }

    /// An item's lines for a card: (price, "mit N Zutaten nach Wahl",
    /// "+ jede weitere Zutat X €").
    pub fn item_lines(
        small: i64,
        large: Option<i64>,
        included: i64,
        flat: Option<i64>,
    ) -> (String, Option<String>, Option<String>) {
        let price = match large {
            Some(_) => format!("ab {}", eur_short(small)),
            None => eur_short(small),
        };
        let included = (included > 0).then(|| format!("mit {included} Zutaten nach Wahl"));
        let extra = flat.map(|f| format!("+ jede weitere Zutat {}", eur_short(f)));
        (price, included, extra)
    }

    /// Each card to its orderable menu item — the linked one, else one of
    /// the same name ("Pizzablech 60×40 cm" = "Pizzablech 60x40cm": case,
    /// spaces and × ignored) — and a linked card's price and topping lines
    /// live from the item.
    pub async fn link_offers(db: &SqlitePool, offers: &mut [Offer]) {
        type Row = (String, String, i64, Option<i64>, i64, Option<i64>, String);
        let items: Vec<Row> = sqlx::query_as(
            "SELECT i.id, i.name, i.price_small_cents, i.price_large_cents,
                    i.included_extras_count, i.flat_extra_price_cents, c.name
             FROM menu_items i JOIN menu_categories c ON c.id = i.category_id
             WHERE i.is_available = 1 AND i.is_listed = 1 AND c.is_active = 1",
        )
        .fetch_all(db)
        .await
        .unwrap_or_default();
        let norm = |s: &str| -> String {
            s.to_lowercase()
                .replace('×', "x")
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect()
        };
        for o in offers.iter_mut() {
            let linked = o
                .menu_item_id
                .as_deref()
                .and_then(|id| items.iter().find(|it| it.0 == id));
            if let Some((id, name, small, large, included, flat, category)) = linked {
                let (price, included_line, extra) = item_lines(*small, *large, *included, *flat);
                o.item_id = Some(id.clone());
                o.motif = Some(crate::pages::menu::item_motif(name, category).to_string());
                o.price_label = price;
                o.included_line = included_line;
                if extra.is_some() {
                    o.add_on = extra;
                }
                if o.title.trim().is_empty() {
                    o.title = name.clone();
                }
                continue;
            }
            let title = norm(&o.title);
            if let Some(it) = items.iter().find(|it| norm(&it.1) == title) {
                o.item_id = Some(it.0.clone());
                o.motif = Some(crate::pages::menu::item_motif(&it.1, &it.6).to_string());
            }
        }
    }

    pub async fn offers_all(db: &SqlitePool) -> Result<Vec<Offer>, sqlx::Error> {
        let rows: Vec<OfferRow> = sqlx::query_as(
            "SELECT id, title, price_label, blurb, add_on, image_path, active, position,
                    menu_item_id
             FROM home_offers
             ORDER BY position, id",
        )
        .fetch_all(db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Offer {
                id: r.0,
                title: r.1,
                price_label: r.2,
                blurb: r.3,
                add_on: r.4,
                image_path: r.5,
                active: r.6 != 0,
                position: r.7,
                menu_item_id: r.8,
                item_id: None,
                included_line: None,
                motif: None,
            })
            .collect())
    }

    pub async fn gallery(db: &SqlitePool) -> Result<Vec<GalleryItem>, sqlx::Error> {
        let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
            "SELECT id, image_path, caption, position
             FROM home_gallery
             ORDER BY position, id",
        )
        .fetch_all(db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| GalleryItem {
                id: r.0,
                image_path: r.1,
                caption: r.2,
                position: r.3,
            })
            .collect())
    }
}
