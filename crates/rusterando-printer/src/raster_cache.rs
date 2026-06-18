//! On-disk cache of rasterized logos, keyed by content hash.
//!
//! The server ships the header logo as SVG once; the Pi rasterizes it to
//! 1-bit and stores the result here so later receipts (and later boots)
//! reuse it without re-rasterizing — and so the server can send a hash
//! *reference* instead of the SVG bytes once the Pi advertises what it has
//! (in `Hello.cached_rasters`). Modeled on `idempotency.rs`: a directory
//! under the systemd `StateDirectory`, one file per hash.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use kitchen_protocol::bitmap::Bitmap;

const DIR_NAME: &str = "rasters";

/// File-backed raster cache. Each entry is `<state_dir>/rasters/<hex>` and
/// stores a tiny header (w,h as u32 LE) + the packed 1-bit bytes.
pub struct RasterCache {
    dir: PathBuf,
    /// Hashes currently on disk — advertised to the server in Hello.
    known: HashSet<[u8; 32]>,
}

impl RasterCache {
    pub fn load(state_dir: &Path) -> Result<Self> {
        let dir = state_dir.join(DIR_NAME);
        std::fs::create_dir_all(&dir).context("create raster cache dir")?;
        let mut known = HashSet::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                if let Some(hash) = e.file_name().to_str().and_then(hex_to_hash) {
                    known.insert(hash);
                }
            }
        }
        Ok(Self { dir, known })
    }

    /// Hashes we have on disk, for `Hello.cached_rasters`.
    pub fn known_hashes(&self) -> Vec<[u8; 32]> {
        self.known.iter().copied().collect()
    }

    /// Fetch a cached raster, or `None` on a miss / unreadable file.
    pub fn get(&self, hash: &[u8; 32]) -> Option<Bitmap> {
        if !self.known.contains(hash) {
            return None;
        }
        let bytes = std::fs::read(self.path(hash)).ok()?;
        decode(&bytes)
    }

    /// Store a raster under `hash` (best-effort; a write failure just means
    /// we'll re-rasterize next time, never a print failure).
    pub fn put(&mut self, hash: [u8; 32], bm: &Bitmap) {
        let encoded = encode(bm);
        // Write to a temp then rename so a crash can't leave a partial file.
        let tmp = self.dir.join(format!("{}.tmp", hex(&hash)));
        if std::fs::write(&tmp, &encoded).is_ok() && std::fs::rename(&tmp, self.path(&hash)).is_ok()
        {
            self.known.insert(hash);
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn path(&self, hash: &[u8; 32]) -> PathBuf {
        self.dir.join(hex(hash))
    }
}

fn encode(bm: &Bitmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + bm.bytes.len());
    out.extend_from_slice(&(bm.width as u32).to_le_bytes());
    out.extend_from_slice(&(bm.height as u32).to_le_bytes());
    out.extend_from_slice(&bm.bytes);
    out
}

fn decode(buf: &[u8]) -> Option<Bitmap> {
    if buf.len() < 8 {
        return None;
    }
    let width = u32::from_le_bytes(buf[0..4].try_into().ok()?) as usize;
    let height = u32::from_le_bytes(buf[4..8].try_into().ok()?) as usize;
    let bytes = buf[8..].to_vec();
    if bytes.len() != width.div_ceil(8) * height {
        return None;
    }
    Some(Bitmap {
        width,
        height,
        bytes,
    })
}

fn hex(hash: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in hash {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_to_hash(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let tmp = std::env::temp_dir().join(format!("rc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut cache = RasterCache::load(&tmp).unwrap();
        let bm = Bitmap {
            width: 16,
            height: 4,
            bytes: vec![0xAB; 2 * 4],
        };
        let hash = [7u8; 32];
        assert!(cache.get(&hash).is_none());
        cache.put(hash, &bm);
        assert_eq!(cache.get(&hash).unwrap(), bm);
        assert!(cache.known_hashes().contains(&hash));

        // Reload from disk → still there.
        let reloaded = RasterCache::load(&tmp).unwrap();
        assert_eq!(reloaded.get(&hash).unwrap(), bm);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
