//! The colour e-ink frame (../e-ink-color: Waveshare 7.3" Spectra 6 on an
//! ESP32-S3, Bluetooth name `EInk-7in3`) shows the Tagesangebot.
//!
//! Pushed, not polled: the server sends the picture's tag on the board feed
//! (`KitchenEvent::Picture`, feed.rs asks for it with `?picture=1`) when the
//! feed opens and whenever the picture changes (a saved Tagesangebot at
//! once). Which device drives the frame is switched on /admin/drucker (the
//! server's database: no flashing, survives power-off): that device gets
//! the tag, any other an empty one and leaves the frame alone. It
//! fetches the picture once (`/tagesangebot.eink`, 192 000 bytes in the
//! frame's own format) and sends it over Bluetooth LE (components/lab_ble,
//! protocol v1); the frame refreshes (about 20 s) and sleeps again. The
//! frame is never polled (an energy-saving frame sleeps most of the time):
//! a picture goes once. Frame not found or a failed send: tried again after
//! 30 s and 1 min, then not until the next picture (or `eink push`). The
//! tag of the picture on the frame is kept in NVS, so a restart of this
//! device doesn't send the same picture again.
//!
//! The address is `{SHOP_URL}/tagesangebot.eink`, or `EINK_URL` from
//! tmp/tab5-wifi.env for a test server (`stage` leaves `EINK_URL` out: the
//! update for the shop's devices uses the shop's address). `EINK_NAME`:
//! another frame's Bluetooth name.
//!
//! Terminal: `eink` (state), `eink push` (fetch and send now — by hand also
//! from a Tab5 that isn't switched on).

use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::http::Method;
use esp_idf_svc::http::client::{Configuration as HttpConfig, EspHttpConnection};
use esp_idf_svc::sys::{self, lab_ble};

#[cfg(not(feature = "jc1060p470"))]
use crate::terminal::{Terminal, Tone};

/// The frame's Bluetooth name.
pub const NAME: &str = match option_env!("EINK_NAME") {
    Some(name) => name,
    None => "EInk-7in3",
};
/// 800 × 480 pixels, two per byte.
const FRAME_LEN: usize = 192_000;
/// After a failed fetch or send: wait this long, doubled per failure (a
/// frame switched off or held by the phone app). A new picture from the
/// server always goes at once.
const RETRY: Duration = Duration::from_secs(30);
/// Tries per picture, then none until the next one (or `eink push`): the
/// frame isn't polled — an energy-saving frame sleeps most of the time.
const TRIES: u32 = 3;
/// How long to look for the frame's advertisement.
const SCAN_MS: u32 = 8_000;

fn url() -> String {
    match option_env!("EINK_URL") {
        Some(url) if !url.is_empty() => url.to_string(),
        _ => format!("{}/tagesangebot.eink", crate::feed::SHOP_URL),
    }
}

/// The picture the server says is current (its tag), from the feed.
static WANTED: Mutex<Option<String>> = Mutex::new(None);
/// The tag the frame shows (sent by this device; kept across restarts).
static SHOWN: Mutex<Option<String>> = Mutex::new(None);
/// What `eink` shows: the last outcome and when.
static LAST: Mutex<Option<(Instant, String)>> = Mutex::new(None);
/// `eink push` typed.
static FORCE: AtomicBool = AtomicBool::new(false);

/// The server's word (feed.rs): the picture is now `tag` — empty when this
/// device doesn't drive the frame (switched on /admin/drucker).
pub fn announce(tag: String) {
    let mut wanted = WANTED.lock().unwrap_or_else(|e| e.into_inner());
    let new = (!tag.is_empty()).then_some(tag);
    if *wanted == new {
        return;
    }
    match (&*wanted, &new) {
        (None, Some(t)) => log::info!("e-ink: this device keeps {NAME} up to date (/admin/drucker) — picture {t}"),
        (Some(_), None) => log::info!("e-ink: not this device any more (/admin/drucker) — the frame is left alone"),
        (_, Some(t)) => log::info!("e-ink: the server's picture is {t}"),
        (None, None) => {}
    }
    *wanted = new;
}

/// Does this device drive the frame? The server says so with every tag.
#[cfg(not(feature = "jc1060p470"))]
fn in_charge() -> bool {
    WANTED.lock().ok().is_some_and(|w| w.is_some())
}

