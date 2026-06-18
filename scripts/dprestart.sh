#!/usr/bin/env bash
# dprestart.sh — friendly wrapper around `systemctl restart` for the
# davidspizzeria / rusterando units. Read the whole thing before
# aliasing this to a keystroke: it WILL restart the service.
#
# What it does:
#   1. Picks the deployment ($1 or default davidspizzeria).
#   2. Snapshots the current pid + uptime so we can show the change.
#   3. Confirms with a y/N prompt unless --yes is passed.
#   4. `systemctl restart` — which sends SIGTERM. The binary's signal
#      handler (main.rs) catches it, closes the long-lived SSE streams
#      immediately, drains in-flight HTTP requests under a hard 5 s cap,
#      closes the sqlite pool cleanly (WAL flush), then exits 0. The fresh
#      process retries its bind for up to 8 s, so it slots in the instant
#      the port frees (no systemd restart bounce). Customer-visible 502
#      window: ~5 s (was >60 s before the SSE streams were taught to close
#      on SIGTERM — they used to pin the old process open until SIGKILL).
#   5. Polls `systemctl is-active` for up to 30 s, then a curl on
#      /api/healthz to confirm the new process is actually serving
#      requests (not just "active" per systemd's view).
#   6. Tails the new process's journal for 5 s so you see the boot
#      messages without having to switch sessions.
#
# Why this is safer than naked `systemctl restart`:
#   * Confirmation gate (--yes to skip in scripts).
#   * Pre/post pid comparison so a no-op silent failure is loud.
#   * Post-restart healthz curl proves the new process took over
#     before declaring victory. systemctl's "active" status reports
#     true the moment exec() succeeds, before axum has bound the
#     listener — a fresh-but-broken binary can show "active" and
#     still be useless. The healthz check catches that.
#
# Usage:
#   sudo ./dprestart.sh                   # davids, with confirm
#   sudo ./dprestart.sh rusterando        # rusterando, with confirm
#   sudo ./dprestart.sh davidspizzeria --yes   # skip the prompt

set -u

# ----- argv parsing -----
APP="davidspizzeria"
ASSUME_YES=0
for a in "$@"; do
  case "$a" in
    --yes|-y) ASSUME_YES=1 ;;
    -h|--help)
      sed -n '2,38p' "$0" | sed 's|^# \{0,1\}||'
      exit 0
      ;;
    *) APP="$a" ;;
  esac
done
SERVICE="${APP}-server"

# ----- colors (tty-only) -----
if [ -t 1 ]; then
  C_OK=$'\033[32m'; C_WARN=$'\033[33m'; C_ERR=$'\033[31m'
  C_DIM=$'\033[2m'; C_BOLD=$'\033[1m'; C_RESET=$'\033[0m'
else
  C_OK=''; C_WARN=''; C_ERR=''; C_DIM=''; C_BOLD=''; C_RESET=''
fi
say()  { printf "${C_BOLD}▌${C_RESET} %s\n" "$*"; }
ok()   { printf "  ${C_OK}✓${C_RESET} %s\n" "$*"; }
warn() { printf "  ${C_WARN}!${C_RESET} %s\n" "$*"; }
err()  { printf "  ${C_ERR}✗${C_RESET} %s\n" "$*" >&2; }

# ----- pre-flight -----
echo
say "$SERVICE restart"

# Service must exist; bail loud if the unit name was a typo.
if ! systemctl status "$SERVICE" --no-pager >/dev/null 2>&1; then
  err "service $SERVICE not found — typo? known: davidspizzeria, rusterando"
  exit 2
fi

OLD_PID=$(systemctl show "$SERVICE" --property=MainPID --value 2>/dev/null)
OLD_UPTIME=""
if [ -n "$OLD_PID" ] && [ "$OLD_PID" != "0" ] && [ -d "/proc/$OLD_PID" ]; then
  OLD_UPTIME=$(ps -o etime= -p "$OLD_PID" 2>/dev/null | tr -d ' ')
fi
ok "current: pid=${OLD_PID:-?} uptime=${OLD_UPTIME:-?}"

