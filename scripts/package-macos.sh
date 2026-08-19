#!/usr/bin/env bash
# Build an INSTALLABLE macOS bundle — the whole two-pass dance, in one command.
#
# Usage: scripts/package-macos.sh [--install]
#          --install   also copy the finished bundle to /Applications
#
# ── Why two passes ───────────────────────────────────────────────────────────
# `build-worker.sh` writes a manifest binding the STAGED sidecar to its SHA-256,
# and a release build EMBEDS that manifest (`worker/manifest.rs`, `include_str!`
# of OUT_DIR) and refuses any sidecar whose bytes disagree
# (`verify_binary`, hard error, four restarts then "worker failed (no worker)").
#
# But making the bundle self-contained REWRITES the sidecar: `bundle-dylibs.sh`
# runs `install_name_tool` over it and re-signs it, so its bytes necessarily
# change after staging. A single-pass build therefore always ships an app that
# rejects its own worker — measured, not theorised: a bundle built straight from
# `docs/PACKAGING.md` launched and logged
#   `bundled worker SHA-256 mismatch: expected e1c6e1a3…, got 045fe7bd…`
# with no geometry backend at all.
#
# The order that works is the one `ci.yml`'s `tauri-composition` job already
# uses, and which existed nowhere else until this script:
#
#   1. build + stage the worker
#   2. build a SEED app
#   3. bundle-dylibs.sh the seed          ← this is what rewrites the sidecar
#   4. re-stage the BUNDLED sidecar and recompute the manifest from it
#   5. build the LOCKSTEP app             ← now the embedded manifest matches
#   6. restore the dylib closure, sign, verify
#
# ── Why `codesign --force --sign -` and never `--deep` ───────────────────────
# `--deep` re-signs nested Mach-Os, which changes the sidecar's bytes again and
# re-breaks the manifest. `bundle-dylibs.sh` has already signed every binary it
# touched; the outer signature is all that is left to apply.
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
    echo "package-macos.sh: macOS-only (uname=$(uname)); refusing to run." >&2
    exit 1
fi

INSTALL=0
if [[ ${1-} == "--install" ]]; then
    INSTALL=1
elif [[ $# -gt 0 ]]; then
    echo "usage: package-macos.sh [--install]" >&2
    exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

APP="src-tauri/target/release/bundle/macos/onecad.app"
MANIFEST="src-tauri/binaries/onecad-worker-manifest.json"
TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
STAGED="src-tauri/binaries/onecad-worker-$TRIPLE"
FRAMEWORKS_STASH="$(mktemp -d)/onecad-frameworks"
trap 'rm -rf "$(dirname "$FRAMEWORKS_STASH")"' EXIT

sha_of() { shasum -a 256 "$1" | awk '{print $1}'; }

echo "==> 1/6  worker (Release) + staged manifest"
: "${ONECAD_OCCT_ROOT:=$HOME/.onecad-occt/8.0.1}"
export ONECAD_OCCT_ROOT
scripts/build-worker.sh Release

echo "==> 2/6  seed bundle"
bun run tauri build --bundles app

echo "==> 3/6  fold the dylib closure in (rewrites + re-signs the sidecar)"
scripts/bundle-dylibs.sh "$APP"

echo "==> 4/6  re-stage the BUNDLED sidecar and recompute its manifest hash"
cp "$APP/Contents/MacOS/onecad-worker" "$STAGED"
chmod +x "$STAGED"
mkdir -p "$(dirname "$FRAMEWORKS_STASH")"
cp -R "$APP/Contents/Frameworks" "$FRAMEWORKS_STASH"
bun -e '
  const [binaryPath, manifestPath] = process.argv.slice(1);
  const manifest = JSON.parse(await Bun.file(manifestPath).text());
  const binary = new Uint8Array(await Bun.file(binaryPath).arrayBuffer());
  manifest.binarySha256 = new Bun.CryptoHasher("sha256").update(binary).digest("hex");
  await Bun.write(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
' "$STAGED" "$MANIFEST"

echo "==> 5/6  lockstep bundle (embeds the manifest that matches what ships)"
bun run tauri build --bundles app

echo "==> 6/6  restore the closure, sign, verify"
mkdir -p "$APP/Contents/Frameworks"
cp -R "$FRAMEWORKS_STASH/." "$APP/Contents/Frameworks/"
# NOT --deep. See the header.
codesign --force --sign - "$APP"
codesign --verify --strict --verbose=2 "$APP"

expected="$(bun -e 'console.log(JSON.parse(await Bun.file(process.argv[1]).text()).binarySha256)' "$MANIFEST")"
actual="$(sha_of "$APP/Contents/MacOS/onecad-worker")"
if [[ "$actual" != "$expected" ]]; then
    echo "package-macos.sh: bundled worker SHA-256 $actual != manifest $expected" >&2
    echo "The app would refuse its own sidecar. Do not ship this bundle." >&2
    exit 3
fi

# The selftest proves the bundled worker resolves its OCCT closure from
# Contents/Frameworks — `bundle-dylibs.sh` strips the build machine's own OCCT
# rpath, so a pass here cannot be borrowing the dev prefix.
"$APP/Contents/MacOS/onecad-worker" --selftest

echo "==> bundle OK: $APP"

if [[ $INSTALL -eq 1 ]]; then
    rm -rf /Applications/onecad.app
    cp -R "$APP" /Applications/onecad.app
    codesign --verify --strict /Applications/onecad.app
    echo "==> installed: /Applications/onecad.app"
    echo "    First launch: right-click → Open once (an ad-hoc signature is not"
    echo "    notarized, so Gatekeeper asks). See docs/PACKAGING.md § 4."
fi
