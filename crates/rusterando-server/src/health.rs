//! Process-level health observability — file-descriptor count + soft
//! limit, mostly. The kind of stuff that, if we had been watching it,
//! would have warned us on 2026-05-30 before customers hit 502s at
//! 16:00 UTC on a Saturday because the long-running server had
//! accumulated CLOSE_WAIT sockets up to the systemd `LimitNOFILE=1024`
//! default. See [[feedback-fd-limit-systemd]].
//!
//! Two surfaces, both designed to be cheap:
//!
//! * [`spawn_periodic_log`] — fires once a tokio task at boot. Every
//!   `interval` (5 min in prod) it reads `/proc/self/fd` + the rlimit
//!   and emits one structured `tracing` event. Level escalates with
//!   the percentage so a casual `journalctl -p warning` immediately
//!   surfaces a leak in progress.
//!
//! * [`fd_snapshot`] — synchronous snapshot used by the
//!   `/api/healthz` handler in `main.rs`. Returns the same numbers
//!   the periodic log uses, plus `uptime_s`. Designed for an external
//!   uptime monitor (UptimeRobot, BetterStack, etc.) to curl every
//!   minute and alert on `status != "ok"`.
//!
//! Linux-only — `/proc/self/fd` doesn't exist on macOS, so the dev
//! shell on the workstation logs zeros. That's deliberate; we only
//! care about prod behaviour.

use serde::Serialize;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Process start time, captured at first call to `uptime_s()`. The
/// server's `main` is async and we don't want to thread an Instant
/// through every layer, so `OnceLock` is the cheapest "boot
/// timestamp" we can have. First read seeds it; in practice that's
/// the periodic logger or `/api/healthz` whichever fires first.
fn boot_instant() -> Instant {
    static BOOT: OnceLock<Instant> = OnceLock::new();
    *BOOT.get_or_init(Instant::now)
}

/// Seconds since `boot_instant()` was first observed. Seeds the
/// boot instant on first call, so the very first reading will be ~0.
pub fn uptime_s() -> u64 {
    boot_instant().elapsed().as_secs()
}

/// What the periodic logger + `/api/healthz` both serialise. Stable
/// JSON shape so an external monitor can parse it.
#[derive(Debug, Clone, Serialize)]
pub struct FdSnapshot {
    /// File descriptors currently held by this process. `None` when
    /// the platform can't tell us (anything non-Linux).
    pub fds: Option<u64>,
    /// Soft limit on open files (`RLIMIT_NOFILE`). `None` when the
    /// `getrlimit` call fails — should never happen in prod.
    pub fd_limit: Option<u64>,
    /// `fds * 100 / fd_limit`, rounded down. `None` when either of
    /// the above is missing.
    pub fd_pct: Option<u64>,
    /// `"ok"` / `"warn"` / `"crit"` per the same thresholds the
    /// periodic logger uses — see [`status_for_pct`].
    pub status: &'static str,
    /// Seconds since process start. Lets the monitor distinguish a
    /// freshly-restarted server from a stale one.
    pub uptime_s: u64,
}

/// Single source of truth for the fd-usage thresholds. Used by both
/// the periodic logger (to escalate log level) and `/api/healthz` (to
/// set `status`). Numbers are deliberately conservative: at 50% we
/// want to know there's a leak in progress; at 80% the operator has
/// time to react before axum starts rejecting accept() calls.
pub fn status_for_pct(pct: u64) -> &'static str {
    if pct >= 80 {
        "crit"
    } else if pct >= 50 {
        "warn"
    } else {
        "ok"
    }
}

/// One synchronous snapshot — counts `/proc/self/fd` entries +
/// reads `RLIMIT_NOFILE`. Cheap enough to call on every
/// `/api/healthz` hit (the dir scan is ~20-30 entries for this
/// process under normal load).
pub fn fd_snapshot() -> FdSnapshot {
    let fds = count_fds();
    let fd_limit = nofile_soft_limit();
    let fd_pct = match (fds, fd_limit) {
        (Some(n), Some(l)) if l > 0 => Some(n.saturating_mul(100) / l),
        _ => None,
    };
    let status = fd_pct.map(status_for_pct).unwrap_or("ok");
    FdSnapshot {
        fds,
        fd_limit,
        fd_pct,
        status,
        uptime_s: uptime_s(),
    }
}

/// `/proc/self/fd` is a directory of symlinks, one per open fd.
/// `Some(count)` on Linux; `None` everywhere else (no fd-counting
/// happens on macOS workstation builds — deliberate).
fn count_fds() -> Option<u64> {
    let entries = std::fs::read_dir("/proc/self/fd").ok()?;
    // Counting entries silently skips Err items (broken symlinks etc.)
    // which matches what `ls /proc/self/fd | wc -l` returns.
    Some(entries.filter_map(|e| e.ok()).count() as u64)
}

/// `getrlimit(RLIMIT_NOFILE)` — the soft limit, which is what
/// actually constrains `accept()`. We don't surface the hard limit
/// because exceeding it requires `setrlimit` (admin action), not
/// something a runtime leak would do.
#[cfg(target_os = "linux")]
fn nofile_soft_limit() -> Option<u64> {
    let mut rl = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: libc::getrlimit is a syscall wrapper; rl is a valid
    // mutable pointer to a properly-aligned rlimit struct.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl as *mut _) };
    if rc == 0 {
        Some(rl.rlim_cur)
    } else {
        None
    }
}

