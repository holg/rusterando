//! Wire protocol between the rusterando server and a kitchen printer
//! client (a Raspberry Pi running rusterando-printer, attached to a
//! receipt printer via USB).
//!
//! Encoding is postcard over length-delimited frames. The schema is
//! forward-compatible as long as enum variants are only added at the
//! end.

use serde::{Deserialize, Serialize};

pub mod bitmap;
pub mod receipt;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct OrderId(pub u64);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct SeqId(pub u64);

impl std::fmt::Display for SeqId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Server → client envelope. Every event has a monotonically-increasing
/// `seq_id` assigned at outbox-insert time. Clients ack by seq_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMessage {
    pub seq_id: SeqId,
    pub event: KitchenEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KitchenEvent {
    /// A fully-composed receipt to print. The server is the Bon-Designer:
    /// it ran `receipt::compose`, resolved the theme, and inlined the
    /// logo rasters — the Pi just executes the lines. Covers both new
    /// orders and reprints (the program carries `is_reprint`).
    Print(crate::receipt::ReceiptProgram),

    // ===== In-band self-update (1.0+) =====
    // The server can ship the Pi a new binary over THIS channel — the
    // same trusted 9001 stream — so a protocol/binary bump no longer
    // strands the Pi (the failure that motivated this: a 0.1.0 Pi silently
    // not printing against a 1.0 server). Admin-gated on the server side.
    //
    // IMPORTANT for forward-compat: postcard is POSITIONAL, so only ever
    // ADD variants at the END of this enum. Never reorder/insert — an old
    // decoder maps a variant by its index. Appending is safe (old Pis just
    // never receive a higher index); inserting shifts everything and
    // corrupts decoding.
    /// Offer a newer artifact. The Pi compares `target_version` to its own,
    /// and if it wants it, pulls the bytes via `ClientMessage::FetchArtifact`,
    /// verifying against `sha256` + `size`.
    UpdateOffer {
        component: Component,
        target_version: String,
        sha256: [u8; 32],
        size: u64,
    },
    /// One chunk of the artifact, in response to a `FetchArtifact`. `offset`
    /// echoes the request; `last` marks the final chunk.
    ArtifactChunk {
        component: Component,
        offset: u64,
        bytes: Vec<u8>,
        last: bool,
    },

    // ===== Kitchen display board (2.1+) — APPEND ONLY =====
    /// The complete set of active orders for a kitchen display terminal.
    /// Only sent to a client that asked via [`ClientMessage::BoardSubscribe`]
    /// (old Pis never do, so they never see this variant). Sent whole on
    /// subscribe and again after every change — the board is a handful of
    /// orders, so a full snapshot is cheaper to reason about than deltas.
    /// Ephemeral: travels with `seq_id` [`BOARD_SEQ`], never touches the
    /// outbox, and must NOT be acked or run through the print idempotency
    /// cache.
    Board(BoardSnapshot),
    /// Shop open/closed state + today's hours for the terminal's
    /// "Öffnungszeiten" page — the quick controls of /admin/hours. Sent to a
    /// board subscriber on subscribe, on every change, and after each
    /// [`ClientMessage::Shop`] (carrying its result in `notice`). Ephemeral
    /// like `Board` ([`BOARD_SEQ`], never acked).
    Shop(ShopPanel),
    /// Alert the kitchen display now — the new-order flash + gong — with a
    /// message from the admin ("Alarm auslösen" on /admin/drucker). Only
    /// board subscribers get it; ephemeral like `Board`.
    Alert { title: String, text: String },
}

/// `seq_id` carried by every ephemeral (non-outbox) frame: update offers and
/// board snapshots. Real outbox rows start at 1.
pub const BOARD_SEQ: SeqId = SeqId(0);

/// A server-managed component the Pi can self-update. Only the printer
/// binary today; the enum leaves room (tunnel unit, init script, …)
/// without a schema break, since it's referenced by appended variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Component {
    /// The `rusterando-printer` executable itself.
    PrinterBinary,
}

