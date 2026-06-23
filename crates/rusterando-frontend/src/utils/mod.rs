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
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    Effect::new(move |_| {
        if order_id.is_empty() {
            return;
        }
        let url = format!("/api/live/orders/{order_id}");

        // Per-order live channel WITH auto-reconnect. The browser's
        // built-in EventSource reconnect does NOT fire once the stream
        // goes to CLOSED (server-initiated close on deploy/restart, the
        // 5s keep-alive reaping a momentarily-stalled client, or a proxy
        // timeout). Before this, that left the customer's order page
        // silently frozen — status + admin messages stopped arriving
        // until a manual reload. We now mirror `subscribe_shop_status`:
        // an `onerror` handler closes the dead socket and reschedules a
        // connect with exponential backoff (1s → 30s cap), reset to 1s on
        // any successful message.
        //
        // Single-threaded WASM ⇒ Rc/RefCell are sound. `connect` is
        // self-referential (onerror re-calls it) so it's held behind an
        // Rc<RefCell<Option<Rc<dyn Fn()>>>> to break the cycle.
        let es_slot: Rc<RefCell<Option<web_sys::EventSource>>> = Rc::new(RefCell::new(None));
        let backoff_ms: Rc<RefCell<u32>> = Rc::new(RefCell::new(1_000));
        // Keep onmessage/onerror closures alive across reconnects.
        let keepalive: Rc<RefCell<Vec<wasm_bindgen::JsValue>>> = Rc::new(RefCell::new(Vec::new()));

        let connect: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let connect_clone = connect.clone();
        let es_slot_c = es_slot.clone();
        let backoff_c = backoff_ms.clone();
        let keepalive_c = keepalive.clone();
        let order_id_c = order_id.clone();

        let connect_fn: Rc<dyn Fn()> = Rc::new(move || {
            // Reuse a healthy socket; only (re)connect when absent/CLOSED.
            let needs_new = es_slot_c
                .borrow()
                .as_ref()
                .map_or(true, |es| es.ready_state() == web_sys::EventSource::CLOSED);
            if !needs_new {
                return;
            }
            if let Some(old) = es_slot_c.borrow_mut().take() {
                old.close();
            }
            let Ok(es) = web_sys::EventSource::new(&url) else {
                return;
            };

            let oid = order_id_c.clone();
            let backoff_for_msg = backoff_c.clone();
            let on_msg = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
                move |ev: web_sys::MessageEvent| {
                    // Any successful message proves the link is healthy →
                    // reset the backoff so the next blip starts from 1s.
                    *backoff_for_msg.borrow_mut() = 1_000;
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
                                let oid = oid.clone();
                                leptos::task::spawn_local(async move {
                                    let _ = crate::pages::order::ack_order_message(oid, id).await;
                                });
                            }
                        }
                        LiveKind::MessageAck { message_id } => {
                            messages.update(|v| {
                                if let Some(m) = v.iter_mut().find(|m| m.id == message_id) {
                                    m.delivered = true;
                                }
                            });
                        }
                        LiveKind::Status(s) => {
                            if notify {
                                let label = crate::pages::order::status_label(&s);
                                notify_customer(
                                    &crate::t!("notify.order_updated"),
                                    &crate::t!("notify.status_prefix").replace("{label}", &label),
                                    &oid,
                                );
                            }
                            status.set(Some(s));
                        }
                        LiveKind::ShopStatus { .. } => {}
                        // Driver live-location pushes (task: customer order page
                        // renders the driver pin). No UI consumer wired here yet
                        // — ignore so the order SSE handler stays exhaustive.
                        LiveKind::DriverLocation { .. } => {}
                    }
                },
            );
            es.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));

            // onerror: close + reschedule with the current backoff, then
            // double it (cap 30s). Reconnect routes through `connect`,
            // which checks readyState first so a flaky link can't pile up
            // duplicate sockets.
            let reconnect = connect_clone.clone();
            let es_slot_err = es_slot_c.clone();
            let backoff_err = backoff_c.clone();
            let on_err = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                if let Some(old) = es_slot_err.borrow_mut().take() {
                    old.close();
                }
                let delay = {
                    let cur = *backoff_err.borrow();
                    let next = cur.saturating_mul(2).min(30_000);
                    *backoff_err.borrow_mut() = next;
                    cur
                };
                if let Some(f) = reconnect.borrow().clone() {
                    leptos::leptos_dom::helpers::set_timeout(
                        move || f(),
                        std::time::Duration::from_millis(delay as u64),
                    );
                }
            });
            es.set_onerror(Some(on_err.as_ref().unchecked_ref()));

            // Hold the closures alive for the lifetime of the subscription
            // (across reconnects). They're dropped wholesale in on_cleanup.
            keepalive_c.borrow_mut().push(on_msg.into_js_value());
            keepalive_c.borrow_mut().push(on_err.into_js_value());
            *es_slot_c.borrow_mut() = Some(es);
        });
        *connect.borrow_mut() = Some(connect_fn.clone());
        connect_fn();

        // Cleanup on unmount: close the socket, drop the held closures,
        // and clear `connect` so any in-flight set_timeout reconnect
        // no-ops. SendWrapper bridges the !Send Rc/RefCell through
        // on_cleanup's Send+Sync bound (sound: single-threaded WASM).
        use send_wrapper::SendWrapper;
        let es_for_drop = SendWrapper::new(es_slot.clone());
        let keepalive_for_drop = SendWrapper::new(keepalive.clone());
        let connect_for_drop = SendWrapper::new(connect.clone());
        leptos::prelude::on_cleanup(move || {
            if let Some(es) = es_for_drop.borrow_mut().take() {
                es.close();
            }
            keepalive_for_drop.borrow_mut().clear();
            // Drop the connect fn so a pending reconnect timer finds None.
            let _ = connect_for_drop.borrow_mut().take();
        });
        // Keep everything alive for the effect's lifetime.
        let _ = (es_slot, backoff_ms, keepalive, connect);
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