/// The terminal's `eink` commands. `false`: not one of them.
#[cfg(not(feature = "jc1060p470"))]
pub fn command(words: &[&str], term: &mut Terminal) -> bool {
    match words {
        ["eink"] => {
            let get = |m: &Mutex<Option<String>>| m.lock().ok().and_then(|t| t.clone());
            term.push(Tone::Normal, &format!("e-ink: {NAME} ← {}", url()));
            term.push(
                Tone::Normal,
                if in_charge() {
                    "  this device keeps the frame up to date (switched on /admin/drucker)"
                } else {
                    "  not this device (switched on /admin/drucker; eink push: once from here)"
                },
            );
            term.push(
                Tone::Normal,
                &format!(
                    "  server's picture {}, on the frame {}",
                    get(&WANTED).unwrap_or_else(|| "(not told yet)".into()),
                    get(&SHOWN).unwrap_or_else(|| "(nothing sent yet)".into()),
                ),
            );
            if let Some((at, what)) = LAST.lock().ok().and_then(|l| l.clone()) {
                term.push(Tone::Normal, &format!("  {what} ({} s ago)", at.elapsed().as_secs()));
            }
        }
        ["eink", "push"] => {
            FORCE.store(true, Ordering::Relaxed);
            term.push(Tone::Ok, &format!("e-ink: fetching the picture and sending it to {NAME} …"));
        }
        _ => return false,
    }
    true
}

/// Start keeping the frame up to date (in its own thread).
pub fn start() -> Result<()> {
    std::thread::Builder::new()
        .name("eink".into())
        .stack_size(16 * 1024)
        .spawn(run)?;
    Ok(())
}

/// A fetched picture, for the tag it was fetched for.
struct Picture {
    tag: Option<String>,
    bytes: Vec<u8>,
}

fn run() {
    let url = url();
    // The picture on the frame survives a restart of this device (e-paper
    // keeps it without power): no sending the same one again.
    if let Some(tag) = memory::load() {
        log::info!("e-ink: {NAME} shows picture {tag} (remembered)");
        if let Ok(mut s) = SHOWN.lock() {
            *s = Some(tag);
        }
    }
    // Fetched but not on the frame yet (the frame wasn't found, say).
    let mut pending: Option<Picture> = None;
    let mut fails = 0u32;
    let mut retry_at = Instant::now();
    let mut gave_up = false;
    let mut last_wanted: Option<String> = None;
    loop {
        // A local look once a second — a typed `eink push`, the retry time.
        // The server is never asked: it tells (announce).
        std::thread::sleep(Duration::from_secs(1));
        let forced = FORCE.swap(false, Ordering::Relaxed);
        let wanted = WANTED.lock().ok().and_then(|w| w.clone());
        // A new picture: no waiting out an earlier failure. Switched off:
        // nothing waits any more.
        if wanted != last_wanted {
            last_wanted = wanted.clone();
            fails = 0;
            gave_up = false;
            retry_at = Instant::now();
            if wanted.is_none() {
                pending = None;
            }
        }
        let shown = SHOWN.lock().ok().and_then(|s| s.clone());
        let behind = wanted.is_some() && wanted != shown;
        if !forced && !(behind && !gave_up && Instant::now() >= retry_at) {
            continue;
        }

        // The picture: fetched anew unless the one waiting here is it.
        let have = pending.as_ref().is_some_and(|p| p.tag == wanted);
        if forced || !have {
            match fetch(&url) {
                Ok(bytes) => {
                    log::info!(
                        "e-ink: picture {} fetched, sending to {NAME}",
                        wanted.as_deref().unwrap_or("(by hand)")
                    );
                    pending = Some(Picture { tag: wanted.clone(), bytes });
                }
                Err(e) => {
                    let again = backoff(&mut fails, &mut retry_at, &mut gave_up);
                    report(Err(format!("{url}: {e:#} — {again}")), forced);
                    continue;
                }
            }
        }
        let Some(picture) = pending.take() else {
            continue;
        };
        match push(&picture.bytes) {
            Ok(took) => {
                fails = 0;
                report(Ok(format!("picture sent to {NAME}, shown after {} s", took.as_secs())), forced);
                // Done for this tag, even if the server moved on meanwhile:
                // its next word starts the next round.
                if let Some(tag) = &picture.tag {
                    memory::save(tag);
                }
                if let Ok(mut s) = SHOWN.lock() {
                    *s = picture.tag;
                }
            }
            Err(e) => {
                let again = backoff(&mut fails, &mut retry_at, &mut gave_up);
                report(Err(format!("{NAME}: {e:#} — {again}")), forced);
                pending = Some(picture);
            }
        }
    }
}

