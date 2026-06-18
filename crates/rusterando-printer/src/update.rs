//! In-band self-update: pull a new binary over the trusted 9001 stream,
//! verify it, install it atomically, and let systemd relaunch us.
//!
//! Motivated by the version-skew failure (a 0.1.0 Pi silently not printing
//! against a 1.0 server). The server is admin-gated, so an `UpdateOffer`
//! only arrives when an operator armed it; this module is the Pi side.
//!
//! ## Apply strategy: self-exec
//!
//! A process can't cleanly `systemctl restart` itself and live to confirm,
//! so instead we:
//!   1. download + verify the artifact,
//!   2. atomically `rename` it over our own executable path,
//!   3. send `UpdateResult{ok}` so the server clears the armed flag,
//!   4. `std::process::exit(0)` — systemd `Restart=always` relaunches the
//!      now-updated binary.
//!
//! Overwriting a running executable is safe on Linux: the kernel keeps the
//! old inode open until we exit, and `rename` is atomic so there's never a
//! half-written binary on disk. The executable MUST live under the unit's
//! one `ReadWritePaths` dir (`/var/lib/rusterando-printer/bin/`), because
//! `ProtectSystem=strict` makes `/usr/local/bin` read-only to us.
//!
//! Never bricks: any failure returns `Ok(())` (declining the update) or
//! reports `UpdateResult{ok:false}` and keeps the current binary running.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use kitchen_protocol::{ClientMessage, Component, KitchenEvent, ServerMessage};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{error, info, warn};

use crate::config::Config;

/// Max bytes we'll accept for an artifact — a sane ceiling well above any
/// plausible Pi binary (the current one is ~2 MB).
const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

/// Drive an offered update to completion. Returns:
///   * never (process exits) on a successful install,
///   * `Ok(())` if we decline (already current / not applicable),
///   * `Err` only on a stream/protocol error that should drop the session
///     (the caller reconnects); a *failed but recoverable* update reports
///     `UpdateResult{ok:false}` and returns `Ok(())`.
pub async fn apply_update(
    cfg: &Config,
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    component: Component,
    target_version: &str,
    sha256: [u8; 32],
    size: u64,
) -> Result<()> {
    if component != Component::PrinterBinary {
        warn!(?component, "update offer for unknown component — declining");
        return Ok(());
    }
    if target_version == cfg.version {
        info!(version = %cfg.version, "update offer matches running version — declining");
        return Ok(());
    }
    if size == 0 || size > MAX_ARTIFACT_BYTES {
        report(
            framed,
            cfg,
            target_version,
            false,
            "implausible artifact size",
        )
        .await;
        return Ok(());
    }

    info!(from = %cfg.version, to = %target_version, size, "applying self-update");

    // ===== Pull the artifact in chunks =====
    let mut buf: Vec<u8> = Vec::with_capacity(size as usize);
    loop {
        let req = ClientMessage::FetchArtifact {
            component,
            offset: buf.len() as u64,
        };
        send(framed, &req).await.context("send FetchArtifact")?;

        let chunk = match next_artifact_chunk(framed).await {
            Ok(c) => c,
            Err(e) => {
                // Stream-level problem — drop the session and let the
                // reconnect retry; the binary is untouched.
                return Err(e);
            }
        };
        let (offset, bytes, last) = chunk;
        if offset != buf.len() as u64 {
            report(framed, cfg, target_version, false, "chunk offset mismatch").await;
            return Ok(());
        }
        buf.extend_from_slice(&bytes);
        if buf.len() as u64 > size {
            report(
                framed,
                cfg,
                target_version,
                false,
                "artifact exceeded declared size",
            )
            .await;
            return Ok(());
        }
        if last {
            break;
        }
    }

    if buf.len() as u64 != size {
        report(framed, cfg, target_version, false, "artifact size mismatch").await;
        return Ok(());
    }

    // ===== Verify sha256 =====
    let mut hasher = Sha256::new();
    hasher.update(&buf);
    let got: [u8; 32] = hasher.finalize().into();
    if got != sha256 {
        error!("self-update sha256 mismatch — refusing to install");
        report(framed, cfg, target_version, false, "sha256 mismatch").await;
        return Ok(());
    }

    // ===== Atomic install over our own executable path =====
    if let Err(e) = install_over_self(&buf) {
        error!(error = %e, "self-update install failed — keeping current binary");
        report(framed, cfg, target_version, false, &format!("install: {e}")).await;
        return Ok(());
    }

    // Report success BEFORE exiting so the server clears the armed flag.
    report(framed, cfg, target_version, true, "").await;
    info!(to = %target_version, "self-update installed — exiting for systemd to relaunch");

    // Give the sink a moment to flush the UpdateResult frame.
    let _ = framed.flush().await;

    // systemd Restart=always relaunches us from the new binary.
    std::process::exit(0);
}

/// Atomically replace our own executable with `bytes`. Writes to a temp
/// file in the SAME directory (so `rename` is atomic, not a cross-device
/// copy), sets it executable, then renames over the running path.
fn install_over_self(bytes: &[u8]) -> Result<()> {
    let exe = std::env::current_exe().context("locate current exe")?;
    let dir = exe
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("exe has no parent dir"))?;
    let tmp = dir.join(".rusterando-printer.update.tmp");

    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(bytes).context("write staged binary")?;
        f.sync_all().context("fsync staged binary")?;
        // chmod 755.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&tmp, perms).context("chmod staged binary")?;
        }
    }

    std::fs::rename(&tmp, &exe)
        .with_context(|| format!("rename {} -> {}", tmp.display(), exe.display()))?;
    Ok(())
}

/// Read frames until we get an `ArtifactChunk`, returning `(offset, bytes,
/// last)`. Heartbeats/other inbound frames during a transfer are unusual
/// (the server is mid-transfer); anything that isn't a chunk is an error.
async fn next_artifact_chunk(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
) -> Result<(u64, Vec<u8>, bool)> {
    let frame = framed
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("server closed during artifact transfer"))?
        .context("read artifact frame")?;
    let postcard_bytes = kitchen_protocol::decode_versioned(&frame)
        .map(|(_minor, inner)| inner)
        .map_err(|e| anyhow::anyhow!("artifact envelope decode: {e}"))?;
    let msg: ServerMessage =
        postcard::from_bytes(postcard_bytes).context("decode artifact ServerMessage")?;
    match msg.event {
        KitchenEvent::ArtifactChunk {
            offset,
            bytes,
            last,
            ..
        } => Ok((offset, bytes, last)),
        other => anyhow::bail!("expected ArtifactChunk during transfer, got {other:?}"),
    }
}

/// Send an `UpdateResult` (best-effort — failures here are logged, not
/// fatal: we've already done the important work).
async fn report(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    cfg: &Config,
    to: &str,
    ok: bool,
    error: &str,
) {
    let msg = ClientMessage::UpdateResult {
        component: Component::PrinterBinary,
        from: cfg.version.clone(),
        to: to.to_string(),
        ok,
        error: error.to_string(),
    };
    if let Err(e) = send(framed, &msg).await {
        warn!(error = %e, "failed to send UpdateResult");
    }
}

async fn send(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    msg: &ClientMessage,
) -> Result<()> {
    let body = postcard::to_allocvec(msg).context("encode update message")?;
    let bytes = kitchen_protocol::encode_versioned(&body);
    framed
        .send(Bytes::from(bytes))
        .await
        .context("send update message")?;
    Ok(())
}
