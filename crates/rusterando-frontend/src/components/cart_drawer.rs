use leptos::prelude::*;
use rusterando_shared::models::{format_eur, CartLine, CartView, SizeChoice};

use crate::pages::cart::{get_cart, AddToCart, ClaimGiveaway, ClearCart, UpdateCartLine};

/// Shared cart state. One Resource (the source of truth) plus three actions
/// (add / update / clear). All routes that need the cart pull this via context.
#[derive(Clone, Copy)]
pub struct CartCtx {
    pub cart: Resource<Result<CartView, ServerFnError>>,
    pub open: RwSignal<bool>,
    pub add: ServerAction<AddToCart>,
    pub update: ServerAction<UpdateCartLine>,
    pub clear: ServerAction<ClearCart>,
    /// Claim the free-Pizzabrötchen giveaway (the "Jetzt sichern" button).
    pub claim_giveaway: ServerAction<ClaimGiveaway>,
    /// Manual refetch trigger. Routes that mutate the cart through other
    /// actions (e.g. place_order clearing it server-side) call `refresh()`
    /// to force the FAB / drawer back into sync.
    pub bump: RwSignal<u32>,
    /// Live shop open/closed override from the SSE `/api/live/shop` channel.
    /// `None` = use the cart's own `orders_closed`; `Some(closed)` once a
    /// ShopStatus event arrives, so the drawer's checkout button flips live.
    pub shop_closed_override: RwSignal<Option<bool>>,
}

impl CartCtx {
    pub fn refresh(&self) {
        self.bump.update(|n| *n += 1);
    }
}

/// Provide it once near the root of `<App/>`.
pub fn provide_cart_ctx() {
    let add = ServerAction::<AddToCart>::new();
    let update = ServerAction::<UpdateCartLine>::new();
    let clear = ServerAction::<ClearCart>::new();
    let claim_giveaway = ServerAction::<ClaimGiveaway>::new();

    // Refetch the cart whenever any mutation lands.
    let bump = RwSignal::new(0u32);
    Effect::new(move |_| {
        if matches!(add.value().get(), Some(Ok(_))) {
            bump.update(|n| *n += 1);
        }
    });
    Effect::new(move |_| {
        if matches!(update.value().get(), Some(Ok(_))) {
            bump.update(|n| *n += 1);
        }
    });
    Effect::new(move |_| {
        if matches!(clear.value().get(), Some(Ok(_))) {
            bump.update(|n| *n += 1);
        }
    });
    Effect::new(move |_| {
        if matches!(claim_giveaway.value().get(), Some(Ok(_))) {
            bump.update(|n| *n += 1);
        }
    });

    let cart = Resource::new(move || bump.get(), |_| async move { get_cart().await });

    // Live shop status (SSE): flips the drawer's checkout button without a
    // reload. Subscription is a post-hydration Effect (hydrate-only).
    let shop_closed_override = RwSignal::new(None::<bool>);
    let (so_read, so_write) =
        signal::<Option<(rusterando_shared::models::ShopLevel, String)>>(None);
    crate::utils::subscribe_shop_status(so_write);
    Effect::new(move |_| {
        if let Some((level, _)) = so_read.get() {
            // The drawer only cares whether checkout is blocked.
            shop_closed_override.set(Some(level.is_closed()));
        }
    });

    provide_context(CartCtx {
        cart,
        open: RwSignal::new(false),
        add,
        update,
        clear,
        claim_giveaway,
        bump,
        shop_closed_override,
    });
}

pub fn use_cart_ctx() -> CartCtx {
    use_context::<CartCtx>().expect("CartCtx not provided")
}