#[cfg(not(target_os = "linux"))]
fn nofile_soft_limit() -> Option<u64> {
    None
}

/// fd_pct at which we proactively exit so systemd restarts us before
/// axum's accept() starts rejecting with EMFILE. Tuned conservatively:
/// at 95% there's still room for the request that's currently in
/// flight to complete before the next sample fires. Yesterday's
/// outage happened because the long-lived process slowly climbed
/// past 100% (1024 default cap) and got stuck spewing accept errors
/// for hours. Self-restarting at 95% means a brief 2-3s gap (systemd
/// RestartSec=5 → actually Restart=always picks us up fast) instead
/// of a 5-hour customer-facing outage.
const FD_SELF_RESTART_PCT: u64 = 95;

/// Spawn a background task that samples fd usage every `interval`
/// (default 30 s in prod). Behaviour:
///   * fd_pct < 50  → INFO log once per `log_every` intervals (to
///     avoid spamming journal during quiet evenings)
///   * fd_pct 50-79 → WARN every interval — leak in progress
///   * fd_pct 80-94 → ERROR every interval — operator awareness
///   * fd_pct >= 95 → ERROR + `std::process::exit(1)` so systemd's
///     existing `Restart=always` brings us back CLEANLY before
///     axum rejects accept(). No 502s reach the customer.
///
/// All in-process. One readdir on `/proc/self/fd` + one `getrlimit`
/// per tick — no disk writes, no nginx round-trip, no external probe.
/// Per [[feedback-in-memory-first]] this is the right shape: the
/// server owns its own health and the OS only sees the
/// already-configured `Restart=always` baseline.
pub fn spawn_self_monitor(interval: Duration) {
    // Seed boot time NOW so uptime reads later are meaningful even
    // if a /healthz hit beats us to the first sample.
    let _ = boot_instant();
    // Log every Nth interval at INFO when healthy, so journal shows
    // proof-of-life every ~5 min (30 s × 10) without flooding.
    let log_every: u32 = 10;
    tokio::spawn(async move {
        let mut tick: u32 = 0;
        loop {
            let snap = fd_snapshot();
            let pct = snap.fd_pct.unwrap_or(0);
            if pct >= FD_SELF_RESTART_PCT {
                // One last log line that's GUARANTEED to land in
                // journal before exit. Use error! so it's
                // never filtered out by RUST_LOG=info.
                tracing::error!(
                    fds = ?snap.fds, fd_limit = ?snap.fd_limit, fd_pct = ?snap.fd_pct,
                    uptime_s = snap.uptime_s,
                    threshold_pct = FD_SELF_RESTART_PCT,
                    "fd usage past self-restart threshold — initiating graceful shutdown"
                );
                // Trigger the main() graceful-shutdown path by sending
                // ourselves SIGTERM. axum's with_graceful_shutdown
                // catches it, drains in-flight requests for 5 s, then
                // closes the sqlite pool — same code path systemd
                // uses on `systemctl restart`. Without this hop the
                // process would just `exit(1)` and customers
                // mid-checkout would see RST'd TCP connections.
                //
                // If the SIGTERM raise fails for any reason (kernel
                // misconfigured? we shouldn't run anywhere it can),
                // fall through to a hard exit(1) so we don't get
                // stuck above the threshold forever. systemd's
                // Restart=always picks us up either way.
                //
                // Gated to linux: libc is a Linux-only direct dep
                // here (see Cargo.toml's
                // [target.'cfg(target_os = "linux")'.dependencies]).
                // macOS dev builds fall through to the hard exit;
                // we never deploy to macOS anyway.
                #[cfg(target_os = "linux")]
                unsafe {
                    // SIGTERM = 15. raise() is async-signal-safe and
                    // delivers the signal to the calling thread —
                    // which is fine, the tokio runtime listens on
                    // every thread.
                    if libc::raise(libc::SIGTERM) != 0 {
                        tracing::error!(
                            "raise(SIGTERM) failed — falling back to exit(1)"
                        );
                        std::process::exit(1);
                    }
                }
                #[cfg(not(target_os = "linux"))]
                std::process::exit(1);
                // Stop the loop on Linux — main() will hand control
                // over to the shutdown future and the process will
                // be down shortly. Looping further would risk a
                // second SIGTERM raise (idempotent at the kernel
                // level, but logs would look confused).
                #[cfg(target_os = "linux")]
                return;
            } else if pct >= 80 {
                tracing::error!(
                    fds = ?snap.fds, fd_limit = ?snap.fd_limit, fd_pct = ?snap.fd_pct,
                    uptime_s = snap.uptime_s,
                    "fd usage critical — self-restart imminent at {FD_SELF_RESTART_PCT}%");
            } else if pct >= 50 {
                tracing::warn!(
                    fds = ?snap.fds, fd_limit = ?snap.fd_limit, fd_pct = ?snap.fd_pct,
                    uptime_s = snap.uptime_s,
                    "fd usage elevated — possible leak in progress");
            } else if tick == 0 {
                tracing::info!(
                    fds = ?snap.fds, fd_limit = ?snap.fd_limit, fd_pct = ?snap.fd_pct,
                    uptime_s = snap.uptime_s,
                    "fd usage healthy");
            }
            tick = (tick + 1) % log_every;
            tokio::time::sleep(interval).await;
        }
    });
}