/// Subscribe to the global shop-status channel and push `(level, reason)`
/// into the caller's signal whenever the shop's open/closed state changes.
///
/// Public signature unchanged for callers, but internally this bridges to a
/// single tab-wide EventSource on `/api/live/shop` (see
/// `ensure_shop_status_singleton`). Network problems are normal, not
/// exceptional: the singleton reuses an OPEN/CONNECTING socket and
/// auto-reconnects with exponential backoff on `onerror`. Five components
/// previously each opened their own EventSource — now they share one, no
/// matter how many times the components mount/remount.
#[cfg(feature = "hydrate")]
pub fn subscribe_shop_status(
    on_status: WriteSignal<Option<(rusterando_shared::models::ShopLevel, String)>>,
) {
    let shared = ensure_shop_status_singleton();
    Effect::new(move |_| {
        shared.with(|s| {
            if let Some(v) = s {
                on_status.set(Some(v.clone()));
            }
        });
    });
}

/// Lazily-initialised tab-singleton for the `/api/live/shop` EventSource.
/// Idempotent — calling multiple times (each `subscribe_shop_status`
/// invocation, hot-reloads, re-renders) is safe:
///   * If the existing EventSource is OPEN or CONNECTING → reuse it, do
///     NOT open a new one. (User's explicit requirement: "does not simply
///     do the new one and it makes no sense to have 3 in parallel.")
///   * If it's CLOSED (or absent) → close any stale handle, open a fresh
///     one, wire onmessage + onerror.
/// `onerror` schedules a reconnect via `set_timeout` with exponential
/// backoff (1s → 2s → 4s → 8s → 16s → 30s cap); resets to 1s on the next
/// successful message so a brief blip doesn't push us to long delays.
#[cfg(feature = "hydrate")]
fn ensure_shop_status_singleton() -> RwSignal<Option<(rusterando_shared::models::ShopLevel, String)>>
{
    use std::cell::{OnceCell, RefCell};
    use std::rc::Rc;
    thread_local! {
        static SIG: OnceCell<RwSignal<Option<(rusterando_shared::models::ShopLevel, String)>>> =
            const { OnceCell::new() };
        static ES: RefCell<Option<web_sys::EventSource>> = const { RefCell::new(None) };
        static BACKOFF_MS: RefCell<u32> = const { RefCell::new(1_000) };
    }

    let sig = SIG.with(|c| *c.get_or_init(|| RwSignal::new(None)));

    // `connect` is recursive (the onerror handler schedules a delayed
    // re-call), so wrap it in an Rc<RefCell<Option<Rc<dyn Fn()>>>> to break
    // the self-reference. WASM is single-threaded so Rc/RefCell are safe.
    let connect: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let connect_clone = connect.clone();
    let connect_fn: Rc<dyn Fn()> = Rc::new(move || {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::JsCast;

        // Reuse if the existing socket is healthy. EventSource readyState
        // constants: CONNECTING = 0, OPEN = 1, CLOSED = 2.
        let needs_new = ES.with(|cell| {
            cell.borrow()
                .as_ref()
                .map_or(true, |es| es.ready_state() == web_sys::EventSource::CLOSED)
        });
        if !needs_new {
            return;
        }

        // Close any stale handle before opening a fresh one.
        ES.with(|cell| {
            if let Some(old) = cell.borrow_mut().take() {
                old.close();
            }
        });
        let Ok(es) = web_sys::EventSource::new("/api/live/shop") else {
            return;
        };

        // onmessage: parse the LiveEvent; only ShopStatus updates the
        // shared signal. Reset the backoff on success — proves the link
        // is healthy again so the next blip starts from 1s, not 30s.
        let on_msg =
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
                use rusterando_shared::models::{LiveEvent, LiveKind};
                let Some(text) = ev.data().as_string() else {
                    return;
                };
                let Ok(event) = serde_json::from_str::<LiveEvent>(&text) else {
                    return;
                };
                if let LiveKind::ShopStatus { level, reason } = event.kind {
                    BACKOFF_MS.with(|b| *b.borrow_mut() = 1_000);
                    sig.set(Some((level, reason)));
                }
            });
        es.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
        on_msg.forget();

        // onerror: close the socket and schedule a reconnect with the
        // current backoff, then double it (capped at 30 s) for next time.
        // The reconnect goes through `connect` again — which checks
        // readyState first, so a flaky network won't multiply connections.
        let reconnect = connect_clone.clone();
        let on_err = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            ES.with(|cell| {
                if let Some(old) = cell.borrow_mut().take() {
                    old.close();
                }
            });
            let delay = BACKOFF_MS.with(|b| {
                let cur = *b.borrow();
                let next = cur.saturating_mul(2).min(30_000);
                *b.borrow_mut() = next;
                cur
            });
            if let Some(f) = reconnect.borrow().clone() {
                leptos::leptos_dom::helpers::set_timeout(
                    move || f(),
                    std::time::Duration::from_millis(delay as u64),
                );
            }
        });
        es.set_onerror(Some(on_err.as_ref().unchecked_ref()));
        on_err.forget();

        ES.with(|cell| *cell.borrow_mut() = Some(es));
    });
    *connect.borrow_mut() = Some(connect_fn.clone());
    connect_fn();

    sig
}

#[cfg(not(feature = "hydrate"))]
pub fn subscribe_shop_status(
    _on_status: WriteSignal<Option<(rusterando_shared::models::ShopLevel, String)>>,
) {
}
