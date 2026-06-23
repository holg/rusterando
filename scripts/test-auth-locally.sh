#!/usr/bin/env bash
# Local end-to-end check for the staff auth-heartbeat + two-way order
# messaging changes. Builds + serves the app from THIS checkout, runs the
# auth-status + order/admin/message Playwright tests against it, tears down.
#
# This is the gate before deploy: the auth tests assert behaviour that only
# the new code has (the live [Abmelden] heartbeat chip, dp_session cookie),
# so they MUST run against a local build of this branch — not against the
# still-old remote. Run this, see green, THEN test-ci-locally.sh, commit,
# push, deploy.
#
# Usage:
#     scripts/test-auth-locally.sh
#     scripts/test-auth-locally.sh --keep-running   # leave server up after
#     scripts/test-auth-locally.sh -k test_logout   # pass extra pytest args
#
# Mirrors scripts/test-hydration-locally.sh (same serve+symlink+poll dance).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

KEEP_RUNNING=false
EXTRA_ARGS=()
for arg in "$@"; do
    case "$arg" in
        --keep-running) KEEP_RUNNING=true ;;
        *) EXTRA_ARGS+=("$arg") ;;
    esac
done

SITE_PORT=3001
SITE_URL="http://127.0.0.1:${SITE_PORT}"
SERVER_LOG="/tmp/dp-local-auth-server.log"
SERVER_PID=""

# Load .env so ADMIN_PASSWORD (and the rest) are available to both the
# server and the tests.
if [[ -f .env ]]; then
    # shellcheck disable=SC1091
    set -a; source .env; set +a
fi

if [[ -z "${ADMIN_PASSWORD:-}" ]]; then
    echo "✗ ADMIN_PASSWORD not set (expected in .env). The auth tests log in"
    echo "  as admin, so this is required."
    exit 1
fi

cleanup() {
    if [[ -n "$SERVER_PID" ]] && [[ "$KEEP_RUNNING" != "true" ]]; then
        echo "→ Stopping local server (PID $SERVER_PID)"
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    elif [[ "$KEEP_RUNNING" == "true" ]]; then
        echo ""
        echo "Server kept running on $SITE_URL (PID $SERVER_PID). Logs: $SERVER_LOG"
        echo "Stop with: kill $SERVER_PID"
    fi
}
trap cleanup EXIT

if ! lsof -i ":${SITE_PORT}" >/dev/null 2>&1; then
    echo "→ Building + starting local server (cargo leptos serve)…"
    cargo leptos serve > "$SERVER_LOG" 2>&1 &
    SERVER_PID=$!

    # Dev (unhashed) builds: bridge wasm-bindgen's hardcoded `<out>_bg.wasm`
    # name to cargo-leptos's `<out>.wasm` with a symlink. The leptos output
    # name is `rusterando` (Cargo.toml [package.metadata.leptos].name), NOT
    # the old `davidspizzeria` — getting this wrong 404s the wasm and
    # silently breaks hydration (the page renders but never comes alive).
    ( for _ in $(seq 1 120); do
        if [[ -f target/site/pkg/rusterando.wasm ]]; then
          ln -sfn rusterando.wasm target/site/pkg/rusterando_bg.wasm
          break
        fi
        sleep 1
      done ) &

    for i in $(seq 1 120); do
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
        echo "✗ Server didn't start within 120s. Last 30 lines of $SERVER_LOG:"
        tail -30 "$SERVER_LOG"
        exit 1
    fi
else
    echo "→ Server already running on port $SITE_PORT — reusing it"
fi

PYTHON="${E2E_PYTHON:-$HOME/Documents/develeop/rust/geodb-rs/crates/geodb-py/.env_py312/bin/python}"
if [[ ! -x "$PYTHON" ]]; then
    PYTHON="$(command -v python3)"
fi
if ! "$PYTHON" -c "import pytest_playwright" 2>/dev/null; then
    echo "✗ pytest-playwright not available in $PYTHON"
    echo "  Set E2E_PYTHON to the venv that has it."
    exit 1
fi

echo "→ Running auth + messaging e2e against $SITE_URL"
cd tests/e2e
# Headless by default (conftest hard-defaults it); the order/messaging test
# self-opens the shop via the admin emergency button, so no manual setup.
BASE_URL="$SITE_URL" \
DPE2E_ADMIN_PW="$ADMIN_PASSWORD" \
"$PYTHON" -m pytest -v \
    test_auth_status.py \
    test_order_admin_message.py \
    "${EXTRA_ARGS[@]}"
RC=$?

if [[ $RC -eq 0 ]]; then
    echo ""
    echo "✓ Auth + messaging e2e passed against local build."
else
    echo ""
    echo "✗ Tests failed. Server log: $SERVER_LOG"
fi

exit $RC
