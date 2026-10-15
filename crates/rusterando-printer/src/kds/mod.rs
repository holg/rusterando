//! Kitchen display on the board (Slint touch UI) — the `kds` feature, built
//! for the STM32MP135F-DK's 480×272 touch screen (`kds-kms`) or as a
//! desktop window for development (`kds-desktop`).
//!
//! The screen itself — layout, cards, detail, Öffnungszeiten page — is the
//! `kds-ui` crate, shared with the browser mirror. This module adds what
//! only the board has: the printer and its chip, Testdruck, the
//! screensaver, the gong, and the kitchen-channel client.
//!
//! Slint owns the main thread. The kitchen-channel client — printing, and
//! the board subscription — runs on a tokio runtime in a background thread
//! and talks to the UI through [`UiLink`]. It starts first: receipts print
//! even while the screen is still held by something else (the boot splash).
//!
//! `KDS_DEMO=1` runs the UI on built-in sample orders with no server or
//! printer, moving cards locally. `KDS_SNAPSHOT=<dir>` renders those sample
//! orders headlessly — same software renderer as the board — to
//! `<dir>/board.png`, `detail.png`, `hours.png` and `start.png` and exits.

#[cfg(feature = "kds-kms")]
mod saver;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use kds_ui::{Event, KitchenWindow, NOTICE_FOR};
use kitchen_protocol::ClientMessage;
use slint::ComponentHandle;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::config::Config;
use crate::link::{UiEvent, UiLink};

/// What only the board knows; the screen state is [`kds_ui::Core`].
#[derive(Default)]
struct State {
    /// `None` until the client reports; `Some(Err)` while prints fail.
    printer: Option<Result<String, String>>,
    pending_prints: usize,
    /// Black cover is up (screen about to be, or already, lent out).
    saver_shown: bool,
    /// Double-tap detection on the saver cover.
    taps: Taps,
    /// `PRINTER_PATH`, for the Start page's Testdruck and info line.
    printer_path: String,
    /// "Bildschirmschoner" pressed: start the screensaver on the next tick.
    saver_force: bool,
    /// USB printer currently plugged in (model name), checked every few
    /// seconds — whether or not this board is the shop's printer yet.
    usb_printer: Option<String>,
    #[cfg(feature = "kds-kms")]
    saver: Option<saver::Saver>,
}

impl State {
    /// The saver program holds the screen: Slint must not present.
    fn screen_lent(&self) -> bool {
        #[cfg(feature = "kds-kms")]
        if let Some(sv) = &self.saver {
            return sv.active();
        }
        false
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// The board's part of every frame.
struct Board;

impl kds_ui::Host for Board {
    fn may_render(&self) -> bool {
        // Presenting now would fail: the saver is DRM master. The state is
        // kept; wake() renders it.
        !STATE.with(|s| s.borrow().screen_lent())
    }

    fn decorate(&self, w: &KitchenWindow, core: &kds_ui::Core) {
        STATE.with(|s| decorate(w, &s.borrow(), core.demo));
    }
}

pub fn run(cfg: Config) -> anyhow::Result<()> {
    if let Ok(dir) = std::env::var("KDS_SNAPSHOT") {
        return snapshot_pngs(std::path::Path::new(&dir));
    }
    let demo = std::env::var("KDS_DEMO").is_ok_and(|v| v == "1");
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<ClientMessage>();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.printer_path = cfg.printer_path.to_string_lossy().into_owned();
        #[cfg(feature = "kds-kms")]
        {
            s.saver = saver::Saver::from_env();
        }
    });

