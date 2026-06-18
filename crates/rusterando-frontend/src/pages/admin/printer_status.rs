//! /admin/drucker — kitchen-printer (Pi) status + OTA control.
//!
//! Surfaces what `kitchen_clients` records on every Hello (running
//! version, arch, last seen) next to the server's target version, with a
//! compat badge — so a version skew like the one that silently stopped
//! printing is **visible**, not discovered by SSHing in. The "Pi
//! aktualisieren" button arms the in-band self-update for the next Hello.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;

/// One row of Pi status for the admin panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiStatus {
    pub shop_slug: String,
    pub version: String,
    pub arch: String,
    /// Unix seconds of last Hello/heartbeat (0 = never).
    pub last_seen_at: i64,
    /// Seconds since last_seen (i64::MAX-ish sentinel if never).
    pub seconds_since_seen: i64,
    pub update_armed: bool,
    /// Server's target version for this Pi's arch, if an artifact is staged.
    pub target_version: Option<String>,
    /// True when running == target (or no target staged → nothing to do).
    pub up_to_date: bool,
    pub last_update_to: String,
    pub last_update_ok: bool,
    pub last_update_error: String,
    pub last_update_at: i64,
}

/// SSR-only DB row shape for `kitchen_clients`. Kept separate from the
/// wire `PiStatus` so the query stays a simple `query_as` (no 9-tuple).
#[cfg(feature = "ssr")]
#[derive(sqlx::FromRow)]
struct ClientRow {
    shop_slug: String,
    version: String,
    arch: String,
    last_seen_at: i64,
    update_armed: i64,
    last_update_to: String,
    last_update_ok: i64,
    last_update_error: String,
    last_update_at: i64,
}

#[server(name = ListPiStatus, prefix = "/api", endpoint = "list_pi_status")]
pub async fn list_pi_status() -> Result<Vec<PiStatus>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows: Vec<ClientRow> = sqlx::query_as(
        "SELECT shop_slug, version, arch, last_seen_at, update_armed, \
                last_update_to, last_update_ok, last_update_error, last_update_at \
         FROM kitchen_clients ORDER BY shop_slug",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load kitchen_clients: {e}")))?;

    let now = chrono::Utc::now().timestamp();

    // The artifact store lives in the server crate; the frontend (SSR) can't
    // depend on it directly. Read the target version straight from the same
    // artifacts dir the server uses, by arch.
    let artifact_dir =
        std::env::var("PRINTER_ARTIFACT_DIR").unwrap_or_else(|_| "artifacts/printer".to_string());

    Ok(rows
        .into_iter()
        .map(|r| {
            let target_version = std::fs::read_to_string(
                std::path::Path::new(&artifact_dir)
                    .join(&r.arch)
                    .join("version"),
            )
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

            let up_to_date = match &target_version {
                Some(t) => *t == r.version,
                None => true, // nothing staged → nothing to push
            };
            let seconds_since_seen = if r.last_seen_at == 0 {
                i64::MAX
            } else {
                (now - r.last_seen_at).max(0)
            };

            PiStatus {
                shop_slug: r.shop_slug,
                version: r.version,
                arch: r.arch,
                last_seen_at: r.last_seen_at,
                seconds_since_seen,
                update_armed: r.update_armed != 0,
                target_version,
                up_to_date,
                last_update_to: r.last_update_to,
                last_update_ok: r.last_update_ok != 0,
                last_update_error: r.last_update_error,
                last_update_at: r.last_update_at,
            }
            // (last_update_at is surfaced in PiStatus for future use.)
        })
        .collect())
}

/// Arm (or disarm) the in-band self-update for a shop's Pi. When armed,
/// the server offers the current target binary on the Pi's next Hello.
#[server(name = SetPiUpdateArmed, prefix = "/api", endpoint = "set_pi_update_armed")]
pub async fn set_pi_update_armed(shop_slug: String, armed: bool) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    sqlx::query("UPDATE kitchen_clients SET update_armed = ?2 WHERE shop_slug = ?1")
        .bind(&shop_slug)
        .bind(if armed { 1 } else { 0 })
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("arm update: {e}")))?;
    Ok(())
}

