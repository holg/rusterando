//! Screensaver for the board build (`kds-kms`): after a while without a touch
//! the display lends the screen to an external full-screen program
//! (`KDS_SAVER_CMD`) and takes it back on a double tap or a new order.
//!
//! Only one process can drive the screen (DRM master). Slint keeps its
//! DRM fd open for the life of the process, so the handover happens on that
//! fd: drop master → start the saver, which becomes master on open → on wake
//! kill it, take master back and re-point the CRTC at Slint's last frame
//! (the saver's exit turned the CRTC off). Slint's next page flip then
//! continues as if nothing happened. The client (printing, board updates)
//! keeps running throughout; only the screen changes hands.
//!
//! Rules: nothing may make Slint present a frame while master is lent out
//! (the flip would fail) — `kds::render` is skipped while active. All DRM
//! calls run on the main thread, which opened the device: without
//! CAP_SYS_ADMIN the kernel only gives master back to that same thread.
//!
//! The saver must not grab the touchscreen (EVIOCGRAB), so our libinput
//! still sees the touches. Single taps and drags are the saver's; a double
//! tap brings the board back (`kds::Taps`).
//!
//! Env: `KDS_SAVER_SECS` idle time (default 300, 0 = off), `KDS_SAVER_CMD`
//! program + args (unset = no saver). `{touch}` in the command is replaced
//! by the touchscreen's event device, looked up by name at start: its
//! number moves between kernels (event0 on 5.15, event1 on 6.6).

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use drm::control::{connector, crtc, framebuffer, Device as ControlDevice, Mode};
use drm::Device;
use tracing::{info, warn};

const DEFAULT_IDLE: Duration = Duration::from_secs(5 * 60);

/// A dup of Slint's DRM fd. Same open file description, so the same DRM
/// master status.
struct Card(OwnedFd);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl Device for Card {}
impl ControlDevice for Card {}

/// What was on screen when the saver started, to put back afterwards.
struct Scanout {
    crtc: crtc::Handle,
    connector: connector::Handle,
    fb: framebuffer::Handle,
    mode: Mode,
}

pub(super) struct Saver {
    idle: Duration,
    argv: Vec<String>,
    child: Option<(Child, Scanout)>,
    started: Option<Instant>,
}

impl Saver {
    /// `None` when switched off or the program isn't installed.
    pub(super) fn from_env() -> Option<Self> {
        let idle = match std::env::var("KDS_SAVER_SECS") {
            Ok(v) => Duration::from_secs(v.trim().parse().unwrap_or(DEFAULT_IDLE.as_secs())),
            Err(_) => DEFAULT_IDLE,
        };
        if idle.is_zero() {
            info!("screensaver off (KDS_SAVER_SECS=0)");
            return None;
        }
        let Ok(cmd) = std::env::var("KDS_SAVER_CMD") else {
            info!("screensaver off (no KDS_SAVER_CMD)");
            return None;
        };
        let argv: Vec<String> = cmd.split_whitespace().map(str::to_string).collect();
        let program = argv.first()?;
        if !std::path::Path::new(program).exists() {
            info!(%program, "screensaver off: program not installed");
            return None;
        }
        info!(%cmd, idle_s = idle.as_secs(), "screensaver armed");
        Some(Self {
            idle,
            argv,
            child: None,
            started: None,
        })
    }

    pub(super) fn idle(&self) -> Duration {
        self.idle
    }

    pub(super) fn active(&self) -> bool {
        self.child.is_some()
    }

