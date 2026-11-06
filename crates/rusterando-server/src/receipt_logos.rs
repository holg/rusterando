//! Receipt logos rendered on the server (crates/kitchen-protocol/RECEIPT_LOGOS.md).
//!
//! A print job's logo is a composed SVG (from the theme's files, see
//! `bon_designer`). The server renders it once to a 1-bit raster — with the
//! same `receipt_escpos::svg::rasterize` the devices used, so the paper looks
//! the same — and keeps SVG and raster in `receipt_logos`, keyed by
//! [`raster_hash`]. Print jobs in the outbox carry only the hash (a
//! `RasterSvg` with an empty `svg`: the protocol's reference form), and every
//! send resolves it for the receiving client ([`LogoStore::wire_message`]):
//! the finished raster for clients that announced raster bits
//! (kitchen-protocol 2.3), the SVG for older ones. So any stored job, old or
//! new, prints on any client.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use kitchen_protocol::bitmap::Bitmap;
use kitchen_protocol::receipt::{
    raster_hash, RasterProvider, RasterSlot, ReceiptLine, ReceiptProgram,
};
use kitchen_protocol::{KitchenEvent, ServerMessage};
use sqlx::{Row, SqlitePool};
use tracing::{debug, info, warn};

/// One logo: the SVG it came from and its 1-bit raster.
pub struct Logo {
    pub svg: Vec<u8>,
    pub target_width: u16,
    pub bitmap: Bitmap,
}

/// What one receiving client can take.
#[derive(Debug, Default, Clone)]
pub struct WireCaps {
    /// Logos as server-rendered `RasterBits` (kitchen-protocol 2.3).
    pub raster_bits: bool,
    /// Tunnel clients: logo hashes the device has cached (its Hello). An SVG
    /// client gets a reference for these instead of the SVG bytes.
    pub cached: HashSet<[u8; 32]>,
}

impl WireCaps {
    /// From a WebSocket device's `caps` URL parameter (`raster-bits`, …).
    pub fn from_param(caps: &str) -> Self {
        Self {
            raster_bits: caps.split(',').any(|c| c.trim() == "raster-bits"),
            cached: HashSet::new(),
        }
    }
}

/// The logo store: the database table plus an in-process cache (a handful
/// of logos, a few KB each).
#[derive(Clone)]
pub struct LogoStore {
    db: SqlitePool,
    cache: Arc<Mutex<HashMap<[u8; 32], Arc<Logo>>>>,
}