    // Client first, so printing never waits for the screen. Its events are
    // held here until the window exists, then handed over in order.
    let early: Arc<Mutex<Option<Vec<UiEvent>>>> = Arc::new(Mutex::new(Some(Vec::new())));
    // Set once the window exists (Slint handles are main-thread only).
    let weak: Arc<Mutex<Option<slint::Weak<KitchenWindow>>>> = Arc::new(Mutex::new(None));
    if !demo {
        let (early_ev, weak_ev) = (early.clone(), weak.clone());
        let events = move |ev: UiEvent| {
            if let Ok(mut held) = early_ev.lock() {
                if let Some(held) = held.as_mut() {
                    held.push(ev);
                    return;
                }
            }
            let Some(weak) = weak_ev.lock().ok().and_then(|w| w.clone()) else {
                return;
            };
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(w) = weak.upgrade() {
                    apply(&w, ev);
                }
            });
        };
        let link = UiLink {
            events: Box::new(events),
            commands: cmd_rx,
        };
        let weak_alert = weak.clone();
        std::thread::Builder::new()
            .name("kitchen-net".into())
            .spawn(move || {
                let result = crate::runtime().map_err(anyhow::Error::from).and_then(|rt| {
                    // `systemctl kill -s USR1 rusterando-kds` → test alert,
                    // for checking the screen + speaker remotely.
                    rt.spawn(async move {
                        use tokio::signal::unix::{signal, SignalKind};
                        let Ok(mut usr1) = signal(SignalKind::user_defined1()) else {
                            return;
                        };
                        while usr1.recv().await.is_some() {
                            let Some(weak) = weak_alert.lock().ok().and_then(|w| w.clone()) else {
                                continue;
                            };
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(w) = weak.upgrade() {
                                    test_alert(&w);
                                }
                            });
                        }
                    });
                    rt.block_on(crate::client::run(cfg, Some(link)))
                });
                // run() only returns on a fatal setup error (state dir
                // unreadable …). Exit so systemd restarts us instead of
                // showing a board that silently stopped updating.
                error!(error = ?result.err(), "kitchen client stopped — exiting");
                std::process::exit(1);
            })?;
    }

    // Weston, the boot splash or a manually started saver may hold the
    // screen; Slint's first frame would fail and systemd would restart us
    // every few seconds.
    #[cfg(feature = "kds-kms")]
    saver::wait_for_screen();
    let window = KitchenWindow::new()?;
    if let Ok(mut w) = weak.lock() {
        *w = Some(window.as_weak());
    }
    kds_ui::wire(&window, Rc::new(Board), move |msg| {
        let _ = cmd_tx.send(msg);
    });

    let weak = window.as_weak();
    window.on_util(move |which| {
        let Some(w) = weak.upgrade() else { return };
        match which {
            0 => test_print(&w),
            1 => STATE.with(|s| s.borrow_mut().saver_force = true),
            2 => test_alert(&w),
            _ => {}
        }
    });
    let weak = window.as_weak();
    window.on_saver_pointer(move |kind, x, y| {
        let double = STATE.with(|s| s.borrow_mut().taps.feed(kind == 0, x, y));
        if double {
            if let Some(w) = weak.upgrade() {
                wake(&w, "double tap");
            }
        }
    });

    // Is a USB printer plugged in? Cheap (sysfs), so poll; render on change.
    let weak = window.as_weak();
    let usb_timer = slint::Timer::default();
    let check_usb = move || {
        let found = crate::usb::detect_printer();
        let changed = STATE.with(|s| {
            let mut s = s.borrow_mut();
            let changed = s.usb_printer != found;
            s.usb_printer = found;
            changed
        });
        if changed {
            if let Some(w) = weak.upgrade() {
                kds_ui::render(&w);
            }
        }
    };
    check_usb();
    usb_timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(3), check_usb);

    // Screensaver supervision: start after the idle time, bring the board
    // back if the saver program died on its own.
    #[cfg(feature = "kds-kms")]
    let saver_timer = {
        let weak = window.as_weak();
        let t = slint::Timer::default();
        t.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || {
            if let Some(w) = weak.upgrade() {
                saver_tick(&w);
            }
        });
        t
    };

    if demo {
        info!("KDS_DEMO=1: sample orders, no server, no printer");
        kds_ui::start_demo();
        STATE.with(|s| s.borrow_mut().printer = Some(Ok("Demo".into())));
    }

    // What the client reported before the window existed, in order; then
    // switch it to the event loop.
    if let Ok(mut held) = early.lock() {
        for ev in held.take().unwrap_or_default() {
            apply(&window, ev);
        }
    }

    kds_ui::render(&window);
    window.run()?;
    drop(usb_timer);
    #[cfg(feature = "kds-kms")]
    drop(saver_timer);
    Ok(())
}

