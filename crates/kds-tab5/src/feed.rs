//! The live kitchen data, the same way the browser mirror (/kitchen/display,
//! crates/kds-web) gets it:
//!
//!   * log in as Küche: `POST /api/kitchen_login` (`password=…`), keep the
//!     session cookies (`dp_session`, and `dp_auth` where the server signs
//!     sessions),
//!   * `GET /api/kds/feed`: server-sent events, each `data:` a base64
//!     postcard `KitchenEvent` (board, shop panel, admin alert),
//!   * taps: `POST /api/kds/action` with a postcard `ClientMessage`
//!     (204 = done, the feed shows the result; 422 + text = refused).
//!
//! Password and shop URL are compiled in from tmp/tab5-wifi.env
//! (`KITCHEN_PASSWORD`, `SHOP_URL`).

use std::ffi::{CStr, CString};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::http::Method;
use embedded_svc::io::Write;
use esp_idf_svc::http::client::{Configuration as HttpConfig, EspHttpConnection};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sys;
use kitchen_protocol::{ClientMessage, KitchenEvent};

use crate::ui::UiMsg;

pub const PASSWORD: Option<&str> = option_env!("KITCHEN_PASSWORD");
pub const SHOP_URL: &str = match option_env!("SHOP_URL") {
    Some(url) => url,
    None => "https://davidspizzeria.de",
};
/// Reconnect delay after the feed dropped.
const RETRY: Duration = Duration::from_secs(5);
/// After a refused login: don't hammer the server.
const RETRY_LOGIN: Duration = Duration::from_secs(60);

/// The Küche session (the `Cookie` header value), shared by the feed, the
/// action sender and the print channel.
pub type Session = Arc<Mutex<Option<String>>>;

/// The session cookie is kept across restarts (valid for a year), so a
/// restart goes straight to the feed instead of logging in again.
pub const NVS_NAMESPACE: &str = "kds";
const KEY_COOKIE: &str = "kitchen_cookie";

/// Start the feed and action threads; returns the session they keep. `status`
/// is the Start page's info line; the feed adds how long the device took to
/// get online.
pub fn start(
    ui: Sender<UiMsg>,
    actions: Receiver<ClientMessage>,
    status: Arc<Mutex<String>>,
    nvs: EspDefaultNvsPartition,
) -> Result<Session> {
    let store = EspNvs::new(nvs, NVS_NAMESPACE, true).map_err(|e| log::warn!("feed: NVS: {e}")).ok();
    let mut buf = [0u8; 512];
    let saved = store
        .as_ref()
        .and_then(|s| s.get_str(KEY_COOKIE, &mut buf).ok().flatten())
        .map(str::to_string);
    let session: Session = Arc::new(Mutex::new(saved));
    let store = Arc::new(Mutex::new(store));
    let feed_session = session.clone();
    let action_session = session.clone();
    let feed_ui = ui.clone();
    std::thread::Builder::new()
        .name("feed".into())
        .stack_size(16 * 1024)
        .spawn(move || feed_loop(feed_ui, feed_session, status, store))?;
    std::thread::Builder::new()
        .name("actions".into())
        .stack_size(16 * 1024)
        .spawn(move || action_loop(actions, ui, action_session))?;
    Ok(session)
}

fn client(timeout: Duration) -> Result<HttpClient<EspHttpConnection>> {
    Ok(HttpClient::wrap(EspHttpConnection::new(&HttpConfig {
        crt_bundle_attach: Some(sys::esp_crt_bundle_attach),
        timeout: Some(timeout),
        buffer_size: Some(4096),
        buffer_size_tx: Some(2048),
        ..Default::default()
    })?))
}

