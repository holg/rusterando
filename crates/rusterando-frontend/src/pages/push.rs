//! Push notification token registration (called by the iOS Tauri app).
//!
//! The app sends its APNs device token + the role it wants to receive
//! pushes for. We upsert by token so re-registration on every launch is
//! cheap (no duplicate rows). The server later uses this table when it
//! fans out kitchen/driver/admin pushes via APNs.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use std::sync::Arc;

/// Trait the server crate implements so the frontend can trigger a
/// broadcast without importing davidspizzeria-server (which would
/// create a dependency cycle). `ApnsHandle` in the server crate is
/// the only real implementor.
/// Who an admin-initiated broadcast goes to. Used by `/admin/broadcast`
/// to expose two buttons: one for staff-only operational pings, one for
/// every registered device (staff + customer apps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BroadcastAudience {
    /// admin + kitchen + driver tokens only.
    Staff,
    /// Every active token regardless of role.
    All,
}

impl BroadcastAudience {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Staff => "staff",
            Self::All => "all",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "staff" => Some(Self::Staff),
            "all" => Some(Self::All),
            _ => None,
        }
    }
    pub fn label_de(&self) -> &'static str {
        match self {
            Self::Staff => "Mitarbeiter",
            Self::All => "Alle",
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait::async_trait]
pub trait BroadcastSink: Send + Sync {
    async fn broadcast(
        &self,
        audience: BroadcastAudience,
        title: &str,
        body: &str,
    ) -> anyhow::Result<usize>;

    /// Fire-and-forget per-role push fan-out. Used by `place_order` to
    /// ping the right staff (kitchen + admin always, driver on delivery)
    /// when a new cash/Abholung order lands. Sync from the caller's POV;
    /// the server-side impl spawns the actual HTTP/2 calls.
    fn notify_roles(&self, roles: &[&str], title: &str, body: &str, deep_link: Option<&str>);
}

/// Convenience alias for the context-injected handle. `None` when the
/// server is running without APNs configured (no `.p8` on disk) — the
/// /admin/broadcast page falls back to "Push nicht konfiguriert".
#[cfg(feature = "ssr")]
pub type BroadcastSinkHandle = Option<Arc<dyn BroadcastSink>>;

/// Trait-erased handle into the kitchen-printer outbox. The
/// rusterando-server crate provides the implementation
/// (`kitchen::KitchenChannel`) and the SQL that turns an order id into
/// a `kitchen_protocol::OrderForKitchen` payload. We expose only
/// `order_id` / `&SqlitePool` so the frontend crate doesn't have to
/// depend on either rusterando-server or kitchen-protocol.
///
/// `None` for the handle on printerless deploys (KITCHEN_LISTEN_ADDR
/// unset in .env). Callers guard with
/// `if let Some(k) = use_context::<KitchenSinkHandle>().flatten()`.
#[cfg(feature = "ssr")]
#[async_trait::async_trait]
pub trait KitchenSink: Send + Sync {
    /// Persist + broadcast a NewOrder kitchen event for `order_id`.
    async fn broadcast_new_order(
        &self,
        db: &sqlx::SqlitePool,
        order_id: &str,
    ) -> anyhow::Result<()>;

    /// Persist + broadcast a Reprint kitchen event for `order_id`. Same
    /// payload as NewOrder; the Pi marks the receipt with a "REPRINT"
    /// banner. Triggered from the admin orders page.
    async fn broadcast_reprint(&self, db: &sqlx::SqlitePool, order_id: &str) -> anyhow::Result<()>;

    /// Compose + broadcast a SAMPLE receipt for the Bon-Editor's "print
    /// this" button — a representative fake order rendered with the current
    /// saved `ReceiptConfig` + theme, so the admin sees the real layout on
    /// real paper. `delivery`/`prepaid`/`theme` come from the editor's
    /// preview toggles. No DB order is touched.
    async fn broadcast_test_print(
        &self,
        db: &sqlx::SqlitePool,
        delivery: bool,
        prepaid: bool,
        theme: String,
    ) -> anyhow::Result<()>;
}

#[cfg(feature = "ssr")]
pub type KitchenSinkHandle = Option<Arc<dyn KitchenSink>>;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisteredDevice {
    pub token: String,
    pub role: String,
    pub platform: String,
    pub label: Option<String>,
}