/// Once a second (board build): start the saver when idle long enough —
/// but never over orders nobody has started yet — and recover if it died.
#[cfg(feature = "kds-kms")]
fn saver_tick(w: &KitchenWindow) {
    let (last_activity, waiting_orders) = kds_ui::with(|c| {
        (
            c.last_activity,
            c.board
                .as_ref()
                .is_some_and(|b| b.orders.iter().any(|o| o.status == kitchen_protocol::BoardStatus::Received)),
        )
    });
    let begin = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(sv) = s.saver.as_mut() else {
            return false;
        };
        if sv.active() {
            if sv.exited() {
                drop(s);
                wake(w, "saver program exited on its own");
            }
            return false;
        }
        let idle = sv.idle();
        let idle_long = last_activity.is_some_and(|t| t.elapsed() >= idle);
        let forced = std::mem::take(&mut s.saver_force);
        !s.saver_shown && (forced || (idle_long && !waiting_orders))
    });
    if !begin {
        return;
    }
    // Black cover first, so the frame left on screen (and restored later)
    // is a clean one; lend the screen once that frame is out.
    STATE.with(|s| s.borrow_mut().saver_shown = true);
    w.set_saver(true);
    let weak = w.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
        let Some(w) = weak.upgrade() else { return };
        let failed = STATE.with(|s| {
            let mut s = s.borrow_mut();
            // A touch in the meantime called the whole thing off.
            if !s.saver_shown {
                return false;
            }
            match s.saver.as_mut().map(|sv| sv.start()) {
                Some(Err(e)) => {
                    error!(error = %e, "screensaver failed to start; turning it off");
                    s.saver = None;
                    true
                }
                _ => false,
            }
        });
        if failed {
            wake(&w, "saver failed to start");
        }
    });
}

/// Two short taps close together = double tap. A tap is a press released
/// within [`Taps::TAP`] without moving more than [`Taps::SLOP`]; drags and
/// long presses (the saver's own gestures) reset the sequence.
#[derive(Default)]
struct Taps {
    down: Option<(Instant, f32, f32)>,
    last_tap: Option<(Instant, f32, f32)>,
}

impl Taps {
    const TAP: std::time::Duration = std::time::Duration::from_millis(300);
    const GAP: std::time::Duration = std::time::Duration::from_millis(450);
    const SLOP: f32 = 25.0;
    const NEAR: f32 = 50.0;

    /// Feed a press (`down`) or release; `true` on the release that
    /// completes a double tap.
    fn feed(&mut self, down: bool, x: f32, y: f32) -> bool {
        self.feed_at(Instant::now(), down, x, y)
    }

    fn feed_at(&mut self, now: Instant, down: bool, x: f32, y: f32) -> bool {
        if down {
            self.down = Some((now, x, y));
            return false;
        }
        let Some((t0, x0, y0)) = self.down.take() else {
            return false;
        };
        let is_tap = now - t0 <= Self::TAP && (x - x0).hypot(y - y0) <= Self::SLOP;
        if !is_tap {
            self.last_tap = None;
            return false;
        }
        match self.last_tap.take() {
            Some((t1, x1, y1)) if now - t1 <= Self::GAP && (x - x1).hypot(y - y1) <= Self::NEAR => {
                true
            }
            _ => {
                self.last_tap = Some((now, x, y));
                false
            }
        }
    }
}