/// Floating cart button (bottom-right). Hidden when the cart is empty.
///
/// The button's visibility depends on the cart Resource, which is async.
/// The `<Suspense>` wrapper is essential: without it, SSR renders the
/// "empty" branch (cart not yet loaded) while hydration may see a
/// non-empty cart, producing a DOM mismatch that panics tachys at
/// hydration.rs:163.
#[component]
pub fn CartFab() -> impl IntoView {
    let ctx = use_cart_ctx();

    // Resource reads MUST happen inside <Suspense> (Leptos warns
    // otherwise and the hydration can mismatch). We give the fallback
    // and the resolved branches IDENTICAL DOM shape — a single
    // <button class="cart-fab …"> with text content. Visibility is
    // toggled via `.hidden` CSS class. SSR + hydrate both walk the
    // same single element so tachys never tries to cast the wrong
    // tag.
    view! {
        <Suspense fallback=|| view! {
            <button class="cart-fab hidden" aria-label=crate::t!("cart.fab_open_aria") aria-hidden="true">
                <span class="icon">"🛒"</span>
                <span class="count">{0}</span>
                <span class="total">"0,00 €"</span>
            </button>
        }>
            {move || {
                let cart = ctx.cart.get().and_then(|r| r.ok()).unwrap_or_default();
                let count = cart.item_count;
                let subtotal = format_eur(cart.subtotal_cents);
                let cls = if count > 0 { "cart-fab" } else { "cart-fab hidden" };
                let aria = if count > 0 { "false" } else { "true" };
                view! {
                    <button class=cls aria-label=crate::t!("cart.fab_open_aria") aria-hidden=aria
                            on:click=move |_| ctx.open.set(true)>
                        <span class="icon">"🛒"</span>
                        <span class="count">{count}</span>
                        <span class="total">{subtotal}</span>
                    </button>
                }
            }}
        </Suspense>
    }
}

