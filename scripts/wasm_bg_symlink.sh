#!/usr/bin/env bash
# wasm_bg_symlink.sh — keep `<name>_bg.wasm` symlinked to `<name>.wasm`.
#
# WHY: with hash-files=false (dev), wasm-bindgen's JS glue + the hydration
# preload request `<name>_bg.wasm`, but cargo-leptos writes the binary as
# `<name>.wasm` (no _bg). The mismatch → 404 on the wasm → no hydration → admin
# inputs render empty / nothing is interactive. (Prod uses hash-files=true,
# which rewrites the glue URL — no symlink needed.) Known cargo-leptos issue.
#
# cargo leptos build/watch WIPES pkg/ on each rebuild, so a one-off symlink
# doesn't survive. Run this ALONGSIDE watch — it re-creates the symlink for
# WHATEVER <name>.wasm appears (works for any LEPTOS_OUTPUT_NAME: rusterando,
# davidspizzeria, …), polling once a second.
#
# Usage (two terminals):
#   ENV_FILE=.env.rusterando cargo leptos watch
#   ./scripts/wasm_bg_symlink.sh        # leave running
#
# Or one-shot after a build:
#   ./scripts/wasm_bg_symlink.sh --once
set -u

PKG_DIR="${PKG_DIR:-target/site/pkg}"
ONCE=0
[[ "${1:-}" == "--once" ]] && ONCE=1

link_once() {
    # Find the real (non-_bg) wasm bundle cargo-leptos wrote.
    local wasm
    wasm="$(ls "$PKG_DIR"/*.wasm 2>/dev/null | grep -v '_bg\.wasm$' | head -1)" || return 1
    [[ -z "$wasm" ]] && return 1
    local base bg
    base="$(basename "$wasm")"            # e.g. rusterando.wasm
    bg="${base%.wasm}_bg.wasm"            # e.g. rusterando_bg.wasm
    [[ "$base" == "$bg" ]] && return 0    # already _bg-named, nothing to do
    # Relative symlink (leptos-style), only if missing or stale.
    if [[ ! -L "$PKG_DIR/$bg" || "$(readlink "$PKG_DIR/$bg")" != "$base" ]]; then
        ln -sfn "$base" "$PKG_DIR/$bg"
        echo "linked $PKG_DIR/$bg → $base"
    fi
    return 0
}

if [[ "$ONCE" == 1 ]]; then
    link_once || { echo "no <name>.wasm in $PKG_DIR yet"; exit 1; }
    exit 0
fi

echo "watching $PKG_DIR for <name>.wasm → keeping <name>_bg.wasm symlinked (Ctrl-C to stop)…"
while true; do
    link_once || true
    sleep 1
done
