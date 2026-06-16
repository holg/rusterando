#!/usr/bin/env bash
# deploy_printer.sh — build + deploy the rusterando-printer binary to the
# Raspberry Pi that drives David's Epson TM-T20III thermal printer.
#
# Mirrors deploy_to_server.sh / dprestart.sh conventions: confirmation
# gate, post-restart health check, journal tail. The Pi runs the binary
# as the systemd unit `pizzeria-printer.service`; we NEVER invoke the
# binary by hand (a manual run wedges the single-open /dev/usb/lp0 and
# blocks the service). The deploy is always: install file → restart unit.
#
# What it does:
#   1. Cross-builds rusterando-printer for aarch64 (via
#      cross_build_on_mac.sh) unless --no-build is given.
#   2. Confirms the binary exists + shows size/mtime.
#   3. Confirms with the operator (y/N) unless --yes — this touches the
#      live shop's printer.
#   4. scp the binary to /tmp on the Pi, `install -m 755` it to
#      /usr/local/bin/pizzeria-printer.
#   5. `systemctl restart pizzeria-printer` — clean handoff of lp0.
#   6. Polls `systemctl is-active` (catches a crash-on-boot, e.g. bad
#      config or an old-server/new-Pi protocol skew) for up to 20 s.
#   7. Tails the unit journal for 5 s so you see it reconnect the tunnel
#      and reopen the printer device.
#
# SSH: uses the `davidspizzeria-pi` host alias from ~/.ssh/config (user,
# key, port all come from there). Override with PI_SSH_HOST=... env.
#
# Usage:
#   ./scripts/deploy_printer.sh                 # build + deploy, with confirm
#   ./scripts/deploy_printer.sh --yes           # skip the prompt
#   ./scripts/deploy_printer.sh --no-build      # deploy the already-built binary
#   ./scripts/deploy_printer.sh --build-only    # cross-build, do NOT deploy

set -u

# ----- argv parsing -----
ASSUME_YES=0
DO_BUILD=1
DO_DEPLOY=1
for a in "$@"; do
  case "$a" in
    --yes|-y)     ASSUME_YES=1 ;;
    --no-build)   DO_BUILD=0 ;;
    --build-only) DO_DEPLOY=0 ;;
    -h|--help)
      sed -n '2,45p' "$0" | sed 's|^# \{0,1\}||'
      exit 0
      ;;
    *) echo "unknown arg: $a (try --help)" >&2; exit 2 ;;
  esac
done

# ----- config -----
PI_SSH_HOST="${PI_SSH_HOST:-davidspizzeria-pi}"
TARGET_TRIPLE="aarch64-unknown-linux-gnu"
BUILT_BINARY="crates/rusterando-printer/target/${TARGET_TRIPLE}/release/rusterando-printer"
SERVICE="rusterando-printer"
REMOTE_TMP="/tmp/rusterando-printer.new"
REMOTE_BIN="/usr/local/bin/rusterando-printer"

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

# Resolve to the repo root so relative paths work no matter where this
# is invoked from.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

echo
say "rusterando-printer → ${PI_SSH_HOST}"

# ----- 1. build -----
if [ "$DO_BUILD" -eq 1 ]; then
  say "cross-building printer only for ${TARGET_TRIPLE}"
  # PRINTER_ONLY=1 → skip the full server/WASM leptos build; build just
  # the Pi printer binary (reusing the aarch64 cross-toolchain env).
  if ! PRINTER_ONLY=1 bash scripts/cross_build_on_mac.sh "$TARGET_TRIPLE"; then
    err "cross build failed — aborting (nothing deployed)"
    exit 3
  fi
  ok "build complete"
else
  warn "--no-build: using the already-built binary"
fi

# ----- 2. verify binary -----
if [ ! -f "$BUILT_BINARY" ]; then
  err "binary not found at $BUILT_BINARY"
  err "run without --no-build, or build first:"
  err "  bash scripts/cross_build_on_mac.sh ${TARGET_TRIPLE}"
  exit 4
fi
# Sanity: it should be an aarch64 ELF, not a stray macOS build.
if command -v file >/dev/null 2>&1; then
  FILETYPE="$(file -b "$BUILT_BINARY" 2>/dev/null)"
  case "$FILETYPE" in
    *aarch64*|*ARM\ aarch64*) ok "binary: aarch64 ELF" ;;
    *) warn "binary doesn't look like aarch64 ELF: $FILETYPE" ;;
  esac
