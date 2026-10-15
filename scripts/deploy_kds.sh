#!/usr/bin/env bash
# deploy_kds.sh — build + deploy the kitchen display terminal to the
# STM32MP135F-DK: rusterando-printer built with `--features kds-kms`
# (printer client + board + Slint touch UI on the 480×272 screen).
#
# Same conventions as deploy_printer.sh: confirmation gate, install the
# binary, restart the unit, health check, journal tail. Never run the
# binary by hand on the board — the unit owns the screen (DRM master) and
# the printer.
#
# What it does:
#   1. Cross-builds for armv7-unknown-linux-gnueabihf with the messense
#      toolchain (glibc 2.28, older than the board's 2.35), linking the
#      board's own libinput / xkbcommon / udev. Those .so files are copied
#      once from the board into tmp/mp135-sysroot (needs the board up).
#   2. Confirms (y/N) unless --yes.
#   3. Installs the binary to /usr/local/bin/rusterando-printer and the
#      systemd units + udev rule from systemd/kds/ (first deploy: --setup).
#   4. Restarts rusterando-kds and tails its journal.
#
# Board access: ssh alias `davidspizzeria-kds` (root), override with
# KDS_SSH_HOST=root@192.168.178.151.
#
# Usage:
#   ./scripts/deploy_kds.sh                 # build + deploy, with confirm
#   ./scripts/deploy_kds.sh --setup         # also install units/udev/env (first time)
#   ./scripts/deploy_kds.sh --yes           # skip the prompt
#   ./scripts/deploy_kds.sh --no-build      # deploy the already-built binary
#   ./scripts/deploy_kds.sh --build-only    # cross-build, do NOT deploy
#   ./scripts/deploy_kds.sh --refresh-sysroot  # re-copy the board's libs

set -u

ASSUME_YES=0
DO_BUILD=1
DO_DEPLOY=1
DO_SETUP=0
REFRESH_SYSROOT=0
for a in "$@"; do
  case "$a" in
    --yes|-y)          ASSUME_YES=1 ;;
    --no-build)        DO_BUILD=0 ;;
    --build-only)      DO_DEPLOY=0 ;;
    --setup)           DO_SETUP=1 ;;
    --refresh-sysroot) REFRESH_SYSROOT=1 ;;
    -h|--help)
      sed -n '2,33p' "$0" | sed 's|^# \{0,1\}||'
      exit 0
      ;;
    *) echo "unknown arg: $a (try --help)" >&2; exit 2 ;;
  esac
done

KDS_SSH_HOST="${KDS_SSH_HOST:-davidspizzeria-kds}"
TARGET_TRIPLE="armv7-unknown-linux-gnueabihf"
CROSS_PREFIX="armv7-unknown-linux-gnueabihf"
MANIFEST="crates/rusterando-printer/Cargo.toml"
BUILT_BINARY="crates/rusterando-printer/target/${TARGET_TRIPLE}/release/rusterando-printer"
SERVICE="rusterando-kds"
REMOTE_TMP="/tmp/rusterando-printer.new"
REMOTE_BIN="/usr/local/bin/rusterando-printer"

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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
SYSROOT="$REPO_ROOT/tmp/mp135-sysroot"

# Board libraries the display build links against (plus what they pull
# in, for -rpath-link). Copied from the board so the sonames match exactly.
BOARD_LIBS="usr/lib/libinput.so.10* usr/lib/libxkbcommon.so.0* lib/libudev.so.1*
  usr/lib/libfontconfig.so.1* usr/lib/libfreetype.so.6* usr/lib/libexpat.so.1*
  usr/lib/libpng16.so.16* usr/lib/libmtdev.so.1* usr/lib/libevdev.so.2*
  usr/lib/libgudev-1.0.so.0* usr/lib/libglib-2.0.so.0* usr/lib/libgobject-2.0.so.0*
  usr/lib/libffi.so* usr/lib/libpcre*.so* lib/libz.so.1*"

