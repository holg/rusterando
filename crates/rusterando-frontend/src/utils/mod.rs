// Shared frontend utilities (formatters, fetch helpers).

use leptos::prelude::*;

/// A process-wide "tick" that increments on a fixed interval in the
/// browser. Resources that should auto-refresh (the order-pause banner,
/// the cart drawer's open/closed state) fold `pause_poll().get()` into
/// their trigger so they re-fetch periodically — the reactive views then
/// update **in place**, with no page reload.
///
/// Why a shared singleton: every consumer reads the same signal, so one
/// interval drives all of them (home banner + cart + checkout) instead of
/// each spinning its own timer. Created lazily on first use.
///
/// Server-side (SSR) this just returns a static `0` signal and starts no
/// timer — there's no event loop and each SSR render already reads live
/// pause state from the DB, so polling only matters after hydration.
#[cfg(feature = "hydrate")]
pub fn pause_poll() -> ReadSignal<u32> {
    use std::cell::OnceCell;
    thread_local! {
        static TICK: OnceCell<ReadSignal<u32>> = const { OnceCell::new() };
    }
    TICK.with(|cell| {
        *cell.get_or_init(|| {
            let (read, write) = signal(0_u32);
            // ~30s: a paused/resumed state self-corrects within half a
            // minute without a reload. Cheap (one tiny server-fn per tab
            // per 30s) for a state that changes a handful of times a day.
            leptos::leptos_dom::helpers::set_interval(
                move || write.update(|n| *n = n.wrapping_add(1)),
                std::time::Duration::from_secs(30),
            );
            read
        })
    })
}

/// SSR / non-hydrate builds: a constant signal, no timer. Resources keyed
/// on it simply never re-fire from polling during SSR.
#[cfg(not(feature = "hydrate"))]
pub fn pause_poll() -> ReadSignal<u32> {
    signal(0_u32).0
}

/// Subscribe to the live order channel (`/api/live/orders/{id}`) for the
/// given order and push updates into the provided signals. Opens a browser
/// `EventSource` (auto-reconnecting) inside a post-hydration `Effect`, so it
/// NEVER alters the SSR/hydrate DOM — it only mutates signals after mount.
/// `msg` receives `LiveKind::Message` payloads; `status` receives
/// `LiveKind::Status`. No-op on SSR.
///
/// IMPORTANT (hydration): call this from a component AFTER its initial
/// render is set up from the page's resource. The Effect runs only in the
/// browser, so SSR output is unaffected and the tachys hydration walker
/// sees identical DOM on both sides.
/// `messages` is read+written: `Message` events append, `MessageAck`
/// events flip the matching entry's `delivered` flag. `status` receives
/// `Status` events. When `ack_on_receive` is true (the CUSTOMER page), a
/// received `Message` triggers an `ack_order_message` call back to the
/// server (delivery receipt). The ADMIN page passes false (it only
/// listens for acks, doesn't send them).
/// Fire an OS notification for a customer order update, but only when it's
/// useful and allowed: the user has granted permission AND the order tab is
/// currently backgrounded (`document.hidden`). If the tab is in the
/// foreground the message already appears in the log, so we stay quiet.
/// `tag` = order id, so successive pings for the same order replace one
/// another instead of stacking. No-op (and no error) when the browser lacks
/// the Notification API or permission isn't granted — e.g. iOS Safari.
#[cfg(feature = "hydrate")]
fn notify_customer(title: &str, body: &str, tag: &str) {
    use web_sys::{Notification, NotificationOptions, NotificationPermission};

    if Notification::permission() != NotificationPermission::Granted {
        return;
    }
    // Only notify when the customer isn't already looking at the page.
    let hidden = web_sys::window()
        .and_then(|w| w.document())
        .map(|d| d.hidden())
        .unwrap_or(false);
    if !hidden {
        return;
    }
    let opts = NotificationOptions::new();
    opts.set_body(body);
    opts.set_tag(tag);
    opts.set_icon("/img/cover.jpg");
    // Ignore the Result: a failed notification must never break the SSE loop.
    let _ = Notification::new_with_options(title, &opts);
}

