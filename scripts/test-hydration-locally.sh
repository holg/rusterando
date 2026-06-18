#!/usr/bin/env bash
# Local hydration regression check. Builds + serves the app, runs the
# Playwright hydration tests, tears down. Catches the tachys
# "entered unreachable code" panic before it ships, so we don't burn
# a 20-minute remote deploy cycle finding out the new code mismatches
# SSR and hydrate.
#
# Usage:
#     scripts/test-hydration-locally.sh
#     scripts/test-hydration-locally.sh --keep-running   # leave server up after tests
#
# Debug-mode hydration errors print the exact element location to the
# WASM console (release mode strips them to `unreachable!()`). The
# script captures the console output and fails on the first error,
# pointing you straight at the offending component.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

KEEP_RUNNING=false
if [[ "${1:-}" == "--keep-running" ]]; then
    KEEP_RUNNING=true
fi

SITE_PORT=3001
SITE_URL="http://127.0.0.1:${SITE_PORT}"
SERVER_LOG="/tmp/dp-local-server.log"
SERVER_PID=""

# Use the test admin password from .env if present; otherwise demand it.
if [[ -f .env ]]; then
    # shellcheck disable=SC1091
    set -a; source .env; set +a
fi

cleanup() {
    if [[ -n "$SERVER_PID" ]] && [[ "$KEEP_RUNNING" != "true" ]]; then
        echo "→ Stopping local server (PID $SERVER_PID)"
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    elif [[ "$KEEP_RUNNING" == "true" ]]; then
        echo ""
        echo "Server kept running on $SITE_URL (PID $SERVER_PID)."
        echo "Logs: $SERVER_LOG"
        echo "Stop with: kill $SERVER_PID"
    fi
}
trap cleanup EXIT

if ! lsof -i ":${SITE_PORT}" >/dev/null 2>&1; then
    echo "→ Building + starting local server (cargo leptos serve)…"
    # `cargo leptos serve` builds incrementally and runs. Background it
    # so we can poll for readiness.
    cargo leptos serve > "$SERVER_LOG" 2>&1 &
    SERVER_PID=$!

    # Unhashed (dev) builds: wasm-bindgen's JS glue hardcodes
    # `new URL('davidspizzeria_bg.wasm', import.meta.url)` and our
    # CachedHydrationScripts preload emits the same `_bg` name, but
    # cargo-leptos writes the binary as `davidspizzeria.wasm` (no _bg).
    # Bridge with a relative symlink, leptos-style. (Prod uses
    # hash-files=true, which rewrites the glue URL — no symlink needed.)
    # Poll briefly: the build writes pkg/ before the HTTP listener is up.
    ( for _ in $(seq 1 90); do
        if [[ -f target/site/pkg/davidspizzeria.wasm ]]; then
          ln -sfn davidspizzeria.wasm target/site/pkg/davidspizzeria_bg.wasm
          break
        fi
        sleep 1
      done ) &

    # Wait up to 90s for the server to start serving HTTP.
    for i in $(seq 1 90); do
        if curl -sf -o /dev/null "$SITE_URL/"; then
            echo "→ Server up after ${i}s"
            break
        fi
        if ! kill -0 "$SERVER_PID" 2>/dev/null; then
            echo "✗ Server process died. Last 30 lines of $SERVER_LOG:"
            tail -30 "$SERVER_LOG"
            exit 1
        fi
        sleep 1
    done
    if ! curl -sf -o /dev/null "$SITE_URL/"; then
        echo "✗ Server didn't start within 90s. Last 30 lines of $SERVER_LOG:"
        tail -30 "$SERVER_LOG"
        exit 1
    fi
else
    echo "→ Server already running on port $SITE_PORT — reusing it"
fi

# Pick the venv with pytest-playwright installed. Falls back to system
# python if the user has set up their own env.
PYTHON="${E2E_PYTHON:-$HOME/Documents/develeop/rust/geodb-rs/crates/geodb-py/.env_py312/bin/python}"
if [[ ! -x "$PYTHON" ]]; then
    PYTHON="$(command -v python3)"
fi
if ! "$PYTHON" -c "import pytest_playwright" 2>/dev/null; then
    echo "✗ pytest-playwright not available in $PYTHON"
    echo "  Install with: $PYTHON -m pip install pytest-playwright"
    echo "  Or set E2E_PYTHON to point at the venv that has it."
    exit 1
fi

echo "→ Running hydration tests against $SITE_URL"
cd tests/e2e
BASE_URL="$SITE_URL" "$PYTHON" -m pytest -v test_public_hydration.py "$@"
RC=$?

if [[ $RC -eq 0 ]]; then
    echo ""
    echo "✓ All hydration tests passed against local build."
else
    echo ""
    echo "✗ Hydration tests failed. Server log: $SERVER_LOG"
fi

exit $RC
