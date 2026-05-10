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
/// rule on `.hero-bg`. Keep the list short and visually distinct; David
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

    let offers = ssr::offers_active(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("offers: {e}")))?;
    let gallery = ssr::gallery(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("gallery: {e}")))?;

    Ok(HomeContent {
        hero: Hero {
            title: hero.0,
            subtitle: hero.1,
            image_path: hero.2,
        },
        offers,
        gallery,
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
) -> Result<String, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;

    let title = title.trim();
    let price_label = price_label.trim();
    let blurb = blurb.trim();
    if title.is_empty() || price_label.is_empty() || blurb.is_empty() {
        return Err(ServerFnError::new(
            "Titel, Preis und Beschreibung dürfen nicht leer sein.",
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
            (id, title, price_label, blurb, add_on, image_path, active, position)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
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
    /// (id, title, price_label, blurb, add_on, image_path, active, position).
    type OfferRow = (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        i64,
    );

    pub async fn offers_active(db: &SqlitePool) -> Result<Vec<Offer>, sqlx::Error> {
        let rows: Vec<OfferRow> = sqlx::query_as(
            "SELECT id, title, price_label, blurb, add_on, image_path, active, position
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
            })
            .collect())
    }

    pub async fn offers_all(db: &SqlitePool) -> Result<Vec<Offer>, sqlx::Error> {
        let rows: Vec<OfferRow> = sqlx::query_as(
            "SELECT id, title, price_label, blurb, add_on, image_path, active, position
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