    /// Hand the screen over and start the saver. Call only after the last
    /// UI frame has been presented. On failure the UI keeps the screen.
    pub(super) fn start(&mut self) -> anyhow::Result<()> {
        if self.active() {
            return Ok(());
        }
        let card = card()?;
        let scanout = scanout(&card)?;
        card.release_master_lock()
            .map_err(|e| anyhow::anyhow!("drop DRM master: {e}"))?;
        let touch = touchscreen().unwrap_or_else(|| "/dev/input/event0".to_string());
        let args: Vec<String> = self.argv[1..].iter().map(|a| a.replace("{touch}", &touch)).collect();
        let spawned = Command::new(&self.argv[0])
            .args(&args)
            .stdin(Stdio::null())
            // Keep a chatty saver out of the journal.
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(child) => {
                info!(pid = child.id(), "screensaver started");
                self.child = Some((child, scanout));
                self.started = Some(Instant::now());
                Ok(())
            }
            Err(e) => {
                let _ = card.acquire_master_lock();
                Err(anyhow::anyhow!("start {}: {e}", self.argv[0]))
            }
        }
    }

    /// The saver exited on its own (crash)? Then the screen must come back.
    pub(super) fn exited(&mut self) -> bool {
        match self.child.as_mut() {
            Some((child, _)) => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
            None => false,
        }
    }

    /// Stop the saver and take the screen back. After this the caller
    /// re-renders; Slint's next page flip lands on a live CRTC again.
    pub(super) fn stop(&mut self) {
        let Some((mut child, scanout)) = self.child.take() else {
            return;
        };
        // It holds no state worth a graceful shutdown; once it's reaped its
        // DRM fd is closed and master is free.
        let _ = child.kill();
        let _ = child.wait();
        let ran = self.started.take().map(|t| t.elapsed().as_secs()).unwrap_or(0);

        let card = match card() {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "screensaver: DRM fd gone; exiting for a clean restart");
                std::process::exit(1);
            }
        };
        // Master is released when the saver's fd closes, which wait() has
        // ensured; retry briefly anyway in case the kernel lags.
        let mut ok = false;
        for _ in 0..25 {
            if card.acquire_master_lock().is_ok() {
                ok = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !ok {
            // Without master the UI can never present again — a restart
            // (systemd Restart=always) is the only way back to a live screen.
            warn!("screensaver: could not take DRM master back; exiting for a clean restart");
            std::process::exit(1);
        }
        if let Err(e) = card.set_crtc(
            scanout.crtc,
            Some(scanout.fb),
            (0, 0),
            &[scanout.connector],
            Some(scanout.mode),
        ) {
            warn!(error = %e, "screensaver: restoring the CRTC failed; exiting for a clean restart");
            std::process::exit(1);
        }
        info!(ran_s = ran, "screensaver stopped, screen back");
    }
}

/// Block until the screen is free, i.e. we could become DRM master. Opening
/// the card makes us master when nobody else is; the probe fd is closed
/// again before Slint opens its own.
pub(super) fn wait_for_screen() {
    let mut waiting_since: Option<Instant> = None;
    loop {
        let free = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/dri/card0")
            .map(|f| Card(f.into()).acquire_master_lock().is_ok())
            .unwrap_or(false);
        if free {
            if let Some(t) = waiting_since {
                info!(waited_s = t.elapsed().as_secs(), "screen is free");
            }
            return;
        }
        if waiting_since.is_none() {
            warn!("screen busy: another program drives the display (Weston, a manually started saver?) — waiting");
            waiting_since = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// The touchscreen's `/dev/input/eventN`, from /proc/bus/input/devices (the
/// device whose name says touch screen).
fn touchscreen() -> Option<String> {
    let list = std::fs::read_to_string("/proc/bus/input/devices").ok()?;
    list.split("\n\n").find_map(|dev| {
        let name = dev.lines().find(|l| l.starts_with("N: Name="))?.to_ascii_lowercase();
        if !(name.contains("touchscreen") || name.contains("touch screen")) {
            return None;
        }
        let handlers = dev.lines().find(|l| l.starts_with("H: Handlers="))?;
        handlers
            .split_whitespace()
            .find(|h| h.starts_with("event"))
            .map(|ev| format!("/dev/input/{ev}"))
    })
}

/// Find Slint's open `/dev/dri/card*` among our fds and dup it.
fn card() -> anyhow::Result<Card> {
    for entry in std::fs::read_dir("/proc/self/fd")? {
        let entry = entry?;
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        if !target.to_string_lossy().starts_with("/dev/dri/card") {
            continue;
        }
        let Ok(raw) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        // SAFETY: the fd is open (we just saw it in /proc/self/fd) and Slint
        // keeps it open for the life of the process; we only borrow it long
        // enough to dup it.
        let borrowed = unsafe { BorrowedFd::borrow_raw(raw) };
        return Ok(Card(borrowed.try_clone_to_owned()?));
    }
    anyhow::bail!("no open /dev/dri/card* fd (not the KMS backend?)")
}

/// The CRTC driving the connected connector, with its current framebuffer
/// (Slint's last frame) and mode.
fn scanout(card: &Card) -> anyhow::Result<Scanout> {
    let res = card.resource_handles()?;
    for &conn in res.connectors() {
        let info = card.get_connector(conn, false)?;
        if info.state() != connector::State::Connected {
            continue;
        }
        let Some(enc) = info.current_encoder() else {
            continue;
        };
        let Some(crtc) = card.get_encoder(enc)?.crtc() else {
            continue;
        };
        let crtc_info = card.get_crtc(crtc)?;
        let (Some(fb), Some(mode)) = (crtc_info.framebuffer(), crtc_info.mode()) else {
            continue;
        };
        return Ok(Scanout {
            crtc,
            connector: conn,
            fb,
            mode,
        });
    }
    anyhow::bail!("no active CRTC on a connected connector")
}