# Show traffic context so the operator can decide to defer a restart
# during peak service. Cheap query: count orders in the last hour.
DB="/var/www/${APP}.de/data/${APP}.db"
if command -v sqlite3 >/dev/null 2>&1 && [ -r "$DB" ]; then
  RECENT=$(sqlite3 "$DB" "SELECT COUNT(*) FROM orders WHERE created_at > datetime('now','-1 hour');" 2>/dev/null || echo "?")
  if [ "$RECENT" = "0" ]; then
    ok "0 orders in last hour — safe window"
  elif [ "$RECENT" -lt 5 ] 2>/dev/null; then
    warn "${RECENT} order(s) in last hour — restart is brief but visible"
  else
    warn "${RECENT} orders in last hour — busy period, consider waiting"
  fi
fi

# ----- confirm -----
if [ "$ASSUME_YES" -eq 0 ]; then
  printf "\n  Restart ${C_BOLD}%s${C_RESET}? [y/N] " "$SERVICE"
  read -r ANSWER
  case "$ANSWER" in
    y|Y|yes|YES) ;;
    *) err "aborted"; exit 1 ;;
  esac
fi

# ----- restart -----
echo
say "restarting"
# `systemctl restart` is synchronous — returns once the new process
# has exec()'d, which is BEFORE axum has finished binding. We poll
# below for the actual ready state.
RESTART_START=$(date +%s)
if ! systemctl restart "$SERVICE"; then
  err "systemctl restart failed"
  exit 3
fi

# ----- wait for the new pid to actually be serving -----
# Step 1: wait for systemd to report active again (catches restart
# loop scenarios where the new binary panics on boot — like the
# VersionMissing migration crash we hit 2026-05-30).
DEADLINE=$((SECONDS + 30))
while [ "$SECONDS" -lt "$DEADLINE" ]; do
  STATE=$(systemctl is-active "$SERVICE" 2>/dev/null || echo "unknown")
  if [ "$STATE" = "active" ]; then
    break
  fi
  sleep 0.5
done
if [ "$STATE" != "active" ]; then
  err "service did not become active within 30 s (state=$STATE) — check journal"
  exit 4
fi

NEW_PID=$(systemctl show "$SERVICE" --property=MainPID --value 2>/dev/null)
if [ "$NEW_PID" = "$OLD_PID" ] || [ "$NEW_PID" = "0" ]; then
  err "pid did not change ($NEW_PID) — something is wrong"
  exit 5
fi
ok "active: pid=${OLD_PID} -> ${NEW_PID}"

# Step 2: actually verify the new process serves requests. systemctl's
# "active" returns true as soon as exec() succeeds; the binary needs
# ~200-500 ms more to bind the port. The healthz endpoint is the
# cheapest probe.
HEALTH_DEADLINE=$((SECONDS + 10))
HEALTHZ_URL="http://127.0.0.1:$(systemctl show "$SERVICE" --property=Environment --value 2>/dev/null \
  | tr ' ' '\n' | grep '^LEPTOS_SITE_ADDR=' | sed 's|.*:||' | head -1)/api/healthz"
# If we couldn't parse the port out of the unit env, fall back to
# whatever the deployment normally uses.
case "$APP" in
  davidspizzeria) PORT_FALLBACK=3001 ;;
  rusterando)     PORT_FALLBACK=3002 ;;
  *)              PORT_FALLBACK=3001 ;;
esac
if ! echo "$HEALTHZ_URL" | grep -qE ':[0-9]+/'; then
  HEALTHZ_URL="http://127.0.0.1:${PORT_FALLBACK}/api/healthz"
fi
HEALTHZ_OK=0
while [ "$SECONDS" -lt "$HEALTH_DEADLINE" ]; do
  if curl -fsS --max-time 2 "$HEALTHZ_URL" >/dev/null 2>&1; then
    HEALTHZ_OK=1
    break
  fi
  sleep 0.3
done
ELAPSED=$(( $(date +%s) - RESTART_START ))
if [ "$HEALTHZ_OK" -eq 1 ]; then
  ok "/api/healthz responding (${ELAPSED}s total)"
else
  warn "/api/healthz still not responding after 10 s — check journal"
fi

# ----- show boot logs -----
echo
say "boot log (next 5 s)"
# `journalctl -fu` follows; cap at 5 s with `timeout` so the script
# returns to the shell promptly.
timeout 5 journalctl -fu "$SERVICE" --no-pager 2>/dev/null \
  | sed "s/^/  ${C_DIM}/" | sed "s/$/${C_RESET}/" \
  || true

echo
ok "done"
echo
