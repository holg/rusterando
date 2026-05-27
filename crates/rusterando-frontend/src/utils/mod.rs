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