/// Back to the board: stop the saver (if running), take the screen back,
/// re-render. Safe to call when no saver is up.
fn wake(w: &KitchenWindow, reason: &str) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.saver_shown {
            info!(reason, "waking the board");
        }
        #[cfg(feature = "kds-kms")]
        if let Some(sv) = s.saver.as_mut() {
            sv.stop();
        }
        s.saver_shown = false;
        s.taps = Taps::default();
    });
    kds_ui::with(|c| {
        c.last_activity = Some(kds_ui::Instant::now());
        c.page = 1;
    });
    w.set_saver(false);
    kds_ui::render(w);
}

/// `KDS_SNAPSHOT`: draw the demo board and one detail view offscreen at the
/// board's resolution and write them as PNGs.
fn snapshot_pngs(dir: &std::path::Path) -> anyhow::Result<()> {
    use slint::platform::software_renderer::{
        MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
    };

    const W: u32 = 480;
    const H: u32 = 272;

    struct Headless(Rc<MinimalSoftwareWindow>);
    impl slint::platform::Platform for Headless {
        fn create_window_adapter(
            &self,
        ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    let surface = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    surface.set_size(slint::PhysicalSize::new(W, H));
    slint::platform::set_platform(Box::new(Headless(surface.clone())))
        .map_err(|e| anyhow::anyhow!("set headless platform: {e:?}"))?;

    let window = KitchenWindow::new()?;
    window.show()?;
    kds_ui::wire(&window, Rc::new(Board), |_| {});
    kds_ui::start_demo();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.printer_path = "usb".into();
        s.printer = Some(Ok("Demo".into()));
    });
    let first_id = kds_ui::with(|c| c.board.as_ref().and_then(|b| b.orders.first()).map(|o| o.id.clone()));

    let shoot = |name: &str| -> anyhow::Result<()> {
        let mut buf = vec![PremultipliedRgbaColor::default(); (W * H) as usize];
        surface.request_redraw();
        surface.draw_if_needed(|r| {
            r.render(&mut buf, W as usize);
        });
        let mut pm = tiny_skia::Pixmap::new(W, H).ok_or_else(|| anyhow::anyhow!("pixmap"))?;
        for (dst, src) in pm.pixels_mut().iter_mut().zip(&buf) {
            *dst = tiny_skia::PremultipliedColorU8::from_rgba(src.red, src.green, src.blue, src.alpha)
                .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
        }
        let path = dir.join(name);
        pm.save_png(&path)?;
        info!(path = %path.display(), "snapshot written");
        Ok(())
    };

    std::fs::create_dir_all(dir)?;
    kds_ui::render(&window);
    shoot("board.png")?;
    kds_ui::with(|c| c.detail = first_id);
    kds_ui::render(&window);
    shoot("detail.png")?;
    kds_ui::with(|c| {
        c.detail = None;
        c.page = 2;
    });
    kds_ui::render(&window);
    shoot("hours.png")?;
    kds_ui::with(|c| c.page = 0);
    kds_ui::render(&window);
    shoot("start.png")?;
    kds_ui::with(|c| c.page = 1);
    kds_ui::render(&window);
    window.set_flash_title("NEUE BESTELLUNG".into());
    window.set_flash_text("#0012 · LIEF · BAR".into());
    window.set_flash(true);
    shoot("flash.png")?;
    Ok(())
}

/// Apply a client notification on the UI thread.
fn apply(w: &KitchenWindow, ev: UiEvent) {
    let ev = match ev {
        UiEvent::Printer(p) => {
            STATE.with(|s| s.borrow_mut().printer = Some(p));
            kds_ui::render(w);
            return;
        }
        UiEvent::PendingPrints(n) => {
            STATE.with(|s| s.borrow_mut().pending_prints = n);
            kds_ui::render(w);
            return;
        }
        UiEvent::Connected(up) => Event::Connected(up),
        UiEvent::Board(b) => Event::Board(b),
        UiEvent::Shop(p) => Event::Shop(p),
        UiEvent::Alert { title, text } => Event::Alert { title, text },
    };
    let saver_shown = STATE.with(|s| s.borrow().saver_shown);
    let out = kds_ui::apply(w, ev, saver_shown);
    if let Some(reason) = out.wake {
        wake(w, reason);
    }
    if let Some((title, text)) = out.flash {
        start_flash(w, title, text);
    }
}