/// One more failure: the next try 30 s, then 1 min later — after [`TRIES`]
/// none until the next picture. What to tell about it.
fn backoff(fails: &mut u32, retry_at: &mut Instant, gave_up: &mut bool) -> String {
    *fails += 1;
    if *fails >= TRIES {
        *gave_up = true;
        return "not again until the next picture (or eink push)".into();
    }
    let wait = RETRY.saturating_mul(1 << (*fails - 1));
    *retry_at = Instant::now() + wait;
    format!("again in {} s", wait.as_secs())
}

/// The tag of the picture on the frame, kept in NVS across restarts.
mod memory {
    use std::ffi::CString;

    use esp_idf_svc::sys;

    const NAMESPACE: &core::ffi::CStr = c"kds";
    const KEY: &core::ffi::CStr = c"eink_shown";

    /// Run `f` on a short-lived thread whose stack is in internal RAM: NVS
    /// goes to flash, which a PSRAM stack doesn't survive (the cache is off
    /// during the access) — and the e-ink thread may have one (lab.rs).
    fn on_internal_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        // SAFETY: plain config struct; the pthread layer copies it. Only
        // this thread's later spawns (these) are affected.
        unsafe {
            let mut cfg = sys::esp_pthread_get_default_config();
            cfg.stack_size = 6144;
            cfg.stack_alloc_caps = sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT;
            cfg.inherit_cfg = false;
            let _ = sys::esp_pthread_set_cfg(&cfg);
        }
        std::thread::Builder::new()
            .name("eink-nvs".into())
            .stack_size(6144)
            .spawn(f)
            .ok()?
            .join()
            .ok()
    }

    pub fn load() -> Option<String> {
        on_internal_stack(|| {
            let mut handle: sys::nvs_handle_t = 0;
            let mut buf = [0u8; 64];
            let mut len: usize = buf.len();
            // SAFETY: NUL-terminated names; `buf` holds `len` bytes.
            unsafe {
                if sys::nvs_open(NAMESPACE.as_ptr(), sys::nvs_open_mode_t_NVS_READONLY, &mut handle)
                    != sys::ESP_OK
                {
                    return None;
                }
                let rc = sys::nvs_get_str(handle, KEY.as_ptr(), buf.as_mut_ptr().cast(), &mut len);
                sys::nvs_close(handle);
                (rc == sys::ESP_OK && len > 1).then(|| String::from_utf8_lossy(&buf[..len - 1]).into_owned())
            }
        })
        .flatten()
    }

    pub fn save(tag: &str) {
        let Ok(tag) = CString::new(tag) else {
            return;
        };
        on_internal_stack(move || {
            let mut handle: sys::nvs_handle_t = 0;
            // SAFETY: NUL-terminated strings; the handle is closed again.
            unsafe {
                if sys::nvs_open(NAMESPACE.as_ptr(), sys::nvs_open_mode_t_NVS_READWRITE, &mut handle)
                    == sys::ESP_OK
                {
                    if sys::nvs_set_str(handle, KEY.as_ptr(), tag.as_ptr()) != sys::ESP_OK
                        || sys::nvs_commit(handle) != sys::ESP_OK
                    {
                        log::warn!("e-ink: couldn't remember the picture");
                    }
                    sys::nvs_close(handle);
                }
            }
        });
    }
}

/// Note the outcome; into the log (and so the terminal) when it's news.
fn report(outcome: Result<String, String>, forced: bool) {
    let text = match &outcome {
        Ok(t) | Err(t) => t.clone(),
    };
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let news = forced || outcome.is_ok() || last.as_ref().map(|(_, t)| t != &text).unwrap_or(true);
    if news {
        match outcome {
            Ok(_) => log::info!("e-ink: {text}"),
            Err(_) => log::warn!("e-ink: {text}"),
        }
    }
    *last = Some((Instant::now(), text));
}

