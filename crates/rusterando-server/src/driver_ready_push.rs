//! "Abholbereit" push to the drivers — a delivery order reached the ready
//! column.
//!
//! Drivers get the new-order push when a delivery comes in, but not the
//! moment it can actually leave. Every board that moves an order (web
//! kitchen board, admin board, the kitchen display terminal and its browser
//! mirror) publishes a LiveHub `Status` event, so one subscriber here covers
//! them all instead of a push call in each status writer.
//!
//! One push per order per process: a Ready → In Arbeit → Ready mis-tap undo
//! doesn't ping the drivers twice.

use std::collections::HashSet;

use rusterando_frontend::live::LiveHub;
use rusterando_shared::models::LiveKind;
use sqlx::SqlitePool;
use tokio::sync::broadcast::error::RecvError;

use crate::apns::ApnsHandle;

/// Forget the pushed-order set beyond this size; a day's orders fit easily.
const SEEN_CAP: usize = 1000;

/// Spawn the listener. Watches Davids' global pool, like the printer monitor
/// and the APNs device table it pushes to; a tenant's order id isn't found
/// there and is skipped.
pub fn spawn(db: SqlitePool, live: LiveHub, apns: ApnsHandle) {
    tokio::spawn(async move {
        let mut rx = live.subscribe();
        let mut seen: HashSet<String> = HashSet::new();
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                // Missed a burst: those orders get no push, the board still
                // shows them.
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(target: "driver_push", missed = n, "live events lagged");
                    continue;
                }
                Err(RecvError::Closed) => break,
            };
            let LiveKind::Status(status) = &ev.kind else {
                continue;
            };
            if status != "ready_for_pickup" || seen.contains(&ev.order_id) {
                continue;
            }
            match ready_delivery(&db, &ev.order_id).await {
                Ok(Some((title, body))) => {
                    if seen.len() >= SEEN_CAP {
                        seen.clear();
                    }
                    seen.insert(ev.order_id.clone());
                    tracing::info!(target: "driver_push", order_id = %ev.order_id, "delivery ready → push to drivers");
                    apns.notify_roles_urgent(&["driver"], &title, &body, Some("/driver"));
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(target: "driver_push", order_id = %ev.order_id, error = %e, "order lookup failed");
                }
            }
        }
        tracing::info!(target: "driver_push", "live hub closed; listener ended");
    });
}

/// Title + body for a delivery order that is ready, `None` for anything
/// else (pickup order, unknown id, status moved on in the meantime).
async fn ready_delivery(
    db: &SqlitePool,
    order_id: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT order_number, contact_name, delivery_address_json FROM orders \
         WHERE id = ?1 AND order_type = 'delivery' AND status = 'ready_for_pickup'",
    )
    .bind(order_id)
    .fetch_optional(db)
    .await?;
    let Some((number, name, addr_json)) = row else {
        return Ok(None);
    };
    let address = addr_json
        .as_deref()
        .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .map(|v| {
            let field = |k: &str| {
                v.get(k)
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };
            format!("{} {}", field("street"), field("house_number"))
                .trim()
                .to_string()
        })
        .unwrap_or_default();
    let body = if address.is_empty() {
        name
    } else {
        format!("{name} · {address}")
    };
    Ok(Some((format!("Abholbereit: Bestellung {number}"), body)))
}

#[cfg(test)]
mod tests {
    use super::ready_delivery;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn order(
        pool: &sqlx::SqlitePool,
        id: &str,
        order_type: &str,
        status: &str,
        addr: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO orders (id, order_number, order_type, status, contact_name, contact_phone,
                                 contact_email, delivery_address_json, subtotal_cents, total_cents,
                                 payment_status)
             VALUES (?1, ?1, ?2, ?3, 'Erika Muster', '0123', 'e@example.com', ?4, 2000, 2000, 'pending')",
        )
        .bind(id)
        .bind(order_type)
        .bind(status)
        .bind(addr)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn only_ready_deliveries_get_a_driver_push() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!().run(&pool).await.expect("migrations");

        order(
            &pool,
            "D-1",
            "delivery",
            "ready_for_pickup",
            Some(r#"{"street":"Marktstraße","house_number":"7","postcode":"45711"}"#),
        )
        .await;
        order(&pool, "D-2", "delivery", "ready_for_pickup", None).await;
        order(&pool, "P-1", "pickup", "ready_for_pickup", None).await;
        // Moved on before the lookup ran (e.g. straight to the driver).
        order(&pool, "D-3", "delivery", "out_for_delivery", None).await;

        assert_eq!(
            ready_delivery(&pool, "D-1").await.unwrap(),
            Some((
                "Abholbereit: Bestellung D-1".into(),
                "Erika Muster · Marktstraße 7".into()
            )),
        );
        assert_eq!(
            ready_delivery(&pool, "D-2").await.unwrap(),
            Some(("Abholbereit: Bestellung D-2".into(), "Erika Muster".into())),
        );
        assert_eq!(ready_delivery(&pool, "P-1").await.unwrap(), None);
        assert_eq!(ready_delivery(&pool, "D-3").await.unwrap(), None);
        assert_eq!(ready_delivery(&pool, "unknown").await.unwrap(), None);
    }
}
