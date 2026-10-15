//! The kitchen display in the browser: the board's screen (`kds-ui`, same
//! Slint file) drawn into `<canvas id="canvas">` on /kitchen/display.
//!
//! Data: `GET /api/kds/feed` (SSE, staff cookie) carries the same board,
//! shop and alert events the board gets over the kitchen channel —
//! postcard-encoded `KitchenEvent`s, base64 per message. Taps go back as
//! `POST /api/kds/action` (a postcard `ClientMessage`); the server applies
//! them with the board's rules and the next feed event shows the result,
//! here and on the board. Printing and the screensaver stay on the board.
//!
//! `?demo` runs on the built-in sample orders, no server.

use std::cell::RefCell;
use std::rc::Rc;

use base64::Engine;
use kds_ui::{Event, KitchenWindow};
use kitchen_protocol::{ClientMessage, KitchenEvent};
use slint::ComponentHandle;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// Reconnect delay after the feed closed for good (403, server restart).
const RETRY_MS: i32 = 5_000;
/// The board's screen, scaled up to fill the browser window.
const W: f32 = 480.0;
const H: f32 = 272.0;

thread_local! {
    static WINDOW: RefCell<Option<KitchenWindow>> = const { RefCell::new(None) };
    static FEED: RefCell<Option<web_sys::EventSource>> = const { RefCell::new(None) };
}

struct Browser;

impl kds_ui::Host for Browser {
    fn decorate(&self, w: &KitchenWindow, core: &kds_ui::Core) {
        // No printer and no screensaver here.
        w.set_printer_ok(true);
        w.set_printer_text(if core.demo { "Demo" } else { "Browser" }.into());
        w.set_can_test_print(false);
        w.set_test_print_text("Testdruck: nur am Küchen-Display".into());
        w.set_has_saver(false);
        w.set_info_text("Browser-Ansicht des Küchen-Displays · Druck und Gong am Gerät".into());
    }
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let window = KitchenWindow::new().map_err(|e| JsValue::from_str(&e.to_string()))?;
    kds_ui::wire(&window, Rc::new(Browser), post_action);
    window.on_util(|which| {
        // 2 = "Alarm testen": flash (and gong, if the browser lets us).
        if which == 2 {
            with_window(|w| alert(w, "TESTALARM".into(), "Neue Bestellung (Test)".into()));
        }
    });

    let demo = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|q| q.contains("demo"));
    WINDOW.with(|w| *w.borrow_mut() = Some(window.clone_strong()));
    if demo {
        kds_ui::start_demo();
    } else {
        connect();
    }
    kds_ui::render(&window);
    window.show().map_err(|e| JsValue::from_str(&e.to_string()))?;
    // The browser window exists only once the event loop runs.
    let first_fit = Closure::once_into_js(fit);
    if let Some(win) = web_sys::window() {
        win.set_timeout_with_callback_and_timeout_and_arguments_0(first_fit.unchecked_ref(), 50)?;
    }
    let on_resize = Closure::<dyn FnMut()>::new(fit);
    if let Some(win) = web_sys::window() {
        win.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref())?;
    }
    on_resize.forget();
    // On the web this hands control to the browser's event loop.
    slint::run_event_loop().map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(())
}

fn with_window(f: impl FnOnce(&KitchenWindow)) {
    WINDOW.with(|w| {
        if let Some(w) = w.borrow().as_ref() {
            f(w)
        }
    });
}

/// Scale the 480×272 screen to the browser window, crisp: a bigger scale
/// factor, not a stretched bitmap.
fn fit() {
    let Some(win) = web_sys::window() else { return };
    let vw = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(W as f64) as f32;
    let vh = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(H as f64) as f32;
    let k = (vw / W).min(vh / H).max(0.5);
    let dpr = win.device_pixel_ratio() as f32;
    with_window(|w| {
        w.window()
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: k * dpr });
        // Physical: winit would convert a logical size with its own factor.
        w.window().set_size(slint::PhysicalSize::new(
            (W * k * dpr).round() as u32,
            (H * k * dpr).round() as u32,
        ));
    });
}

/// Open the live feed; reopen it after it closed for good.
fn connect() {
    let es = match web_sys::EventSource::new("/api/kds/feed") {
        Ok(es) => es,
        Err(_) => return retry(),
    };
    let on_open = Closure::<dyn FnMut()>::new(|| deliver(Event::Connected(true)));
    es.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();
    let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(|m: web_sys::MessageEvent| {
        let Some(text) = m.data().as_string() else { return };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(text.trim()) else {
            return;
        };
        match postcard::from_bytes::<KitchenEvent>(&bytes) {
            Ok(KitchenEvent::Board(b)) => deliver(Event::Board(b)),
            Ok(KitchenEvent::Shop(p)) => deliver(Event::Shop(p)),
            Ok(KitchenEvent::Alert { title, text }) => deliver(Event::Alert { title, text }),
            _ => {}
        }
    });
    es.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();
    let on_error = Closure::<dyn FnMut()>::new(|| {
        deliver(Event::Connected(false));
        // CONNECTING = the browser retries by itself; CLOSED = it gave up.
        let closed = FEED.with(|f| f.borrow().as_ref().is_some_and(|es| es.ready_state() == 2));
        if closed {
            retry();
        }
    });
    es.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();
    FEED.with(|f| {
        if let Some(old) = f.borrow_mut().replace(es) {
            old.close();
        }
    });
}

fn retry() {
    let cb = Closure::once_into_js(connect);
    if let Some(win) = web_sys::window() {
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), RETRY_MS);
    }
}

fn deliver(ev: Event) {
    with_window(|w| {
        let out = kds_ui::apply(w, ev, false);
        if let Some((title, text)) = out.flash {
            alert(w, title, text);
        }
    });
}

/// Flash + the board's gong (browsers only play sound after a first click
/// on the page; without that it is just the flash).
fn alert(w: &KitchenWindow, title: String, text: String) {
    kds_ui::flash(w, title, text);
    if let Ok(audio) = web_sys::HtmlAudioElement::new_with_src("/pkg/kds/gong.wav") {
        let _ = audio.play();
    }
}

/// A tap → the server. A refusal comes back as text and shows like the
/// board's notices; the result itself arrives through the feed.
fn post_action(msg: ClientMessage) {
    wasm_bindgen_futures::spawn_local(async move {
        let notice = match send(&msg).await {
            Ok(None) => return,
            Ok(Some(reason)) => reason,
            Err(_) => "Keine Verbindung zum Server".to_string(),
        };
        kds_ui::with(|c| c.set_notice(notice, false));
        with_window(kds_ui::render);
    });
}

async fn send(msg: &ClientMessage) -> Result<Option<String>, JsValue> {
    let body = postcard::to_allocvec(msg).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    let bytes = js_sys::Uint8Array::from(body.as_slice());
    init.set_body(&bytes);
    let req = web_sys::Request::new_with_str_and_init("/api/kds/action", &init)?;
    req.headers().set("content-type", "application/octet-stream")?;
    let win = web_sys::window().ok_or("no window")?;
    let resp: web_sys::Response =
        wasm_bindgen_futures::JsFuture::from(win.fetch_with_request(&req)).await?.dyn_into()?;
    if resp.ok() {
        return Ok(None);
    }
    let text = wasm_bindgen_futures::JsFuture::from(resp.text()?).await?.as_string().unwrap_or_default();
    Ok(Some(if text.is_empty() { format!("Server: Fehler {}", resp.status()) } else { text }))
}
