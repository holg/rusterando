use leptos::prelude::*;
use rusterando_shared::models::{format_eur, CartLine, CartView, SizeChoice};

use crate::pages::cart::{get_cart, AddToCart, ClearCart, UpdateCartLine};

/// Shared cart state. One Resource (the source of truth) plus three actions
/// (add / update / clear). All routes that need the cart pull this via context.
#[derive(Clone, Copy)]
pub struct CartCtx {
    pub cart: Resource<Result<CartView, ServerFnError>>,
    pub open: RwSignal<bool>,
    pub add: ServerAction<AddToCart>,
    pub update: ServerAction<UpdateCartLine>,
    pub clear: ServerAction<ClearCart>,
    /// Manual refetch trigger. Routes that mutate the cart through other
    /// actions (e.g. place_order clearing it server-side) call `refresh()`
    /// to force the FAB / drawer back into sync.
    pub bump: RwSignal<u32>,
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

    let cart = Resource::new(move || bump.get(), |_| async move { get_cart().await });

    provide_context(CartCtx {
        cart,
        open: RwSignal::new(false),
        add,
        update,
        clear,
        bump,
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
            <button class="cart-fab hidden" aria-label="Warenkorb öffnen" aria-hidden="true">
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
                    <button class=cls aria-label="Warenkorb öffnen" aria-hidden=aria
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
                <h2>"Warenkorb"</h2>
                <button class="close" aria-label="Schließen" on:click=close>"×"</button>
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
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || ctx.cart.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(cart) if cart.lines.is_empty() => {
                            view! { <p class="empty">"Ihr Warenkorb ist leer."</p> }.into_any()
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
                        <span>"Zwischensumme"</span>
                        <strong>"…"</strong>
                    </div>
                    <p class="free-delivery hidden"></p>
                    <p class="hint">"Zahlung an der Kasse oder online beim Bestellen."</p>
                    <a class="btn primary disabled" href="/checkout">"Zur Kasse"</a>
                    <button class="btn ghost" disabled=true>"Warenkorb leeren"</button>
                }>
                    {move || {
                        let cart = ctx.cart.get().and_then(|r| r.ok()).unwrap_or_default();
                        let total = format_eur(cart.subtotal_cents);
                        let has_items = !cart.lines.is_empty();
                        let threshold = cart.free_delivery_threshold_cents;
                        let unlocked = threshold > 0 && cart.subtotal_cents >= threshold;
                        let missing = (threshold - cart.subtotal_cents).max(0);
                        let (banner_cls, banner_text) = if threshold > 0 && unlocked {
                            ("free-delivery ok", "✓ Lieferung kostenlos".to_string())
                        } else if threshold > 0 && !unlocked && has_items {
                            (
                                "free-delivery hint",
                                format!("Noch {} bis kostenlose Lieferung.", format_eur(missing)),
                            )
                        } else {
                            ("free-delivery hidden", String::new())
                        };
                        let cta_cls = if has_items { "btn primary" } else { "btn primary disabled" };
                        view! {
                            <div class="row">
                                <span>"Zwischensumme"</span>
                                <strong>{total}</strong>
                            </div>
                            <p class=banner_cls>{banner_text}</p>
                            <p class="hint">"Zahlung an der Kasse oder online beim Bestellen."</p>
                            <a class=cta_cls href="/checkout" on:click=close>"Zur Kasse"</a>
                            <button class="btn ghost"
                                disabled=!has_items
                                on:click=move |_| { ctx.clear.dispatch(ClearCart {}); }>
                                "Warenkorb leeren"
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

    let extras_render = (!line.extras.is_empty()).then(|| {
        let items = line.extras.clone();
        view! {
            <ul class="extras">
                {items.into_iter().map(|e| {
                    let price_label = if e.price_cents == 0 {
                        "gratis".to_string()
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
                    <button on:click=dec aria-label="weniger">"−"</button>
                    <span>{qty}</span>
                    <button on:click=inc aria-label="mehr">"+"</button>
                </div>
                <strong class="line-total">{format_eur(line.line_total_cents)}</strong>
                <button class="remove" on:click=remove aria-label="entfernen">"🗑"</button>
            </div>
        </li>
    }
}
