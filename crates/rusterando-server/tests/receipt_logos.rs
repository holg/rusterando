//! Receipt logos rendered on the server (crates/kitchen-protocol/RECEIPT_LOGOS.md):
//! new jobs are stored with logo references, every send resolves them for
//! the receiving client, jobs stored before (logo inline) print on every
//! client too, and the one-time migration moves their SVG into the logo
//! store without changing anything else about them.

use kitchen_protocol::receipt::{raster_hash, LineStyle, ReceiptLine, ReceiptProgram};
use kitchen_protocol::{KitchenEvent, SeqId, ServerMessage};
use rusterando_server::kitchen::KitchenChannel;
use rusterando_server::receipt_logos::WireCaps;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

const LOGO: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="10"><rect width="40" height="10" fill="white"/><rect x="5" y="2" width="30" height="6" fill="black"/></svg>"#;
const WIDTH: u16 = 32;

fn hash() -> [u8; 32] {
    raster_hash(LOGO.as_bytes(), WIDTH)
}

fn job(svg: &[u8]) -> KitchenEvent {
    KitchenEvent::Print(ReceiptProgram {
        display_number: 7,
        is_reprint: false,
        lines: vec![
            ReceiptLine::RasterSvg {
                hash: hash(),
                target_width: WIDTH,
                svg: svg.to_vec(),
            },
            ReceiptLine::Text {
                text: "DP-0001".into(),
                style: LineStyle::NORMAL,
            },
            ReceiptLine::Cut { feed: 3 },
        ],
    })
}

fn bits_client() -> WireCaps {
    WireCaps {
        raster_bits: true,
        ..Default::default()
    }
}

fn svg_client() -> WireCaps {
    WireCaps::default()
}

async fn channel() -> (KitchenChannel, SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite");
    sqlx::migrate!().run(&pool).await.expect("migrations");
    (KitchenChannel::new(pool.clone()), pool)
}

