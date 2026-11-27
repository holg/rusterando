//! Bluetooth LE serial link (components/lab_ble, C): the Nordic UART
//! Service on NimBLE, the radio being the ESP32-C6's via esp_hosted. Safe
//! wrappers around its C API.

use std::ffi::{CStr, CString};

use anyhow::Result;
use esp_idf_svc::sys::{esp, lab_ble};

/// Start the host stack, advertising as `name`.
pub fn start(name: &str) -> Result<()> {
    let name = CString::new(name)?;
    // SAFETY: the C side copies the name.
    esp!(unsafe { lab_ble::lab_ble_start(name.as_ptr()) })?;
    Ok(())
}

/// The C6's controller answered.
pub fn ready() -> bool {
    // SAFETY: reads a flag.
    unsafe { lab_ble::lab_ble_ready() }
}

/// A phone is connected with notifications on.
pub fn connected() -> bool {
    // SAFETY: reads a flag.
    unsafe { lab_ble::lab_ble_connected() }
}

/// Counts connections: a change means a new phone (or the same again).
pub fn session() -> u32 {
    // SAFETY: reads a counter.
    unsafe { lab_ble::lab_ble_session() }
}

/// Bytes received since the last call (empty when none).
pub fn read() -> Vec<u8> {
    let mut buf = [0u8; 512];
    // SAFETY: the C side writes at most `buf.len()` bytes.
    let n = unsafe { lab_ble::lab_ble_read(buf.as_mut_ptr(), buf.len()) };
    buf[..n].to_vec()
}

/// Send bytes to the phone (notifications, split to the MTU).
pub fn send(data: &[u8]) -> Result<()> {
    // SAFETY: the C side reads `data.len()` bytes during the call.
    esp!(unsafe { lab_ble::lab_ble_send(data.as_ptr(), data.len()) })?;
    Ok(())
}

/// The controller's address, once ready.
pub fn address() -> String {
    // SAFETY: a NUL-terminated static buffer on the C side.
    unsafe { CStr::from_ptr(lab_ble::lab_ble_address()) }
        .to_string_lossy()
        .into_owned()
}
