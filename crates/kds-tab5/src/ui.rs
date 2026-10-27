//! The kitchen screen (`kds-ui`, the same Slint file as the STM32 board and
//! the browser) on the Tab5: a software-rendered window, turned to landscape
//! on the portrait panel, fed by the touch controller.
//!
//! The screen is designed at 480×272 and compiled with a fixed scale factor
//! (KDS_UI_SCALE = 2.647 → 1271×720); the panel is 720×1280 portrait. Slint
//! renders rotated by 90° straight into a portrait frame, which the display
//! copies to the panel; touch points come in portrait coordinates and are
//! turned back here.

use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use esp_idf_svc::sys::{self, bsp, esp};
use kds_ui::KitchenWindow;
use kitchen_protocol::ClientMessage;
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, RenderingRotation, RepaintBufferType, Rgb565Pixel,
};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize};

use crate::battery::{BatteryStatus, Care, SharedBattery};
use crate::display::{self, Display};
use crate::gong::Gong;
use crate::imu::Orientation;
use crate::printing::{self, PrintStatus, SharedStatus};
use crate::usb_printer::UsbPrinter;

/// From the network threads to the screen.
pub enum UiMsg {
    /// Feed up/down, board, shop panel, admin alert.
    Event(kds_ui::Event),
    /// A refused tap or a login problem, shown like the board's notices.
    Notice(String),
    /// A success note (e.g. "Testdruck gesendet").
    Info(String),
}

/// Physical UI size (landscape).
const UI_W: u32 = display::HEIGHT as u32;
const UI_H: u32 = display::WIDTH as u32;
/// Poll interval for touch and timers.
const TICK: Duration = Duration::from_millis(20);
/// Dim the backlight after this long without a touch, a new order or a
/// message; any of them brings it back (a waking touch presses nothing).
const DIM_AFTER: Duration = Duration::from_secs(10 * 60);

/// Something happened: full brightness again, restart the dim timer.
fn wake(dimmed: &mut bool, last_activity: &mut Instant) {
    *last_activity = Instant::now();
    if std::mem::take(dimmed) {
        log::info!("ui: awake, backlight {} %", display::BRIGHTNESS);
        display::set_brightness(display::BRIGHTNESS);
    }
}

struct Tab5Platform {
    window: Rc<MinimalSoftwareWindow>,
    start: Instant,
}

impl Platform for Tab5Platform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}

/// What the screen shows about this device. `status` is written by the
/// network thread.
struct Tab5Host {
    status: Arc<Mutex<String>>,
    /// Why the previous run ended, if it crashed (crashlog).
    crash: Option<String>,
    /// The USB printer and the print channel.
    print: SharedStatus,
    battery: SharedBattery,
    care: Care,
}

impl kds_ui::Host for Tab5Host {
    fn decorate(&self, w: &KitchenWindow, core: &kds_ui::Core) {
        let print = self.print.lock().map(|p| p.clone()).unwrap_or_default();
        let (ok, text) = printer_chip(&print, core.demo);
        w.set_printer_ok(ok);
        w.set_printer_text(text.into());
        let (can, label) = match &print.printer {
            Some(name) => (true, format!("Testdruck ({name})")),
            None => (false, "Testdruck: kein Drucker angeschlossen".to_string()),
        };
        w.set_can_test_print(can);
        w.set_test_print_text(label.into());
        // No screensaver on the Tab5.
        w.set_has_saver(false);
        let mut status = self.status.lock().map(|s| s.clone()).unwrap_or_default();
        let battery = self.battery.lock().ok().and_then(|b| *b).and_then(|b| b.describe());
        // The battery care switch, only with a pack in.
        let care = match (&battery, self.care.enabled()) {
            (None, _) => "",
            (Some(_), true) => "Akku schonen: an",
            (Some(_), false) => "Akku schonen: aus",
        };
        w.set_extra_text(care.into());
        if let Some(battery) = battery {
            status = format!("{battery} · {status}");
        }
        let info = match &self.crash {
            Some(crash) => format!("{crash} · {status}"),
            None => format!("{} · {status}", crate::board::device_name()),
        };
        w.set_info_text(info.into());
    }
}

/// The header chip: no printer is a normal state (display only); a plugged-in
/// printer that can't get or print receipts is a problem.
fn printer_chip(p: &PrintStatus, demo: bool) -> (bool, String) {
    match &p.printer {
        _ if demo => (true, "Demo".into()),
        None => (true, "Ohne Drucker".into()),
        Some(_) if !p.online => (false, "Drucker offline".into()),
        Some(_) if p.problem.is_some() => (false, "Druckfehler".into()),
        Some(_) => (true, "Drucker".into()),
    }
}