impl LogoStore {
    pub fn new(db: SqlitePool) -> Self {
        Self {
            db,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The logo with this hash, from memory or the database.
    pub async fn get(&self, hash: &[u8; 32]) -> Result<Option<Arc<Logo>>> {
        if let Some(logo) = self.cache.lock().unwrap().get(hash) {
            return Ok(Some(logo.clone()));
        }
        let row = sqlx::query(
            "SELECT svg, target_width, width, height, bits FROM receipt_logos WHERE hash = ?",
        )
        .bind(&hash[..])
        .fetch_optional(&self.db)
        .await
        .context("load receipt logo")?;
        let Some(row) = row else {
            return Ok(None);
        };
        let logo = Arc::new(Logo {
            svg: row.get("svg"),
            target_width: row.get::<i64, _>("target_width") as u16,
            bitmap: Bitmap {
                width: row.get::<i64, _>("width") as usize,
                height: row.get::<i64, _>("height") as usize,
                bytes: row.get("bits"),
            },
        });
        self.cache.lock().unwrap().insert(*hash, logo.clone());
        Ok(Some(logo))
    }

    /// Put a logo into the store, rendering it if it's new; returns its hash.
    pub async fn put(&self, svg: &[u8], target_width: u16) -> Result<[u8; 32]> {
        let hash = raster_hash(svg, target_width);
        if self.get(&hash).await?.is_some() {
            return Ok(hash);
        }
        let started = Instant::now();
        let bitmap = render(svg.to_vec(), target_width).await?;
        sqlx::query(
            "INSERT OR IGNORE INTO receipt_logos \
               (hash, svg, target_width, width, height, bits, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, strftime('%s', 'now'))",
        )
        .bind(&hash[..])
        .bind(svg)
        .bind(target_width as i64)
        .bind(bitmap.width as i64)
        .bind(bitmap.height as i64)
        .bind(&bitmap.bytes)
        .execute(&self.db)
        .await
        .context("store receipt logo")?;
        info!(
            hash = %short(&hash),
            width = bitmap.width,
            height = bitmap.height,
            svg_bytes = svg.len(),
            raster_bytes = bitmap.bytes.len(),
            ms = started.elapsed().as_millis() as u64,
            "receipt logo rendered and stored"
        );
        self.cache.lock().unwrap().insert(
            hash,
            Arc::new(Logo {
                svg: svg.to_vec(),
                target_width,
                bitmap,
            }),
        );
        Ok(hash)
    }

    /// The stored form of an event: every inline logo goes into the store,
    /// the job keeps a reference. A logo that can't be rendered stays inline
    /// (older devices may still manage; bits clients get it left out).
    pub async fn externalize(&self, event: KitchenEvent) -> KitchenEvent {
        let KitchenEvent::Print(mut program) = event else {
            return event;
        };
        for line in &mut program.lines {
            if let ReceiptLine::RasterSvg {
                hash,
                target_width,
                svg,
            } = line
            {
                if svg.is_empty() {
                    continue;
                }
                match self.put(svg, *target_width).await {
                    Ok(stored) if stored == *hash => svg.clear(),
                    Ok(_) => warn!(
                        hash = %short(hash),
                        "receipt logo: the job's hash doesn't match its SVG; kept inline"
                    ),
                    Err(e) => warn!(error = %e, "receipt logo not rendered; kept inline"),
                }
            }
        }
        KitchenEvent::Print(program)
    }

    /// `msg` as one client should get it.
    pub async fn wire_message(&self, msg: ServerMessage, caps: &WireCaps) -> ServerMessage {
        match msg.event {
            KitchenEvent::Print(program) => ServerMessage {
                seq_id: msg.seq_id,
                event: KitchenEvent::Print(self.wire_program(program, caps).await),
            },
            event => ServerMessage {
                seq_id: msg.seq_id,
                event,
            },
        }
    }

    /// `program` as one client should get it.
    pub async fn wire_program(&self, program: ReceiptProgram, caps: &WireCaps) -> ReceiptProgram {
        let mut lines = Vec::with_capacity(program.lines.len());
        for line in program.lines {
            match line {
                ReceiptLine::RasterSvg {
                    hash,
                    target_width,
                    svg,
                } => {
                    if let Some(line) = self.wire_logo(hash, target_width, svg, caps).await {
                        lines.push(line);
                    }
                }
                other => lines.push(other),
            }
        }
        ReceiptProgram { lines, ..program }
    }

    /// One logo line for one client. `None` leaves it out — what a device
    /// does anyway with a logo it can't resolve (the text banners still say
    /// channel and payment).
    async fn wire_logo(
        &self,
        hash: [u8; 32],
        target_width: u16,
        svg: Vec<u8>,
        caps: &WireCaps,
    ) -> Option<ReceiptLine> {
        if caps.raster_bits {
            let logo = match self.get(&hash).await {
                Ok(Some(logo)) => Some(logo),
                // A job from before the logo store, not migrated yet.
                Ok(None) if !svg.is_empty() => match self.put(&svg, target_width).await {
                    Ok(stored) if stored == hash => self.get(&hash).await.ok().flatten(),
                    Ok(_) => None,
                    Err(e) => {
                        warn!(hash = %short(&hash), error = %e, "receipt logo not rendered");
                        None
                    }
                },
                Ok(None) => None,
                Err(e) => {
                    warn!(hash = %short(&hash), error = %e, "receipt logo lookup failed");
                    None
                }
            };
            let Some(logo) = logo else {
                debug!(hash = %short(&hash), "receipt logo unknown; left out");
                return None;
            };
            return Some(ReceiptLine::RasterBits {
                hash,
                width: logo.bitmap.width as u16,
                height: logo.bitmap.height as u16,
                bits: logo.bitmap.bytes.clone(),
            });
        }
        // SVG clients (before 2.3).
        if caps.cached.contains(&hash) {
            return Some(ReceiptLine::RasterSvg {
                hash,
                target_width,
                svg: Vec::new(),
            });
        }
        if svg.is_empty() {
            match self.get(&hash).await {
                Ok(Some(logo)) => {
                    return Some(ReceiptLine::RasterSvg {
                        hash,
                        target_width,
                        svg: logo.svg.clone(),
                    })
                }
                // An unknown reference: the device leaves it out.
                Ok(None) => {}
                Err(e) => warn!(hash = %short(&hash), error = %e, "receipt logo lookup failed"),
            }
        }
        Some(ReceiptLine::RasterSvg {
            hash,
            target_width,
            svg,
        })
    }

    /// Render the configured theme's logos at start, so broken or oversized
    /// artwork shows in the log at deploy time rather than on a printer, and
    /// the first order of the day doesn't wait for a render.
    pub async fn warm_up(&self, theme: &str, provider: &(dyn RasterProvider + Sync)) {
        if theme.is_empty() {
            info!("receipt logos: default theme, none to render");
            return;
        }
        for is_delivery in [false, true] {
            for is_paid in [false, true] {
                let slot = RasterSlot::ChannelAndBrand {
                    is_delivery,
                    is_paid,
                };
                let Some(composed) = provider.resolve(theme, slot) else {
                    warn!(%theme, is_delivery, is_paid, "receipt logo missing from the theme");
                    continue;
                };
                match self.put(&composed.svg, composed.target_width).await {
                    Ok(hash) => info!(
                        %theme, is_delivery, is_paid, hash = %short(&hash),
                        "receipt logo ready"
                    ),
                    Err(e) => warn!(
                        %theme, is_delivery, is_paid, error = %e,
                        "receipt logo does NOT render — orders print without it"
                    ),
                }
            }
        }
    }
}

/// Render on the blocking pool (resvg recurses deeply; the blocking
/// threads have full-size stacks).
async fn render(svg: Vec<u8>, target_width: u16) -> Result<Bitmap> {
    tokio::task::spawn_blocking(move || receipt_escpos::svg::rasterize(&svg, target_width))
        .await
        .context("logo renderer task")?
}

/// The first bytes of a hash, for the log.
fn short(hash: &[u8; 32]) -> String {
    hash[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// What [`migrate_outbox`] found and did.
#[derive(Debug, Default)]
pub struct MigrationReport {
    pub dry_run: bool,
    /// Outbox rows looked at.
    pub rows: usize,
    /// Print jobs that still carried a logo's SVG inline.
    pub with_inline_logo: usize,
    /// Of those, rewritten to references (0 in a dry run).
    pub migrated: usize,
    /// Distinct logos among them.
    pub logos: usize,
    /// Payload bytes of the jobs with an inline logo, before and after.
    pub bytes_before: usize,
    pub bytes_after: usize,
    /// Left untouched: another protocol major, not decodable, or a logo that
    /// doesn't render / whose hash doesn't match (logged by id).
    pub other_major: usize,
    pub undecodable: usize,
    pub failed: usize,
}

impl fmt::Display for MigrationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "receipt logo migration{}:",
            if self.dry_run {
                " (dry run, nothing written)"
            } else {
                ""
            }
        )?;
        writeln!(f, "  outbox rows:                {}", self.rows)?;
        writeln!(f, "  jobs with an inline logo:   {}", self.with_inline_logo)?;
        writeln!(f, "  migrated to references:     {}", self.migrated)?;
        writeln!(f, "  distinct logos:             {}", self.logos)?;
        writeln!(
            f,
            "  their payload bytes:        {} -> {}",
            self.bytes_before, self.bytes_after
        )?;
        write!(
            f,
            "  left untouched:             {} other major, {} undecodable, {} failed",
            self.other_major, self.undecodable, self.failed
        )
    }
}

/// One-time pass over `kitchen_outbox`: every stored print job that carries
/// a logo's SVG inline gets it moved into the logo store and keeps a
/// reference. Each row's `id`, `created_at`, `acked_at` and stored version
/// header stay as they are; rows of another major, undecodable rows and
/// rows whose logo doesn't render are left untouched. Idempotent: a second
/// run finds nothing. `dry_run` renders and checks everything but writes
/// nothing (neither the jobs nor the logo store).
pub async fn migrate_outbox(
    db: &SqlitePool,
    store: &LogoStore,
    dry_run: bool,
) -> Result<MigrationReport> {
    let mut report = MigrationReport {
        dry_run,
        ..Default::default()
    };
    let mut logos: HashSet<[u8; 32]> = HashSet::new();
    let mut after_id = 0i64;
    loop {
        let rows = sqlx::query(
            "SELECT id, payload FROM kitchen_outbox WHERE id > ? ORDER BY id LIMIT 200",
        )
        .bind(after_id)
        .fetch_all(db)
        .await
        .context("read kitchen_outbox")?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let id: i64 = row.get("id");
            after_id = id;
            let payload: Vec<u8> = row.get("payload");
            report.rows += 1;

            let body = match kitchen_protocol::decode_versioned(&payload) {
                Ok((_minor, body)) => body,
                Err(_) => {
                    report.other_major += 1;
                    continue;
                }
            };
            let Ok(event) = postcard::from_bytes::<KitchenEvent>(body) else {
                report.undecodable += 1;
                continue;
            };
            let KitchenEvent::Print(program) = event else {
                continue;
            };
            let inline: Vec<([u8; 32], u16, Vec<u8>)> = program
                .lines
                .iter()
                .filter_map(|line| match line {
                    ReceiptLine::RasterSvg {
                        hash,
                        target_width,
                        svg,
                    } if !svg.is_empty() => Some((*hash, *target_width, svg.clone())),
                    _ => None,
                })
                .collect();
            if inline.is_empty() {
                continue;
            }
            report.with_inline_logo += 1;

            // Every logo must render and match its hash (dry run too).
            let mut ok = true;
            for (hash, target_width, svg) in &inline {
                if raster_hash(svg, *target_width) != *hash {
                    warn!(
                        outbox_id = id,
                        "logo hash doesn't match its SVG; row left as is"
                    );
                    ok = false;
                    break;
                }
                if logos.contains(hash) {
                    continue;
                }
                let rendered = if dry_run {
                    render(svg.clone(), *target_width).await.map(|_| ())
                } else {
                    store.put(svg, *target_width).await.map(|_| ())
                };
                if let Err(e) = rendered {
                    warn!(outbox_id = id, error = %e, "logo doesn't render; row left as is");
                    ok = false;
                    break;
                }
                logos.insert(*hash);
            }
            if !ok {
                report.failed += 1;
                continue;
            }

            // The same job, logos as references, under the stored header.
            let mut stored = program;
            for line in &mut stored.lines {
                if let ReceiptLine::RasterSvg { svg, .. } = line {
                    svg.clear();
                }
            }
            let new_body = postcard::to_allocvec(&KitchenEvent::Print(stored.clone()))
                .context("encode migrated job")?;
            let mut new_payload = payload[..2].to_vec();
            new_payload.extend_from_slice(&new_body);
            // It must decode back to exactly that job.
            match postcard::from_bytes::<KitchenEvent>(&new_payload[2..]) {
                Ok(KitchenEvent::Print(back)) if back == stored => {}
                _ => anyhow::bail!("outbox row {id}: the migrated job doesn't decode back"),
            }
            report.bytes_before += payload.len();
            report.bytes_after += new_payload.len();
            if dry_run {
                continue;
            }
            // Only if the row is still what we read (the server may run).
            let done =
                sqlx::query("UPDATE kitchen_outbox SET payload = ? WHERE id = ? AND payload = ?")
                    .bind(&new_payload)
                    .bind(id)
                    .bind(&payload)
                    .execute(db)
                    .await
                    .context("write migrated job")?
                    .rows_affected();
            if done == 1 {
                report.migrated += 1;
            } else {
                warn!(outbox_id = id, "row changed meanwhile; left as is");
                report.failed += 1;
            }
        }
    }
    report.logos = logos.len();
    Ok(report)
}

/// Outbox rows that look like they still carry a logo's SVG inline (big
/// payloads) — a cheap check for the startup log.
pub async fn inline_logo_rows(db: &SqlitePool) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM kitchen_outbox WHERE length(payload) > 4096")
        .fetch_one(db)
        .await
        .context("count big outbox rows")
}