/// Slide-out drawer with line items, qty controls, subtotal, checkout CTA.
#[component]
pub fn CartDrawer() -> impl IntoView {
    let ctx = use_cart_ctx();
    let is_open = move || ctx.open.get();
    let close = move |_| ctx.open.set(false);

    view! {
        <div class="cart-overlay" class:open=is_open on:click=close></div>
        <aside class="cart-drawer" class:open=is_open aria-hidden=move || (!is_open()).to_string()>
            <header class="drawer-head">
                <h2>{crate::t!("cart.title")}</h2>
                <button class="close" aria-label=crate::t!("common.close") on:click=close>"×"</button>
            </header>

            // <Suspense> wrap for the resource read. The fallback
            // matches the resolved-empty shape (single <p>), so the
            // walker doesn't have to cast between different elements.
            // The drawer body has TWO possible shapes after resolve:
            // either a <p class="empty"> (cart empty) or a <ul> of
            // lines. We render the <ul> conditionally — that's fine
            // because by the time it appears, the resource has
            // resolved on the client and tachys is past the initial
            // hydrate walk.
            <div class="drawer-body">
                <Suspense fallback=|| view! { <p class="loading">{crate::t!("common.loading")}</p> }>
                    {move || ctx.cart.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("{}: {e}", crate::t!("common.error"))}</p> }.into_any(),
                        Ok(cart) if cart.lines.is_empty() => {
                            view! { <p class="empty">{crate::t!("cart.empty")}</p> }.into_any()
                        }
                        Ok(cart) => view! { <Lines cart/> }.into_any(),
                    })}
                </Suspense>
            </div>

            <footer class="drawer-foot">
                // <Suspense> wraps the entire footer block. Fallback +
                // resolved both emit the EXACT same five elements:
                // <div class="row"> + <p class="free-delivery"> + <p>
                // + <a> + <button>. Visibility/text differs but the
                // DOM tag list is identical, so tachys' walker doesn't
                // trip on element-cast mismatches.
                <Suspense fallback=|| view! {
                    <div class="row">
                        <span>{crate::t!("cart.subtotal")}</span>
                        <strong>"…"</strong>
                    </div>
                    <p class="free-delivery hidden"></p>
                    <p class="order-closed-note hidden"></p>
                    <p class="hint">{crate::t!("cart.payment_hint")}</p>
                    <a class="btn primary disabled" href="/checkout">{crate::t!("cart.proceed")}</a>
                    <button class="btn ghost" disabled=true>{crate::t!("cart.clear")}</button>
                }>
                    {move || {
                        let cart = ctx.cart.get().and_then(|r| r.ok()).unwrap_or_default();
                        let total = format_eur(cart.subtotal_cents);
                        let has_items = !cart.lines.is_empty();
                        // Live SSE override wins over the cart's own snapshot.
                        let orders_closed = ctx.shop_closed_override.get().unwrap_or(cart.orders_closed);
                        let threshold = cart.free_delivery_threshold_cents;
                        let unlocked = threshold > 0 && cart.subtotal_cents >= threshold;
                        let missing = (threshold - cart.subtotal_cents).max(0);
                        let (banner_cls, banner_text) = if threshold > 0 && unlocked {
                            ("free-delivery ok", crate::t!("cart.free_delivery_unlocked"))
                        } else if threshold > 0 && !unlocked && has_items {
                            (
                                "free-delivery hint",
                                crate::t!("cart.free_delivery_progress")
                                    .replace("{amount}", &format_eur(missing)),
                            )
                        } else {
                            ("free-delivery hidden", String::new())
                        };
                        // Closed (paused / outside hours) wins over the
                        // empty-cart state for disabling checkout.
                        let cta_cls = if has_items && !orders_closed {
                            "btn primary"
                        } else {
                            "btn primary disabled"
                        };
                        let (closed_note_cls, closed_note_text) = if orders_closed {
                            ("order-closed-note", "Wir nehmen gerade keine Online-Bestellungen an.")
                        } else {
                            ("order-closed-note hidden", "")
                        };
                        // Giveaway (free Pizzabrötchen). Three states:
                        //   • not qualified yet → nudge "noch X € bis zum …"
                        //   • qualified, unclaimed → "Jetzt sichern" + sauce choice
                        //   • claimed → shown as a 0 € line in the list (no block)
                        let giveaway = cart.giveaway.clone();
                        let give_view = giveaway.map(|g| {
                            if g.claimed {
                                // Already in the cart; the line list shows it.
                                view! { <></> }.into_any()
                            } else if g.qualifies {
                                let sauce = RwSignal::new(String::new());
                                let claim = ctx.claim_giveaway;
                                let on_claim = move |_| {
                                    let s = sauce.get();
                                    if !s.is_empty() {
                                        claim.dispatch(ClaimGiveaway { sauce_option_id: s });
                                    }
                                };
                                let claim_pending = move || sauce.get().is_empty();
                                view! {
                                    <div class="giveaway-offer">
                                        <p class="giveaway-headline">
                                            {crate::t!("cart.giveaway_unlocked")}
                                        </p>
                                        <p class="giveaway-sauce-label">
                                            {crate::t!("cart.giveaway_sauce_prompt")}
                                        </p>
                                        <label class="giveaway-sauce">
                                            <input type="radio" name="giveaway-sauce"
                                                value="opt-sauce-knoblauch"
                                                on:change=move |_| sauce.set("opt-sauce-knoblauch".to_string())/>
                                            <span>"Knoblauchsauce"</span>
                                        </label>
                                        <label class="giveaway-sauce">
                                            <input type="radio" name="giveaway-sauce"
                                                value="opt-sauce-kraeuter"
                                                on:change=move |_| sauce.set("opt-sauce-kraeuter".to_string())/>
                                            <span>"Kräuterbutter"</span>
                                        </label>
                                        <button class="btn primary small giveaway-claim"
                                            disabled=claim_pending
                                            on:click=on_claim>
                                            {crate::t!("cart.giveaway_claim")}
                                        </button>
                                    </div>
                                }
                                .into_any()
                            } else {
                                // Not qualified yet — nudge toward the threshold.
                                let missing = (g.min_order_cents - cart.subtotal_cents).max(0);
                                view! {
                                    <p class="giveaway-hint">
                                        {crate::t!("cart.giveaway_progress")
                                            .replace("{amount}", &format_eur(missing))}
                                    </p>
                                }
                                .into_any()
                            }
                        });

                        view! {
                            <div class="row">
                                <span>{crate::t!("cart.subtotal")}</span>
                                <strong>{total}</strong>
                            </div>
                            <p class=banner_cls>{banner_text}</p>
                            {give_view}
                            <p class=closed_note_cls>{closed_note_text}</p>
                            <p class="hint">{crate::t!("cart.payment_hint")}</p>
                            <a class=cta_cls href="/checkout" on:click=close>{crate::t!("cart.proceed")}</a>
                            <button class="btn ghost"
                                disabled=!has_items
                                on:click=move |_| { ctx.clear.dispatch(ClearCart {}); }>
                                {crate::t!("cart.clear")}
                            </button>
                        }
                    }}
                </Suspense>
            </footer>
        </aside>
    }
}

#[component]
fn Lines(cart: CartView) -> impl IntoView {
    view! {
        <ul class="cart-lines">
            {cart.lines.into_iter().map(|line| view! { <Line line/> }).collect_view()}
        </ul>
    }
}

