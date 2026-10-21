#!/usr/bin/env bash
# Rebuild every generated brand PNG and .icns from the masters in assets/native/brand.
# Deterministic: rerunning must leave `git diff --exit-code` clean.
set -euo pipefail
cd "$(dirname "$0")/.."

BRAND=assets/native/brand
SVG=$BRAND/svg
SMALL=$BRAND/small
APP_ICONS=assets/native/app-icons
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

render() { rsvg-convert -w "$2" -h "$2" "$1" -o "$3"; }

# Sizes <=32 px use the design's simplified small-size drawing, shipped as PNGs.
icns() {
  local master=$1 small=$2 out=$3
  local set="$TMP/$(basename "$out" .icns).iconset"
  mkdir -p "$set"
  cp "$SMALL/${small}_16.png" "$set/icon_16x16.png"
  cp "$SMALL/${small}_32.png" "$set/icon_16x16@2x.png"
  cp "$SMALL/${small}_32.png" "$set/icon_32x32.png"
  render "$master" 64 "$set/icon_32x32@2x.png"
  for s in 128 256 512; do
    render "$master" "$s" "$set/icon_${s}x${s}.png"
    render "$master" $((s * 2)) "$set/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns "$set" -o "$out"
}

mkdir -p "$BRAND/macos" "$BRAND/png"
icns "$SVG/app-icon-macos-light.svg" icon "$BRAND/macos/OneCAD.icns"
icns "$SVG/document-icon.svg" doc "$BRAND/macos/OneCADDocument.icns"

# Start-screen mark: 28 pt well at 2x.
render "$SVG/symbol.svg" 56 "$BRAND/png/symbol-56.png"
render "$SVG/symbol-dark.svg" 56 "$BRAND/png/symbol-dark-56.png"

cp "$SMALL/icon_32.png" "$APP_ICONS/32x32.png"
for entry in master:1024 icon:512 128x128@2x:256 128x128:128 64x64:64 \
  Square310x310Logo:310 Square284x284Logo:284 Square150x150Logo:150 \
  Square142x142Logo:142 Square107x107Logo:107 Square89x89Logo:89 \
  Square71x71Logo:71 StoreLogo:50 Square44x44Logo:44 Square30x30Logo:30; do
  render "$SVG/app-icon-macos-light.svg" "${entry#*:}" "$APP_ICONS/${entry%%:*}.png"
done
