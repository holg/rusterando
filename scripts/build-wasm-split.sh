#!/usr/bin/env bash
# build-wasm-split.sh — generic hand-split-wasm packager.
#
# The reusable shell half of the split-wasm pattern (the Rust half is the
# rusterando-wasm-split crate). Given a cdylib crate, it: cargo-builds it for
# wasm32 → wasm-bindgen --target web → wasm-opt -Oz → content-hashes the js +
# _bg.wasm → copies to hashed names → writes manifest.json → hashes a loader JS
# → brotli pre-compresses → drops everything in <site>/pkg/<dist>/.
#
# Hashed filenames are the cache-buster: a returning visitor whose cached hash
# matches fetches nothing. Mirrors the eulumdat-rs / gldf-rs build-wasm-split.sh.
#
# Required env (callers set these, then exec this):
#   WS_CRATE      cargo package name (e.g. rusterando-i18n-pack)
#   WS_WASM_NAME  cargo's underscored wasm name (e.g. rusterando_i18n_pack)
#   WS_DIST       /pkg subdir to serve from (e.g. i18n  →  /pkg/i18n/)
# Optional:
#   WS_LOADER     path to a loader .js to hash + copy alongside (stable +
#                 hashed names both written)
#   WS_EXTRA_MANIFEST   extra JSON key/values merged into manifest.json, e.g.
#                       '"locales":"en,it","topics":"shop,menu"'
#
# Used by scripts/build-i18n-pack.sh and scripts/build-i18n-mgmt.sh.
set -euo pipefail

: "${WS_CRATE:?set WS_CRATE}"
: "${WS_WASM_NAME:?set WS_WASM_NAME}"
: "${WS_DIST:?set WS_DIST}"
WS_LOADER="${WS_LOADER:-}"
WS_EXTRA_MANIFEST="${WS_EXTRA_MANIFEST:-}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

BINDGEN_OUT="$ROOT_DIR/target/${WS_DIST}-bindgen"
DIST_DIR="$ROOT_DIR/target/site/pkg/${WS_DIST}"

hash_md5() {
    if command -v md5sum &>/dev/null; then md5sum "$1" | cut -c1-16
    else md5 -q "$1" | cut -c1-16; fi
}
sed_inplace() {
    if [[ "$(uname)" == "Darwin" ]]; then sed -i '' "$@"; else sed -i "$@"; fi
}

echo "  [wasm-split] cargo build $WS_CRATE (wasm32, release)"
cargo build -p "$WS_CRATE" --target wasm32-unknown-unknown --release
RAW_WASM="$ROOT_DIR/target/wasm32-unknown-unknown/release/${WS_WASM_NAME}.wasm"
[[ -f "$RAW_WASM" ]] || { echo "wasm not found: $RAW_WASM" >&2; exit 1; }

command -v wasm-bindgen &>/dev/null || {
    echo "wasm-bindgen CLI not found. cargo install wasm-bindgen-cli --version 0.2.114" >&2
    exit 1; }
echo "  [wasm-split] wasm-bindgen --target web"
rm -rf "$BINDGEN_OUT"; mkdir -p "$BINDGEN_OUT"
wasm-bindgen --out-dir "$BINDGEN_OUT" --target web --no-typescript "$RAW_WASM"

if command -v wasm-opt &>/dev/null; then
    echo "  [wasm-split] wasm-opt -Oz"
    wasm-opt -Oz -o "$BINDGEN_OUT/${WS_WASM_NAME}_bg_opt.wasm" "$BINDGEN_OUT/${WS_WASM_NAME}_bg.wasm"
    mv "$BINDGEN_OUT/${WS_WASM_NAME}_bg_opt.wasm" "$BINDGEN_OUT/${WS_WASM_NAME}_bg.wasm"
fi

echo "  [wasm-split] hashing → $DIST_DIR"
JS_HASH="$(hash_md5 "$BINDGEN_OUT/${WS_WASM_NAME}.js")"
WASM_HASH="$(hash_md5 "$BINDGEN_OUT/${WS_WASM_NAME}_bg.wasm")"
mkdir -p "$DIST_DIR"
rm -f "$DIST_DIR/"*.js "$DIST_DIR/"*.wasm "$DIST_DIR/"*.br "$DIST_DIR/manifest.json"
JS_FILE="${WS_WASM_NAME}-${JS_HASH}.js"
WASM_FILE="${WS_WASM_NAME}-${WASM_HASH}_bg.wasm"
cp "$BINDGEN_OUT/${WS_WASM_NAME}.js"     "$DIST_DIR/$JS_FILE"
cp "$BINDGEN_OUT/${WS_WASM_NAME}_bg.wasm" "$DIST_DIR/$WASM_FILE"
sed_inplace "s/${WS_WASM_NAME}_bg.wasm/${WS_WASM_NAME}-${WASM_HASH}_bg.wasm/g" "$DIST_DIR/$JS_FILE"

EXTRA=""
[[ -n "$WS_EXTRA_MANIFEST" ]] && EXTRA=",$WS_EXTRA_MANIFEST"
cat > "$DIST_DIR/manifest.json" <<JSON
{"js":"$JS_FILE","wasm":"$WASM_FILE"$EXTRA}
JSON

if [[ -n "$WS_LOADER" && -f "$WS_LOADER" ]]; then
    LOADER_BASE="$(basename "$WS_LOADER" .js)"
    LOADER_HASH="$(hash_md5 "$WS_LOADER")"
    cp "$WS_LOADER" "$DIST_DIR/${LOADER_BASE}-${LOADER_HASH}.js"
    cp "$WS_LOADER" "$DIST_DIR/${LOADER_BASE}.js"
fi

if command -v brotli &>/dev/null; then
    echo "  [wasm-split] brotli -q11"
    for f in "$DIST_DIR"/*.js "$DIST_DIR"/*.wasm "$DIST_DIR"/manifest.json; do
        [[ -f "$f" ]] && brotli -f -q 11 "$f"
    done
fi

W="$DIST_DIR/$WASM_FILE"
echo "  [wasm-split] done → /pkg/${WS_DIST}/  (wasm $(ls -l "$W" | awk '{print $5}') B; .br $(ls -l "$W.br" 2>/dev/null | awk '{print $5}') B)"