/// Client → server envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Sent immediately after TCP connect. The server uses `last_seen_seq`
    /// to replay any unacked outbox entries newer than that, and records
    /// `version` + `arch` for the admin Pi panel / update targeting.
    Hello {
        shop_slug: String,
        version: String,
        last_seen_seq: Option<SeqId>,
        /// Target triple the Pi binary was built for, e.g.
        /// "aarch64-unknown-linux-gnu". Lets the server pick the right
        /// artifact. Added in 1.0.
        arch: String,
        /// Content hashes ([`crate::receipt::raster_hash`]) of the logo
        /// rasters the Pi already has cached on disk. The server sends a
        /// `RasterSvg` with EMPTY `svg` (a reference) for any hash listed
        /// here, and full SVG bytes otherwise. Added in 2.0; keeps the
        /// steady-state wire tiny (the header logo rarely changes).
        cached_rasters: Vec<[u8; 32]>,
    },
    /// Acknowledge a printed (or already-deduped) event. Server marks outbox
    /// row as acked.
    Ack(SeqId),
    /// Keepalive. Server uses these to mark per-shop last_seen_at.
    Heartbeat,

    // ===== In-band self-update (1.0+) — APPEND ONLY (see KitchenEvent) =====
    /// Request the next chunk of an offered artifact, starting at `offset`
    /// (0 for the first). The server replies with an `ArtifactChunk`.
    FetchArtifact { component: Component, offset: u64 },
    /// Report the outcome of an update attempt, so the server can clear the
    /// armed flag / surface success or failure in the admin panel. The Pi
    /// sends this BEFORE exiting to self-restart on success.
    UpdateResult {
        component: Component,
        from: String,
        to: String,
        ok: bool,
        /// Empty on success; a short reason on failure.
        error: String,
    },

    // ===== Kitchen display board (2.1+) — APPEND ONLY =====
    /// Opt this connection into [`KitchenEvent::Board`] snapshots. Sent once
    /// after Hello by a display terminal; a plain printer never sends it.
    BoardSubscribe,
    /// Move an order between board columns. `from` is the status the
    /// terminal was showing — the server applies the change only if the
    /// order is still in that status (so a stale tap after a web-board
    /// change is a no-op) and only along [`board_transition_allowed`].
    /// The fresh snapshot that follows is the reply.
    BoardAdvance {
        /// The TEXT `orders.id` (not the hashed [`OrderId`]).
        order_id: String,
        from: BoardStatus,
        to: BoardStatus,
    },
    /// A quick control from the terminal's "Öffnungszeiten" page — the same
    /// actions as the /admin/hours buttons. Answered by a [`ShopPanel`].
    Shop(ShopAction),

    // ===== WebSocket kitchen devices (2.2+) — APPEND ONLY =====
    /// Whether a printer is attached right now. Sent by devices that print
    /// only while a USB printer is plugged in (the ESP32-P4 displays), on
    /// connect and on every change. The server shows it in the admin panel
    /// and watches only printing devices for the printer-offline alarm.
    /// Old Pis never send it: they count as printing whenever connected.
    PrinterPresent(bool),
}

