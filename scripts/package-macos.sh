#!/usr/bin/env bash
# Assemble the native host with the exact worker manifest embedded at build time.
# Rewrite/sign the worker's dylib closure before hashing it; do not use codesign
# --deep afterward, because changing the worker invalidates that embedded hash.
# ONECAD_SIGNING_IDENTITY defaults to "-" (local ad-hoc). Set an installed
# Developer ID Application certificate name or SHA-1 for distribution signing.
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
    echo "package-macos.sh: macOS-only" >&2
    exit 1
fi
if [[ $# -gt 1 ]]; then
    echo "usage: scripts/package-macos.sh [output.app]" >&2
    exit 2
fi
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
source "$ROOT/scripts/macos-signing.sh"
onecad_signing_init
TARGET_DIR="$(cargo run --quiet -p xtask -- target-directory)"
APP="${1:-$TARGET_DIR/release/bundle/macos/onecad.app}"
TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
STAGED="packaging/binaries/onecad-worker-$TRIPLE"
MANIFEST="packaging/binaries/onecad-worker-manifest.json"
: "${ONECAD_OCCT_ROOT:=$HOME/.onecad-occt/8.0.1}"
export ONECAD_OCCT_ROOT
scripts/build-worker.sh Release
ADDON_STAGING="$(mktemp -d "${TMPDIR:-/tmp}/onecad-bundled-addons.XXXXXX")"
trap 'rm -rf "$ADDON_STAGING"' EXIT
ONECAD_ADDON_STAGING_DIR="$ADDON_STAGING" scripts/build-addons.sh
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$STAGED" "$APP/Contents/MacOS/onecad-worker"
scripts/bundle-dylibs.sh "$APP"
cp "$APP/Contents/MacOS/onecad-worker" "$STAGED"
cargo run --quiet -p xtask -- rehash-worker-manifest "$STAGED" "$MANIFEST"
cargo build --release -p onecad-ui
cp "$TARGET_DIR/release/onecad-ui" "$APP/Contents/MacOS/onecad"
cp assets/native/brand/macos/OneCAD.icns "$APP/Contents/Resources/OneCAD.icns"
rm -rf "$APP/Contents/Resources/addons"
mkdir -p "$APP/Contents/Resources/addons"
cp -R "$ADDON_STAGING/." "$APP/Contents/Resources/addons/"
MINIMUM_MACOS="$(scripts/macos-minimum.sh "$APP")"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>OneCAD</string>
<key>CFBundleDisplayName</key><string>OneCAD</string>
<key>CFBundleIdentifier</key><string>dev.onecad.native</string>
<key>CFBundleExecutable</key><string>onecad</string>
<key>CFBundleIconFile</key><string>OneCAD</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>NSHighResolutionCapable</key><true/>
<key>LSMinimumSystemVersion</key><string>${MINIMUM_MACOS}</string>
</dict></plist>
PLIST
HOST_ENTITLEMENTS="$ROOT/packaging/macos/host-entitlements.plist"
onecad_sign_target "$APP/Contents/MacOS/onecad" "$HOST_ENTITLEMENTS"
onecad_sign_target "$APP" "$HOST_ENTITLEMENTS"
expected="$(cargo run --quiet -p xtask -- manifest-hash "$MANIFEST")"
actual="$(shasum -a 256 "$APP/Contents/MacOS/onecad-worker" | awk '{print $1}')"
if [[ "$actual" != "$expected" ]]; then
    echo "bundled worker SHA-256 $actual != manifest $expected" >&2
    exit 3
fi
"$APP/Contents/MacOS/onecad-worker" --selftest
printf 'Native bundle verified: %s\n' "$APP"
printf 'Bundle minimum macOS: %s (Mach-O build floor; OS qualification is separate)\n' "$MINIMUM_MACOS"
if [[ "$ONECAD_DEVELOPER_ID" == 1 ]]; then
    echo "Developer ID signing complete; notarization and stapling remain separate release steps."
else
    echo "Ad-hoc local bundle only; use a Developer ID Application identity for distribution."
fi