/// The touch controller (GT911 or ST712x, chosen by the board package).
struct Touch(bsp::esp_lcd_touch_handle_t);

impl Touch {
    fn new() -> Result<Self> {
        let mut handle = core::ptr::null_mut();
        // SAFETY: null config is allowed ("prepared for future use").
        esp!(unsafe { bsp::bsp_touch_new(core::ptr::null(), &mut handle) })?;
        Ok(Self(handle))
    }

    /// The first touch point in panel (portrait) coordinates, if touched.
    fn read(&self) -> Option<(u16, u16)> {
        // SAFETY: live handle; the point buffer holds max_point_cnt entries.
        unsafe {
            if bsp::esp_lcd_touch_read_data(self.0) != sys::ESP_OK {
                return None;
            }
            let mut point: bsp::esp_lcd_touch_point_data_t = core::mem::zeroed();
            let mut count: u8 = 0;
            if bsp::esp_lcd_touch_get_data(self.0, &mut point, &mut count, 1) != sys::ESP_OK || count == 0 {
                return None;
            }
            Some((point.x, point.y))
        }
    }
}

/// Panel (portrait) pixels → UI logical coordinates, matching the rendering
/// rotation (90°, or 270° when the Tab5 stands upside down) and the window's
/// scale factor.
fn to_ui(px: u16, py: u16, scale: f32, flipped: bool) -> LogicalPosition {
    let (px, py) = (px as f32, py as f32);
    let (x, y) = if flipped {
        ((display::HEIGHT as f32 - 1.0) - py, px)
    } else {
        (py, (display::WIDTH as f32 - 1.0) - px)
    };
    LogicalPosition::new(x / scale, y / scale)
}

fn rotation(flipped: bool) -> RenderingRotation {
    if flipped {
        RenderingRotation::Rotate270
    } else {
        RenderingRotation::Rotate90
    }
}

