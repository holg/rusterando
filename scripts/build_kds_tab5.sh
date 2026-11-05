#!/usr/bin/env bash
# build_kds_tab5.sh — build, flash and watch crates/kds-tab5 on the M5Stack Tab5.
#
#   scripts/build_kds_tab5.sh build      # compile (first run fetches ESP-IDF, ~15 min)
#   scripts/build_kds_tab5.sh flash      # build + flash + restart, then show the log
#   scripts/build_kds_tab5.sh monitor    # just show the log (SECS=60, RESET=1 restarts first)
#   scripts/build_kds_tab5.sh stage      # release build → server, as the update for every Tab5
#
# BOARD=jc1060p470 builds the same firmware for the 7" Guition JC1060P470
# (ESP32-P4, 1024×600, Ethernet + Wi-Fi hotspot) instead: cargo feature
# `jc1060p470`, its board package and sdkconfig from
# crates/kds-tab5/boards/jc1060p470/, its own target dir
# (target/jc1060p470) and update slot (artifacts/printer/esp32p4-jc1060p470).
#
# Updates over the network (src/ota.rs): `stage` uploads the app image to
# <DEPLOY_REMOTE_BASE>/artifacts/printer/esp32p4/ on the server (SSH_HOST and
# DEPLOY_REMOTE_BASE from .env.davids, or STAGE_ENV=<file>). Then arm the
# update per device on /admin/drucker: the device fetches it over the print
# channel, checks it, restarts into it, and rolls back if it can't connect.
#
# Env: PORT (default: the first /dev/cu.usbmodem*), PROFILE=release,
# PYTHON (default python3.12 — ESP-IDF 5.4 doesn't support Python 3.14 yet),
# ESPTOOL (default tmp/esptool-venv/bin/esptool: python3.12 -m venv
# tmp/esptool-venv && tmp/esptool-venv/bin/pip install esptool).
#
# Flashing: espflash at full speed with the normal USB reset (~10 s). If that
# fails (SLOW=1 forces it), a fallback writes through esptool in 16 KB pieces,
# each verified on the chip and retried, and restarts with a watchdog reset.
# That needs the Tab5 in download mode first: hold reset until the green LED
# blinks. Needed when a firmware leaves the USB link flaky — restoring the
# factory backup brings it back (tmp/tab5-backup/README.md).
#
# The USB console only shows the boot after a USB-triggered restart (espflash,
# RESET=1); after the reset button or a watchdog reset the app runs but stays
# silent on USB. Check it over the network instead (ping its IP).
#
# Wi-Fi: WIFI_SSID/WIFI_PASS (+ optional WIFI2_* … WIFI5_*), the kitchen:
# KITCHEN_PASSWORD (+ optional SHOP_URL), from tmp/tab5-wifi.env (gitignored,
# KEY=VALUE lines), compiled into the image. Without KITCHEN_PASSWORD the
# screen runs on demo orders.
#
# Flashing replaces the Tab5 factory firmware. Restore it from the verified
# backup: tmp/tab5-backup/README.md.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="$ROOT_DIR/crates/kds-tab5"
CMD="${1:-build}"
PROFILE="${PROFILE:-debug}"
TARGET=riscv32imafc-esp-espidf
BOARD="${BOARD:-tab5}"
FEATURES=()
case "$BOARD" in
    tab5)
        TARGET_DIR="$CRATE/target"
        ARCH=esp32p4
        ;;
    jc1060p470)
        BOARD_DIR="$CRATE/boards/jc1060p470"
        TARGET_DIR="$CRATE/target/jc1060p470"
        ARCH=esp32p4-jc1060p470
        FEATURES=(--features jc1060p470)
        export CARGO_TARGET_DIR="$TARGET_DIR"
        # The 480x272 kitchen screen on the 1024x600 panel: 1024 / 480.
        export KDS_UI_SCALE=2.133
        # The shared settings, then the board's (later files win).
        export ESP_IDF_SDKCONFIG_DEFAULTS="$CRATE/sdkconfig.defaults;$BOARD_DIR/sdkconfig.defaults"
        # Replaces the Tab5's component list in Cargo.toml. esp-idf-sys wants
        # absolute paths in this file, hence generated per checkout.
        mkdir -p "$TARGET_DIR"
        sed "s|@BOARD_DIR@|$BOARD_DIR|g" "$BOARD_DIR/components.toml.in" >"$TARGET_DIR/components.toml"
        export ESP_IDF_SYS_EXTRA_COMPONENTS_FILE="$TARGET_DIR/components.toml"
        ;;
    *)
        echo "unknown BOARD=$BOARD (tab5 or jc1060p470)" >&2
        exit 2
        ;;
