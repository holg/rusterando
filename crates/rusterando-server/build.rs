//! Download font files into OUT_DIR/fonts so include_bytes! can reference
//! them. The Inter release ships only as a zip; we extract the four TTFs we
//! need. Noto Color Emoji is hosted as a single .ttf at a stable Google CDN
//! path (versioned), so it's a plain GET.
//!
//! Cached per-target via OUT_DIR — network only on first build.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const INTER_RELEASE_URL: &str =
    "https://github.com/rsms/inter/releases/download/v4.0/Inter-4.0.zip";
const INTER_FILES: &[(&str, &str)] = &[
    ("Inter-Regular.ttf", "extras/ttf/Inter-Regular.ttf"),
    ("Inter-Bold.ttf", "extras/ttf/Inter-Bold.ttf"),
    ("Inter-Italic.ttf", "extras/ttf/Inter-Italic.ttf"),
];

// jsdelivr mirror is faster + more reliable than github raw for build agents.
const NOTO_EMOJI_URL: &str =
    "https://cdn.jsdelivr.net/gh/googlefonts/noto-emoji@v2.038/fonts/NotoColorEmoji.ttf";
const NOTO_EMOJI_FILE: &str = "NotoColorEmoji.ttf";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"));
    let cache = out_dir.join("fonts");
    std::fs::create_dir_all(&cache).expect("create font cache");

    fetch_inter(&cache);
    fetch_simple(&cache, NOTO_EMOJI_FILE, NOTO_EMOJI_URL);

    println!("cargo:rustc-env=FONT_CACHE_DIR={}", cache.display());
}

fn need_download(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(m) => m.len() == 0,
        Err(_) => true,
    }
}

fn http_get(url: &str) -> Vec<u8> {
    // The Inter zip is ~28MB; ureq's per-call read timeout has to cover the
    // whole download. We build an Agent with a generous read_timeout instead
    // of relying on the default which can fire mid-stream on slow links.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(30))
        .timeout_read(std::time::Duration::from_secs(600))
        .build();
    let resp = agent
        .get(url)
        .call()
        .unwrap_or_else(|e| panic!("download {url}: {e}"));
    let mut body = Vec::new();
    resp.into_reader()
        .read_to_end(&mut body)
        .unwrap_or_else(|e| panic!("read {url}: {e}"));
    body
}

fn fetch_simple(cache: &Path, name: &str, url: &str) {
    let dst = cache.join(name);
    if !need_download(&dst) {
        return;
    }
    eprintln!("[build.rs] downloading {name}");
    let bytes = http_get(url);
    std::fs::write(&dst, &bytes).unwrap_or_else(|e| panic!("write {name}: {e}"));
    eprintln!("[build.rs] {name}: {} bytes", bytes.len());
}

fn fetch_inter(cache: &Path) {
    let all_present = INTER_FILES
        .iter()
        .all(|(out_name, _)| !need_download(&cache.join(out_name)));
    if all_present {
        return;
    }

    eprintln!("[build.rs] downloading Inter release zip");
    let zip_bytes = http_get(INTER_RELEASE_URL);
    eprintln!("[build.rs] zip: {} bytes", zip_bytes.len());

    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).expect("parse Inter zip");

    for (out_name, in_path) in INTER_FILES {
        let mut entry = archive
            .by_name(in_path)
            .unwrap_or_else(|e| panic!("Inter zip missing {in_path}: {e}"));
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .unwrap_or_else(|e| panic!("read {in_path}: {e}"));
        let dst = cache.join(out_name);
        let mut f =
            std::fs::File::create(&dst).unwrap_or_else(|e| panic!("create {out_name}: {e}"));
        f.write_all(&bytes)
            .unwrap_or_else(|e| panic!("write {out_name}: {e}"));
        eprintln!("[build.rs] {out_name}: {} bytes", bytes.len());
    }
}
