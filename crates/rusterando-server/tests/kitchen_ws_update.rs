//! Updates for WebSocket kitchen devices (the ESP32-P4 displays): the
//! admin-armed offer for the device's arch, the chunks that rebuild the
//! staged image, and the reported result — the same handshake the tunnel
//! clients get, without touching the tunnel path.

use std::path::PathBuf;

use kitchen_protocol::KitchenEvent;
use rusterando_server::kitchen::KitchenChannel;
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;

const ARCH: &str = "esp32p4";

/// Stage an image under a test artifact dir. Must run before the server's
/// artifact store is first used (it reads PRINTER_ARTIFACT_DIR once).
fn stage(version: &str, image: &[u8]) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("kitchen-ws-update");
    std::fs::create_dir_all(dir.join(ARCH)).unwrap();
    std::fs::write(dir.join(ARCH).join("version"), version).unwrap();
    std::fs::write(dir.join(ARCH).join("rusterando-printer"), image).unwrap();
    std::env::set_var("PRINTER_ARTIFACT_DIR", &dir);
}

async fn channel() -> (KitchenChannel, sqlx::SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite");
    sqlx::migrate!().run(&pool).await.expect("migrations");
    (KitchenChannel::new(pool.clone()), pool)
}

async fn arm(db: &sqlx::SqlitePool, armed: bool) {
    sqlx::query("UPDATE kitchen_clients SET update_armed = ?1 WHERE shop_slug = 'tab5'")
        .bind(armed as i64)
        .execute(db)
        .await
        .unwrap();
}

async fn armed(db: &sqlx::SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT update_armed FROM kitchen_clients WHERE shop_slug = 'tab5'")
        .fetch_one(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn armed_update_is_offered_streamed_and_recorded() {
    let image: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 251) as u8).collect();
    stage("0.2.0-test", &image);
    let (chan, db) = channel().await;
    let _feed = chan.print_feed("tab5".into(), "0.1.0".into(), ARCH.into(), None).await;

    assert!(chan.update_offer("tab5", "0.1.0", ARCH).await.is_none(), "not armed: no offer");

    arm(&db, true).await;
    let Some(KitchenEvent::UpdateOffer { target_version, sha256, size, .. }) =
        chan.update_offer("tab5", "0.1.0", ARCH).await
    else {
        panic!("armed: offer expected");
    };
    assert_eq!(target_version, "0.2.0-test");
    assert_eq!(size, image.len() as u64);
    assert_eq!(sha256, <[u8; 32]>::from(Sha256::digest(&image)));
    assert!(chan.update_offer("tab5", "0.1.0", "aarch64").await.is_none(), "no image for that arch");

    // The device's FetchArtifact loop rebuilds the image exactly.
    let mut rebuilt = Vec::new();
    loop {
        let Some(KitchenEvent::ArtifactChunk { offset, bytes, last, .. }) =
            chan.artifact_chunk(ARCH, rebuilt.len() as u64)
        else {
            panic!("chunk expected at {}", rebuilt.len());
        };
        assert_eq!(offset, rebuilt.len() as u64);
        rebuilt.extend_from_slice(&bytes);
        if last {
            break;
        }
    }
    assert_eq!(rebuilt, image);
    assert!(chan.artifact_chunk(ARCH, image.len() as u64 + 1).is_none(), "past the end");

    // The reported result is recorded and disarms.
    chan.update_result("tab5", "0.2.0-test", true, "").await;
    assert_eq!(armed(&db).await, 0);
    let (to, ok): (String, i64) =
        sqlx::query_as("SELECT last_update_to, last_update_ok FROM kitchen_clients WHERE shop_slug = 'tab5'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!((to.as_str(), ok), ("0.2.0-test", 1));

    // A device already on the staged version gets no offer and is disarmed.
    arm(&db, true).await;
    assert!(chan.update_offer("tab5", "0.2.0-test", ARCH).await.is_none());
    assert_eq!(armed(&db).await, 0);
}