/// "Alarm testen" (Start page) or SIGUSR1: the new-order alert — flash +
/// gong — without an order, to check screen and speaker.
fn test_alert(w: &KitchenWindow) {
    start_flash(w, "TESTALARM".to_string(), "Neue Bestellung (Test)".to_string());
}

/// Ring the new-order gong: write the trigger file that `rusterando-gong.path`
/// (root, with access to PipeWire and the Bluetooth speaker) watches.
/// `KDS_GONG` overrides the path; empty disables. Best effort: no speaker,
/// no gong, the flash still shows.
fn gong() {
    let path = std::env::var("KDS_GONG").unwrap_or_else(|_| "/run/rusterando-kds/gong".into());
    if path.is_empty() {
        return;
    }
    if let Err(e) = std::fs::write(&path, format!("{}\n", kds_ui::local_now())) {
        tracing::debug!(%path, error = %e, "gong trigger not written");
    }
}

fn start_flash(w: &KitchenWindow, title: String, text: String) {
    gong();
    kds_ui::flash(w, title, text);
}

/// The board's part of a frame: printer chip, Start page buttons + info.
fn decorate(w: &KitchenWindow, s: &State, demo: bool) {
    let usb_mode = crate::usb::UsbSelector::parse(&s.printer_path).is_some();
    let (ok, text) = match &s.printer {
        // Not this board's job yet: say what's plugged in.
        Some(Ok(d)) if d == crate::printer::DISPLAY_ONLY => match &s.usb_printer {
            Some(_) => (true, "Drucker erkannt".to_string()),
            None => (true, "Kein Drucker".to_string()),
        },
        // Printing over USB: unplugged shows before the next receipt fails.
        _ if usb_mode && s.usb_printer.is_none() && !demo => (false, "Drucker aus".to_string()),
        None => (false, "Drucker …".to_string()),
        Some(Ok(_)) if s.pending_prints == 0 => (true, "Drucker".to_string()),
        Some(Ok(_)) => (true, format!("Drucker ({})", s.pending_prints)),
        Some(Err(_)) if s.pending_prints == 0 => (false, "Drucker aus".to_string()),
        Some(Err(_)) => (false, format!("Drucker aus ({})", s.pending_prints)),
    };
    w.set_printer_ok(ok);
    w.set_printer_text(text.into());

    let display_only = s.printer_path == crate::printer::DISPLAY_ONLY;
    let (can, label) = match (&s.usb_printer, test_print_path(s).is_some()) {
        (Some(model), _) => (true, format!("Testdruck ({model})")),
        // A file / lp device path (e.g. a Pi-style /dev/usb/lp0).
        (None, true) if !display_only && crate::usb::UsbSelector::parse(&s.printer_path).is_none() => {
            (true, "Testdruck".to_string())
        }
        _ => (false, "Testdruck: kein Drucker angeschlossen".to_string()),
    };
    w.set_can_test_print(can);
    w.set_test_print_text(label.into());
    #[cfg(feature = "kds-kms")]
    w.set_has_saver(s.saver.is_some());
    let printer = match (&s.usb_printer, display_only) {
        (Some(model), true) => format!("{model} (USB) – nicht aktiviert"),
        (None, true) => "keiner (nur Anzeige)".to_string(),
        (Some(model), false) => format!("{model} (USB)"),
        (None, false) => s.printer_path.clone(),
    };
    let mut info = format!("v{} · Drucker: {printer}", env!("CARGO_PKG_VERSION"));
    if let Some(ip) = lan_ip() {
        info.push_str(&format!(" · LAN {ip}"));
    }
    w.set_info_text(info.into());
}

