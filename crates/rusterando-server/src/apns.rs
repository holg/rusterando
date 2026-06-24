//! APNs sender + per-role fan-out.
//!
//! On startup the server reads four env vars:
//!
//! ```ignore
//! APNS_TEAM_ID    = <your 10-char Apple team id>
//! APNS_KEY_ID     = <your 10-char APNs key id>
//! APNS_BUNDLE_ID  = com.example.shop
//! APNS_KEY_PATH   = /etc/your-shop/apns.p8
//! APNS_PRODUCTION = false   # true in prod, false (sandbox) for Xcode-built dev installs
//! ```
//!
//! The .p8 file is the auth key downloaded from
//! developer.apple.com → Keys (see docs/apple_setup.md). One key works
//! for both APNs sandbox + production environments; we just route to
//! the right HTTP/2 endpoint via `APNS_PRODUCTION`.
//!
//! All sends are best-effort: errors are logged but never propagated up
//! into the originating action (a failed APNs push must NEVER cause the
//! kitchen to lose an order).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use apns_h2::{
    Client, ClientConfig, DefaultNotificationBuilder, Endpoint, NotificationBuilder,
    NotificationOptions, Priority,
};
use async_trait::async_trait;
use rusterando_frontend::pages::push::{BroadcastAudience, BroadcastSink};
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct ApnsHandle(Arc<Inner>);

struct Inner {
    client: Client,
    bundle_id: String,
    db: SqlitePool,
}

impl ApnsHandle {
    /// Build the sender from env. Returns `Ok(None)` if APNs config is
    /// incomplete — the server still boots, just without push.
    /// (Useful for local dev where the .p8 isn't on disk.)
    pub fn from_env(db: SqlitePool) -> Result<Option<Self>> {
        let key_path = match std::env::var("APNS_KEY_PATH") {
            Ok(p) => PathBuf::from(p),
            Err(_) => {
                tracing::info!("APNS_KEY_PATH not set; push notifications disabled");
                return Ok(None);
            }
        };
        let team_id = std::env::var("APNS_TEAM_ID")
            .context("APNS_KEY_PATH is set but APNS_TEAM_ID is missing")?;
        let key_id = std::env::var("APNS_KEY_ID")
            .context("APNS_KEY_PATH is set but APNS_KEY_ID is missing")?;
        let bundle_id = std::env::var("APNS_BUNDLE_ID")
            .context("APNS_KEY_PATH is set but APNS_BUNDLE_ID is missing")?;
        let production = std::env::var("APNS_PRODUCTION")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let mut file = std::fs::File::open(&key_path)
            .with_context(|| format!("open APNs key at {}", key_path.display()))?;
        let endpoint_label = if production { "production" } else { "sandbox" };
        let config = ClientConfig {
            endpoint: if production {
                Endpoint::Production
            } else {
                Endpoint::Sandbox
            },
            ..Default::default()
        };
        let client = Client::token(&mut file, &key_id, &team_id, config)
            .map_err(|e| anyhow!("apns client init: {e:?}"))?;
        tracing::info!(
            target: "apns",
            %team_id,
            %key_id,
            %bundle_id,
            endpoint = endpoint_label,
            "APNs sender ready"
        );
        Ok(Some(ApnsHandle(Arc::new(Inner {
            client,
            bundle_id,
            db,
        }))))
    }

    /// Fan out a push to every active device registered for `role`.
    /// Spawns the actual HTTP/2 calls so the caller doesn't block — the
    /// originating order/cart action returns immediately.
    pub fn notify(&self, role: &str, title: &str, body: &str, deep_link: Option<&str>) {
        let role = role.to_string();
        let title = title.to_string();
        let body = body.to_string();
        let deep_link = deep_link.map(str::to_string);
        let inner = self.0.clone();

        tokio::spawn(async move {
            if let Err(e) = inner
                .fan_out(&role, &title, &body, deep_link.as_deref())
                .await
            {
                tracing::warn!(target: "apns", error = %e, %role, "fan-out failed");
            }
        });
    }

    /// Same as `notify` but fans out to several roles in parallel. Used
    /// for events that multiple staff roles need to act on at once
    /// (e.g. a new order pings kitchen + admin + driver simultaneously).
    pub fn notify_roles(&self, roles: &[&str], title: &str, body: &str, deep_link: Option<&str>) {
        for role in roles {
            self.notify(role, title, body, deep_link);
        }
    }

    /// Send a manual broadcast to the configured audience. Awaitable
    /// (unlike `notify`) so the caller can show "Sent to N devices"
    /// feedback. Returns the count of devices the message actually
    /// went to. iOS-only here; the FCM sender does the parallel job
    /// for android tokens.
    pub async fn broadcast(
        &self,
        audience: BroadcastAudience,
        title: &str,
        body: &str,
    ) -> Result<usize> {
        self.0.broadcast_all(audience, title, body).await
    }
}