// ===== Domain types =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderForKitchen {
    pub order_id: OrderId,
    /// Numeric shorthand of the order, e.g. 1042. May reset daily.
    /// Kept for backwards compatibility and the reprint banner; for
    /// the big header line use `display_label` which carries the
    /// full server-assigned identifier ("DP-1105-0001").
    pub display_number: u32,
    /// Full human-readable order number as shown in /admin/orders,
    /// e.g. "DP-1105-0001". Printed in the receipt header so staff
    /// can grep the same string across admin UI, the receipt, and
    /// the order-detail URL. `#[serde(default)]` for forward-compat
    /// with older clients that only sent `display_number`.
    #[serde(default)]
    pub display_label: String,
    pub created_at_unix: i64,
    /// Server-formatted "Eingang" timestamp in the SHOP's timezone, ready to
    /// print verbatim. The printer must NOT reformat from the epoch — the Pi's
    /// clock/TZ may differ (it had no TZ set, so chrono::Local printed UTC).
    /// `#[serde(default)]` empty string → old servers; the printer then falls
    /// back to formatting `created_at_unix` itself.
    #[serde(default)]
    pub created_at_label: String,
    pub channel: OrderChannel,
    pub customer: Customer,
    pub items: Vec<LineItem>,
    pub subtotal_cents: u32,
    pub delivery_fee_cents: u32,
    pub total_cents: u32,
    pub payment: PaymentStatus,
    pub note: Option<String>,
    /// Brand line at the top of the receipt (e.g. "Mein Restaurant").
    /// Server-side pulled from BrandingHandle.display_name(). Empty
    /// string when missing so old clients keep printing.
    #[serde(default)]
    pub shop_name: String,
    /// Order-accepted timestamp. Distinct from created_at: this is
    /// when the order advanced to status='received' (kitchen ack /
    /// Stripe payment confirmation). `None` when still pending.
    #[serde(default)]
    pub accepted_at_unix: Option<i64>,
    /// Server-formatted "Angenommen" timestamp in the shop's timezone, printed
    /// verbatim (same rationale as `created_at_label`). `None` when the order
    /// hasn't been accepted yet.
    #[serde(default)]
    pub accepted_at_label: Option<String>,
    /// Full URL the QR code at the bottom encodes. Same target the
    /// customer's email links to: `<base>/orders/<id>`. None disables
    /// the QR block entirely.
    #[serde(default)]
    pub qr_url: Option<String>,
    /// True when this order was placed against Stripe sandbox keys.
    /// The Pi prints a `*** TEST ***` banner top + bottom so the
    /// kitchen never accidentally fulfills a Stripe-test order.
    /// `#[serde(default)]` for forward-compat with older clients
    /// (defaults to `false` = treat-as-live, safe for legacy data).
    #[serde(default)]
    pub is_test_mode: bool,
    /// Voucher snapshot for the kitchen receipt. Empty string + 0
    /// when no voucher was applied. `#[serde(default)]` for
    /// forward-compat — older Pi binaries simply ignore the line.
    #[serde(default)]
    pub voucher_code: String,
    #[serde(default)]
    pub voucher_discount_cents: u32,
    /// Customer's scheduled fulfillment time, server-formatted in the
    /// shop's timezone (e.g. "13:30", or "25.05. 15:00" for a far-out
    /// pre-order). `None` = ASAP (no scheduled time chosen). The printer
    /// renders it in brackets after the big order-received timestamp,
    /// labelled per channel ("Abholung" / "Lieferung"), so the kitchen
    /// sees both when it came in AND when it's due. `#[serde(default)]`
    /// → older Pi binaries simply omit it.
    #[serde(default)]
    pub pickup_time_label: Option<String>,
    /// Receipt layout theme, chosen by the admin
    /// (`app_settings.printer_theme`). Known values:
    ///
    ///   * "" / "rusterando-default" → classic text-only layout.
    ///   * "rusterando-rando" → adds the combined channel+brand logo band.
    ///
    /// The **server** (`receipt::compose`) resolves this into the wire
    /// program; unknown values fall back to the default text-only layout.
    /// `#[serde(default)]` keeps older payloads decodable.
    #[serde(default)]
    pub printer_theme: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrderChannel {
    Delivery { address: DeliveryAddress },
    Pickup,
    DineIn { table: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryAddress {
    pub street: String,
    pub postal: String,
    pub city: String,
    pub bell: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Customer {
    pub name: String,
    pub phone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineItem {
    pub qty: u32,
    pub name: String,
    /// "extra Käse", "ohne Zwiebel", etc.
    pub modifications: Vec<String>,
    pub unit_price_cents: u32,
    /// Menu category snapshot ("Pizzabrötchen", "Salatspezialitäten",
    /// etc.). The Pi groups consecutive items with the same category
    /// under one heading on the receipt. Empty string when missing.
    #[serde(default)]
    pub category: String,
    /// Per-modification prices (extras). Same length as
    /// `modifications` when populated; empty otherwise. The receipt
    /// prints each on its own line with a right-aligned price.
    #[serde(default)]
    pub modification_prices_cents: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PaymentStatus {
    /// Already paid (Stripe, etc.). Kitchen just makes it.
    Prepaid,
    /// Driver collects cash on delivery. Amount is the full receipt total.
    CollectOnDelivery { amount_cents: u32 },
}

// ===== Kitchen display board (2.1+) =====

/// One [`KitchenEvent::Board`] payload: every order currently on the board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardSnapshot {
    /// Server wall clock at send time. The terminal derives "waiting for N
    /// minutes" from `created_at_unix`; this lets it correct for its own
    /// clock being off instead of trusting NTP on the device.
    pub server_now_unix: i64,
    /// Oldest first (`created_at`), all three columns mixed.
    pub orders: Vec<BoardOrder>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardOrder {
    /// The TEXT `orders.id`, echoed back in [`ClientMessage::BoardAdvance`].
    pub id: String,
    pub status: BoardStatus,
    /// Same DTO the receipt is composed from (items, channel, customer,
    /// note, payment, shop-TZ time labels), so screen and Bon never disagree.
    pub order: OrderForKitchen,
}

/// Board-relevant order statuses. The first three are the columns; the last
/// two are exits the terminal may request (the order then leaves the board).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardStatus {
    /// `received` — column 1, "Neu".
    Received,
    /// `preparing` — column 2, "In Arbeit".
    Preparing,
    /// `ready_for_pickup` — column 3, ready for the customer or the driver.
    Ready,
    /// `picked_up` — pickup order handed over at the counter.
    PickedUp,
    /// `out_for_delivery` — driver left with it (no tour; the driver board
    /// shows it as a loose stop and marks it delivered).
    OutForDelivery,
    /// `cancelled` — Storno from the terminal, same as the admin board's
    /// "Stornieren" (status only; an online payment is NOT refunded).
    Cancelled,
}

impl BoardStatus {
    /// The `orders.status` value.
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Preparing => "preparing",
            Self::Ready => "ready_for_pickup",
            Self::PickedUp => "picked_up",
            Self::OutForDelivery => "out_for_delivery",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_db(s: &str) -> Option<Self> {
        Some(match s {
            "received" => Self::Received,
            "preparing" => Self::Preparing,
            "ready_for_pickup" => Self::Ready,
            "picked_up" => Self::PickedUp,
            "out_for_delivery" => Self::OutForDelivery,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// The moves a kitchen terminal may make: forward one column, back one
/// column (mis-tap undo), out of the ready column — handed over for pickup
/// orders, driver gone for deliveries — and Storno from any column. Refunds
/// and jumps stay admin territory. The server enforces this; the terminal
/// uses it to decide which buttons to show.
pub fn board_transition_allowed(from: BoardStatus, to: BoardStatus, is_delivery: bool) -> bool {
    use BoardStatus::*;
    matches!(
        (from, to, is_delivery),
        (Received, Preparing, _)
            | (Preparing, Ready, _)
            | (Preparing, Received, _)
            | (Ready, Preparing, _)
            | (Ready, PickedUp, false)
            | (Ready, OutForDelivery, true)
            | (Received | Preparing | Ready, Cancelled, _)
    )
}

/// The customer-facing traffic light (mirrors `rusterando_shared::ShopLevel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShopLight {
    /// Taking orders now.
    Open,
    /// Closed now, opens later today (pre-orders possible).
    OpensLater,
    Closed,
}

/// The status line + quick-control state of /admin/hours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShopPanel {
    pub light: ShopLight,
    /// Customer-facing reason, e.g. "Heute ab 17:00 Uhr geöffnet"; empty
    /// when plainly open.
    pub reason: String,
    /// Can a customer place an order right now (the real `place_order`
    /// gate)? Can be false while `light` is Open — near closing time —
    /// which is when "Notbetrieb" is offered.
    pub gate_open: bool,
    pub gate_reason: String,
    /// Indefinite manual pause ("Jetzt schließen").
    pub orders_paused: bool,
    /// Timed pause active until this local "HH:MM"; empty when none.
    pub snooze_until: String,
    /// Force-open ("Jetzt öffnen" / Notbetrieb) active until this local
    /// "HH:MM"; empty when none.
    pub force_open_until: String,
    /// "Keine Lieferung (nur Abholung)".
    pub delivery_paused: bool,
    /// Today's effective window, "HH:MM"; empty when closed all day.
    pub today_open: String,
    pub today_close: String,
    /// Today's hours were adjusted ("Heute angepasst").
    pub today_extended: bool,
    /// Result of the terminal's last [`ShopAction`]: empty on success, else
    /// the server's message (e.g. why an hours change was refused).
    pub notice: String,
}

/// The /admin/hours quick controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShopAction {
    /// `true`: "Jetzt öffnen" / Notbetrieb (force-open until end of day);
    /// `false`: "Jetzt schließen" (indefinite pause).
    SetOpen(bool),
    /// "Auf Plan zurücksetzen": drop every manual override.
    ResetToSchedule,
    /// "Bei Überlastung kurz pausieren" for this many minutes; 0 cancels.
    Snooze { minutes: u16 },
    /// "Keine Lieferung (nur Abholung)" on/off.
    SetDeliveryPaused(bool),
    /// "Heute früher / länger offen": new open (earlier only) and/or close
    /// (later only) as "HH:MM".
    SetTodayHours {
        open: Option<String>,
        close: Option<String>,
    },
}

// ===== Small helpers for the wire side, optional =====

/// Maximum reasonable frame size — 1 MiB is generous for any plausible order.
pub const FRAME_MAX_BYTES: usize = 1024 * 1024;

// ===== Schema versioning =====
//
// Every persisted payload (outbox row, wire frame) carries a 2-byte
// header before the postcard blob: `[major, minor]`. Parsed from this
// crate's Cargo.toml version at compile time, so both server and Pi
// see the same numbers when built against the same source revision.
//
//   - **Major mismatch** = incompatible schema. Decoder skips+logs
//     and leaves outbox rows unacked for forensics. Bump the major
//     in Cargo.toml whenever you add/remove/reorder fields in any
//     of the protocol types above (postcard is positional — even
//     renaming a variant by reordering breaks bytes).
//   - **Minor mismatch** = log only; decoder still tries. Cosmetic
//     bumps (doc edits, helper additions) can ship without breaking
//     in-flight outbox rows. Bump minor in Cargo.toml for those.
//
// CAVEAT while we're in `0.x.y`: by semver, every 0.x bump is
// potentially breaking, so you'd want to bump the version to
// `0.(x+1).0` AND treat it as a major-mismatch scenario. The
// envelope's "major byte" is the first dotted component (0 right
// now), so changing `0.1.0` → `0.2.0` actually triggers the
// **minor-mismatch** path here, which logs but still tries to
// decode (and postcard will then error on the real schema break).
// Once we cut a `1.0.0` release this aligns with normal semver and
// the two paths behave as advertised. Until then, treat the next
// breaking schema change as a `1.0.0` cut.
//
// Layout of a versioned payload:
//
//     +-------+-------+--------------------------+
//     | major | minor | postcard bytes (payload) |
//     +-------+-------+--------------------------+

/// Major + minor parsed from the crate's `CARGO_PKG_VERSION` at
/// compile time. Both fall back to 0 if parsing fails for any reason,
/// so we never crash the build for a malformed version string.
pub const SCHEMA_VERSION_MAJOR: u8 = parse_version_component(env!("CARGO_PKG_VERSION"), 0);
pub const SCHEMA_VERSION_MINOR: u8 = parse_version_component(env!("CARGO_PKG_VERSION"), 1);

/// Pull the Nth dotted component out of "X.Y.Z" at const-eval time.
/// Saturates at 255 (u8 max) — we'd be in trouble if the protocol
/// hits major 256, but that's not 2026's problem.
const fn parse_version_component(ver: &str, idx: usize) -> u8 {
    let bytes = ver.as_bytes();
    let mut i = 0;
    let mut seen_dots = 0;
    let mut value: u16 = 0;
    let mut have_digit = false;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'.' {
            if seen_dots == idx {
                break;
            }
            seen_dots += 1;
            value = 0;
            have_digit = false;
        } else if seen_dots == idx {
            if b >= b'0' && b <= b'9' {
                value = value.saturating_mul(10).saturating_add((b - b'0') as u16);
                have_digit = true;
            } else {
                // Hit a pre-release suffix like "1.2.3-rc1" — stop.
                break;
            }
        }
        i += 1;
    }
    if !have_digit {
        return 0;
    }
    if value > 255 {
        255
    } else {
        value as u8
    }
}

/// Errors from [`decode_versioned`].
#[derive(Debug)]
pub enum SchemaDecodeError {
    /// Payload was shorter than the 2-byte version header.
    TooShort,
    /// Major version doesn't match this crate's compiled-in value.
    /// Caller should log + skip the payload.
    MajorMismatch {
        got_major: u8,
        got_minor: u8,
        expected_major: u8,
    },
}

impl core::fmt::Display for SchemaDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "payload too short for schema header"),
            Self::MajorMismatch {
                got_major,
                got_minor,
                expected_major,
            } => write!(
                f,
                "schema major mismatch: got {}.{}, expected {}.x",
                got_major, got_minor, expected_major
            ),
        }
    }
}

