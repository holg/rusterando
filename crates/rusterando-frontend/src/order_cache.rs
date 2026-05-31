//! Per-tab localStorage cache of the customer's recent
//! `OrderDetail`s. Lets `/orders/<id>` render INSTANTLY on revisit
//! without waiting for the SSR/server round-trip — pure WASM, no
//! server-side state.
//!
//! Layout (one key per order, plus an index):
//!   * `dp_order:<id>`  → JSON of `OrderDetail`
//!   * `dp_orders`      → JSON `["<id1>","<id2>",…]` newest-first,
//!                        capped to MAX_CACHED entries. On overflow
//!                        the oldest entry's blob is deleted too.
//!
//! Why one-key-per-order rather than a single big array: avoids
//! rewriting MAX_CACHED blobs on every save, and lets the eviction
//! step touch O(1) keys. localStorage is synchronous + small, so
//! both write strategies are cheap, but per-key generalises better
//! if we ever want a per-order TTL.
//!
//! SSR is a no-op: every function returns `None` / does nothing
//! when compiled without the `hydrate` feature. That guarantees SSR
//! and the first hydrate paint render identical DOM (the cache only
//! kicks in via post-hydration Effects).
//!
//! Per [[feedback-in-memory-first]] this lives in the browser
//! window, not on the server.

use crate::pages::order::OrderDetail;

/// How many `OrderDetail` blobs we keep around at most. Customer
/// realistically only revisits the most recent 1-2 orders; 5 is
/// enough headroom for a family that splits orders or a household
/// re-ordering across days. Each blob is a few KB at most, so 5×
/// stays well under the 5 MB-ish localStorage quota every browser
/// honours.
// SSR target compiles these unused (the bodies that use them are
// cfg-gated to `hydrate`). Keep the constants visible regardless so
// the schema is documented in one place; allow(dead_code) silences
// the SSR-only warning.
#[allow(dead_code)]
const MAX_CACHED: usize = 5;
#[allow(dead_code)]
const INDEX_KEY: &str = "dp_orders";
#[allow(dead_code)]
const BLOB_PREFIX: &str = "dp_order:";

#[cfg(feature = "hydrate")]
fn storage() -> Option<web_sys::Storage> {
    // window().local_storage() returns Result<Option<Storage>, JsValue>
    // — both arms can fail (sandboxed iframe, private mode without
    // storage, etc.). On any failure we just degrade to "cache is
    // off" rather than panicking.
    web_sys::window()?.local_storage().ok().flatten()
}

/// Fetch a cached `OrderDetail` for `order_id`. Returns `None` on
/// cache miss, on parse error (forward-compat schema drift), or
/// when localStorage is unavailable. Safe to call during render.
pub fn load(order_id: &str) -> Option<OrderDetail> {
    #[cfg(feature = "hydrate")]
    {
        let s = storage()?;
        let key = format!("{BLOB_PREFIX}{order_id}");
        let raw = s.get_item(&key).ok().flatten()?;
        // Parse failure → silently treat as miss. Schema-drift over
        // a deploy boundary is exactly when this matters: we'd
        // rather refetch from the server than render half-decoded
        // fields. Drop the bad row so we don't keep retrying it.
        match serde_json::from_str::<OrderDetail>(&raw) {
            Ok(o) => Some(o),
            Err(_) => {
                let _ = s.remove_item(&key);
                None
            }
        }
    }
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = order_id;
        None
    }
}

/// Cache `order` and bump it to the head of the recency index.
/// Evicts the oldest entry's blob when the index would exceed
/// `MAX_CACHED`. No-op on SSR or when localStorage is denied.
pub fn save(order: &OrderDetail) {
    #[cfg(feature = "hydrate")]
    {
        let Some(s) = storage() else { return };
        let id = &order.id;
        // Serialise FIRST — if this fails we don't want to mutate
        // the index.
        let Ok(blob) = serde_json::to_string(order) else { return };
        let blob_key = format!("{BLOB_PREFIX}{id}");
        // setItem can fail if the quota is hit (5 MB-ish on most
        // browsers). Swallow the error — the cache is a "nice to
        // have", never load-bearing.
        if s.set_item(&blob_key, &blob).is_err() {
            return;
        }
        // Index: move id to front, dedupe, trim to MAX_CACHED.
        let mut ids: Vec<String> = s
            .get_item(INDEX_KEY)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        ids.retain(|x| x != id);
        ids.insert(0, id.clone());
        // Evict overflow: drop the blobs for ids we're about to
        // forget. Anything beyond MAX_CACHED is unreachable from
        // the index and would just sit in localStorage taking
        // space until the user clears the site data.
        for evicted in ids.iter().skip(MAX_CACHED) {
            let _ = s.remove_item(&format!("{BLOB_PREFIX}{evicted}"));
        }
        ids.truncate(MAX_CACHED);
        if let Ok(index_json) = serde_json::to_string(&ids) {
            let _ = s.set_item(INDEX_KEY, &index_json);
        }
    }
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = order;
    }
}