impl Inner {
    /// Audience-aware broadcast. `Staff` filters to admin/kitchen/driver
    /// roles; `All` includes any active iOS token regardless of role
    /// (so a future customer app receives marketing/announcement pushes
    /// from /admin/broadcast). iOS-only here — the FCM sender does the
    /// parallel job against android tokens.
    async fn broadcast_all(
        &self,
        audience: BroadcastAudience,
        title: &str,
        body: &str,
    ) -> Result<usize> {
        let sql = match audience {
            BroadcastAudience::Staff => {
                "SELECT token FROM push_devices
                 WHERE role IN ('admin', 'kitchen', 'driver')
                       AND platform = 'ios'
                       AND invalidated_at IS NULL
                       AND last_seen_at > datetime('now', '-30 days')"
            }
            BroadcastAudience::All => {
                "SELECT token FROM push_devices
                 WHERE platform = 'ios'
                       AND invalidated_at IS NULL
                       AND last_seen_at > datetime('now', '-30 days')"
            }
        };
        let tokens: Vec<(String,)> = sqlx::query_as(sql).fetch_all(&self.db).await?;

        let total = tokens.len();
        if total == 0 {
            tracing::info!(target: "apns", "broadcast: no devices registered, nothing to send");
            return Ok(0);
        }

        for (token,) in tokens {
            let builder = DefaultNotificationBuilder::new()
                .title(title)
                .body(body)
                .sound("default");
            let opts = NotificationOptions {
                apns_priority: Some(Priority::High),
                apns_topic: Some(self.bundle_id.as_str()),
                ..Default::default()
            };
            let payload = builder.build(&token, opts);
            match self.client.send(payload).await {
                Ok(resp) => {
                    tracing::debug!(target: "apns", token = %short_token(&token), status = ?resp.code, "broadcast sent");
                    if matches!(resp.code, 410 | 400) {
                        let _ = sqlx::query(
                            "UPDATE push_devices
                             SET invalidated_at = CURRENT_TIMESTAMP
                             WHERE token = ?1",
                        )
                        .bind(&token)
                        .execute(&self.db)
                        .await;
                    }
                }
                Err(e) => {
                    tracing::warn!(target: "apns", token = %short_token(&token), error = ?e, "broadcast send failed");
                }
            }
        }
        Ok(total)
    }

    async fn fan_out(
        &self,
        role: &str,
        title: &str,
        body: &str,
        deep_link: Option<&str>,
    ) -> Result<()> {
        let tokens: Vec<(String,)> = sqlx::query_as(
            "SELECT token FROM push_devices
             WHERE role = ?1 AND platform = 'ios'
                   AND invalidated_at IS NULL
                   AND last_seen_at > datetime('now', '-30 days')",
        )
        .bind(role)
        .fetch_all(&self.db)
        .await?;

        if tokens.is_empty() {
            tracing::info!(target: "apns", %role, "fan-out: no active devices for role, nothing to send");
            return Ok(());
        }

        // Info-level so production (RUST_LOG=info) shows the fan-out reached
        // real devices — the per-token result below stays at debug to avoid
        // noise. Staff pushes are low-frequency, so this is cheap.
        tracing::info!(target: "apns", %role, devices = tokens.len(), "fan-out: sending push");

        for (token,) in tokens {
            let builder = DefaultNotificationBuilder::new()
                .title(title)
                .body(body)
                .sound("default");

            let opts = NotificationOptions {
                apns_priority: Some(Priority::High),
                apns_topic: Some(self.bundle_id.as_str()),
                ..Default::default()
            };
            let mut payload = builder.build(&token, opts);

            if let Some(link) = deep_link {
                if let Err(e) = payload.add_custom_data("deep_link", &link) {
                    tracing::warn!(target: "apns", error = %e, "add_custom_data failed");
                }
            }

            match self.client.send(payload).await {
                Ok(resp) => {
                    tracing::debug!(target: "apns", token = %short_token(&token), status = ?resp.code, "sent");
                    // Token-level invalidation (Apple says "Unregistered" or
                    // "BadDeviceToken") — mark so next fan-out skips it.
                    if matches!(resp.code, 410 | 400) {
                        let _ = sqlx::query(
                            "UPDATE push_devices
                             SET invalidated_at = CURRENT_TIMESTAMP
                             WHERE token = ?1",
                        )
                        .bind(&token)
                        .execute(&self.db)
                        .await;
                    }
                }
                Err(e) => {
                    tracing::warn!(target: "apns", token = %short_token(&token), error = ?e, "send failed");
                }
            }
        }
        Ok(())
    }
}

/// First 8 chars of the token; full token in logs would be a security
/// liability (anyone with it can forge our pushes).
fn short_token(token: &str) -> String {
    token.chars().take(8).collect::<String>() + "…"
}

/// Plumb ApnsHandle through the trait the frontend exposes, so server
/// fns in the frontend crate can broadcast without taking a direct
/// dependency on davidspizzeria-server.
#[async_trait]
impl BroadcastSink for ApnsHandle {
    async fn broadcast(
        &self,
        audience: BroadcastAudience,
        title: &str,
        body: &str,
    ) -> anyhow::Result<usize> {
        ApnsHandle::broadcast(self, audience, title, body).await
    }

    fn notify_roles(&self, roles: &[&str], title: &str, body: &str, deep_link: Option<&str>) {
        ApnsHandle::notify_roles(self, roles, title, body, deep_link);
    }
}
