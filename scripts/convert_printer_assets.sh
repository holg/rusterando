#!/usr/bin/env bash
# convert_printer_assets.sh — turn theme source artwork into the
# pre-decoded 1-bit assets used by the printer + the admin preview.
#
# For each source PNG named  rusterando-printer-<theme>-<part>.png  this
# produces two siblings, both pure 1-bit, dithered, ≤384 px wide (the
# 80-mm thermal head is 384 dots):
#
#   rusterando-printer-<theme>-<part>.1bit.png   ← browser preview (web-served)
#   rusterando-printer-<theme>-<part>.1bit.bmp   ← embedded in the Pi binary
#                                                  (trivial to parse, no image crate)
#
# The browser shows EXACTLY what the Pi prints because both derive from
# the identical Floyd-Steinberg dither of the same source. Conversion is
# OFFLINE (run this when artwork changes) — nothing dithers at runtime,
# and the Pi never needs an image decoder.
#
# Source files are the *-src marker-free PNGs already in public/img.
# We deliberately do NOT treat the already-generated .1bit.* files as
# sources on a re-run (the glob excludes them).
#
# Usage:
#   ./scripts/convert_printer_assets.sh                 # all themes
#   ./scripts/convert_printer_assets.sh rando           # one theme
#   WIDTH=512 ./scripts/convert_printer_assets.sh       # override target width

set -euo pipefail

THEME_FILTER="${1:-}"
WIDTH="${WIDTH:-384}"
IMG_DIR="public/img"

# Resolve to repo root.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

if ! command -v magick >/dev/null 2>&1; then
  echo "ImageMagick (magick) not found. brew install imagemagick" >&2
  exit 1
fi

# Canonical source assets per theme. Add a line here when a theme gains
# a part. We list them explicitly rather than globbing so stray draft
# files in public/img (older e-car variants, photos) are never picked up.
#   <theme>:<part-filename-without-extension>
SOURCES=(
  # Brand mark now has two payment variants — printed on the RIGHT of the
  # combined header band so the kitchen reads PAID vs UNPAID at a glance
  # alongside the channel icon on the left.
  "rando:rusterando-printer-rando-brand-mark-paid"
  "rando:rusterando-printer-rando-brand-mark-unpaid"
  "rando:rusterando-printer-rando-pickup-rust"
  "rando:rusterando-printer-rando-delivery-e-car"
)

count=0
for entry in "${SOURCES[@]}"; do
  theme="${entry%%:*}"
  base="${entry#*:}"
  if [[ -n "$THEME_FILTER" && "$theme" != "$THEME_FILTER" ]]; then
    continue
  fi
  src="$IMG_DIR/${base}.png"
  if [[ ! -f "$src" ]]; then
    echo "  ! source missing, skipping: $src" >&2
    continue
  fi

  out_png="$IMG_DIR/${base}.1bit.png"
  out_bmp="$IMG_DIR/${base}.1bit.bmp"

  echo "→ ${base}  (theme=${theme})"
  # Shared pipeline: fit to WIDTH (never upscale past it), flatten any
  # alpha onto white, grayscale, Floyd-Steinberg dither, hard 1-bit.
  common=( "$src"
    -background white -flatten
    -resize "${WIDTH}x>"
    -colorspace Gray
    -dither FloydSteinberg
    -monochrome )

  magick "${common[@]}" "$out_png"
  # BMP3 = uncompressed Windows bitmap, 1-bpp — a fixed header + packed
  # bits the Pi reads with ~20 lines, no zlib / no image crate.
  magick "${common[@]}" -define bmp:format=bmp3 "$out_bmp"

  dims="$(magick identify -format '%wx%h %z-bit' "$out_png" 2>/dev/null)"
  echo "   ${out_png##*/}  +  ${out_bmp##*/}   (${dims})"
  count=$((count + 1))
done

if [[ "$count" -eq 0 ]]; then
  echo "No source assets matched${THEME_FILTER:+ for theme '$THEME_FILTER'}."
  exit 0
fi
echo "Done: ${count} asset(s) converted."