fi
BIN_SIZE="$(du -h "$BUILT_BINARY" 2>/dev/null | cut -f1)"
BIN_MTIME="$(date -r "$BUILT_BINARY" '+%Y-%m-%d %H:%M' 2>/dev/null || echo '?')"
ok "binary: ${BUILT_BINARY} (${BIN_SIZE:-?}, built ${BIN_MTIME})"

if [ "$DO_DEPLOY" -eq 0 ]; then
  echo
  ok "--build-only: stopping before deploy"
  echo
  exit 0
fi

# ----- 3. reachability + current state -----
say "checking ${PI_SSH_HOST}"
if ! ssh -o ConnectTimeout=8 "$PI_SSH_HOST" true 2>/dev/null; then
  err "cannot reach ${PI_SSH_HOST} over SSH — check ~/.ssh/config + that the Pi is up"
  exit 5
fi
# Current service state + the binary it's running (so a no-op deploy is loud).
CUR_ACTIVE="$(ssh "$PI_SSH_HOST" "systemctl is-active ${SERVICE} 2>/dev/null" 2>/dev/null || echo unknown)"
CUR_VER="$(ssh "$PI_SSH_HOST" "${REMOTE_BIN} --version 2>/dev/null" 2>/dev/null || echo '?')"
ok "service ${SERVICE}: ${CUR_ACTIVE}"
ok "installed now: ${CUR_VER}"

# ----- 4. confirm -----
if [ "$ASSUME_YES" -eq 0 ]; then
  printf "\n  Deploy to ${C_BOLD}%s${C_RESET} and restart ${C_BOLD}%s${C_RESET}? [y/N] " \
    "$PI_SSH_HOST" "$SERVICE"
  read -r ANSWER
  case "$ANSWER" in
    y|Y|yes|YES) ;;
    *) err "aborted"; exit 1 ;;
  esac
fi

# ----- 5. upload + install + restart -----
say "uploading"
if ! scp -q "$BUILT_BINARY" "${PI_SSH_HOST}:${REMOTE_TMP}"; then
  err "scp failed"
  exit 6
fi
ok "uploaded to ${REMOTE_TMP}"

say "installing + restarting (clean handoff of /dev/usb/lp0)"
# install (atomic mv into place) THEN restart. Done as one ssh command so
# a dropped connection can't leave a half-installed state.
if ! ssh "$PI_SSH_HOST" "sudo install -m 755 ${REMOTE_TMP} ${REMOTE_BIN} \
    && sudo rm -f ${REMOTE_TMP} \
    && sudo systemctl restart ${SERVICE}"; then
  err "install/restart failed on the Pi — the OLD binary may still be running"
  err "check: ssh ${PI_SSH_HOST} 'sudo systemctl status ${SERVICE}'"
  exit 7
fi
ok "installed + restart issued"

# ----- 6. health check -----
say "waiting for ${SERVICE} to come back"
HEALTHY=0
for _ in $(seq 1 20); do
  STATE="$(ssh "$PI_SSH_HOST" "systemctl is-active ${SERVICE} 2>/dev/null" 2>/dev/null || echo unknown)"
  if [ "$STATE" = "active" ]; then
    HEALTHY=1
    break
  fi
  sleep 1
done
if [ "$HEALTHY" -eq 1 ]; then
  NEW_VER="$(ssh "$PI_SSH_HOST" "${REMOTE_BIN} --version 2>/dev/null" 2>/dev/null || echo '?')"
  ok "active — now running: ${NEW_VER}"
else
  err "service did not become active within 20 s — likely a crash on boot"
  err "(bad config, or a protocol skew if the SERVER is still on an older build)"
  err "logs: ssh ${PI_SSH_HOST} 'sudo journalctl -u ${SERVICE} -n 40 --no-pager'"
  exit 8
fi

# ----- 7. boot log -----
echo
say "boot log (next 5 s)"
ssh "$PI_SSH_HOST" "timeout 5 sudo journalctl -fu ${SERVICE} --no-pager 2>/dev/null" \
  | sed "s/^/  ${C_DIM}/" | sed "s/$/${C_RESET}/" || true

echo
ok "done"
echo
say "Tip: have David place one test order and confirm a receipt prints."
echo
