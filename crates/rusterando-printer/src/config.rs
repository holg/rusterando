use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// TCP target. Always loopback in production — the SSH tunnel terminates
    /// here, so we're really talking to the davidspizzeria server on the IONOS
    /// box via the autossh forward.
    #[serde(default = "default_server_addr")]
    pub server_addr: String,

    /// Identifies this shop to the server. Free-form; used in Hello and logs.
    #[serde(default = "default_shop_slug")]
    pub shop_slug: String,

    /// Build version, populated at compile time. Read-only from env's POV.
    #[serde(default = "default_version")]
    pub version: String,

    /// Target triple this binary was built for (e.g.
    /// "aarch64-unknown-linux-gnu"), captured at compile time by build.rs.
    /// Sent in Hello so the server picks the matching update artifact.
    #[serde(default = "default_arch")]
    pub arch: String,

    /// Device or file path to write ESC/POS bytes to. `/dev/usb/lp0` on real
    /// hardware; a regular file path for dev (writes act as a mock printer).
    #[serde(default = "default_printer_path")]
    pub printer_path: PathBuf,

    /// Where to persist the idempotency cache (last-acked seq + recent seq IDs).
    #[serde(default = "default_state_dir")]
    pub state_dir: PathBuf,

    #[serde(default = "default_reconnect_min")]
    pub reconnect_min_ms: u64,
    #[serde(default = "default_reconnect_max")]
    pub reconnect_max_ms: u64,
}

fn default_server_addr() -> String {
    "127.0.0.1:9001".into()
}
fn default_shop_slug() -> String {
    "rusterando".into()
}
fn default_version() -> String {
    env!("CARGO_PKG_VERSION").into()
}
fn default_arch() -> String {
    // Set by build.rs from cargo's TARGET. Falls back to "unknown" if the
    // build script didn't run for some reason.
    option_env!("BUILD_TARGET").unwrap_or("unknown").into()
}
fn default_printer_path() -> PathBuf {
    "/dev/usb/lp0".into()
}
fn default_state_dir() -> PathBuf {
    // Matches systemd's StateDirectory=rusterando-printer.
    "/var/lib/rusterando-printer".into()
}
fn default_reconnect_min() -> u64 {
    1_000
}
fn default_reconnect_max() -> u64 {
    30_000
}

impl Config {
    /// If $PRINTER_CONFIG points at a TOML file, load that. Otherwise pull
    /// individual env vars with sensible defaults. This lets you use a
    /// systemd EnvironmentFile in production or a one-liner env override
    /// during development.
    pub fn from_env_or_file() -> Result<Self> {
        if let Ok(path) = std::env::var("PRINTER_CONFIG") {
            let text =
                std::fs::read_to_string(&path).with_context(|| format!("read config {path}"))?;
            return toml::from_str(&text).context("parse config TOML");
        }
        Ok(Self {
            server_addr: env_or("PRINTER_SERVER_ADDR", default_server_addr()),
            shop_slug: env_or("PRINTER_SHOP_SLUG", default_shop_slug()),
            version: default_version(),
            arch: default_arch(),
            printer_path: env_or(
                "PRINTER_PATH",
                default_printer_path().to_string_lossy().into_owned(),
            )
            .into(),
            state_dir: env_or(
                "PRINTER_STATE_DIR",
                default_state_dir().to_string_lossy().into_owned(),
            )
            .into(),
            reconnect_min_ms: env_parse("PRINTER_RECONNECT_MIN_MS", default_reconnect_min()),
            reconnect_max_ms: env_parse("PRINTER_RECONNECT_MAX_MS", default_reconnect_max()),
        })
    }
}

fn env_or(key: &str, default: String) -> String {
    std::env::var(key).unwrap_or(default)
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
