//! A delivery fee is snapshotted on the order: changing the zone's fee
//! later must not change what an existing order shows (receipts, PDFs and
//! Buchhaltung read the snapshot). Also proves the geo migration applies on
//! a fresh database.

use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn zone_fee_change_leaves_existing_order_untouched() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite");
    sqlx::migrate!().run(&pool).await.expect("migrations");

    sqlx::query(
        "INSERT INTO delivery_zones (id, postcode, name, fee_cents, min_order_cents, eta_minutes, is_active,
                                     priority, geometry_kind, geometry)
         VALUES ('z-centre', '00000-test', 'Innenstadt', 150, 1500, 30, 1, 10, 'radius', '51.7701,7.4419,2000')",
    )
    .execute(&pool)
    .await
    .unwrap();

    // The order stores the fee it was placed with, plus the resolved point.
    sqlx::query(
        "INSERT INTO orders (id, order_number, order_type, status, contact_name, contact_phone,
                             contact_email, delivery_address_json, subtotal_cents, delivery_fee_cents,
                             total_cents, payment_status, lat, lon, address_verified)
         VALUES ('o1', 'T-1', 'delivery', 'new', 'Test', '0123', 't@example.com',
                 '{\"zone_id\":\"z-centre\",\"zone_name\":\"Innenstadt\",\"verified\":true}',
                 2000, 150, 2150, 'pending', 51.77, 7.44, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("UPDATE delivery_zones SET fee_cents = 999 WHERE id = 'z-centre'")
        .execute(&pool)
        .await
        .unwrap();

    let (fee, total, verified, lat): (i64, i64, i64, Option<f64>) = sqlx::query_as(
        "SELECT delivery_fee_cents, total_cents, address_verified, lat FROM orders WHERE id = 'o1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((fee, total, verified), (150, 2150, 1));
    assert_eq!(lat, Some(51.77));

    // New orders would see the new fee — read through the zone, not the order.
    let live: i64 =
        sqlx::query_scalar("SELECT fee_cents FROM delivery_zones WHERE id = 'z-centre'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(live, 999);

    // Geo tables + admin switch exist after the migration.
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM geo_streets")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let mode: String =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'address_lookup_mode'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(mode, "nominatim", "old behaviour is the default");
}
