#!/usr/bin/env bash
# build-i18n-mgmt.sh — build the per-shop translation management .wasm.
#
# Thin wrapper over the generic scripts/build-wasm-split.sh. The mgmt wasm
# carries the i18n pack's menu COVERAGE (baked by its build.rs from the pack's
# generated/menu.json) and exposes gap_report() for the /admin/translations
# page. Run build-i18n-pack.sh (or the generator) first if the menu data
# changed, else this ships stale coverage.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "=== build-i18n-mgmt ==="
WS_CRATE="rusterando-i18n-mgmt" \
WS_WASM_NAME="rusterando_i18n_mgmt" \
WS_DIST="i18n-mgmt" \
WS_LOADER="$ROOT_DIR/crates/rusterando-frontend/src/static/i18n-mgmt-loader.js" \
    bash "$ROOT_DIR/scripts/build-wasm-split.sh"

# The generic split-wasm loader factory (shared by all split wasms) lives at
# /pkg/wasm-split-loader.js — ensure it's deployed.
cp "$ROOT_DIR/crates/rusterando-frontend/src/static/wasm-split-loader.js" \
   "$ROOT_DIR/target/site/pkg/wasm-split-loader.js"
command -v brotli &>/dev/null && brotli -f -q 11 "$ROOT_DIR/target/site/pkg/wasm-split-loader.js" || true
echo "  generic loader → /pkg/wasm-split-loader.js"