#[component]
fn Line(line: CartLine) -> impl IntoView {
    let ctx = use_cart_ctx();
    let id = line.id.clone();
    let qty = line.quantity;

    let size_label = match line.size {
        SizeChoice::Single => None,
        _ => line.size_label.clone(),
    };

    let dec = {
        let id = id.clone();
        move |_| {
            ctx.update.dispatch(UpdateCartLine {
                line_id: id.clone(),
                quantity: qty - 1,
            });
        }
    };
    let inc = {
        let id = id.clone();
        move |_| {
            ctx.update.dispatch(UpdateCartLine {
                line_id: id.clone(),
                quantity: qty + 1,
            });
        }
    };
    let remove = {
        let id = id.clone();
        move |_| {
            ctx.update.dispatch(UpdateCartLine {
                line_id: id.clone(),
                quantity: 0,
            });
        }
    };

    // "Bearbeiten" — remove this line and navigate back to the menu
    // item so the customer can re-pick extras. We pass the previous
    // extras through the URL hash so the menu page can pre-fill the
    // checkboxes; that's lighter than threading state through a
    // signal and survives a full page reload.
    let edit = {
        let id = id.clone();
        let menu_item_id = line.menu_item_id.clone();
        let extras_ids: Vec<String> = line.extras.iter().map(|e| e.id.clone()).collect();
        move |_| {
            ctx.update.dispatch(UpdateCartLine {
                line_id: id.clone(),
                quantity: 0,
            });
            ctx.open.set(false);
            let hash = if extras_ids.is_empty() {
                format!("#{}", menu_item_id)
            } else {
                format!("#{}?extras={}", menu_item_id, extras_ids.join(","))
            };
            // window.location.assign — full navigation so the menu
            // page mounts fresh and reads the hash on init.
            #[cfg(feature = "hydrate")]
            if let Some(win) = web_sys::window() {
                let _ = win.location().set_href(&format!("/menu{hash}"));
            }
            #[cfg(not(feature = "hydrate"))]
            {
                let _ = &hash; // silence unused on SSR
            }
        }
    };

    let extras_render = (!line.extras.is_empty()).then(|| {
        let items = line.extras.clone();
        view! {
            <ul class="extras">
                {items.into_iter().map(|e| {
                    let price_label = if e.price_cents == 0 {
                        crate::t!("common.free")
                    } else {
                        format!("+{}", format_eur(e.price_cents))
                    };
                    view! {
                        <li>
                            "+ " {e.label} " "
                            <span class="muted">{price_label}</span>
                        </li>
                    }
                }).collect_view()}
            </ul>
        }
    });

    // The giveaway line is a single free item: render the chosen sauce, badge
    // it "Gratis", and drop the quantity stepper + edit button (it's fixed at
    // qty 1 and has no editable extras). It stays removable.
    let is_giveaway = line.is_giveaway;
    let sauce_text = line
        .selected_options
        .iter()
        .map(|o| o.option_label.clone())
        .collect::<Vec<_>>()
        .join(", ");

    if is_giveaway {
        return view! {
            <li class="cart-line giveaway-line">
                <div class="info">
                    <strong>"🎁 " {line.name.clone()}</strong>
                    {(!sauce_text.is_empty()).then(|| view! {
                        <div class="meta">{sauce_text}</div>
                    })}
                </div>
                <div class="actions">
                    <strong class="line-total giveaway-free">{crate::t!("common.free")}</strong>
                    <div class="line-buttons">
                        <button class="remove" on:click=remove aria-label=crate::t!("cart.remove")>"🗑"</button>
                    </div>
                </div>
            </li>
        }
        .into_any();
    }

    view! {
        <li class="cart-line">
            <div class="info">
                <strong>{line.name.clone()}</strong>
                {size_label.map(|s| view! { <span class="variant">" — " {s}</span> })}
                <div class="meta">
                    {format_eur(line.unit_price_cents)} " × " {line.quantity}
                </div>
                {extras_render}
            </div>
            <div class="actions">
                <div class="qty">
                    <button on:click=dec aria-label=crate::t!("cart.decrement")>"−"</button>
                    <span>{qty}</span>
                    <button on:click=inc aria-label=crate::t!("cart.increment")>"+"</button>
                </div>
                <strong class="line-total">{format_eur(line.line_total_cents)}</strong>
                <div class="line-buttons">
                    <button class="edit" on:click=edit aria-label=crate::t!("cart.edit_extras") title=crate::t!("cart.edit_extras")>"✎"</button>
                    <button class="remove" on:click=remove aria-label=crate::t!("cart.remove")>"🗑"</button>
                </div>
            </div>
        </li>
    }
    .into_any()
}
