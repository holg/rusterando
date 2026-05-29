//! Live customer channel hub (SSE).
//!
//! A process-wide `tokio::sync::broadcast` of [`LiveEvent`]s. Server fns
//! (e.g. `set_order_message`, status updates) publish to it; the
//! `/api/live/orders/{id}` SSE handler in the server binary subscribes and
//! fans matching events out to the customer's open `/orders/{id}` page.
//!
//! Lives in the frontend crate (server-only) so the `#[server]` fns there
//! can reach it via `use_context::<LiveHub>()` — the same arrangement as
//! `OrdersPausedHandle` and the other runtime handles. Held in the
//! server's `AppState` and provided into context at request time.

#[cfg(feature = "ssr")]
pub use ssr::{broadcast_shop_status, LiveHub};

#[cfg(feature = "ssr")]
mod ssr {
    use rusterando_shared::models::LiveEvent;
    use tokio::sync::broadcast;

    /// Broadcast hub for live order events. Cloneable; all clones share the
    /// same channel. Capacity is generous — events are tiny and consumers
    /// (open order pages) are few; a lagging receiver just drops old events
    /// (fine, the page also has the persisted state).
    #[derive(Clone)]
    pub struct LiveHub {
        tx: broadcast::Sender<LiveEvent>,
    }

    impl Default for LiveHub {
        fn default() -> Self {
            Self::new()
        }
    }

    impl LiveHub {
        pub fn new() -> Self {
            let (tx, _rx) = broadcast::channel(512);
            Self { tx }
        }

        /// Publish an event. Errors only when there are no subscribers,
        /// which is normal (no one watching that order) — ignored.
        pub fn send(&self, ev: LiveEvent) {
            let _ = self.tx.send(ev);
        }

        /// New receiver for an SSE connection.
        pub fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
            self.tx.subscribe()
        }
    }

    /// Recompute the shop's open/closed state from the DB and broadcast it
    /// as a `ShopStatus` event (consumed by `/api/live/shop` → home + cart).
    /// Call after any change that can flip the state: pause, snooze,
    /// force-open, or a snooze expiring. `order_id` is empty for these
    /// global events.
    pub async fn broadcast_shop_status(db: &sqlx::SqlitePool, hub: &LiveHub) {
        let (level, reason) = crate::pages::order::ssr::shop_open_state(db).await;
        hub.send(rusterando_shared::models::LiveEvent {
            order_id: String::new(),
            kind: rusterando_shared::models::LiveKind::ShopStatus { level, reason },
        });
    }
}