/// Run the kitchen screen forever: live orders from `events`, taps out
/// through `taps`; demo orders when `demo` (no Küche password compiled in).
#[allow(clippy::too_many_arguments)]
pub fn run(
    display: Display,
    status: Arc<Mutex<String>>,
    print: SharedStatus,
    printer: Option<UsbPrinter>,
    battery: SharedBattery,
    care: Care,
    events: Receiver<UiMsg>,
    notices: Sender<UiMsg>,
    taps: Sender<ClientMessage>,
    demo: bool,
    crash: Option<String>,
    gong: Gong,
    orientation: Orientation,
) -> Result<()> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    window.set_size(PhysicalSize::new(UI_W, UI_H));
    slint::platform::set_platform(Box::new(Tab5Platform {
        window: window.clone(),
        start: Instant::now(),
    }))
    .map_err(|e| anyhow!("slint platform: {e:?}"))?;

    let kitchen = KitchenWindow::new().map_err(|e| anyhow!("slint: {e:?}"))?;
    let host = Tab5Host {
        status: status.clone(),
        crash,
        print: print.clone(),
        battery: battery.clone(),
        care: care.clone(),
    };
    kds_ui::wire(&kitchen, Rc::new(host), move |msg| {
        if taps.send(msg).is_err() {
            log::warn!("kitchen: tap dropped, no sender thread");
        }
    });
    let weak = kitchen.as_weak();
    let test_gong = gong.clone();
    kitchen.on_util(move |which| match which {
        // 0 = "Testdruck": a sample receipt on the USB printer, in the
        // background (a print takes seconds); the result as a notice.
        0 => {
            let Some(printer) = printer.clone() else { return };
            let notices = notices.clone();
            let spawned = std::thread::Builder::new()
                .name("test-print".into())
                .stack_size(16 * 1024)
                .spawn(move || {
                    let msg = match printing::test_print(&printer) {
                        Ok(()) => UiMsg::Info("Testdruck gesendet".into()),
                        Err(e) => UiMsg::Notice(format!("Testdruck fehlgeschlagen: {e:#}")),
                    };
                    let _ = notices.send(msg);
                });
            if let Err(e) = spawned {
                log::warn!("test print: {e}");
            }
        }
        // 2 = "Alarm testen": the new-order flash and gong.
        2 => {
            if let Some(w) = weak.upgrade() {
                kds_ui::flash(&w, "TESTALARM".into(), "Neue Bestellung (Test)".into());
                test_gong.ring();
            }
        }
        // 3 = "Akku schonen": switch the battery care over.
        3 => {
            let text = if care.toggle() {
                "Akku schonen an: lädt bis etwa 85–90 %, langsam"
            } else {
                "Akku schonen aus: lädt bis 100 %, schnell"
            };
            let _ = notices.send(UiMsg::Info(text.into()));
        }
        _ => {}
    });
    if demo {
        kds_ui::start_demo();
    }
    kds_ui::render(&kitchen);
    kitchen.show().map_err(|e| anyhow!("slint: {e:?}"))?;
    let scale = window.scale_factor();
    log::info!("ui: kitchen screen up, scale factor {scale}");

    let touch = match Touch::new() {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("touch: {e}");
            None
        }
    };

    let mut frame = vec![Rgb565Pixel(0); display::WIDTH * display::HEIGHT];
    let mut pressed: Option<LogicalPosition> = None;
    let mut shown_status = String::new();
    let mut shown_print = PrintStatus::default();
    let mut shown_battery: Option<BatteryStatus> = None;
    let mut flipped = orientation.flipped();
    let mut turned = false;
    let mut last_activity = Instant::now();
    let mut dimmed = false;
    // A touch that woke the screen, until it's lifted.
    let mut waking_touch = false;

    loop {
        slint::platform::update_timers_and_animations();

        while let Ok(msg) = events.try_recv() {
            match msg {
                UiMsg::Event(ev) => {
                    let out = kds_ui::apply(&kitchen, ev, false);
                    if let Some((title, text)) = out.flash {
                        wake(&mut dimmed, &mut last_activity);
                        kds_ui::flash(&kitchen, title, text);
                        gong.ring();
                    }
                }
                UiMsg::Notice(text) => {
                    wake(&mut dimmed, &mut last_activity);
                    kds_ui::with(|c| c.set_notice(text, false));
                    kds_ui::render(&kitchen);
                }
                UiMsg::Info(text) => {
                    wake(&mut dimmed, &mut last_activity);
                    kds_ui::with(|c| c.set_notice(text, true));
                    kds_ui::render(&kitchen);
                }
            }
        }

        // The info line carries the network and battery state, the header
        // chip the printer's: redraw when one changes.
        if let Ok(s) = status.lock() {
            if *s != shown_status {
                shown_status = s.clone();
                drop(s);
                kds_ui::render(&kitchen);
            }
        }
        if let Ok(p) = print.lock() {
            if *p != shown_print {
                shown_print = p.clone();
                drop(p);
                kds_ui::render(&kitchen);
            }
        }
        if let Ok(b) = battery.lock() {
            if *b != shown_battery {
                shown_battery = *b;
                drop(b);
                kds_ui::render(&kitchen);
            }
        }

        // The Tab5 was turned over: render the other way round.
        if orientation.flipped() != flipped {
            flipped = !flipped;
            turned = true;
            if let Some(position) = pressed.take() {
                window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
                window.dispatch_event(WindowEvent::PointerExited);
            }
            window.request_redraw();
        }

        match touch.as_ref().and_then(Touch::read) {
            Some((px, py)) => {
                if dimmed {
                    waking_touch = true;
                }
                wake(&mut dimmed, &mut last_activity);
                if !waking_touch {
                    let position = to_ui(px, py, scale, flipped);
                    let event = if pressed.is_none() {
                        WindowEvent::PointerPressed { position, button: PointerEventButton::Left }
                    } else {
                        WindowEvent::PointerMoved { position }
                    };
                    window.dispatch_event(event);
                    pressed = Some(position);
                }
            }
            None => {
                waking_touch = false;
                if let Some(position) = pressed.take() {
                    window.dispatch_event(WindowEvent::PointerReleased {
                        position,
                        button: PointerEventButton::Left,
                    });
                    window.dispatch_event(WindowEvent::PointerExited);
                }
            }
        }

        if !dimmed && last_activity.elapsed() >= DIM_AFTER {
            log::info!("ui: nothing happened for {} min, backlight {} %", DIM_AFTER.as_secs() / 60, display::DIMMED);
            display::set_brightness(display::DIMMED);
            dimmed = true;
        }

        window.draw_if_needed(|renderer| {
            renderer.set_rendering_rotation(rotation(flipped));
            // After a turn every pixel moves: one frame without the partial
            // rendering cache (changing the buffer type clears it).
            renderer.set_repaint_buffer_type(if std::mem::take(&mut turned) {
                RepaintBufferType::NewBuffer
            } else {
                RepaintBufferType::ReusedBuffer
            });
            renderer.render(&mut frame, display::WIDTH);
            if let Err(e) = display.draw(as_u16(&frame)) {
                log::warn!("display: {e:#}");
            }
        });

        std::thread::sleep(TICK);
    }
}

/// `Rgb565Pixel` is a transparent wrapper around the u16 the panel takes.
fn as_u16(frame: &[Rgb565Pixel]) -> &[u16] {
    const _: () = assert!(core::mem::size_of::<Rgb565Pixel>() == 2);
    // SAFETY: same size and alignment, plain integer data.
    unsafe { core::slice::from_raw_parts(frame.as_ptr().cast(), frame.len()) }
}
