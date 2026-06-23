#!/usr/bin/env bash
# build-i18n-pack.sh — build the hand-split translation .wasm pack.
#
# The main hydrate bundle ships German only; all other-locale translations live
# in this separate, on-demand pack (see crates/rusterando-i18n-pack +
# feedback_i18n_db_first_wasm_overlay). This is a thin wrapper:
#   1. generate pack data (chrome.json/menu.json/hash.txt/topics.txt) for the
#      chosen profile via rusterando-scrape/gen_i18n_pack.py
#   2. hand off to scripts/build-wasm-split.sh (the generic packager: cargo
#      wasm32 → wasm-bindgen → wasm-opt → hash → manifest.json → loader → brotli)
#
# Hash-based caching is the point: a returning visitor whose cached hash still
# matches fetches NOTHING. Only a content change → new hash → re-fetch.
#
# Profiles + topics live in i18n-pack.toml (the topic manifest). A pack = the
# profile's LOCALES x TOPICS; topics map to chrome namespaces (shared/shop/
# kitchen) + the menu vocabulary. Product-agnostic so eulumdat/gldf can reuse it.
#   --profile fat     all 7 non-German locales, every topic            (default)
#   --profile small   the manifest's lean profile
#   --locales en,it   comma list (overrides the profile's locales)
#   --topics shop,menu   comma list (overrides; 'shared' topics auto-included)
#
# Usage:
#   scripts/build-i18n-pack.sh                          # fat
#   scripts/build-i18n-pack.sh --profile small
#   scripts/build-i18n-pack.sh --locales en,it --topics kitchen
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PROFILE="fat"
LOCALES=""
TOPICS=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --profile) PROFILE="$2"; shift 2 ;;
        --locales) LOCALES="$2"; shift 2 ;;
        --topics)  TOPICS="$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
done

GEN_ARGS=(--profile "$PROFILE")
[[ -n "$LOCALES" ]] && GEN_ARGS+=(--locales "$LOCALES")
[[ -n "$TOPICS" ]]  && GEN_ARGS+=(--topics "$TOPICS")

GEN_DIR="$ROOT_DIR/crates/rusterando-i18n-pack/generated"

echo "=== build-i18n-pack: profile=$PROFILE locales=${LOCALES:-<profile>} topics=${TOPICS:-<profile>} ==="

# --- 1. generate pack data -------------------------------------------------
echo "[1/2] generating pack data → $GEN_DIR"
python3 rusterando-scrape/gen_i18n_pack.py --out "$GEN_DIR" "${GEN_ARGS[@]}"

# Coverage to record in the pack's manifest.json (so the build is inspectable).
DATA_HASH="$(cat "$GEN_DIR/hash.txt" 2>/dev/null || echo dev)"
RESOLVED_TOPICS="$(cat "$GEN_DIR/topics.txt" 2>/dev/null || echo "")"
RESOLVED_LOCALES="$(python3 -c "import json; a=json.load(open('$GEN_DIR/chrome.json')); b=json.load(open('$GEN_DIR/menu.json')); print(','.join(sorted(set(a)|set(b))))" 2>/dev/null || echo "")"

# --- 2. package via the generic split-wasm builder -------------------------
echo "[2/2] packaging via build-wasm-split.sh"
WS_CRATE="rusterando-i18n-pack" \
WS_WASM_NAME="rusterando_i18n_pack" \
WS_DIST="i18n" \
WS_LOADER="$ROOT_DIR/crates/rusterando-frontend/src/static/i18n-loader.js" \
WS_EXTRA_MANIFEST="\"locales\":\"$RESOLVED_LOCALES\",\"topics\":\"$RESOLVED_TOPICS\",\"data_hash\":\"$DATA_HASH\"" \
    bash "$ROOT_DIR/scripts/build-wasm-split.sh"

# The generic split-wasm loader factory (shared by all split wasms).
cp "$ROOT_DIR/crates/rusterando-frontend/src/static/wasm-split-loader.js" \
   "$ROOT_DIR/target/site/pkg/wasm-split-loader.js"
command -v brotli &>/dev/null && brotli -f -q 11 "$ROOT_DIR/target/site/pkg/wasm-split-loader.js" || true

echo "  done. /pkg/i18n/ — locales=$RESOLVED_LOCALES topics=$RESOLVED_TOPICS data_hash=$DATA_HASH"
