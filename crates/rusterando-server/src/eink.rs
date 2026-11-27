//! The Tagesangebot as the colour e-ink frame's picture (800 × 480 in the
//! panel's six colours, rendered by pdf.rs): what goes on it, and its
//! version tag. `/tagesangebot.png` and `/tagesangebot.eink` (main.rs)
//! answer with the tag as ETag; the kitchen board feed pushes it to a
//! display that drives a frame (`KitchenEvent::Picture`, kitchen_board.rs),
//! which then fetches the picture once.

use rusterando_frontend::branding::{Branding, BrandingHandle};
use sqlx::SqlitePool;

use crate::pdf::EinkOffer;

/// Where a board feed gets the picture's data: the shop's branding and its
/// public address, and the device asking (`?device=`): whether it drives the
/// frame is switched on /admin/drucker (`kitchen_clients.eink`). `None`:
/// older firmware, which decides that itself.
#[derive(Clone)]
pub struct PictureSource {
    pub branding: BrandingHandle,
    pub site: String,
    pub device: Option<String>,
}

/// Does `device` drive the frame (/admin/drucker)? Older firmware without a
/// device name decides itself, so: yes.
pub async fn drives_frame(db: &SqlitePool, device: Option<&str>) -> bool {
    let Some(device) = device else {
        return true;
    };
    sqlx::query_scalar::<_, i64>("SELECT eink FROM kitchen_clients WHERE shop_slug = ?1")
        .bind(device)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .is_some_and(|on| on != 0)
}

/// What goes on the picture: today's offer (or the shop's card when there
/// is none), the shop's name, web address and phone.
pub async fn offer(db: &SqlitePool, branding: &Branding, site: &str) -> EinkOffer {
    let eur = rusterando_frontend::pages::home_content::ssr::eur_short;
    let view = rusterando_frontend::pages::daily_offer::ssr::view(db).await;
    let url = site
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    EinkOffer {
        offer: view.is_some(),
        name: view.as_ref().map(|v| v.name.clone()).unwrap_or_default(),
        detail: view.as_ref().map(|v| v.detail.clone()).unwrap_or_default(),
        price: view.as_ref().map(|v| eur(v.price_cents)).unwrap_or_default(),
        regular: view
            .as_ref()
            .and_then(|v| v.regular_cents)
            .map(eur)
            .unwrap_or_default(),
        motif: view
            .as_ref()
            .map(|v| v.motif.clone())
            .unwrap_or_else(|| "pizza".into()),
        shop: branding.display_name(),
        url,
        phone: branding.shop_phone.clone(),
    }
}

/// The picture's version: changes with what is on it and with its look
/// (template, colour mapping). 16 hex digits; the ETag is it in quotes.
pub fn tag(o: &EinkOffer) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(serde_json::to_string(o).unwrap_or_default());
    h.update(crate::pdf::EINK_OFFER_SRC);
    h.update(crate::pdf::EINK_QUANTIZER.to_le_bytes());
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> EinkOffer {
        EinkOffer {
            offer: true,
            name: "Broccoli Roma".into(),
            detail: "mit kl. Salat".into(),
            price: "9 €".into(),
            regular: "14 €".into(),
            motif: "pizza".into(),
            shop: "Musterpizzeria".into(),
            url: "musterpizzeria.example".into(),
            phone: "01234 567890".into(),
        }
    }

    #[test]
    fn tag_follows_the_content() {
        let a = sample();
        let mut b = sample();
        assert_eq!(tag(&a), tag(&b));
        assert_eq!(tag(&a).len(), 16);
        b.price = "8 €".into();
        assert_ne!(tag(&a), tag(&b));
    }
}
