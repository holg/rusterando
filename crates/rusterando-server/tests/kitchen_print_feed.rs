//! The print feed for WebSocket kitchen devices (/api/kitchen/ws): a device's
//! first connect starts from now (no backlog printed), a reconnect replays
//! the unacked events after its last seq and then goes live without
//! duplicates, and PrinterPresent lands in kitchen_clients.

use std::time::Duration;

use kitchen_protocol::{ClientMessage, KitchenEvent, SeqId, ServerMessage};
use rusterando_server::kitchen::KitchenChannel;
use sqlx::sqlite::SqlitePoolOptions;
use tokio::sync::mpsc::Receiver;

fn event(n: u32) -> KitchenEvent {
    KitchenEvent::Alert { title: format!("t{n}"), text: String::new() }
}

async fn next(rx: &mut Receiver<ServerMessage>) -> Option<u64> {
    tokio::time::timeout(Duration::from_millis(500), rx.recv())
        .await
        .ok()
        .flatten()
        .map(|m| m.seq_id.0)
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

#[tokio::test]
async fn first_connect_starts_from_now() {
    let (chan, _db) = channel().await;
    chan.broadcast(event(1)).await.unwrap();
    chan.broadcast(event(2)).await.unwrap();

    let mut rx = chan.print_feed("tab5".into(), "0.1.0".into(), "riscv".into(), None).await;
    tokio::time::sleep(Duration::from_millis(100)).await; // subscribed
    assert_eq!(next(&mut rx).await, None, "old unacked events must not be replayed");

    let seq = chan.broadcast(event(3)).await.unwrap();
    assert_eq!(next(&mut rx).await, Some(seq.0));
}

#[tokio::test]
async fn reconnect_replays_after_last_seen_without_duplicates() {
    let (chan, _db) = channel().await;
    let s1 = chan.broadcast(event(1)).await.unwrap();
    let s2 = chan.broadcast(event(2)).await.unwrap();
    let s3 = chan.broadcast(event(3)).await.unwrap();

    let mut rx = chan.print_feed("tab5".into(), "0.1.0".into(), "riscv".into(), Some(s1)).await;
    assert_eq!(next(&mut rx).await, Some(s2.0));
    assert_eq!(next(&mut rx).await, Some(s3.0));
    let s4 = chan.broadcast(event(4)).await.unwrap();
    assert_eq!(next(&mut rx).await, Some(s4.0));
    assert_eq!(next(&mut rx).await, None, "nothing twice");

    // An acked event is not replayed again.
    chan.device_message("tab5", ClientMessage::Ack(s2)).await.unwrap();
    let mut rx = chan.print_feed("tab5".into(), "0.1.0".into(), "riscv".into(), Some(SeqId(0))).await;
    assert_eq!(next(&mut rx).await, Some(s1.0));
    assert_eq!(next(&mut rx).await, Some(s3.0));
}

#[tokio::test]
async fn printer_presence_is_recorded() {
    let (chan, db) = channel().await;
    let _rx = chan.print_feed("tab5".into(), "0.1.0".into(), "riscv".into(), None).await;

    let has = || async {
        sqlx::query_scalar::<_, Option<i64>>("SELECT has_printer FROM kitchen_clients WHERE shop_slug = 'tab5'")
            .fetch_one(&db)
            .await
            .unwrap()
    };
    assert_eq!(has().await, Some(0), "not printing until the device reports a printer");
    chan.device_message("tab5", ClientMessage::PrinterPresent(true)).await.unwrap();
    assert_eq!(has().await, Some(1));
    chan.device_message("tab5", ClientMessage::PrinterPresent(false)).await.unwrap();
    assert_eq!(has().await, Some(0));
}