esac

# ESP-IDF's installer runs whatever `python3` is first on PATH.
PYTHON="${PYTHON:-$(command -v python3.12 || true)}"
[ -n "$PYTHON" ] || { echo "python3.12 not found; set PYTHON=/path/to/python3.x (3.9–3.13)" >&2; exit 3; }
SHIM="$(mktemp -d)"
trap 'rm -rf "$SHIM"' EXIT
ln -s "$PYTHON" "$SHIM/python3"
ln -s "$PYTHON" "$SHIM/python"
export PATH="$SHIM:$HOME/.cargo/bin:$PATH"

# Read the Wi-Fi file literally (no shell expansion: passwords may contain
# $, spaces or quotes). Values are never printed.
WIFI_ENV="${WIFI_ENV:-$ROOT_DIR/tmp/tab5-wifi.env}"
if [ -f "$WIFI_ENV" ]; then
    while IFS='=' read -r key value || [ -n "$key" ]; do
        value="${value%$'\r'}"
        case "$value" in \"*\") value="${value:1:${#value}-2}" ;; \'*\') value="${value:1:${#value}-2}" ;; esac
        case "$key" in
            WIFI_SSID|WIFI_PASS|WIFI[2-5]_SSID|WIFI[2-5]_PASS|KITCHEN_PASSWORD|SHOP_URL|KITCHEN_SLUG|HOTSPOT_SSID|HOTSPOT_PASS)
                export "$key=$value" ;;
        esac
    done < "$WIFI_ENV"
else
    echo "note: $WIFI_ENV not found — building without Wi-Fi credentials" >&2
fi

# The firmware's version (ota::VERSION): crate version + commit (+ -dirty)
# + build time, for every build (USB too). The server only offers a staged
# image whose build time is newer than the running one's — a Tab5 flashed
# over USB after the last staging is never "updated" back to it.
if [ -z "${KDS_VERSION:-}" ]; then
    PKGVER="$(sed -n 's/^version = "\(.*\)"/\1/p' "$CRATE/Cargo.toml" | head -1)"
    GITREV="$(git -C "$ROOT_DIR" rev-parse --short HEAD 2>/dev/null || echo nogit)"
    git -C "$ROOT_DIR" diff --quiet HEAD -- crates/kds-tab5 crates/kds-ui crates/receipt-escpos crates/kitchen-protocol \
        2>/dev/null || GITREV="$GITREV-dirty"
    KDS_VERSION="$PKGVER-$GITREV-$(date +%Y%m%d-%H%M)"
fi
export KDS_VERSION

build() {
    local flags=()
    [ "$PROFILE" = release ] && flags=(--release)
    (cd "$CRATE" && cargo build "${flags[@]}" "${FEATURES[@]}")
}

# Offset of a partition in partitions.csv, e.g. `part_offset ota_0`.
part_offset() {
    awk -F, -v n="$1" '$1 ~ "^"n"[ ]*$" { gsub(/ /, "", $4); print $4 }' "$CRATE/partitions.csv"
}

# The bootloader ESP-IDF built for this chip revision (not espflash's
# bundled one, which may target the newer v3.x silicon).
bootloader() {
    find "$TARGET_DIR/$TARGET/$PROFILE/build" -path "*esp-idf-sys*/out/build/bootloader/bootloader.bin" -print0 \
        | xargs -0 ls -t | head -1
}

# The app image via esptool (as ESP-IDF itself does), with the flash and
# chip-revision settings of this build: `app_image <bootloader> <out.bin>`.
app_image() {
    local sdkconfig
    sdkconfig="$(dirname "$(dirname "$(dirname "$1")")")/sdkconfig"
    cfg() { sed -n "s/^CONFIG_$1=\"\{0,1\}\([^\"]*\)\"\{0,1\}$/\1/p" "$sdkconfig"; }
    "$ESPTOOL" --chip esp32p4 elf2image \
        --flash-mode "$(cfg ESPTOOLPY_FLASHMODE)" \
        --flash-freq "$(cfg ESPTOOLPY_FLASHFREQ)" \
        --flash-size 16MB \
        --min-rev-full "$(cfg ESP32P4_REV_MIN_FULL)" \
        --max-rev-full "$(cfg ESP32P4_REV_MAX_FULL)" \
        --elf-sha256-offset 0xb0 \
        -o "$2" "$TARGET_DIR/$TARGET/$PROFILE/kds-tab5" >"$LOG" 2>&1 \
        || { echo "elf2image failed — see $LOG" >&2; exit 8; }
}

port() {
    if [ -n "${PORT:-}" ]; then echo "$PORT"; return; fi
    ls /dev/cu.usbmodem* 2>/dev/null | head -1
}

ESPTOOL="${ESPTOOL:-$ROOT_DIR/tmp/esptool-venv/bin/esptool}"
PIECE=16384
LOG="$ROOT_DIR/tmp/kds-tab5-flash.log"

# Write one piece and check it on the chip; retry when the link drops.
# A piece that already matches is skipped, so a rerun resumes where an
# interrupted flash stopped.
write_piece() {
    local addr="$1" file="$2" try
    "$ESPTOOL" --chip esp32p4 --port "$P" --before no-reset --after no-reset \
        verify-flash "$addr" "$file" >"$LOG" 2>&1 && return 0
    for try in 1 2 3 4 5 6 7 8; do
        if "$ESPTOOL" --chip esp32p4 --port "$P" --before no-reset --after no-reset \
                write-flash "$addr" "$file" >"$LOG" 2>&1 \
            && "$ESPTOOL" --chip esp32p4 --port "$P" --before no-reset --after no-reset \
                verify-flash "$addr" "$file" >>"$LOG" 2>&1; then
            return 0
        fi
        sleep 2
    done
    echo "write failed at $addr after $try tries — see $LOG" >&2
    return 1
}

# Flash `file` at `base` in PIECE-sized, sector-aligned pieces.
flash_image() {
    local base="$1" file="$2" dir i=0 piece
    dir="$(mktemp -d)"
    split -b "$PIECE" -a 4 "$file" "$dir/p_"
    for piece in "$dir"/p_*; do
        write_piece "$(printf '0x%x' $(( base + i * PIECE )))" "$piece" || { rm -rf "$dir"; return 1; }
        i=$(( i + 1 ))
    done
    rm -rf "$dir"
    echo "  $(basename "$file"): $i pieces at $(printf '0x%x' "$base"), verified"
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
        OUT="$TARGET_DIR/$TARGET/$PROFILE"
        BOOTLOADER="$(bootloader)"
        [ -n "$BOOTLOADER" ] || { echo "ESP-IDF bootloader.bin not found under $OUT/build" >&2; exit 5; }
        [ -x "$ESPTOOL" ] || { echo "esptool not found at $ESPTOOL (see header)" >&2; exit 6; }
        P="$(port)"
        [ -n "$P" ] || { echo "Tab5 not found on USB (switched on? cable seated?)" >&2; exit 4; }
        # A second reader on the port (a monitor left running) swallows the
        # chip's replies and every connect fails with "No serial data".
        if lsof "$P" >/dev/null 2>&1; then
            echo "$P is open in another process — close it first:" >&2
            lsof "$P" >&2
            exit 9
        fi
        # The app goes into ota_0 and the OTA selection (otadata) is erased:
        # after a flash over USB the device always starts the new image.
        if [ -z "${SLOW:-}" ] && espflash flash --port "$P" --non-interactive \
                --bootloader "$BOOTLOADER" \
                --partition-table "$CRATE/partitions.csv" \
                --target-app-partition ota_0 \
                --erase-parts otadata \
                --flash-size 16mb \
                "$OUT/kds-tab5" >"$LOG.espflash" 2>&1; then
            echo "  flashed with espflash, restarted over USB"
            monitor
            exit 0
        fi
        [ -n "${SLOW:-}" ] || echo "  espflash failed (see $LOG.espflash) — falling back to piece-by-piece" >&2
        IMG="$(mktemp -d)"
        espflash partition-table --to-binary "$CRATE/partitions.csv" -o "$IMG/partitions.bin" >/dev/null
        app_image "$BOOTLOADER" "$IMG/app.bin"
        # Is the chip in download mode? (One quiet connect, no reset.)
        # Judged by the MAC line: esptool sometimes reports a broken stream on
        # the way out although the chip answered fine.
        "$ESPTOOL" --chip esp32p4 --port "$P" --before no-reset --after no-reset \
            --connect-attempts 8 read-mac >"$LOG" 2>&1 || true
        grep -q "^MAC:" "$LOG" \
            || { echo "Tab5 not in download mode: hold reset until the green LED blinks, then rerun" >&2; exit 7; }
        flash_image 0x2000 "$BOOTLOADER"
        flash_image 0x8000 "$IMG/partitions.bin"
        "$ESPTOOL" --chip esp32p4 --port "$P" --before no-reset --after no-reset \
            erase-region "$(part_offset otadata)" 0x2000 >>"$LOG" 2>&1 \
            || { echo "erasing otadata failed — see $LOG" >&2; exit 10; }
        flash_image "$(part_offset ota_0)" "$IMG/app.bin"
        rm -rf "$IMG"
        "$ESPTOOL" --chip esp32p4 --port "$P" --before no-reset --after watchdog-reset read-mac >>"$LOG" 2>&1 \
            || echo "restart failed — press reset or power-cycle the Tab5" >&2
        monitor
        ;;
    monitor)
        if [ -n "${RESET:-}" ]; then
            # Into download mode and straight back out with an RTS reset: the
            # one restart after which the USB console shows the boot.
            "$ESPTOOL" --chip esp32p4 --port "$(port)" --before default-reset --after hard-reset \
                read-mac >"$LOG" 2>&1 || echo "USB restart failed — see $LOG" >&2
        fi
        monitor
        ;;
    stage)
        PROFILE=release
        build
        BOOTLOADER="$(bootloader)"
        [ -n "$BOOTLOADER" ] || { echo "ESP-IDF bootloader.bin not found" >&2; exit 5; }
        [ -x "$ESPTOOL" ] || { echo "esptool not found at $ESPTOOL (see header)" >&2; exit 6; }
        IMG="$(mktemp -d)"
        app_image "$BOOTLOADER" "$IMG/app.bin"
        # Server and site from the deploy env (only these two keys are read).
        STAGE_ENV="${STAGE_ENV:-$ROOT_DIR/.env.davids}"
        SSH_HOST="$(sed -n 's/^SSH_HOST=//p' "$STAGE_ENV" | tail -1)"
        BASE="$(sed -n 's/^DEPLOY_REMOTE_BASE=//p' "$STAGE_ENV" | tail -1)"
        [ -n "$SSH_HOST" ] && [ -n "$BASE" ] \
            || { echo "SSH_HOST / DEPLOY_REMOTE_BASE missing in $STAGE_ENV" >&2; exit 11; }
        DIR="$BASE/artifacts/printer/$ARCH"
        REMOTE_TMP="/tmp/kds-tab5-$KDS_VERSION.bin"
        scp -q "$IMG/app.bin" "$SSH_HOST:$REMOTE_TMP"
        # Version first, then the image swapped in atomically: the server
        # reloads when the image changes and reads the version with it.
        ssh "$SSH_HOST" "set -e
            sudo install -d -o www-data -g www-data '$DIR'
            printf '%s' '$KDS_VERSION' | sudo tee '$DIR/version.new' >/dev/null
            sudo mv '$DIR/version.new' '$DIR/version'
            sudo chown www-data:www-data '$DIR/version'
            sudo install -o www-data -g www-data -m 0644 '$REMOTE_TMP' '$DIR/rusterando-printer.new'
            sudo mv '$DIR/rusterando-printer.new' '$DIR/rusterando-printer'
            rm -f '$REMOTE_TMP'"
        echo "  staged $KDS_VERSION ($(wc -c <"$IMG/app.bin" | tr -d ' ') bytes) on $SSH_HOST:$DIR"
        echo "  arm the update per device on /admin/drucker"
        rm -rf "$IMG"
        ;;
    *)
        echo "usage: $0 build|flash|monitor|stage" >&2
        exit 2
        ;;
esac
