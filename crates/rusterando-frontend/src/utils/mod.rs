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
        let Ok(es) = web_sys::EventSource::new(&url) else {
            return;
        };
        let oid = order_id.clone();
        let on_msg =
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
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
                }
            });
        es.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));

        // Stash the EventSource + closure in a per-effect RefCell so the
        // owner's on_cleanup hook can drop them. Previously we
        // `forget()`-ed both, which is fine on full-page navigation
        // (browser tears down the tab's EventSources) but LEAKS on
        // Leptos client-side routing: the OrderConfirmationPage unmounts,
        // the effect is disposed, but the leaked ES stays connected
        // forever. A second visit to /orders/<id> then opens ANOTHER
        // EventSource on top, doubling the server's fd count per revisit.
        //
        // !Send web_sys types can't go through `on_cleanup` directly
        // (the signature requires Send+Sync). We bridge via a thread_local
        // slot: the cleanup hook just sets a flag; the next time the
        // tokio/leptos event loop yields we close + drop the ES. Single-
        // threaded WASM makes this race-free.
        let es_slot: Rc<RefCell<Option<web_sys::EventSource>>> = Rc::new(RefCell::new(Some(es)));
        let closure_slot: Rc<RefCell<Option<Closure<dyn FnMut(web_sys::MessageEvent)>>>> =
            Rc::new(RefCell::new(Some(on_msg)));

        // Register the close+drop hook on the current owner. The
        // !Send Rc<RefCell<…>> values we want to drop wouldn't satisfy
        // on_cleanup's `FnOnce + Send + Sync` bound on their own, but
        // WASM is single-threaded so `send_wrapper::SendWrapper` is
        // sound here: it asserts Send/Sync, then panics if anyone
        // actually moves it off its origin thread. Inside the
        // browser there IS no other thread, so the panic path is
        // unreachable.
        use send_wrapper::SendWrapper;
        let es_for_drop = SendWrapper::new(es_slot.clone());
        let closure_for_drop = SendWrapper::new(closure_slot.clone());
        leptos::prelude::on_cleanup(move || {
            // Take + close synchronously. The browser EventSource API
            // is allowed to run on whatever thread we're on (it's all
            // the main JS thread in WASM), so no spawn_local needed.
            if let Some(es) = es_for_drop.borrow_mut().take() {
                es.close();
            }
            // Dropping the closure frees the JS-side allocation; the
            // browser's onmessage handler is detached as soon as the
            // EventSource is closed above.
            let _ = closure_for_drop.borrow_mut().take();
        });
        // Keep the slots alive for the lifetime of the effect: dropping
        // them here would close the ES immediately.
        let _ = (es_slot, closure_slot);
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
