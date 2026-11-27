//! Printing on the device itself, where it has a receipt printer built in.
//!
//! Only the staff Android app on such a handheld (Sunmi V2, or anything with
//! Sunmi's printer service) has one. There every print button prints on the
//! device: "Erneut drucken" on /kitchen and /admin/orders ([`ReprintButtons`],
//! with a second button for the kitchen printer), "Drucken" on an order's
//! page and the Bon-Editor's "Diesen Bon drucken" ([`test_bon_id`]). Everywhere
//! else they do what they always did. New orders still print on the kitchen
//! printer only.
//!
//! That app exposes `window.RusterandoPrinter` — an androidx.webkit message
//! listener, offered to the main frame of the shop's own origin only. The
//! page posts the order id; the app fetches the Bon
//! (`/api/orders/{id}/bon-58mm`) with its own session, prints it and shows
//! the outcome as a native toast.
//!
//! Hydration: SSR and the first client render are the same (no device
//! printer); an effect (client only, after hydration) switches over when the
//! bridge is there. Only the second button's visibility changes, never the
//! tag shape — see `test_mode_banner.rs`.

use leptos::prelude::*;

use crate::pages::admin::orders::ReprintOrder;

/// The app's printer bridge, if this page runs inside it on such a device.
#[cfg(not(feature = "ssr"))]
fn bridge() -> Option<leptos::wasm_bindgen::JsValue> {
    use leptos::wasm_bindgen::JsValue;
    use leptos::web_sys::{self, js_sys};
    let window = web_sys::window()?;
    let b = js_sys::Reflect::get(&window, &JsValue::from_str("RusterandoPrinter")).ok()?;
    (!b.is_undefined() && !b.is_null()).then_some(b)
}

/// `true` once hydrated inside the app on a device with a printer. Starts
/// `false` on both sides, so SSR and hydration render the same.
pub fn use_device_printer() -> RwSignal<bool> {
    let on_device = RwSignal::new(false);
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        on_device.set(bridge().is_some());
    });
    on_device
}

/// Hand a Bon to the app's printer: an order id, or a [`test_bon_id`].
/// `false`: there is no device printer (print the usual way instead).
#[cfg(feature = "ssr")]
pub fn print_on_device(_id: &str) -> bool {
    false
}

/// Hand a Bon to the app's printer: an order id, or a [`test_bon_id`].
/// `false`: there is no device printer (print the usual way instead).
#[cfg(not(feature = "ssr"))]
pub fn print_on_device(id: &str) -> bool {
    use leptos::wasm_bindgen::{JsCast, JsValue};
    use leptos::web_sys::js_sys;
    let Some(b) = bridge() else { return false };
    let post = js_sys::Reflect::get(&b, &JsValue::from_str("postMessage"))
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
    match post {
        Some(post) => post.call1(&b, &JsValue::from_str(id)).is_ok(),
        None => false,
    }
}

/// The Bon-Editor's sample Bon, as an id the app accepts: it fetches
/// `/api/orders/{id}/bon-58mm` for any id of `[0-9A-Za-z-]{1,64}`, and the
/// server answers this one with the sample ([`parse_test_bon_id`]). Real
/// order ids are UUIDs. `None`: a theme the id can't carry.
pub fn test_bon_id(delivery: bool, prepaid: bool, theme: &str) -> Option<String> {
    let mut id = format!("{TEST_BON}{}{}", u8::from(delivery), u8::from(prepaid));
    if !theme.is_empty() {
        id.push('-');
        id.push_str(theme);
    }
    (id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).then_some(id)
}

const TEST_BON: &str = "probedruck-";

/// (delivery, prepaid, theme) of a [`test_bon_id`].
pub fn parse_test_bon_id(id: &str) -> Option<(bool, bool, String)> {
    let rest = id.strip_prefix(TEST_BON)?;
    let flag = |c: u8| match c {
        b'0' => Some(false),
        b'1' => Some(true),
        _ => None,
    };
    let bytes = rest.as_bytes();
    let (delivery, prepaid) = (flag(*bytes.first()?)?, flag(*bytes.get(1)?)?);
    let theme = match &rest[2..] {
        "" => String::new(),
        t => t.strip_prefix('-').filter(|t| !t.is_empty())?.to_string(),
    };
    theme
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        .then_some((delivery, prepaid, theme))
}

#[component]
pub fn ReprintButtons(id: String) -> impl IntoView {
    // Own action per card: its `pending()` disables only this card's button.
    let reprinter = ServerAction::<ReprintOrder>::new();
    let on_device = use_device_printer();

    let to_kitchen = {
        let id = id.clone();
        move || {
            reprinter.dispatch(ReprintOrder { id: id.clone() });
        }
    };
    let on_reprint = {
        let to_kitchen = to_kitchen.clone();
        let id = id.clone();
        move |_| {
            if on_device.get_untracked() && print_on_device(&id) {
                return;
            }
            to_kitchen();
        }
    };
    view! {
        <button class="btn ghost" on:click=on_reprint
            disabled=move || reprinter.pending().get()
            title=move || if on_device.get() { "Auf diesem Gerät drucken" } else { "Bon an den Küchendrucker" }>
            "🖨 Erneut drucken"
        </button>
        <button class="btn ghost" on:click=move |_| to_kitchen()
            disabled=move || reprinter.pending().get()
            title="Bon an den Küchendrucker"
            style:display=move || if on_device.get() { "" } else { "none" }>
            "🍳 An Küchendrucker"
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bon_ids_round_trip() {
        for (d, p, theme) in [
            (true, false, "rusterando-rando"),
            (false, true, ""),
            (true, true, "x"),
        ] {
            let id = test_bon_id(d, p, theme).unwrap();
            assert!(id.len() <= 64, "{id}");
            assert_eq!(parse_test_bon_id(&id), Some((d, p, theme.to_string())));
        }
        assert_eq!(
            test_bon_id(true, false, "rusterando-rando").as_deref(),
            Some("probedruck-10-rusterando-rando")
        );
    }

    #[test]
    fn order_ids_are_not_test_bons() {
        assert_eq!(
            parse_test_bon_id("0ca5c65f-6a59-4d38-9000-6971d6808333"),
            None
        );
        assert_eq!(parse_test_bon_id("probedruck-2"), None);
        assert_eq!(parse_test_bon_id("probedruck-10-"), None);
        assert_eq!(parse_test_bon_id("probedruck-10x"), None);
        assert_eq!(test_bon_id(true, true, "a b"), None);
    }
}