prepare_sysroot() {
  if [ "$REFRESH_SYSROOT" -eq 1 ] || [ ! -e "$SYSROOT/usr/lib/libinput.so.10" ] \
      || [ ! -e "$SYSROOT/usr/lib/libfontconfig.so.1" ]; then
    say "copying board libraries from ${KDS_SSH_HOST} into tmp/mp135-sysroot"
    mkdir -p "$SYSROOT"
    # shellcheck disable=SC2086
    if ! ssh -o ConnectTimeout=8 "$KDS_SSH_HOST" "cd / && tar cf - $BOARD_LIBS 2>/dev/null" \
        | tar xf - -C "$SYSROOT"; then
      err "could not copy libraries from ${KDS_SSH_HOST} (board up? ssh alias?)"
      exit 3
    fi
  fi
  # Unversioned link names + pkg-config files, generated each run (cheap).
  local lib="$SYSROOT/usr/lib"
  mkdir -p "$lib/pkgconfig"
  ln -sf libinput.so.10 "$lib/libinput.so"
  ln -sf libxkbcommon.so.0 "$lib/libxkbcommon.so"
  ln -sf ../../lib/libudev.so.1 "$lib/libudev.so"
  ln -sf libfontconfig.so.1 "$lib/libfontconfig.so"
  for pc in libudev:udev libinput:input xkbcommon:xkbcommon fontconfig:fontconfig; do
    cat > "$lib/pkgconfig/${pc%%:*}.pc" <<EOF
libdir=$lib
Name: ${pc%%:*}
Description: board copy (deploy_kds.sh)
Version: 0
Libs: -L\${libdir} -l${pc##*:}
Cflags:
EOF
  done
  ok "sysroot ready ($(du -sh "$SYSROOT" | cut -f1))"
}

echo
say "rusterando kitchen display → ${KDS_SSH_HOST}"

# ----- 1. build -----
if [ "$DO_BUILD" -eq 1 ]; then
  if ! command -v "${CROSS_PREFIX}-gcc" >/dev/null 2>&1; then
    err "${CROSS_PREFIX}-gcc not found. Install with:"
    err "  brew install messense/macos-cross-toolchains/${CROSS_PREFIX}"
    exit 3
  fi
  rustup target list --installed | grep -q "^${TARGET_TRIPLE}$" \
    || rustup target add "$TARGET_TRIPLE"
  prepare_sysroot

  say "cross-building rusterando-printer --features kds-kms for ${TARGET_TRIPLE}"
  export CC_armv7_unknown_linux_gnueabihf="${CROSS_PREFIX}-gcc"
  export CXX_armv7_unknown_linux_gnueabihf="${CROSS_PREFIX}-g++"
  export AR_armv7_unknown_linux_gnueabihf="${CROSS_PREFIX}-ar"
  export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER="${CROSS_PREFIX}-gcc"
  # The board libs were built against glibc 2.35; we link against the
  # toolchain's 2.28. Their newer symbol versions resolve on the board at
  # runtime, so don't make ld check them here.
  export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_RUSTFLAGS="-L $SYSROOT/usr/lib -L $SYSROOT/lib -C link-arg=-Wl,-rpath-link,$SYSROOT/usr/lib:$SYSROOT/lib -C link-arg=-Wl,--allow-shlib-undefined"
  export PKG_CONFIG_ALLOW_CROSS=1
  export PKG_CONFIG_PATH=""
  export PKG_CONFIG_LIBDIR="$SYSROOT/usr/lib/pkgconfig"
  if ! cargo build --release --manifest-path "$MANIFEST" \
      --target "$TARGET_TRIPLE" --features kds-kms; then
    err "cross build failed — aborting (nothing deployed)"
    exit 3
  fi
  ok "build complete"
else
  warn "--no-build: using the already-built binary"
fi

# ----- 2. verify binary -----
if [ ! -f "$BUILT_BINARY" ]; then
  err "binary not found at $BUILT_BINARY (run without --no-build)"
  exit 4
fi
FILETYPE="$(file -b "$BUILT_BINARY" 2>/dev/null)"
case "$FILETYPE" in
  *ARM,*EABI5*) ok "binary: 32-bit ARM ELF" ;;
  *) warn "binary doesn't look like an armv7 ELF: $FILETYPE" ;;
