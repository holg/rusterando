#!/usr/bin/env bash
# build_tab5_lab.sh — build, flash and watch crates/tab5-lab on an M5Stack Tab5
# (sample firmware for the Tab5 Keyboard, Module13.2 LAN and Module LoRaWAN-EU868).
#
#   scripts/build_tab5_lab.sh build      # compile (first run builds ESP-IDF, ~10 min)
#   scripts/build_tab5_lab.sh flash      # build + flash + restart, then show the log
#   scripts/build_tab5_lab.sh monitor    # just show the log (SECS=60, RESET=1 restarts first)
#
# Env: PORT (default: the first /dev/cu.usbmodem*), PROFILE=release,
# LAB_AUTORUN="rak server" (commands run at start, `;`-separated),
# PYTHON (default python3.12), ESPTOOL (default tmp/esptool-venv/bin/esptool).
#
# Flashing replaces the kitchen firmware on that Tab5: same partition table
# (crates/kds-tab5/partitions.csv), so nvs — Wi-Fi, kitchen login — stays.
# Back to the kitchen screen: `PROFILE=release scripts/build_kds_tab5.sh flash`.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="$ROOT_DIR/crates/tab5-lab"
CMD="${1:-build}"
PROFILE="${PROFILE:-debug}"
TARGET=riscv32imafc-esp-espidf
OUT="$CRATE/target/$TARGET/$PROFILE"

# ESP-IDF's installer runs whatever `python3` is first on PATH.
PYTHON="${PYTHON:-$(command -v python3.12 || true)}"
[ -n "$PYTHON" ] || { echo "python3.12 not found; set PYTHON=/path/to/python3.x (3.9–3.13)" >&2; exit 3; }
SHIM="$(mktemp -d)"
trap 'rm -rf "$SHIM"' EXIT
ln -s "$PYTHON" "$SHIM/python3"
ln -s "$PYTHON" "$SHIM/python"
export PATH="$SHIM:$HOME/.cargo/bin:$PATH"

# tmp/ (gitignored) lives in the main checkout; a git worktree has none.
MAIN_DIR="$(cd "$(git -C "$ROOT_DIR" rev-parse --path-format=absolute --git-common-dir)/.." && pwd)"
ESPTOOL="${ESPTOOL:-$MAIN_DIR/tmp/esptool-venv/bin/esptool}"
mkdir -p "$MAIN_DIR/tmp"
LOG="$MAIN_DIR/tmp/tab5-lab-flash.log"

# The phone terminal's key (src/remote.rs): LAB_KEY from tmp/tab5-wifi.env,
# made once if there is none. The same key goes into every Tab5 and is
# entered once in the Tab5 Terminal app. Read literally, never printed
# except when it is new.
KEY_ENV="${KEY_ENV:-$MAIN_DIR/tmp/tab5-wifi.env}"
if [ -z "${LAB_KEY:-}" ] && [ -f "$KEY_ENV" ]; then
    LAB_KEY="$(sed -n 's/^LAB_KEY=//p' "$KEY_ENV" | tail -1 | tr -d '\r')"
fi
if [ -z "${LAB_KEY:-}" ]; then
    LAB_KEY="$("$PYTHON" -I -c 'import secrets
a = "abcdefghijkmnpqrstuvwxyz23456789"
k = "".join(secrets.choice(a) for _ in range(16))
print("-".join(k[i:i + 4] for i in range(0, 16, 4)))')"
    printf 'LAB_KEY=%s\n' "$LAB_KEY" >>"$KEY_ENV"
    echo "  new phone key, saved in $KEY_ENV: $LAB_KEY" >&2
    echo "  enter it once in the Tab5 Terminal app" >&2
fi
export LAB_KEY
# The shop server for the admin page's LoRa tab (src/report.rs): SHOP_URL
# from the same file, as for the kitchen firmware. No default: without it the
# LoRa log stays off (the phone terminal works all the same).
if [ -z "${SHOP_URL:-}" ] && [ -f "$KEY_ENV" ]; then
    SHOP_URL="$(sed -n 's/^SHOP_URL=//p' "$KEY_ENV" | tail -1 | tr -d '\r')"
fi
[ -n "${SHOP_URL:-}" ] && export SHOP_URL

build() {
    local flags=()
    [ "$PROFILE" = release ] && flags=(--release)
    (cd "$CRATE" && cargo build "${flags[@]}")
}

bootloader() {
    find "$OUT/build" -path "*esp-idf-sys*/out/build/bootloader/bootloader.bin" -print0 \
        | xargs -0 ls -t | head -1
}

port() {
    if [ -n "${PORT:-}" ]; then echo "$PORT"; return; fi
    ls /dev/cu.usbmodem* 2>/dev/null | head -1
}

# Print the serial log for SECS seconds, following the port across USB
# reconnects (it drops briefly while the chip restarts).
monitor() {
    "$(dirname "$ESPTOOL")/python" -I - "${SECS:-60}" <<'EOF'
import glob, sys, time, serial
end = time.time() + float(sys.argv[1])
while time.time() < end:
    ports = glob.glob("/dev/cu.usbmodem*")
    if not ports:
        time.sleep(0.3); continue
    try:
        with serial.Serial(ports[0], 115200, timeout=0.5) as s:
            while time.time() < end:
                sys.stdout.write(s.read(4096).decode(errors="replace")); sys.stdout.flush()
    except Exception:
        time.sleep(0.3)
EOF
}

case "$CMD" in
    build)
        build
        ;;
    flash)
        build
        BOOTLOADER="$(bootloader)"
        [ -n "$BOOTLOADER" ] || { echo "ESP-IDF bootloader.bin not found under $OUT/build" >&2; exit 5; }
        P="$(port)"
        [ -n "$P" ] || { echo "Tab5 not found on USB (switched on? cable seated?)" >&2; exit 4; }
        if lsof "$P" >/dev/null 2>&1; then
            echo "$P is open in another process — close it first:" >&2
            lsof "$P" >&2
            exit 9
        fi
        # Into ota_0 with the OTA selection erased: the device starts this
        # image (as build_kds_tab5.sh does for the kitchen firmware).
        espflash flash --port "$P" --non-interactive \
            --bootloader "$BOOTLOADER" \
            --partition-table "$ROOT_DIR/crates/kds-tab5/partitions.csv" \
            --target-app-partition ota_0 \
            --erase-parts otadata \
            --flash-size 16mb \
            "$OUT/tab5-lab" >"$LOG" 2>&1 \
            || { echo "espflash failed — see $LOG (for a flaky USB link: scripts/build_kds_tab5.sh has a slow fallback)" >&2; exit 6; }
        echo "  flashed with espflash, restarted over USB"
        monitor
        ;;
    monitor)
        if [ -n "${RESET:-}" ]; then
            "$ESPTOOL" --chip esp32p4 --port "$(port)" --before default-reset --after hard-reset \
                read-mac >"$LOG" 2>&1 || echo "USB restart failed — see $LOG" >&2
        fi
        monitor
        ;;
    *)
        echo "usage: $0 build|flash|monitor" >&2
        exit 2
        ;;
esac