/// This machine's address on the default route (no packet is sent: a UDP
/// "connect" only picks the outgoing interface).
fn lan_ip() -> Option<std::net::IpAddr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    Some(sock.local_addr().ok()?.ip())
}

/// Where a test receipt goes: the configured printer, or — display-only —
/// a detected USB printer (a test receipt never touches the order channel,
/// so this is safe before the board takes over printing).
fn test_print_path(s: &State) -> Option<String> {
    if s.printer_path == crate::printer::DISPLAY_ONLY {
        return s.usb_printer.as_ref().map(|_| "usb".to_string());
    }
    (!s.printer_path.is_empty()).then(|| s.printer_path.clone())
}

/// Start page "Testdruck": one sample receipt on the configured printer,
/// in the background (a USB print can take seconds), result as a message.
fn test_print(w: &KitchenWindow) {
    let Some(path) = STATE.with(|s| test_print_path(&s.borrow())) else {
        return;
    };
    let weak = w.as_weak();
    std::thread::spawn(move || {
        let (msg, ok) = match crate::test_print::run(&path) {
            Ok(()) => ("Testdruck gesendet".to_string(), true),
            Err(e) => (format!("Testdruck fehlgeschlagen: {e:#}"), false),
        };
        let _ = slint::invoke_from_event_loop(move || {
            kds_ui::with(|c| c.set_notice(msg, ok));
            if let Some(w) = weak.upgrade() {
                kds_ui::render(&w);
                let weak = w.as_weak();
                slint::Timer::single_shot(NOTICE_FOR, move || {
                    if let Some(w) = weak.upgrade() {
                        kds_ui::render(&w);
                    }
                });
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::Taps;
    use std::time::{Duration, Instant};

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    #[test]
    fn double_tap_wakes() {
        let t0 = Instant::now();
        let mut t = Taps::default();
        assert!(!t.feed_at(ms(t0, 0), true, 100.0, 100.0));
        assert!(!t.feed_at(ms(t0, 80), false, 102.0, 101.0));
        assert!(!t.feed_at(ms(t0, 250), true, 110.0, 105.0));
        assert!(t.feed_at(ms(t0, 320), false, 110.0, 105.0));
    }

    #[test]
    fn single_taps_and_drags_do_not() {
        let t0 = Instant::now();
        let mut t = Taps::default();
        // two taps too far apart in time
        t.feed_at(ms(t0, 0), true, 100.0, 100.0);
        t.feed_at(ms(t0, 80), false, 100.0, 100.0);
        t.feed_at(ms(t0, 900), true, 100.0, 100.0);
        assert!(!t.feed_at(ms(t0, 980), false, 100.0, 100.0));
        // a drag (the saver's own gesture) breaks the sequence
        let mut t = Taps::default();
        t.feed_at(ms(t0, 0), true, 100.0, 100.0);
        t.feed_at(ms(t0, 80), false, 100.0, 100.0);
        t.feed_at(ms(t0, 150), true, 100.0, 100.0);
        assert!(!t.feed_at(ms(t0, 260), false, 180.0, 100.0));
        // two quick taps on opposite sides of the screen
        let mut t = Taps::default();
        t.feed_at(ms(t0, 0), true, 50.0, 100.0);
        t.feed_at(ms(t0, 80), false, 50.0, 100.0);
        t.feed_at(ms(t0, 200), true, 400.0, 100.0);
        assert!(!t.feed_at(ms(t0, 280), false, 400.0, 100.0));
        // a long press is not a tap
        let mut t = Taps::default();
        t.feed_at(ms(t0, 0), true, 100.0, 100.0);
        t.feed_at(ms(t0, 80), false, 100.0, 100.0);
        t.feed_at(ms(t0, 150), true, 100.0, 100.0);
        assert!(!t.feed_at(ms(t0, 900), false, 100.0, 100.0));
    }
}