/// `notify`: customer side only — fire `notify_customer` on new messages and
/// status changes (the admin order page passes `false`).
#[cfg(feature = "hydrate")]
pub fn subscribe_order_live(
    order_id: String,
    messages: RwSignal<Vec<rusterando_shared::models::OrderMessage>>,
    status: WriteSignal<Option<String>>,
    ack_on_receive: bool,
    notify: bool,
) {
    use rusterando_shared::models::{LiveEvent, LiveKind};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    Effect::new(move |_| {
        if order_id.is_empty() {
            return;
        }
        let url = format!("/api/live/orders/{order_id}");
        let Ok(es) = web_sys::EventSource::new(&url) else {
            return;
        };
        let oid = order_id.clone();
        let on_msg = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |ev: web_sys::MessageEvent| {
                let Some(text) = ev.data().as_string() else {
                    return;
                };
                let Ok(event) = serde_json::from_str::<LiveEvent>(&text) else {
                    return;
                };
                match event.kind {
                    // Append to the chat-style log (oldest first).
                    LiveKind::Message(m) => {
                        let id = m.id;
                        // Status changes are logged into the message stream too
                        // (body "Status: …") AND arrive as a dedicated `Status`
                        // event. Notify only via the `Status` branch so a single
                        // status change doesn't pop two notifications — fire here
                        // only for genuine admin-typed messages.
                        if notify && !m.body.starts_with("Status: ") {
                            notify_customer("📣 Nachricht vom Restaurant", &m.body, &oid);
                        }
                        messages.update(|v| v.push(m));
                        if ack_on_receive {
                            // Confirm delivery back to the server (customer side).
                            let oid = oid.clone();
                            leptos::task::spawn_local(async move {
                                let _ = crate::pages::order::ack_order_message(oid, id).await;
                            });
                        }
                    }
                    // Flip the matching message to delivered (admin side).
                    LiveKind::MessageAck { message_id } => {
                        messages.update(|v| {
                            if let Some(m) = v.iter_mut().find(|m| m.id == message_id) {
                                m.delivered = true;
                            }
                        });
                    }
                    LiveKind::Status(s) => {
                        if notify {
                            let label = crate::pages::order::status_label_de(&s);
                            notify_customer(
                                "Bestellung aktualisiert",
                                &format!("Status: {label}"),
                                &oid,
                            );
                        }
                        status.set(Some(s));
                    }
                    LiveKind::ShopStatus { .. } => {}
                }
            },
        );
        es.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
        // Leak the closure + EventSource: they live for the page's lifetime;
        // the browser tears the connection down on navigation/close.
        on_msg.forget();
        std::mem::forget(es);
    });
}

/// SSR / non-hydrate: no live channel.
#[cfg(not(feature = "hydrate"))]
pub fn subscribe_order_live(
    _order_id: String,
    _messages: RwSignal<Vec<rusterando_shared::models::OrderMessage>>,
    _status: WriteSignal<Option<String>>,
    _ack_on_receive: bool,
    _notify: bool,
) {
}

/// Subscribe to the global shop-status channel (`/api/live/shop`) and push
/// `(level, reason)` into the callback signal whenever the shop's
/// open/closed state changes (pause/snooze/force-open or snooze expiry).
/// Effect-based + hydrate-only, same hydration-safety contract as
/// `subscribe_order_live`. No-op on SSR.
#[cfg(feature = "hydrate")]
pub fn subscribe_shop_status(
    on_status: WriteSignal<Option<(rusterando_shared::models::ShopLevel, String)>>,
) {
    use rusterando_shared::models::{LiveEvent, LiveKind};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    Effect::new(move |_| {
        let Ok(es) = web_sys::EventSource::new("/api/live/shop") else {
            return;
        };
        let on_msg = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |ev: web_sys::MessageEvent| {
                let Some(text) = ev.data().as_string() else {
                    return;
                };
                let Ok(event) = serde_json::from_str::<LiveEvent>(&text) else {
                    return;
                };
                if let LiveKind::ShopStatus { level, reason } = event.kind {
                    on_status.set(Some((level, reason)));
                }
            },
        );
        es.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
        on_msg.forget();
        std::mem::forget(es);
    });
}

#[cfg(not(feature = "hydrate"))]
pub fn subscribe_shop_status(
    _on_status: WriteSignal<Option<(rusterando_shared::models::ShopLevel, String)>>,
) {
}
