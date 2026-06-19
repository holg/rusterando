#!/usr/bin/env bash
# mt_local.sh — run the multi-tenant (Model B) binary locally with two demo
# tenants (demo + flizza) and exercise the routing. No nginx, no DNS, no certs.
#
# How Model B is triggered: the binary globs `.env*` in its WORKING DIR. ≥2
# files (and no MULTI_TENANT=0 / ENV_FILE pin) → multi-tenant. So we run from a
# scratch dir that contains ONLY the two tenant env files — the repo root has
# .env.davids etc. which we DON'T want to pull in.
#
# Subdomain on localhost: there is no real DNS. Two equivalent ways to pick a
# tenant without nginx:
#   1. The `X-Tenant: <slug>` header (what nginx sets in prod).  ← simplest
#   2. The Host header's first label: `Host: flizza.localhost`   ← "subdomain"
# `*.localhost` already resolves to 127.0.0.1 on macOS/Linux, so you can even
# hit http://flizza.localhost:PORT in a browser. curl examples below use both.
#
# Usage:  ./scripts/mt_local.sh [PORT]      (default 3009)
set -euo pipefail

PORT="${1:-3009}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
SITE="$REPO/target/site"
BIN="$REPO/target/debug/rusterando-server"
WORK="$REPO/tmp/mt_local"      # scratch working dir (gitignored ./tmp)

# --- prerequisites ---------------------------------------------------------
if [[ ! -d "$SITE/pkg" ]]; then
  echo "✗ $SITE/pkg missing — build the site first:"
  echo "    cargo leptos build"
  exit 1
fi
if [[ ! -x "$BIN" ]]; then
  echo "→ building server binary…"
  ( cd "$REPO" && cargo build -p rusterando-server )
fi

# --- fresh scratch working dir with exactly two tenant env files -----------
rm -rf "$WORK"; mkdir -p "$WORK/data"
cat > "$WORK/.env.demo"   <<EOF
DATABASE_URL=sqlite:./data/demo.db
EOF
cat > "$WORK/.env.flizza" <<EOF
DATABASE_URL=sqlite:./data/flizza.db
EOF

echo "Working dir: $WORK"
echo "Tenants: demo (data/demo.db), flizza (data/flizza.db)"
echo "Port: $PORT   ADMIN_TOKEN: localtoken"
echo

# --- boot --------------------------------------------------------------------
( cd "$WORK" && \
  ADMIN_TOKEN=localtoken \
  LEPTOS_SITE_ADDR="127.0.0.1:$PORT" \
  LEPTOS_SITE_ROOT="$SITE" \
  LEPTOS_OUTPUT_NAME=rusterando \
  RUST_LOG=info \
  "$BIN" ) &
SRV=$!
trap 'kill $SRV 2>/dev/null || true' EXIT
sleep 5

echo "=== MODE (expect multi-tenant) ==="
echo "(see server log above for: MODE: multi-tenant — 2 env files: .env.demo, .env.flizza)"
echo
echo "=== /__whoami via X-Tenant header ==="
curl -s -H "X-Tenant: demo"   "http://127.0.0.1:$PORT/__whoami"
echo "---"
curl -s -H "X-Tenant: flizza" "http://127.0.0.1:$PORT/__whoami"
echo
echo "=== /__whoami via SUBDOMAIN (Host header — *.localhost resolves to 127.0.0.1) ==="
curl -s -H "Host: flizza.localhost" "http://127.0.0.1:$PORT/__whoami"
echo
echo "=== a server fn per tenant (each hits its own DB) ==="
curl -s -o /dev/null -w 'demo   list_menu: %{http_code}\n' -H "X-Tenant: demo" \
  -X POST "http://127.0.0.1:$PORT/api/list_menu" -H 'Content-Type: application/x-www-form-urlencoded'
curl -s -o /dev/null -w 'flizza list_menu: %{http_code}\n' -H "X-Tenant: flizza" \
  -X POST "http://127.0.0.1:$PORT/api/list_menu" -H 'Content-Type: application/x-www-form-urlencoded'
curl -s -o /dev/null -w 'unknown -> %{http_code} (want 404)\n' -H "X-Tenant: nope" \
  -X POST "http://127.0.0.1:$PORT/api/list_menu" -H 'Content-Type: application/x-www-form-urlencoded'
echo
echo "=== hot-add a third tenant WITHOUT restarting ==="
echo 'DATABASE_URL=sqlite:./data/neu.db' > "$WORK/.env.neu"
curl -s -w ' [%{http_code}]\n' -H "x-admin-token: localtoken" \
  -X POST "http://127.0.0.1:$PORT/admin/tenant/neu/reload"
curl -s -o /dev/null -w 'neu now routes: %{http_code}\n' -H "X-Tenant: neu" \
  -X POST "http://127.0.0.1:$PORT/api/list_menu" -H 'Content-Type: application/x-www-form-urlencoded'
echo
echo "Server still running on http://127.0.0.1:$PORT  (Ctrl-C to stop)."
echo "Browser test: open  http://demo.localhost:$PORT/  and  http://flizza.localhost:$PORT/"
wait $SRV
