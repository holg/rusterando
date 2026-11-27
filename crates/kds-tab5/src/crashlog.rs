//! Why the previous run ended, kept across the reset. The USB console is
//! often silent after a reset (button, watchdog, panic), so the screen has to
//! tell: a Rust panic writes its message to NVS before the chip restarts, and
//! the next boot shows the reset reason plus that message.

#[cfg(not(feature = "cardputer"))]
use std::ffi::CStr;
use std::sync::{Mutex, OnceLock};

use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sys;

const NAMESPACE: &str = "kds";
const KEY: &str = "last_panic";
/// NVS strings are limited; the location and the start of the message suffice.
const MAX_LEN: usize = 300;

static STORE: OnceLock<Mutex<EspNvs<NvsDefault>>> = OnceLock::new();

/// Install the panic hook and return the note about the last run, if it
/// ended in a crash.
pub fn init(nvs: EspDefaultNvsPartition) -> Option<String> {
    let store = match EspNvs::new(nvs, NAMESPACE, true) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("crashlog: NVS: {e}");
            return None;
        }
    };
    let mut buf = [0u8; MAX_LEN + 1];
    let saved = store.get_str(KEY, &mut buf).ok().flatten().map(str::to_string);
    if saved.is_some() {
        let _ = store.remove(KEY);
    }
    let _ = STORE.set(Mutex::new(store));

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut text = info.to_string();
        if text.len() > MAX_LEN {
            let mut cut = MAX_LEN;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        if let Some(store) = STORE.get() {
            if let Ok(store) = store.try_lock() {
                let _ = store.set_str(KEY, &text);
            }
        }
        default_hook(info);
    }));

    // The last error/warning lines of the previous run (lab_ble diag.c),
    // then keep this run's: a software restart (esp_hosted's "restart
    // host", the LAN guard, …) logs why just before, and the USB console
    // often loses those lines and stays silent afterwards.
    #[cfg(not(feature = "cardputer"))]
    // SAFETY: a NUL-terminated static buffer on the C side; then the hook.
    let before = unsafe {
        let last = CStr::from_ptr(sys::lab_ble::lab_diag_take_last())
            .to_string_lossy()
            .into_owned();
        sys::lab_ble::lab_diag_install();
        last
    };
    #[cfg(feature = "cardputer")]
    let before = String::new();

    // SAFETY: plain query.
    let reason = match unsafe { sys::esp_reset_reason() } {
        sys::esp_reset_reason_t_ESP_RST_PANIC => "Absturz (Panic)",
        sys::esp_reset_reason_t_ESP_RST_INT_WDT => "Interrupt-Watchdog",
        sys::esp_reset_reason_t_ESP_RST_TASK_WDT => "Task-Watchdog",
        sys::esp_reset_reason_t_ESP_RST_WDT => "Watchdog",
        sys::esp_reset_reason_t_ESP_RST_BROWNOUT => "Unterspannung",
        sys::esp_reset_reason_t_ESP_RST_CPU_LOCKUP => "CPU-Lockup",
        sys::esp_reset_reason_t_ESP_RST_PWR_GLITCH => "Spannungseinbruch",
        // Not crashes, but every restart should say what it was.
        sys::esp_reset_reason_t_ESP_RST_SW => "Neustart durch die Firmware",
        sys::esp_reset_reason_t_ESP_RST_EXT => "Reset-Taste",
        sys::esp_reset_reason_t_ESP_RST_USB => "USB",
        sys::esp_reset_reason_t_ESP_RST_JTAG => "JTAG",
        _ => return saved.map(|p| format!("Letzter Absturz: {p}")),
    };
    let mut note = match saved {
        Some(p) => format!("Neustart nach {reason}: {p}"),
        None if reason.starts_with("Neustart") => reason.to_string(),
        None => format!("Neustart nach {reason}"),
    };
    if !before.is_empty() {
        note.push_str(" — zuletzt: ");
        note.push_str(&before);
    }
    Some(note)
}