esac
BIN_SIZE="$(du -h "$BUILT_BINARY" | cut -f1)"
ok "binary: ${BUILT_BINARY} (${BIN_SIZE})"

if [ "$DO_DEPLOY" -eq 0 ]; then
  echo
  ok "--build-only: stopping before deploy"
  exit 0
fi

# ----- 3. reachability -----
say "checking ${KDS_SSH_HOST}"
if ! ssh -o ConnectTimeout=8 "$KDS_SSH_HOST" true 2>/dev/null; then
  err "cannot reach ${KDS_SSH_HOST} over SSH"
  exit 5
fi
CUR_ACTIVE="$(ssh "$KDS_SSH_HOST" "systemctl is-active ${SERVICE} 2>/dev/null" 2>/dev/null || true)"
CUR_VER="$(ssh "$KDS_SSH_HOST" "${REMOTE_BIN} --version 2>/dev/null" 2>/dev/null || echo '-')"
ok "service ${SERVICE}: ${CUR_ACTIVE:-not installed}"
ok "installed now: ${CUR_VER}"

# ----- 4. confirm -----
if [ "$ASSUME_YES" -eq 0 ]; then
  printf "\n  Deploy to ${C_BOLD}%s${C_RESET} and restart ${C_BOLD}%s${C_RESET}? [y/N] " \
    "$KDS_SSH_HOST" "$SERVICE"
  read -r ANSWER
  case "$ANSWER" in
    y|Y|yes|YES) ;;
    *) warn "aborted"; exit 0 ;;
  esac
fi

# ----- 5. install -----
say "installing"
scp -q "$BUILT_BINARY" "${KDS_SSH_HOST}:${REMOTE_TMP}" || { err "scp failed"; exit 6; }
# The stock image has no /usr/local/bin.
ssh "$KDS_SSH_HOST" "mkdir -p ${REMOTE_BIN%/*} && install -m 755 ${REMOTE_TMP} ${REMOTE_BIN} && rm -f ${REMOTE_TMP}" \
  || { err "install failed"; exit 6; }
# Root's non-interactive PATH on the board is /usr/bin:/bin only.
ssh "$KDS_SSH_HOST" "ln -sf ${REMOTE_BIN} /usr/bin/rusterando-printer" || true
ok "binary installed at ${REMOTE_BIN}"

