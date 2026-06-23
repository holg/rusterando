//! Asset content-hashing + manifest helpers, shared across the server and
//! the build/import tooling.
//!
//! The project's convention is **SHA-256, lowercase hex** (the same hash the
//! server already used inline for content-addressed image uploads and the
//! printer-artifact pipeline). Consolidating it here keeps a single source of
//! truth for:
//!   - hashing bytes / files,
//!   - deriving content-hashed filenames (`name-<hash>.ext`),
//!   - writing/reading a small JSON manifest mapping logical names → hashed
//!     files (used for the wasm/JS bundle).
//!
//! WASM note: `rusterando-shared` is compiled into the hydrate (browser)
//! build, so the filesystem + manifest helpers are `cfg`-gated to non-wasm
//! targets — only the pure `sha256_hex` byte hasher ships in the WASM bundle.

use sha2::{Digest, Sha256};

/// SHA-256 of `bytes` as lowercase hex (64 chars). Available on every target.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    to_hex(&h.finalize())
}

/// Short content hash (first `n` hex chars of the SHA-256) for filenames.
/// `n` is clamped to `[1, 64]`.
pub fn short_hash(bytes: &[u8], n: usize) -> String {
    let full = sha256_hex(bytes);
    let n = n.clamp(1, full.len());
    full[..n].to_string()
}

/// `name-<hash>.ext` — content-addressed filename. Pass `hash_len` to control
/// how many hex chars to use (e.g. 8 for cache-busting, 64 for full dedup).
pub fn hashed_filename(stem: &str, ext: &str, bytes: &[u8], hash_len: usize) -> String {
    let h = short_hash(bytes, hash_len);
    if ext.is_empty() {
        format!("{stem}-{h}")
    } else {
        format!("{stem}-{h}.{}", ext.trim_start_matches('.'))
    }
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

// ---------------------------------------------------------------------------
// Filesystem + manifest — server / tooling only (not in the WASM bundle).
// ---------------------------------------------------------------------------
#[cfg(not(target_arch = "wasm32"))]
mod fs_impl {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::io::{self, Read};
    use std::path::Path;

    /// SHA-256 of a file's contents as lowercase hex. Streams the file so a
    /// large wasm binary isn't fully buffered twice.
    pub fn file_sha256_hex(path: impl AsRef<Path>) -> io::Result<String> {
        let mut f = std::fs::File::open(path)?;
        let mut h = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Ok(to_hex(&h.finalize()))
    }

    /// One built asset: its content hash and the on-disk (possibly hashed)
    /// filename a client should request.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct AssetEntry {
        /// Lowercase hex SHA-256 of the file contents.
        pub sha256: String,
        /// Filename to serve (e.g. `rusterando-1a2b3c4d_bg.wasm`).
        pub file: String,
    }

    /// A manifest mapping logical names (`"wasm"`, `"js"`, …) to their built,
    /// content-hashed assets. Serialised as stable, sorted JSON so identical
    /// inputs produce byte-identical manifests (good for caching/diffing).
    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
    pub struct AssetManifest {
        pub entries: BTreeMap<String, AssetEntry>,
    }

    impl AssetManifest {
        pub fn new() -> Self {
            Self::default()
        }

        /// Hash `path`'s contents and record it under `name`. `file` is what
        /// a client requests (often the basename of a hashed copy).
        pub fn add_file(
            &mut self,
            name: impl Into<String>,
            path: impl AsRef<Path>,
            file: impl Into<String>,
        ) -> io::Result<&mut Self> {
            let sha256 = file_sha256_hex(path)?;
            self.entries.insert(
                name.into(),
                AssetEntry {
                    sha256,
                    file: file.into(),
                },
            );
            Ok(self)
        }

        /// Record an entry whose bytes are already in memory.
        pub fn add_bytes(
            &mut self,
            name: impl Into<String>,
            bytes: &[u8],
            file: impl Into<String>,
        ) -> &mut Self {
            self.entries.insert(
                name.into(),
                AssetEntry {
                    sha256: sha256_hex(bytes),
                    file: file.into(),
                },
            );
            self
        }

        pub fn get(&self, name: &str) -> Option<&AssetEntry> {
            self.entries.get(name)
        }

        pub fn to_json(&self) -> String {
            // pretty + trailing newline → diff-friendly, stable ordering via BTreeMap.
            let mut s = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into());
            s.push('\n');
            s
        }

        pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
            serde_json::from_str(s)
        }

        /// Write the manifest to `path` (overwriting).
        pub fn write(&self, path: impl AsRef<Path>) -> io::Result<()> {
            std::fs::write(path, self.to_json())
        }

        /// Read a manifest from `path`.
        pub fn read(path: impl AsRef<Path>) -> io::Result<Self> {
            let s = std::fs::read_to_string(path)?;
            Self::from_json(&s).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }

        /// Write a sibling `<file>.sha256` checksum file for each entry, in the
        /// usual `<hex>  <file>` shasum format, under `dir`. Lets the server
        /// (or a deploy step) verify artifacts independently of the manifest.
        pub fn write_checksum_files(&self, dir: impl AsRef<Path>) -> io::Result<()> {
            let dir = dir.as_ref();
            for entry in self.entries.values() {
                let line = format!("{}  {}\n", entry.sha256, entry.file);
                std::fs::write(dir.join(format!("{}.sha256", entry.file)), line)?;
            }
            Ok(())
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use fs_impl::{file_sha256_hex, AssetEntry, AssetManifest};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        // SHA-256("") and SHA-256("abc") are well-known.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn short_and_filename() {
        assert_eq!(short_hash(b"abc", 8), "ba7816bf");
        assert_eq!(short_hash(b"abc", 1000), sha256_hex(b"abc")); // clamps to 64
        assert_eq!(
            hashed_filename("rusterando", "wasm", b"abc", 8),
            "rusterando-ba7816bf.wasm"
        );
        assert_eq!(
            hashed_filename("rusterando_bg", ".wasm", b"abc", 8),
            "rusterando_bg-ba7816bf.wasm"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn manifest_roundtrip() {
        let mut m = AssetManifest::new();
        m.add_bytes("wasm", b"abc", "rusterando-ba7816bf_bg.wasm");
        m.add_bytes("js", b"def", "rusterando-x.js");
        let json = m.to_json();
        let back = AssetManifest::from_json(&json).unwrap();
        assert_eq!(m, back);
        assert_eq!(back.get("wasm").unwrap().sha256, sha256_hex(b"abc"));
    }
}