/// Log in as Küche; returns every cookie the server set, as one `Cookie`
/// header value (`dp_session=kitchen; dp_auth=…`: the role and, on servers
/// that sign sessions, its signature).
///
/// ESP-IDF's HTTP client directly: the esp-idf-svc wrapper keeps only the
/// last header of a name, and the login sets two cookies.
fn login() -> Result<String> {
    let password = PASSWORD.ok_or_else(|| anyhow!("KITCHEN_PASSWORD not compiled in"))?;
    let body = format!("password={}", urlencode(password));
    let url = CString::new(format!("{SHOP_URL}/api/kitchen_login"))?;
    let mut answer = LoginAnswer::default();

    // SAFETY: a zeroed config is the client's "all defaults"; url, body and
    // `answer` outlive the client, which is cleaned up before returning.
    let status = unsafe {
        let mut cfg: sys::esp_http_client_config_t = core::mem::zeroed();
        cfg.url = url.as_ptr();
        cfg.method = sys::esp_http_client_method_t_HTTP_METHOD_POST;
        cfg.timeout_ms = 15_000;
        cfg.crt_bundle_attach = Some(sys::esp_crt_bundle_attach);
        cfg.event_handler = Some(on_login_event);
        cfg.user_data = (&mut answer as *mut LoginAnswer).cast();
        let client = sys::esp_http_client_init(&cfg);
        if client.is_null() {
            bail!("Küche-Login: HTTP client init failed");
        }
        sys::esp_http_client_set_header(
            client,
            c"content-type".as_ptr(),
            c"application/x-www-form-urlencoded".as_ptr(),
        );
        sys::esp_http_client_set_post_field(client, body.as_ptr().cast(), body.len() as i32);
        let result = sys::esp_http_client_perform(client);
        let status = sys::esp_http_client_get_status_code(client);
        sys::esp_http_client_cleanup(client);
        sys::esp!(result).map_err(|e| anyhow!("Küche-Login: {e}"))?;
        status
    };
    let text = String::from_utf8_lossy(&answer.body);
    if status != 200 || text.trim() == "false" {
        bail!("Küche-Login abgelehnt (HTTP {status})");
    }
    if !answer.cookies.iter().any(|c| c.starts_with("dp_session=")) {
        bail!("Küche-Login: kein Session-Cookie");
    }
    Ok(answer.cookies.join("; "))
}

#[derive(Default)]
struct LoginAnswer {
    /// `name=value` of every Set-Cookie header.
    cookies: Vec<String>,
    body: Vec<u8>,
}

/// The login's HTTP events: one per response header, then the body.
unsafe extern "C" fn on_login_event(event: *mut sys::esp_http_client_event_t) -> sys::esp_err_t {
    let event = &*event;
    let answer = &mut *(event.user_data as *mut LoginAnswer);
    #[allow(non_upper_case_globals)]
    match event.event_id {
        sys::esp_http_client_event_id_t_HTTP_EVENT_ON_HEADER => {
            let key = CStr::from_ptr(event.header_key).to_string_lossy();
            if key.eq_ignore_ascii_case("set-cookie") {
                let value = CStr::from_ptr(event.header_value).to_string_lossy();
                if let Some(pair) = value.split(';').next() {
                    let pair = pair.trim();
                    // A deletion (empty value) is no session.
                    if pair.split_once('=').is_some_and(|(_, v)| !v.is_empty()) {
                        answer.cookies.push(pair.to_string());
                    }
                }
            }
        }
        sys::esp_http_client_event_id_t_HTTP_EVENT_ON_DATA => {
            if !event.data.is_null() && event.data_len > 0 {
                answer
                    .body
                    .extend_from_slice(core::slice::from_raw_parts(event.data.cast::<u8>(), event.data_len as usize));
            }
        }
        _ => {}
    }
    sys::ESP_OK
}

fn feed_loop(
    ui: Sender<UiMsg>,
    session: Session,
    status: Arc<Mutex<String>>,
    store: Arc<Mutex<Option<EspNvs<NvsDefault>>>>,
) {
    let keep = |cookie: Option<&str>| {
        if let Ok(guard) = store.lock() {
            if let Some(s) = guard.as_ref() {
                let _ = match cookie {
                    Some(c) => s.set_str(KEY_COOKIE, c).map(|_| ()),
                    None => s.remove(KEY_COOKIE).map(|_| ()),
                };
            }
        }
    };
    let mut first_connect = true;
    loop {
        // One connection per round. The server sends a keep-alive every 5 s:
        // a read timeout well above that means the connection is dead.
        let mut client = match client(Duration::from_secs(20)) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("feed: {e:#}");
                std::thread::sleep(RETRY);
                continue;
            }
        };
        let cookie = match session.lock().ok().and_then(|s| s.clone()) {
            Some(c) => c,
            None => match login() {
                Ok(c) => {
                    log::info!("feed: logged in as Küche");
                    if let Ok(mut s) = session.lock() {
                        *s = Some(c.clone());
                    }
                    keep(Some(&c));
                    c
                }
                Err(e) => {
                    log::warn!("feed: {e:#}");
                    let _ = ui.send(UiMsg::Notice(format!("{e:#}")));
                    std::thread::sleep(RETRY_LOGIN);
                    continue;
                }
            },
        };
        let on_connect = || {
            if std::mem::take(&mut first_connect) {
                // SAFETY: plain query (µs since boot).
                let secs = unsafe { sys::esp_timer_get_time() } as f32 / 1_000_000.0;
                log::info!("feed: online {secs:.1} s after boot");
                if let Ok(mut s) = status.lock() {
                    *s = format!("{s} · online nach {secs:.1} s");
                }
            }
        };
        match read_feed(&mut client, &cookie, &ui, on_connect) {
            Ok(()) => log::info!("feed: stream ended"),
            Err(e) => {
                log::warn!("feed: {e:#}");
                if format!("{e:#}").contains("HTTP 403") {
                    // Session gone (logged out, server key rotated): log in again.
                    if let Ok(mut s) = session.lock() {
                        *s = None;
                    }
                    keep(None);
                }
            }
        }
        let _ = ui.send(UiMsg::Event(kds_ui::Event::Connected(false)));
        std::thread::sleep(RETRY);
    }
}