#[component]
pub fn PrinterStatusPage() -> impl IntoView {
    let armer = ServerAction::<SetPiUpdateArmed>::new();
    let rows = Resource::new(
        move || armer.version().get(),
        |_| async move { list_pi_status().await },
    );

    view! {
        <AdminShell>
            <section class="printer-status">
                <header class="admin-bar"><h1>"Drucker (Pi)"</h1></header>
                <p class="hint">
                    "Status der Küchen-Drucker. Zeigt die laufende Version gegen die "
                    "Server-Zielversion — so bleibt ein Versions-Mismatch sichtbar. "
                    "Updates laufen over-the-air über den bestehenden 9001-Kanal."
                </p>

                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || rows.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(list) if list.is_empty() => {
                            view! { <p class="empty">"Noch kein Drucker verbunden."</p> }.into_any()
                        }
                        Ok(list) => view! {
                            <div class="pi-cards">
                                {list.into_iter().map(|p| view! { <PiCard p armer/> }).collect_view()}
                            </div>
                        }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn PiCard(p: PiStatus, armer: ServerAction<SetPiUpdateArmed>) -> impl IntoView {
    // Connection freshness: green <90s, amber <10min, red otherwise.
    let (conn_cls, conn_label) = if p.seconds_since_seen == i64::MAX {
        ("pi-conn offline", "nie gesehen".to_string())
    } else if p.seconds_since_seen < 90 {
        ("pi-conn online", "online".to_string())
    } else if p.seconds_since_seen < 600 {
        ("pi-conn stale", format!("vor {}s", p.seconds_since_seen))
    } else {
        (
            "pi-conn offline",
            format!("vor {}min", p.seconds_since_seen / 60),
        )
    };

    let (ver_cls, ver_label) = match &p.target_version {
        Some(t) if !p.up_to_date => ("pi-ver mismatch", format!("{} → Ziel {}", p.version, t)),
        _ => ("pi-ver ok", format!("{} (aktuell)", p.version)),
    };

    let can_update = p.target_version.is_some() && !p.up_to_date;
    let shop_for_arm = p.shop_slug.clone();
    let shop_for_disarm = p.shop_slug.clone();
    let armed = p.update_armed;

    view! {
        <div class="pi-card">
            <header class="pi-head">
                <strong class="pi-shop">{p.shop_slug.clone()}</strong>
                <span class=conn_cls>{conn_label}</span>
            </header>
            <dl class="pi-meta">
                <dt>"Version"</dt>
                <dd class=ver_cls>{ver_label}</dd>
                <dt>"Arch"</dt>
                <dd class="muted">{p.arch.clone()}</dd>
                {(!p.last_update_to.is_empty()).then(|| {
                    let cls = if p.last_update_ok { "ok" } else { "error" };
                    let txt = if p.last_update_ok {
                        format!("✓ auf {} aktualisiert", p.last_update_to)
                    } else {
                        format!("✗ Update auf {} fehlgeschlagen: {}", p.last_update_to, p.last_update_error)
                    };
                    view! { <><dt>"Letztes Update"</dt><dd class=cls>{txt}</dd></> }
                })}
            </dl>

            <div class="pi-actions">
                {move || if armed {
                    let shop = shop_for_disarm.clone();
                    view! {
                        <span class="armed">"⏳ Update vorgemerkt — wird beim nächsten Verbinden ausgeführt"</span>
                        <button class="btn ghost small"
                            on:click=move |_| { armer.dispatch(SetPiUpdateArmed { shop_slug: shop.clone(), armed: false }); }>
                            "Abbrechen"
                        </button>
                    }.into_any()
                } else if can_update {
                    let shop = shop_for_arm.clone();
                    view! {
                        <button class="btn primary small"
                            on:click=move |_| { armer.dispatch(SetPiUpdateArmed { shop_slug: shop.clone(), armed: true }); }>
                            "Pi aktualisieren"
                        </button>
                    }.into_any()
                } else {
                    view! { <span class="muted small">"Kein Update verfügbar"</span> }.into_any()
                }}
            </div>
        </div>
    }
}
