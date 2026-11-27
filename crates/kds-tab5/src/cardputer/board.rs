//! The Cardputer ADV as a device: its name and its update slot.

use esp_idf_svc::sys;

/// The update image's slot on the server (artifacts/printer/<ARCH>), and
/// what the device reports when it registers.
#[allow(dead_code)]
pub const ARCH: &str = "esp32s3-cardputer";
/// The device in messages.
#[allow(dead_code)]
pub const MODEL: &str = "Cardputer";

/// "cardputer-445566": the model and the end of the MAC, like the Tab5s.
/// (Not KITCHEN_SLUG: that is one name for every device built with it.)
pub fn device_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let mut mac = [0u8; 6];
        // SAFETY: the function fills the 6-byte buffer.
        if unsafe { sys::esp_efuse_mac_get_default(mac.as_mut_ptr()) } != 0 {
            return "cardputer".to_string();
        }
        format!("cardputer-{:02x}{:02x}{:02x}", mac[3], mac[4], mac[5])
    })
}