/// Read the event stream until it ends; forward every event to the UI.
fn read_feed(
    client: &mut HttpClient<EspHttpConnection>,
    cookie: &str,
    ui: &Sender<UiMsg>,
    on_connect: impl FnOnce(),
) -> Result<()> {
    let headers = [("cookie", cookie), ("accept", "text/event-stream")];
    let url = format!("{SHOP_URL}/api/kds/feed");
    let request = client.request(Method::Get, &url, &headers)?;
    let mut response = request.submit()?;
    let status = response.status();
    if status != 200 {
        bail!("feed: HTTP {status}");
    }
    log::info!("feed: connected");
    on_connect();
    ui.send(UiMsg::Event(kds_ui::Event::Connected(true)))?;

    let mut pending: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        // One byte per read: esp_http_client_read returns only once the
        // buffer is full (or on timeout), and an event stream never fills
        // one — the board would sit in a 2 KB buffer until enough keep-alives
        // had arrived. No byte for the whole timeout = dead connection (Err).
        let n = response.read(&mut byte).map_err(|e| anyhow!("read: {e:?}"))?;
        if n == 0 {
            return Ok(());
        }
        pending.push(byte[0]);
        // One event = lines up to a blank line; only `data:` lines matter.
        if !pending.ends_with(b"\n\n") {
            continue;
        }
        let block = String::from_utf8_lossy(&pending).into_owned();
        pending.clear();
        let data: String = block
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(str::trim)
            .collect();
        if data.is_empty() {
            continue; // keep-alive comment
        }
        if let Some(ev) = decode(&data) {
            ui.send(UiMsg::Event(ev))?;
        }
    }
}

fn decode(data: &str) -> Option<kds_ui::Event> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).ok()?;
    match postcard::from_bytes::<KitchenEvent>(&bytes) {
        Ok(KitchenEvent::Board(b)) => Some(kds_ui::Event::Board(b)),
        Ok(KitchenEvent::Shop(p)) => Some(kds_ui::Event::Shop(p)),
        Ok(KitchenEvent::Alert { title, text }) => Some(kds_ui::Event::Alert { title, text }),
        Ok(_) => None,
        Err(e) => {
            log::warn!("feed: undecodable event: {e}");
            None
        }
    }
}

fn action_loop(actions: Receiver<ClientMessage>, ui: Sender<UiMsg>, session: Session) {
    for msg in actions {
        let notice = match session.lock().ok().and_then(|s| s.clone()) {
            None => Some("Keine Verbindung zum Server".to_string()),
            Some(cookie) => match send_action(&cookie, &msg) {
                Ok(None) => None,
                Ok(Some(reason)) => Some(reason),
                Err(e) => {
                    log::warn!("action: {e:#}");
                    Some("Keine Verbindung zum Server".to_string())
                }
            },
        };
        if let Some(text) = notice {
            let _ = ui.send(UiMsg::Notice(text));
        }
    }
}

/// `Ok(None)`: done. `Ok(Some(text))`: refused, with the server's reason.
fn send_action(cookie: &str, msg: &ClientMessage) -> Result<Option<String>> {
    let body = postcard::to_allocvec(msg).context("encode")?;
    let len = body.len().to_string();
    let headers = [
        ("cookie", cookie),
        ("content-type", "application/octet-stream"),
        ("content-length", len.as_str()),
    ];
    let url = format!("{SHOP_URL}/api/kds/action");
    let mut client = client(Duration::from_secs(15))?;
    let mut request = client.request(Method::Post, &url, &headers)?;
    request.write_all(&body).map_err(|e| anyhow!("send: {e:?}"))?;
    let mut response = request.submit()?;
    let status = response.status();
    if (200..300).contains(&status) {
        return Ok(None);
    }
    let mut buf = [0u8; 256];
    let n = embedded_svc::utils::io::try_read_full(&mut response, &mut buf).map_err(|e| anyhow!("read: {:?}", e.0))?;
    let text = String::from_utf8_lossy(&buf[..n]).trim().to_string();
    Ok(Some(if text.is_empty() { format!("Server: Fehler {status}") } else { text }))
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
