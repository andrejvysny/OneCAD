#!/usr/bin/env bash
# Assemble the native host with the exact worker manifest embedded at build time.
# Rewrite/sign the worker's dylib closure before hashing it; do not use codesign
# --deep afterward, because changing the worker invalidates that embedded hash.
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
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | bun -e 'console.log(JSON.parse(await Bun.stdin.text()).target_directory)')"
APP="${1:-$TARGET_DIR/release/bundle/macos/onecad.app}"
TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
STAGED="packaging/binaries/onecad-worker-$TRIPLE"
MANIFEST="packaging/binaries/onecad-worker-manifest.json"
: "${ONECAD_OCCT_ROOT:=$HOME/.onecad-occt/8.0.1}"
export ONECAD_OCCT_ROOT
scripts/build-worker.sh Release
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$STAGED" "$APP/Contents/MacOS/onecad-worker"
scripts/bundle-dylibs.sh "$APP"
cp "$APP/Contents/MacOS/onecad-worker" "$STAGED"
bun -e '
const [binaryPath, manifestPath] = process.argv.slice(1);
const manifest = JSON.parse(await Bun.file(manifestPath).text());
const binary = new Uint8Array(await Bun.file(binaryPath).arrayBuffer());
manifest.binarySha256 = new Bun.CryptoHasher("sha256").update(binary).digest("hex");
await Bun.write(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
' "$STAGED" "$MANIFEST"
cargo build --release -p onecad-ui
cp "$TARGET_DIR/release/onecad-ui" "$APP/Contents/MacOS/onecad"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>OneCAD</string>
<key>CFBundleDisplayName</key><string>OneCAD</string>
<key>CFBundleIdentifier</key><string>dev.onecad.native</string>
<key>CFBundleExecutable</key><string>onecad</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
codesign --force --sign - "$APP/Contents/MacOS/onecad"
codesign --force --sign - "$APP"
codesign --verify --strict --verbose=2 "$APP"
expected="$(bun -e 'console.log(JSON.parse(await Bun.file(process.argv[1]).text()).binarySha256)' "$MANIFEST")"
actual="$(shasum -a 256 "$APP/Contents/MacOS/onecad-worker" | awk '{print $1}')"
if [[ "$actual" != "$expected" ]]; then
    echo "bundled worker SHA-256 $actual != manifest $expected" >&2
    exit 3
fi
"$APP/Contents/MacOS/onecad-worker" --selftest
printf 'Native bundle verified: %s\n' "$APP"