/// A job as the server stored it before the logo store: logo inline, under
/// the 2.2 header.
async fn insert_old_job(db: &SqlitePool, acked_at: Option<i64>) -> i64 {
    let mut payload = vec![2u8, 2];
    payload.extend(postcard::to_allocvec(&job(LOGO.as_bytes())).unwrap());
    sqlx::query_scalar(
        "INSERT INTO kitchen_outbox (payload, created_at, acked_at) \
         VALUES (?, 1700000000, ?) RETURNING id",
    )
    .bind(payload)
    .bind(acked_at)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn payload(db: &SqlitePool, id: i64) -> Vec<u8> {
    sqlx::query_scalar("SELECT payload FROM kitchen_outbox WHERE id = ?")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

fn stored_job(payload: &[u8]) -> ServerMessage {
    let (_minor, body) = kitchen_protocol::decode_versioned(payload).unwrap();
    ServerMessage {
        seq_id: SeqId(0),
        event: postcard::from_bytes(body).unwrap(),
    }
}

fn logo_line(msg: &ServerMessage) -> Option<&ReceiptLine> {
    let KitchenEvent::Print(program) = &msg.event else {
        panic!("not a print job");
    };
    program.lines.iter().find(|l| {
        matches!(
            l,
            ReceiptLine::RasterSvg { .. } | ReceiptLine::RasterBits { .. }
        )
    })
}

fn expected_bits() -> Vec<u8> {
    receipt_escpos::svg::rasterize(LOGO.as_bytes(), WIDTH)
        .unwrap()
        .bytes
}

async fn logo_rows(db: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM receipt_logos")
        .fetch_one(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn new_jobs_are_stored_with_a_reference_and_resolved_per_client() {
    let (chan, db) = channel().await;
    let seq = chan.broadcast(job(LOGO.as_bytes())).await.unwrap();

    // Stored: the reference, the logo once in the store.
    let stored = stored_job(&payload(&db, seq.0 as i64).await);
    assert!(matches!(
        logo_line(&stored),
        Some(ReceiptLine::RasterSvg { svg, .. }) if svg.is_empty()
    ));
    assert_eq!(logo_rows(&db).await, 1);

    // A 2.3 device: the server-rendered raster, the same pixels a device
    // rendering the SVG would get.
    let sent = chan
        .logos()
        .wire_message(stored.clone(), &bits_client())
        .await;
    match logo_line(&sent) {
        Some(ReceiptLine::RasterBits {
            hash: h,
            width,
            height,
            bits,
        }) => {
            assert_eq!(*h, hash());
            assert_eq!(*width, WIDTH);
            assert_eq!(*height, 8);
            assert_eq!(bits, &expected_bits());
        }
        other => panic!("expected RasterBits, got {other:?}"),
    }

    // An older device: the SVG, back in full.
    let sent = chan
        .logos()
        .wire_message(stored.clone(), &svg_client())
        .await;
    assert!(matches!(
        logo_line(&sent),
        Some(ReceiptLine::RasterSvg { svg, .. }) if svg == LOGO.as_bytes()
    ));

    // An older tunnel client that has the logo cached: just the reference.
    let cached = WireCaps {
        raster_bits: false,
        cached: [hash()].into_iter().collect(),
    };
    let sent = chan.logos().wire_message(stored, &cached).await;
    assert!(matches!(
        logo_line(&sent),
        Some(ReceiptLine::RasterSvg { svg, .. }) if svg.is_empty()
    ));
}

#[tokio::test]
async fn old_jobs_with_the_logo_inline_print_on_every_client() {
    let (chan, db) = channel().await;
    let id = insert_old_job(&db, None).await;
    let stored = stored_job(&payload(&db, id).await);

    let sent = chan
        .logos()
        .wire_message(stored.clone(), &bits_client())
        .await;
    assert!(matches!(
        logo_line(&sent),
        Some(ReceiptLine::RasterBits { bits, .. }) if *bits == expected_bits()
    ));
    let sent = chan.logos().wire_message(stored, &svg_client()).await;
    assert!(matches!(
        logo_line(&sent),
        Some(ReceiptLine::RasterSvg { svg, .. }) if svg == LOGO.as_bytes()
    ));
}

#[tokio::test]
async fn an_unknown_reference_is_left_out_for_bits_clients() {
    let (chan, _db) = channel().await;
    let reference = ServerMessage {
        seq_id: SeqId(1),
        event: job(b""),
    };
    let sent = chan
        .logos()
        .wire_message(reference.clone(), &bits_client())
        .await;
    assert!(
        logo_line(&sent).is_none(),
        "nothing to render: logo left out"
    );
    let KitchenEvent::Print(program) = &sent.event else {
        unreachable!()
    };
    assert_eq!(program.lines.len(), 2, "text and cut stay");

    // An SVG client gets the reference and leaves it out itself.
    let sent = chan.logos().wire_message(reference, &svg_client()).await;
    assert!(matches!(
        logo_line(&sent),
        Some(ReceiptLine::RasterSvg { svg, .. }) if svg.is_empty()
    ));
}

#[tokio::test]
async fn migration_moves_inline_logos_and_keeps_everything_else() {
    let (chan, db) = channel().await;
    let unacked = insert_old_job(&db, None).await;
    let acked = insert_old_job(&db, Some(1700000100)).await;
    // Not a print job, and a row of another protocol major: untouched.
    let mut alert = vec![2u8, 2];
    alert.extend(
        postcard::to_allocvec(&KitchenEvent::Alert {
            title: "t".into(),
            text: String::new(),
        })
        .unwrap(),
    );
    let foreign = vec![1u8, 0, 0xde, 0xad];
    for p in [&alert, &foreign] {
        sqlx::query("INSERT INTO kitchen_outbox (payload, created_at) VALUES (?, 1700000000)")
            .bind(p)
            .execute(&db)
            .await
            .unwrap();
    }
    let before = payload(&db, unacked).await;

    // Dry run: everything checked, nothing written.
    let dry = chan.migrate_outbox_logos(true).await.unwrap();
    assert_eq!(dry.rows, 4);
    assert_eq!(dry.with_inline_logo, 2);
    assert_eq!(dry.migrated, 0);
    assert_eq!(dry.logos, 1);
    assert_eq!(dry.other_major, 1);
    assert_eq!(dry.failed, 0);
    assert_eq!(payload(&db, unacked).await, before);
    assert_eq!(logo_rows(&db).await, 0);

    // The real run.
    let done = chan.migrate_outbox_logos(false).await.unwrap();
    assert_eq!(done.migrated, 2);
    assert!(done.bytes_after < done.bytes_before);
    assert_eq!(logo_rows(&db).await, 1);

    for (id, acked_at) in [(unacked, None), (acked, Some(1700000100i64))] {
        let row =
            sqlx::query("SELECT payload, created_at, acked_at FROM kitchen_outbox WHERE id = ?")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap();
        let p: Vec<u8> = row.get("payload");
        assert_eq!(&p[..2], &[2, 2], "the stored version header stays");
        assert_eq!(row.get::<i64, _>("created_at"), 1700000000);
        assert_eq!(row.get::<Option<i64>, _>("acked_at"), acked_at);
        let stored = stored_job(&p);
        assert!(matches!(
            logo_line(&stored),
            Some(ReceiptLine::RasterSvg { svg, .. }) if svg.is_empty()
        ));
        // Still printable on both kinds of client.
        let sent = chan
            .logos()
            .wire_message(stored.clone(), &bits_client())
            .await;
        assert!(matches!(
            logo_line(&sent),
            Some(ReceiptLine::RasterBits { bits, .. }) if *bits == expected_bits()
        ));
        let sent = chan.logos().wire_message(stored, &svg_client()).await;
        assert!(matches!(
            logo_line(&sent),
            Some(ReceiptLine::RasterSvg { svg, .. }) if svg == LOGO.as_bytes()
        ));
    }
    assert_eq!(payload(&db, 3).await, alert);
    assert_eq!(payload(&db, 4).await, foreign);

    // A second run finds nothing to do.
    let again = chan.migrate_outbox_logos(false).await.unwrap();
    assert_eq!(again.with_inline_logo, 0);
    assert_eq!(again.migrated, 0);
}

/// The hash covers only the SVG: when the renderer or its fonts change, a
/// stored logo keeps its hash but must get the new pixels at the next start.
#[tokio::test]
async fn refresh_rerenders_logos_whose_pixels_changed() {
    let (chan, db) = channel().await;
    chan.logos().put(LOGO.as_bytes(), WIDTH).await.unwrap();
    // As if rendered by an older renderer.
    sqlx::query("UPDATE receipt_logos SET bits = zeroblob(length(bits))")
        .execute(&db)
        .await
        .unwrap();

    let store = rusterando_server::receipt_logos::LogoStore::new(db.clone());
    assert_eq!(store.refresh().await.unwrap(), (1, 1));
    let bits: Vec<u8> = sqlx::query_scalar("SELECT bits FROM receipt_logos")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(bits, expected_bits());
    assert_eq!(store.refresh().await.unwrap(), (1, 0), "nothing left to do");
}
