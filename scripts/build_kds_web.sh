#!/usr/bin/env bash
# build_kds_web.sh — the kitchen display's browser build (crates/kds-web):
# the board's Slint screen as WASM, served at /kitchen/display.
#
#   cargo wasm32 → wasm-pack (wasm-bindgen matching the lockfile, wasm-opt)
#   → <site_root>/pkg/kds/{kds_web.js, kds_web_bg.wasm, gong.wav, hash.txt}
#   → brotli/gzip sidecars.
#
# Run AFTER `cargo leptos build` (cross_build_on_mac.sh does): leptos hashes
# whatever already sits in site/pkg. The page asks for ?v=<hash.txt>, so the
# immutable /pkg/ cache still picks up every new build.
#
# Usage: scripts/build_kds_web.sh [SITE_ROOT]   (default target/site)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
SITE_ROOT="${1:-target/site}"
OUT="$ROOT_DIR/$SITE_ROOT/pkg/kds"

command -v wasm-pack >/dev/null || { echo "wasm-pack missing: cargo install wasm-pack" >&2; exit 3; }
rustup target list --installed | grep -q '^wasm32-unknown-unknown$' || rustup target add wasm32-unknown-unknown

echo "=== kds-web → $OUT ==="
rm -rf "$OUT"
wasm-pack build crates/kds-web --release --target web --no-pack --no-typescript --out-dir "$OUT"
cp crates/rusterando-printer/assets/sounds/gong.wav "$OUT/gong.wav"   # the board's gong
rm -f "$OUT/.gitignore"

cat "$OUT/kds_web.js" "$OUT/kds_web_bg.wasm" | shasum -a 256 | cut -c1-12 > "$OUT/hash.txt"
for f in "$OUT"/*.wasm "$OUT"/*.js; do
  if command -v brotli >/dev/null 2>&1; then brotli -f -q 11 -k "$f"; fi
  if command -v gzip >/dev/null 2>&1; then gzip -9 -k -f "$f"; fi
done
ls -la "$OUT" | awk 'NR>1 {print $5, $9}'
echo "hash $(cat "$OUT/hash.txt")"