if [ "$DO_SETUP" -eq 1 ]; then
  say "installing units, udev rule, env template"
  scp -q systemd/kds/kamera "${KDS_SSH_HOST}:/usr/local/bin/kamera" \
    && ssh "$KDS_SSH_HOST" "chmod 755 /usr/local/bin/kamera && ln -sf /usr/local/bin/kamera /usr/bin/kamera"
  scp -q systemd/kds/rusterando-kds.service systemd/kds/rusterando-tunnel.service \
    systemd/kds/99-rusterando-printer.rules systemd/kds/printer.env.example \
    systemd/kds/tunnel.env.example systemd/kds/wait-online-any.conf \
    systemd/kds/50-kds-wifi-ap.network \
    systemd/kds/rusterando-gong.path systemd/kds/rusterando-gong.service \
    crates/rusterando-printer/assets/sounds/gong.wav \
    "${KDS_SSH_HOST}:/tmp/" || { err "scp units failed"; exit 6; }
  ssh "$KDS_SSH_HOST" '
    set -e
    install -m 0644 /tmp/rusterando-kds.service /tmp/rusterando-tunnel.service \
      /tmp/rusterando-gong.path /tmp/rusterando-gong.service /etc/systemd/system/
    # New-order gong: the sound, and PipeWire as a lingering session of root
    # (the Weston session that normally starts it is disabled here).
    install -D -m 0644 /tmp/gong.wav /usr/local/share/rusterando/gong.wav
    loginctl enable-linger root
    XDG_RUNTIME_DIR=/run/user/0 systemctl --user enable --now pipewire.socket pipewire-pulse.socket wireplumber.service 2>/dev/null || true
    install -m 0644 /tmp/99-rusterando-printer.rules /etc/udev/rules.d/
    # Unplugged ports must not hold the boot ~2 min in networkd-wait-online.
    install -D -m 0644 /tmp/wait-online-any.conf /etc/systemd/system/systemd-networkd-wait-online.service.d/any.conf
    # Board WiFi AP addressing (192.168.72.1 + DHCP, no internet sharing).
    # hostapd itself is enabled by hand once /etc/hostapd.conf has its own
    # SSID + password (the image ships ST demo credentials).
    install -D -m 0644 /tmp/50-kds-wifi-ap.network /etc/systemd/network/50-kds-wifi-ap.network
    # Hardening: demo/debug services off (tcf-agent = unauthenticated remote
    # debugging), netdata local-only, SSH key-only — the latter only when a
    # login key is in place, so setup can never lock you out.
    systemctl disable --now tcf-agent rpcbind rpcbind.socket 2>/dev/null || true
    rm -f /etc/systemd/system/multi-user.target.wants/iiod.service /etc/systemd/system/iiod.service
    systemctl daemon-reload; systemctl stop iiod 2>/dev/null || true; systemctl mask iiod 2>/dev/null || true
    if [ -f /etc/netdata/netdata.conf ] && ! grep -q "bind to = 127.0.0.1" /etc/netdata/netdata.conf; then
      printf "\n[web]\n    bind to = 127.0.0.1\n" >> /etc/netdata/netdata.conf; systemctl restart netdata || true
    fi
    if [ -s /home/root/.ssh/authorized_keys ]; then
      sed -i "s/^DROPBEAR_EXTRA_ARGS=.*/DROPBEAR_EXTRA_ARGS=\"-s\"/" /etc/default/dropbear
    fi
    udevadm control --reload && udevadm trigger --subsystem-match=usb
    install -d -m 0755 /etc/rusterando-printer
    [ -f /etc/rusterando-printer/printer.env ] || install -m 0644 /tmp/printer.env.example /etc/rusterando-printer/printer.env
    [ -f /etc/rusterando-printer/tunnel.env ] || install -m 0644 /tmp/tunnel.env.example /etc/rusterando-printer/tunnel.env
    systemctl daemon-reload
    # The display owns the screen: Weston must not hold DRM master.
    systemctl disable --now weston-launch.service weston-graphical-session.service 2>/dev/null || true
    systemctl enable rusterando-kds.service rusterando-gong.path
    systemctl start rusterando-gong.path
  ' || { err "setup failed"; exit 6; }
  ok "units installed; weston disabled; ${SERVICE} enabled"
  warn "tunnel: put the key at /etc/rusterando-printer/tunnel_key, then"
  warn "  systemctl enable --now rusterando-tunnel  (see systemd/kds/README.md)"
fi

# ----- 6. restart + health -----
say "restarting ${SERVICE}"
ssh "$KDS_SSH_HOST" "systemctl restart ${SERVICE}" || { err "restart failed"; exit 7; }
for _ in $(seq 1 10); do
  sleep 2
  STATE="$(ssh "$KDS_SSH_HOST" "systemctl is-active ${SERVICE}" 2>/dev/null || true)"
  [ "$STATE" = "active" ] && break
done
if [ "$STATE" = "active" ]; then
  ok "${SERVICE} is active"
else
  err "${SERVICE} is ${STATE:-unknown} — journal follows"
fi
ssh "$KDS_SSH_HOST" "journalctl -u ${SERVICE} -n 25 --no-pager" 2>/dev/null | sed "s/^/  ${C_DIM}│${C_RESET} /"
echo
