//! Updates over the network, through the print channel — the same handshake
//! as the STM32 board's: when an admin arms the update on /admin/drucker
//! and the server holds a different image for `esp32p4`, it offers it; the
//! device fetches it in chunks, checks size and SHA-256, and boots it.
//!
//! Two app slots (partitions.csv). A new image runs "pending verify" until
//! it confirms itself — once the print channel is connected, which proves
//! Wi-Fi, TLS, login and server. A reset before that boots the previous
//! image again, and a pending image that hasn't confirmed itself within
//! [`CONFIRM_WITHIN`] resets on purpose: a broken update undoes itself.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{bail, Result};
use esp_idf_svc::sys::{self, esp};
use sha2::{Digest, Sha256};

/// This firmware's version, compared by the server with the staged image's
/// (scripts/build_kds_tab5.sh sets it per build).
pub const VERSION: &str = match option_env!("KDS_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// A new image must confirm itself within this time after boot.
const CONFIRM_WITHIN: Duration = Duration::from_secs(5 * 60);

static PENDING: AtomicBool = AtomicBool::new(false);

/// At start: is this a fresh update that still has to prove itself? Then a
/// timer goes back to the previous image unless it confirms in time —
/// independent of the network, which may be what's broken.
pub fn init() {
    // SAFETY: read-only queries about the running partition.
    unsafe {
        let running = sys::esp_ota_get_running_partition();
        let mut state: sys::esp_ota_img_states_t = 0;
        if !running.is_null()
            && sys::esp_ota_get_state_partition(running, &mut state) == sys::ESP_OK
            && state == sys::esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY
        {
            PENDING.store(true, Ordering::Relaxed);
            log::warn!("ota: running a new image ({VERSION}), not confirmed yet");
            let deadline = std::thread::Builder::new()
                .name("ota-deadline".into())
                .stack_size(4 * 1024)
                .spawn(|| {
                    std::thread::sleep(CONFIRM_WITHIN);
                    roll_back_unless_confirmed();
                });
            if let Err(e) = deadline {
                log::warn!("ota: no deadline timer: {e}");
            }
        }
    }
}

/// The new image works (the print channel is up): keep it.
pub fn confirm() {
    if PENDING.swap(false, Ordering::Relaxed) {
        // SAFETY: plain call; marks the running slot valid.
        match esp!(unsafe { sys::esp_ota_mark_app_valid_cancel_rollback() }) {
            Ok(()) => log::info!("ota: update to {VERSION} confirmed"),
            Err(e) => log::warn!("ota: confirming the update failed: {e}"),
        }
    }
}

/// An image that never confirmed itself goes back to the previous one.
fn roll_back_unless_confirmed() {
    if PENDING.load(Ordering::Relaxed) {
        log::error!(
            "ota: {VERSION} not confirmed after {} min, back to the previous image",
            CONFIRM_WITHIN.as_secs() / 60
        );
        // SAFETY: marks this slot invalid and restarts; does not return.
        unsafe { sys::esp_ota_mark_app_invalid_rollback_and_reboot() };
    }
}

/// An update image being received into the other slot. Dropping it before
/// [`Download::finish`] abandons it; the running image stays.
pub struct Download {
    handle: sys::esp_ota_handle_t,
    partition: *const sys::esp_partition_t,
    hasher: Sha256,
    expected: [u8; 32],
    size: u64,
    received: u64,
    logged_tenth: u64,
    pub target: String,
}

// SAFETY: the handle and partition pointer are only used from the printing
// thread that owns the download.
unsafe impl Send for Download {}

impl Download {
    pub fn begin(target: String, size: u64, sha256: [u8; 32]) -> Result<Self> {
        if PENDING.load(Ordering::Relaxed) {
            bail!("the running image isn't confirmed yet");
        }
        // SAFETY: partition info is static; the OTA handle is released by
        // esp_ota_end / esp_ota_abort (Drop).
        unsafe {
            let partition = sys::esp_ota_get_next_update_partition(core::ptr::null());
            if partition.is_null() {
                bail!("no update slot (old partition layout: flash once over USB)");
            }
            if size > (*partition).size as u64 {
                bail!("image ({size} B) larger than the slot ({} B)", (*partition).size);
            }
            let mut handle: sys::esp_ota_handle_t = 0;
            // Sequential writes: erase sector by sector as the data comes,
            // instead of blocking for the whole slot up front.
            esp!(sys::esp_ota_begin(partition, sys::OTA_WITH_SEQUENTIAL_WRITES as usize, &mut handle))?;
            log::info!("ota: receiving {target} ({size} B)");
            Ok(Self {
                handle,
                partition,
                hasher: Sha256::new(),
                expected: sha256,
                size,
                received: 0,
                logged_tenth: 0,
                target,
            })
        }
    }

    /// Where the next chunk starts.
    pub fn offset(&self) -> u64 {
        self.received
    }

    /// Store a chunk; `true` once the whole image is in.
    pub fn write(&mut self, offset: u64, bytes: &[u8]) -> Result<bool> {
        if offset != self.received {
            bail!("chunk at {offset}, expected {}", self.received);
        }
        if self.received + bytes.len() as u64 > self.size {
            bail!("more data than announced");
        }
        // SAFETY: live handle; the buffer outlives the call.
        esp!(unsafe { sys::esp_ota_write(self.handle, bytes.as_ptr().cast(), bytes.len()) })?;
        self.hasher.update(bytes);
        self.received += bytes.len() as u64;
        let tenth = self.received * 10 / self.size.max(1);
        if tenth > self.logged_tenth {
            self.logged_tenth = tenth;
            log::info!("ota: {} %", tenth * 10);
        }
        Ok(self.received == self.size)
    }

    /// Check size and SHA-256, validate the image and make it the next boot.
    pub fn finish(mut self) -> Result<()> {
        if self.received != self.size {
            bail!("got {} of {} bytes", self.received, self.size);
        }
        let digest: [u8; 32] = self.hasher.clone().finalize().into();
        if digest != self.expected {
            bail!("SHA-256 doesn't match the offer");
        }
        let handle = std::mem::replace(&mut self.handle, 0);
        // SAFETY: the handle came from esp_ota_begin; esp_ota_end checks the
        // image (header, chip revision, checksum) and frees the handle.
        unsafe {
            esp!(sys::esp_ota_end(handle))?;
            esp!(sys::esp_ota_set_boot_partition(self.partition))?;
        }
        log::info!("ota: {} installed, boots next", self.target);
        Ok(())
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        if self.handle != 0 {
            // SAFETY: an unfinished download's handle.
            unsafe { sys::esp_ota_abort(self.handle) };
        }
    }
}