/// Called by the iOS app once it has obtained an APNs device token.
///
/// Auth model: must be invoked while a valid `dp_session` cookie is set,
/// and the role argument must match the session's role (or be `admin`,
/// since admin can register for anything). This means the app needs the
/// user to log in BEFORE calling register_push_token; the registration
/// is what binds the device to a specific staff role.
#[server(
    name = RegisterPushToken,
    prefix = "/api",
    endpoint = "register_push_token"
)]
pub async fn register_push_token(
    /// Hex-encoded APNs device token (~64 chars from iOS).
    token: String,
    /// "admin" | "kitchen" | "driver"
    role: String,
    /// "ios" | "android" — only "ios" today.
    platform: String,
    /// Free-text device label for admin diagnostics ("Davids iPad",
    /// "Kitchen Tablet 1"). Optional.
    label: Option<String>,
) -> Result<(), ServerFnError> {
    use crate::pages::session::{ssr::current_role, Role};
    use sqlx::SqlitePool;

    // Basic input sanity. APNs tokens are hex strings; reject obvious
    // garbage rather than letting it land in the DB.
    let token = token.trim().to_string();
    if token.len() < 32 || token.len() > 256 || !token.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ServerFnError::new("Ungültiges Push-Token."));
    }
    if !matches!(role.as_str(), "admin" | "kitchen" | "driver") {
        return Err(ServerFnError::new("Ungültige Rolle."));
    }
    if !matches!(platform.as_str(), "ios" | "android") {
        return Err(ServerFnError::new("Plattform nicht unterstützt."));
    }

    // Authn: require a valid session. Admin may register any role;
    // kitchen/driver may only register their own role (no escalation
    // path via push registration).
    let session_role = current_role()
        .await
        .ok_or_else(|| ServerFnError::new("Bitte zuerst anmelden."))?;
    let target = Role::parse(&role).ok_or_else(|| ServerFnError::new("Ungültige Rolle."))?;
    if session_role != Role::Admin && session_role != target {
        return Err(ServerFnError::new(
            "Push-Registrierung nur für die eigene Rolle erlaubt.",
        ));
    }

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Upsert by token. Re-registration on every app launch is the
    // common path; we just bump last_seen_at and refresh the label/role.
    sqlx::query(
        "INSERT INTO push_devices (token, role, platform, label)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(token) DO UPDATE SET
             role = excluded.role,
             platform = excluded.platform,
             label = COALESCE(excluded.label, push_devices.label),
             last_seen_at = CURRENT_TIMESTAMP,
             invalidated_at = NULL",
    )
    .bind(&token)
    .bind(&role)
    .bind(&platform)
    .bind(label.as_deref())
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("upsert push device: {e}")))?;

    Ok(())
}

/// One row per past broadcast for the audit list at /admin/broadcast.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BroadcastRow {
    pub id: String,
    pub title: String,
    pub body: String,
    pub recipient_count: i64,
    pub sent_by_role: String,
    pub sent_at: String,
    /// "staff" or "all" — captured at send time. Backfilled to "staff"
    /// for legacy rows by the migration default.
    pub audience: String,
}

/// Result of a successful send — handed back to the admin form so the
/// UI can show "Push an N Geräte gesendet."
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BroadcastResult {
    pub broadcast_id: String,
    pub recipient_count: i64,
}

/// Admin-only: send a manual push.
///
/// `audience` selects the recipient set:
///   * `"staff"` — admin + kitchen + driver only (operational pings).
///   * `"all"`   — every active token (staff + customer apps once we ship one).
///
/// Defaults to `"staff"` if the value is missing or unrecognised — that's the
/// safer side of the choice (matches pre-split behaviour).
#[server(
    name = SendBroadcast,
    prefix = "/api",
    endpoint = "send_broadcast"
)]
pub async fn send_broadcast(
    title: String,
    body: String,
    audience: String,
) -> Result<BroadcastResult, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let title = title.trim().to_string();
    let body = body.trim().to_string();
    if title.is_empty() {
        return Err(ServerFnError::new("Bitte einen Titel angeben."));
    }
    if body.is_empty() {
        return Err(ServerFnError::new("Bitte einen Nachrichtentext angeben."));
    }
    if title.chars().count() > 80 {
        return Err(ServerFnError::new("Titel ist zu lang (max. 80 Zeichen)."));
    }
    if body.chars().count() > 240 {
        return Err(ServerFnError::new("Text ist zu lang (max. 240 Zeichen)."));
    }

    let audience = BroadcastAudience::parse(audience.trim()).unwrap_or(BroadcastAudience::Staff);

    let sink: BroadcastSinkHandle =
        use_context().ok_or_else(|| ServerFnError::new("Push-Sender fehlt im Kontext."))?;
    let Some(sink) = sink else {
        return Err(ServerFnError::new(
            "Push-Notifications sind auf diesem Server nicht konfiguriert (.p8 fehlt).",
        ));
    };

    let recipient_count = sink
        .broadcast(audience, &title, &body)
        .await
        .map_err(|e| ServerFnError::new(format!("Senden fehlgeschlagen: {e}")))?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO push_broadcasts (id, title, body, sent_by_role, recipient_count, audience)
         VALUES (?1, ?2, ?3, 'admin', ?4, ?5)",
    )
    .bind(&id)
    .bind(&title)
    .bind(&body)
    .bind(recipient_count as i64)
    .bind(audience.as_db_str())
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("audit insert: {e}")))?;

    Ok(BroadcastResult {
        broadcast_id: id,
        recipient_count: recipient_count as i64,
    })
}

/// Read the last 20 broadcasts so the admin can see what they've
/// already sent. Admin-only.
#[server(
    name = ListBroadcasts,
    prefix = "/api",
    endpoint = "list_broadcasts"
)]
pub async fn list_broadcasts() -> Result<Vec<BroadcastRow>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows: Vec<(String, String, String, i64, String, String, String)> = sqlx::query_as(
        "SELECT id, title, body, recipient_count, sent_by_role,
                strftime('%Y-%m-%d %H:%M', sent_at), audience
         FROM push_broadcasts
         ORDER BY sent_at DESC
         LIMIT 20",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load broadcasts: {e}")))?;

    Ok(rows
        .into_iter()
        .map(
            |(id, title, body, recipient_count, sent_by_role, sent_at, audience)| BroadcastRow {
                id,
                title,
                body,
                recipient_count,
                sent_by_role,
                sent_at,
                audience,
            },
        )
        .collect())
}

/// Called when the app is being signed out — removes the token so the
/// device stops getting pushes for that role. Idempotent.
#[server(
    name = UnregisterPushToken,
    prefix = "/api",
    endpoint = "unregister_push_token"
)]
pub async fn unregister_push_token(token: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    let token = token.trim();
    if token.is_empty() {
        return Ok(());
    }
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    sqlx::query("DELETE FROM push_devices WHERE token = ?1")
        .bind(token)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("delete push device: {e}")))?;
    Ok(())
}