impl std::error::Error for SchemaDecodeError {}

/// Prepend the 2-byte version header to a postcard-encoded payload.
/// Both call sites (server outbox insert, server→Pi frame) wrap
/// before writing/sending.
pub fn encode_versioned(postcard_payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(postcard_payload.len() + 2);
    buf.push(SCHEMA_VERSION_MAJOR);
    buf.push(SCHEMA_VERSION_MINOR);
    buf.extend_from_slice(postcard_payload);
    buf
}

/// Strip the 2-byte version header. Returns the postcard slice ready
/// for `postcard::from_bytes`. On `MajorMismatch` the caller should
/// log + skip the payload (server: leave the outbox row unacked;
/// Pi: ack the seq + move on so the server stops replaying it).
///
/// Minor mismatch is intentionally NOT an error here — caller can
/// branch on the returned `(minor, slice)` if it cares, otherwise
/// just proceed. The slice may still fail to decode via postcard if
/// the minor bump turns out to be a breaking change, which surfaces
/// as a plain postcard error.
pub fn decode_versioned(bytes: &[u8]) -> Result<(u8, &[u8]), SchemaDecodeError> {
    if bytes.len() < 2 {
        return Err(SchemaDecodeError::TooShort);
    }
    let got_major = bytes[0];
    let got_minor = bytes[1];
    if got_major != SCHEMA_VERSION_MAJOR {
        return Err(SchemaDecodeError::MajorMismatch {
            got_major,
            got_minor,
            expected_major: SCHEMA_VERSION_MAJOR,
        });
    }
    Ok((got_minor, &bytes[2..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> OrderForKitchen {
        OrderForKitchen {
            order_id: OrderId(42),
            display_number: 1042,
            display_label: "MR-1105-0001".into(),
            created_at_unix: 1_700_000_000,
            created_at_label: "14.11. 23:13".into(),
            channel: OrderChannel::Delivery {
                address: DeliveryAddress {
                    street: "Musterstraße 12".into(),
                    postal: "12345".into(),
                    city: "Musterstadt".into(),
                    bell: Some("Mustermann".into()),
                },
            },
            customer: Customer {
                name: "Mustermann".into(),
                phone: Some("0123-456789".into()),
            },
            items: vec![LineItem {
                qty: 2,
                name: "Pizza Funghi".into(),
                modifications: vec!["extra Käse".into()],
                unit_price_cents: 1150,
                category: "Pizzen".into(),
                modification_prices_cents: vec![100],
            }],
            subtotal_cents: 2300,
            delivery_fee_cents: 150,
            total_cents: 2450,
            payment: PaymentStatus::CollectOnDelivery { amount_cents: 2450 },
            note: None,
            shop_name: "Mein Restaurant".into(),
            accepted_at_unix: None,
            accepted_at_label: None,
            qr_url: Some("https://example.com/orders/abc".into()),
            is_test_mode: false,
            voucher_code: String::new(),
            voucher_discount_cents: 0,
            pickup_time_label: Some("13:30".into()),
            printer_theme: "rusterando-rando".into(),
        }
    }

    #[test]
    fn server_message_round_trip() {
        // postcard isn't a dep of this crate, so we just round-trip through
        // serde_json to prove the schema is serde-clean. The wire event is
        // now a composed ReceiptProgram, not a raw order.
        use crate::receipt::{ReceiptLine, ReceiptProgram};
        let _ = sample(); // keep the fixture exercised
        let prog = ReceiptProgram {
            display_number: 7,
            is_reprint: false,
            lines: vec![
                ReceiptLine::Text {
                    text: "DP-1105-0001".into(),
                    style: crate::receipt::LineStyle::NORMAL,
                },
                ReceiptLine::Cut { feed: 4 },
            ],
        };
        let msg = ServerMessage {
            seq_id: SeqId(7),
            event: KitchenEvent::Print(prog),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.seq_id, SeqId(7));
    }

    #[test]
    fn update_messages_round_trip() {
        // The self-update wire types must serde cleanly (server↔Pi).
        let offer = ServerMessage {
            seq_id: SeqId(1),
            event: KitchenEvent::UpdateOffer {
                component: Component::PrinterBinary,
                target_version: "0.3.0".into(),
                sha256: [7u8; 32],
                size: 2_125_904,
            },
        };
        let chunk = ServerMessage {
            seq_id: SeqId(2),
            event: KitchenEvent::ArtifactChunk {
                component: Component::PrinterBinary,
                offset: 65536,
                bytes: vec![1, 2, 3, 4],
                last: false,
            },
        };
        for m in [offer, chunk] {
            let j = serde_json::to_string(&m).unwrap();
            let back: ServerMessage = serde_json::from_str(&j).unwrap();
            assert_eq!(format!("{back:?}"), format!("{m:?}"));
        }

        let hello = ClientMessage::Hello {
            shop_slug: "davidspizzeria".into(),
            version: "0.2.0".into(),
            last_seen_seq: Some(SeqId(9)),
            arch: "aarch64-unknown-linux-gnu".into(),
            cached_rasters: vec![[3u8; 32], [9u8; 32]],
        };
        let fetch = ClientMessage::FetchArtifact {
            component: Component::PrinterBinary,
            offset: 0,
        };
        let result = ClientMessage::UpdateResult {
            component: Component::PrinterBinary,
            from: "0.2.0".into(),
            to: "0.3.0".into(),
            ok: true,
            error: String::new(),
        };
        for m in [hello, fetch, result] {
            let j = serde_json::to_string(&m).unwrap();
            let back: ClientMessage = serde_json::from_str(&j).unwrap();
            assert_eq!(format!("{back:?}"), format!("{m:?}"));
        }
    }

    #[test]
    fn version_constants_match_crate() {
        // Sanity-check the const parser against the runtime version
        // string. If someone bumps Cargo.toml the constants follow.
        let ver = env!("CARGO_PKG_VERSION");
        let parts: Vec<&str> = ver.split('.').collect();
        let want_major: u8 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let want_minor: u8 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        assert_eq!(SCHEMA_VERSION_MAJOR, want_major);
        assert_eq!(SCHEMA_VERSION_MINOR, want_minor);
    }

    #[test]
    fn versioned_envelope_round_trip() {
        let payload = b"hello, world";
        let wrapped = encode_versioned(payload);
        assert_eq!(&wrapped[..2], &[SCHEMA_VERSION_MAJOR, SCHEMA_VERSION_MINOR]);

        let (minor, inner) = decode_versioned(&wrapped).expect("ok");
        assert_eq!(minor, SCHEMA_VERSION_MINOR);
        assert_eq!(inner, payload);
    }

    #[test]
    fn versioned_envelope_rejects_short() {
        assert!(matches!(
            decode_versioned(&[]),
            Err(SchemaDecodeError::TooShort)
        ));
        assert!(matches!(
            decode_versioned(&[0u8]),
            Err(SchemaDecodeError::TooShort)
        ));
    }

    #[test]
    fn versioned_envelope_rejects_wrong_major() {
        let mut bad = encode_versioned(b"payload");
        // Flip the major to something we definitely don't compile as.
        bad[0] = SCHEMA_VERSION_MAJOR.wrapping_add(1);
        let err = decode_versioned(&bad).unwrap_err();
        match err {
            SchemaDecodeError::MajorMismatch {
                got_major,
                expected_major,
                ..
            } => {
                assert_ne!(got_major, expected_major);
            }
            _ => panic!("expected MajorMismatch, got {err:?}"),
        }
    }
    #[test]
    fn board_status_db_round_trip() {
        use BoardStatus::*;
        for st in [
            Received,
            Preparing,
            Ready,
            PickedUp,
            OutForDelivery,
            Cancelled,
        ] {
            assert_eq!(BoardStatus::from_db(st.as_db()), Some(st));
        }
        assert_eq!(BoardStatus::from_db("pending_payment"), None);
        assert_eq!(BoardStatus::from_db("delivered"), None);
    }

    #[test]
    fn board_transitions() {
        use BoardStatus::*;
        // forward / back one column, either channel
        for d in [true, false] {
            assert!(board_transition_allowed(Received, Preparing, d));
            assert!(board_transition_allowed(Preparing, Ready, d));
            assert!(board_transition_allowed(Preparing, Received, d));
            assert!(board_transition_allowed(Ready, Preparing, d));
            // no jumps
            assert!(!board_transition_allowed(Received, Ready, d));
            assert!(!board_transition_allowed(Received, PickedUp, d));
        }
        // the exit depends on the channel
        assert!(board_transition_allowed(Ready, PickedUp, false));
        assert!(!board_transition_allowed(Ready, PickedUp, true));
        assert!(board_transition_allowed(Ready, OutForDelivery, true));
        assert!(!board_transition_allowed(Ready, OutForDelivery, false));
        // Storno from every column, but not out of a finished state
        for d in [true, false] {
            for from in [Received, Preparing, Ready] {
                assert!(board_transition_allowed(from, Cancelled, d));
            }
            assert!(!board_transition_allowed(PickedUp, Cancelled, d));
            assert!(!board_transition_allowed(Cancelled, Received, d));
        }
    }
}
