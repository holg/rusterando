//! Server-held printer-binary artifacts for in-band Pi self-update.
//!
//! The server keeps, per target arch, the binary it wants every Pi to run
//! plus its version + sha256. On an admin-armed Hello it offers this to the
//! Pi, which pulls + verifies + installs it over the kitchen channel — no
//! SSH, no manual deploy. This is what stops a version skew from silently
//! stranding the Pi.
//!
//! Layout (under `PRINTER_ARTIFACT_DIR`, default `artifacts/printer`):
//!
//! ```text
//! <dir>/<arch>/rusterando-printer    the binary to ship
//! <dir>/<arch>/version               its version string (e.g. "0.3.0")
//! ```
//!
//! `cross_build_on_mac.sh` populates this on a release build. The version
//! file is the single source of truth for "what the server wants"; the
//! admin panel shows it next to each Pi's running version.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

/// One resolved artifact: the bytes, its version, and sha256.
#[derive(Clone)]
pub struct Artifact {
    pub version: String,
    pub sha256: [u8; 32],
    pub bytes: std::sync::Arc<Vec<u8>>,
}

impl Artifact {
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
}

/// Reads + caches printer artifacts by arch. Cheap to clone (Arc inside).
pub struct ArtifactStore {
    dir: PathBuf,
    // arch -> (mtime_secs, Artifact). Reloads if the file mtime changes so a
    // fresh deploy is picked up without a server restart.
    cache: Mutex<std::collections::HashMap<String, (i64, Artifact)>>,
}

impl ArtifactStore {
    pub fn from_env() -> Self {
        let dir = std::env::var("PRINTER_ARTIFACT_DIR")
            .unwrap_or_else(|_| "artifacts/printer".to_string());
        Self {
            dir: PathBuf::from(dir),
            cache: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn bin_path(&self, arch: &str) -> PathBuf {
        self.dir.join(arch).join("rusterando-printer")
    }
    fn version_path(&self, arch: &str) -> PathBuf {
        self.dir.join(arch).join("version")
    }

    /// The artifact for `arch`, or `None` if none is staged (or unreadable).
    /// Reloads when the binary's mtime changes.
    pub fn artifact(&self, arch: &str) -> Option<Artifact> {
        let bin = self.bin_path(arch);
        let mtime = file_mtime_secs(&bin)?;

        if let Some((cached_mtime, art)) = self.cache.lock().unwrap().get(arch) {
            if *cached_mtime == mtime {
                return Some(art.clone());
            }
        }

        let bytes = std::fs::read(&bin).ok()?;
        if bytes.is_empty() {
            return None;
        }
        let version = std::fs::read_to_string(self.version_path(arch))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;
        let sha256: [u8; 32] = {
            let mut h = Sha256::new();
            h.update(&bytes);
            h.finalize().into()
        };
        let art = Artifact {
            version,
            sha256,
            bytes: std::sync::Arc::new(bytes),
        };
        self.cache
            .lock()
            .unwrap()
            .insert(arch.to_string(), (mtime, art.clone()));
        Some(art)
    }

    /// Just the target version for `arch` (cheap; reads the version file).
    pub fn target_version(&self, arch: &str) -> Option<String> {
        std::fs::read_to_string(self.version_path(arch))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

fn file_mtime_secs(p: &Path) -> Option<i64> {
    let meta = std::fs::metadata(p).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_secs() as i64)
}

/// Process-wide store.
static STORE: std::sync::OnceLock<ArtifactStore> = std::sync::OnceLock::new();

pub fn store() -> &'static ArtifactStore {
    STORE.get_or_init(ArtifactStore::from_env)
}

/// Chunk size for in-band artifact transfer. 48 KiB keeps each frame well
/// under the 1 MiB `FRAME_MAX_BYTES` ceiling with headroom for the
/// postcard envelope, and is large enough that a ~2 MB binary is ~45
/// round-trips.
pub const CHUNK_BYTES: usize = 48 * 1024;