/// `GET url`: the picture in the frame's format.
fn fetch(url: &str) -> Result<Vec<u8>> {
    // Neither SHOP_URL nor EINK_URL compiled in (demo build): no address.
    if !url.contains("://") {
        bail!("no shop address compiled in (SHOP_URL / EINK_URL)");
    }
    let mut client = HttpClient::wrap(EspHttpConnection::new(&HttpConfig {
        crt_bundle_attach: Some(sys::esp_crt_bundle_attach),
        timeout: Some(Duration::from_secs(20)),
        buffer_size: Some(4096),
        buffer_size_tx: Some(1024),
        ..Default::default()
    })?);
    // Compressed: about 13 KB instead of 192 KB. A big download threw the
    // W5500 out of step (lan.rs) until the Tab5 restarted; an older server
    // just sends it plain.
    let headers = [("accept-encoding", "deflate")];
    let mut response = client.request(Method::Get, url, &headers)?.submit()?;
    let status = response.status();
    if status != 200 {
        bail!("HTTP {status}");
    }
    let deflated = response
        .header("content-encoding")
        .is_some_and(|e| e.eq_ignore_ascii_case("deflate"));
    // Up to 192 KB: from PSRAM (above the internal-RAM malloc limit).
    let mut body = Vec::with_capacity(if deflated { 32 * 1024 } else { FRAME_LEN });
    let mut buf = [0u8; 2048];
    loop {
        let n = embedded_svc::io::Read::read(&mut response, &mut buf).map_err(|e| anyhow!("read: {e:?}"))?;
        if n == 0 {
            break;
        }
        if body.len() + n > FRAME_LEN {
            bail!("more than the frame's {FRAME_LEN} bytes");
        }
        body.extend_from_slice(&buf[..n]);
    }
    let bytes = if deflated {
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&body, FRAME_LEN + 1)
            .map_err(|e| anyhow!("deflate: {e:?}"))?
    } else {
        body
    };
    if bytes.len() != FRAME_LEN {
        bail!("{} bytes, the frame takes {FRAME_LEN}", bytes.len());
    }
    Ok(bytes)
}

/// Send one picture to the frame; how long it took until it showed.
fn push(frame: &[u8]) -> Result<Duration> {
    let name = CString::new(NAME)?;
    let crc = crc32(frame);
    let started = Instant::now();
    let mut detail: i32 = 0;
    let mut attempt = || {
        // SAFETY: the C side reads `name` and `frame` only during the call
        // and writes `detail`, a local.
        unsafe {
            lab_ble::lab_ble_eink_push(name.as_ptr(), frame.as_ptr(), frame.len(), crc, SCAN_MS, &mut detail)
        }
    };
    let mut rc = attempt();
    // One more go at once for the link itself: the first search after a
    // start sometimes misses the frame, and a fresh link sometimes drops
    // right after connecting — both went through on the next try. Not for
    // what the frame reports itself (a hung panel isn't hammered).
    if matches!(
        rc as u32,
        lab_ble::LAB_EINK_NOT_FOUND
            | lab_ble::LAB_EINK_CONNECT
            | lab_ble::LAB_EINK_LINK
            | lab_ble::LAB_EINK_SERVICE
    ) {
        log::info!("e-ink: link to {NAME} failed ({rc}), trying again");
        std::thread::sleep(Duration::from_secs(2));
        rc = attempt();
    }
    match rc as u32 {
        lab_ble::LAB_EINK_OK => Ok(started.elapsed()),
        lab_ble::LAB_EINK_UNSUPPORTED => bail!("built without Bluetooth central"),
        lab_ble::LAB_EINK_NOT_READY => bail!("Bluetooth not ready"),
        lab_ble::LAB_EINK_BUSY => bail!("already sending"),
        lab_ble::LAB_EINK_NOT_FOUND => bail!("not found (switched on, in range, phone app closed?)"),
        lab_ble::LAB_EINK_CONNECT => bail!("connecting failed ({detail})"),
        lab_ble::LAB_EINK_SERVICE => bail!("no picture service on it ({detail})"),
        lab_ble::LAB_EINK_LINK => bail!("link lost while sending ({detail})"),
        lab_ble::LAB_EINK_TIMEOUT => bail!("the frame stopped answering"),
        lab_ble::LAB_EINK_FRAME => bail!(
            "the frame refused it: {}",
            match detail {
                0x80 => "wrong length",
                0x81 => "checksum wrong",
                0x82 => "panel not answering",
                0x83 => "out of sequence",
                0x84 => "unknown command",
                _ => "error",
            }
        ),
        other => bail!("error {other}"),
    }
}

/// CRC-32 as zlib's (IEEE, reflected): what COMMIT carries.
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *entry = c;
    }
    let mut crc = !0u32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

#[cfg(test)]
mod tests {
    #[test]
    fn crc32_check_value() {
        assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926);
    }
}
