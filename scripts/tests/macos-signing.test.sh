#!/usr/bin/env bash
# Exercise signing policy and digest order without invoking Apple signing services.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/onecad-signing-test.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
export TEST_PROJECT="$SCRATCH/project" TEST_LOG="$SCRATCH/calls" TEST_IDENTITIES="$SCRATCH/identities"
mkdir -p "$TEST_PROJECT/scripts" "$TEST_PROJECT/packaging/macos" "$SCRATCH/bin" \
    "$TEST_PROJECT/assets/native/brand/macos" "$TEST_PROJECT/packaging/binaries" "$SCRATCH/source"
cp "$ROOT/scripts/"{package-macos.sh,bundle-dylibs.sh,macos-signing.sh,macos-minimum.sh} "$TEST_PROJECT/scripts/"
cp "$ROOT/packaging/macos/host-entitlements.plist" "$TEST_PROJECT/packaging/macos/"
touch "$TEST_PROJECT/assets/native/brand/macos/OneCAD.icns" "$TEST_IDENTITIES" "$TEST_LOG"
printf 'mock dylib\n' > "$SCRATCH/source/libOCCT.dylib"
export TEST_DYLIB="$SCRATCH/source/libOCCT.dylib"

cat > "$TEST_PROJECT/scripts/build-worker.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
echo BUILD_WORKER >> "$TEST_LOG"
printf '#!/bin/sh\nexit 0\n' > packaging/binaries/onecad-worker-aarch64-apple-darwin
chmod +x packaging/binaries/onecad-worker-aarch64-apple-darwin
SH
cat > "$TEST_PROJECT/scripts/build-addons.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
mkdir -p "$ONECAD_ADDON_STAGING_DIR/mock"
printf '{}\n' > "$ONECAD_ADDON_STAGING_DIR/mock/addon.json"
SH
cat > "$SCRATCH/bin/uname" <<'SH'
#!/bin/sh
echo Darwin
SH
cat > "$SCRATCH/bin/rustc" <<'SH'
#!/bin/sh
echo 'host: aarch64-apple-darwin'
SH
cat > "$SCRATCH/bin/security" <<'SH'
#!/bin/sh
cat "$TEST_IDENTITIES"
SH
cat > "$SCRATCH/bin/cargo" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case "$*" in
    'run --quiet -p xtask -- target-directory') echo "$TEST_PROJECT/target" ;;
    'run --quiet -p xtask -- rehash-worker-manifest '*)
        echo REHASH >> "$TEST_LOG"
        shift 6
        shasum -a 256 "$1" | awk '{print $1}' > "$2" ;;
    'run --quiet -p xtask -- manifest-hash '*) shift 6; cat "$1" ;;
    'build --release -p onecad-ui')
        echo BUILD_HOST >> "$TEST_LOG"
        mkdir -p "$TEST_PROJECT/target/release"
        printf '#!/bin/sh\n# embedded worker SHA: %s\nexit 0\n' \
            "$(cat "$TEST_PROJECT/packaging/binaries/onecad-worker-manifest.json")" \
            > "$TEST_PROJECT/target/release/onecad-ui" ;;
    *) echo "unexpected mock cargo: $*" >&2; exit 1 ;;
esac
SH
cat > "$SCRATCH/bin/otool" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == -l ]]; then
    printf 'cmd LC_RPATH\npath @executable_path/../Frameworks (offset 12)\n'
elif [[ "$2" == */onecad-worker ]]; then
    printf '%s:\n' "$2"
    if grep -q '^# rewritten$' "$2"; then
        printf '\t@rpath/libOCCT.dylib (compatibility version 1.0.0)\n'
    else
        printf '\t%s (compatibility version 1.0.0)\n' "$TEST_DYLIB"
    fi
else
    printf '%s:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n' "$2"
fi
SH
cat > "$SCRATCH/bin/install_name_tool" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" != -change ]] || printf '# rewritten\n' >> "${!#}"
SH
cat > "$SCRATCH/bin/file" <<'SH'
#!/bin/sh
case "$2" in
    */Contents/MacOS/* | *.dylib) echo 'Mach-O 64-bit arm64' ;;
    *) echo 'ASCII text' ;;
esac
SH
cat > "$SCRATCH/bin/xcrun" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
    lipo) exit 0 ;;
    vtool)
        printf 'SCAN\t%s\n' "${!#}" >> "$TEST_LOG"
        if [[ "${TEST_INVALID_BUILD_METADATA:-0}" == 1 && "${!#}" == *.dylib ]]; then
            printf 'cmd LC_BUILD_VERSION\nplatform MACOS\n'; exit 0
        fi
        case "${!#}" in
            */onecad) minimum=11.0 ;;
            */onecad-worker) minimum=15.0 ;;
            *) minimum=26.0 ;;
        esac
        printf 'cmd LC_BUILD_VERSION\nplatform MACOS\nminos %s\n' "$minimum" ;;
    *) exit 1 ;;
esac
SH
cat > "$SCRATCH/bin/codesign" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == --force ]]; then
    printf 'SIGN' >> "$TEST_LOG"
    printf '\t%s' "$@" >> "$TEST_LOG"
    printf '\n' >> "$TEST_LOG"
    [[ ! -f "${!#}" ]] || printf '# signed\n' >> "${!#}"
fi
SH
chmod +x "$SCRATCH/bin/"* "$TEST_PROJECT/scripts/"*.sh
export PATH="$SCRATCH/bin:$PATH"
unset ONECAD_SIGNING_IDENTITY

assert_order() {
    awk -F '\t' '
        $1 == "SIGN" && $NF ~ /libOCCT.dylib$/ { dylib = NR }
        $1 == "SIGN" && $NF ~ /onecad-worker$/ { worker = NR }
        $1 == "REHASH" { rehash = NR }
        $1 == "BUILD_HOST" { build = NR }
        $1 == "SCAN" { scan = NR }
        $1 == "SIGN" && $NF ~ /\/onecad$/ { host = NR }
        $1 == "SIGN" && $NF ~ /\.app$/ { app = NR }
        END { exit !(dylib < worker && worker < rehash && rehash < build && build < scan && scan < host && host < app) }
    ' "$TEST_LOG"
}

check_package() {
    local app="$1"
    "$TEST_PROJECT/scripts/package-macos.sh" "$app" > "$SCRATCH/output" 2>&1
    assert_order
    local digest
    digest="$(shasum -a 256 "$app/Contents/MacOS/onecad-worker" | awk '{print $1}')"
    grep -qF "embedded worker SHA: $digest" "$app/Contents/MacOS/onecad"
    grep -qF '<key>LSMinimumSystemVersion</key><string>26.0.0</string>' "$app/Contents/Info.plist" || {
        echo 'packaged minimum missing or below bundled dylib minimum' >&2; return 1
    }
    if grep -q -- --deep "$TEST_LOG"; then
        echo 'recursive signing unexpectedly enabled' >&2; exit 1
    fi
}

check_package "$SCRATCH/local app.app"
if grep -qE -- '--timestamp|--options|--entitlements' "$TEST_LOG"; then
    echo 'ad-hoc default unexpectedly enables distribution signing flags' >&2; exit 1
fi
"$TEST_PROJECT/scripts/bundle-dylibs.sh" "$SCRATCH/local app.app" > "$SCRATCH/rerun" 2>&1
grep -q 'existing dylib closure and worker signed' "$SCRATCH/rerun"

printf '%s\n' '  1) AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA "Apple Development: Test (TEAM)"' > "$TEST_IDENTITIES"
: > "$TEST_LOG"
if ONECAD_SIGNING_IDENTITY='Apple Development: Test (TEAM)' \
    "$TEST_PROJECT/scripts/package-macos.sh" "$SCRATCH/refused.app" > "$SCRATCH/refused" 2>&1; then
    echo 'Apple Development identity unexpectedly accepted' >&2; exit 1
fi
[[ ! -s "$TEST_LOG" && ! -e "$SCRATCH/refused.app" ]]
grep -q 'Developer ID Application identity not found' "$SCRATCH/refused"

printf '%s\n' \
    '  1) AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA "Developer ID Application: Test (TEAM)"' \
    '  2) BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB "Developer ID Application: Test (TEAM)"' > "$TEST_IDENTITIES"
if ONECAD_SIGNING_IDENTITY='Developer ID Application: Test (TEAM)' \
    bash -c 'source "$TEST_PROJECT/scripts/macos-signing.sh"; onecad_signing_init' > "$SCRATCH/ambiguous" 2>&1; then
    echo 'ambiguous Developer ID identity unexpectedly accepted' >&2; exit 1
fi

printf '%s\n' '  1) AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA "Developer ID Application: Test (TEAM)"' > "$TEST_IDENTITIES"
ONECAD_SIGNING_IDENTITY='Developer ID Application: Test (TEAM)' check_package "$SCRATCH/developer.app"
awk -F '\t' '
    $1 == "SIGN" {
        if (!index($0, "--timestamp") || !index($0, "--options\truntime")) exit 1
        host = ($NF ~ /\/onecad$/ || $NF ~ /\.app$/)
        if (host != (index($0, "--entitlements") > 0)) exit 1
        count++
    }
    END { if (count != 4) exit 1 }
' "$TEST_LOG"
ONECAD_SIGNING_IDENTITY=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA \
    bash -c 'source "$TEST_PROJECT/scripts/macos-signing.sh"; onecad_signing_init; [[ "$ONECAD_DEVELOPER_ID" == 1 ]]'
bash -u -c 'unset ONECAD_SIGNING_IDENTITY; source "$TEST_PROJECT/scripts/macos-signing.sh";
    onecad_signing_init; onecad_signing_init;
    [[ "$ONECAD_DEVELOPER_ID" == 0 && "${ONECAD_CODESIGN_ARGS[*]}" == "--force --sign -" ]]'
: > "$TEST_LOG"
if TEST_INVALID_BUILD_METADATA=1 "$TEST_PROJECT/scripts/package-macos.sh" \
    "$SCRATCH/invalid-minimum.app" > "$SCRATCH/invalid-minimum" 2>&1; then
    echo 'missing dylib minimum unexpectedly packaged' >&2; exit 1
fi
grep -q 'invalid or ambiguous macOS build version' "$SCRATCH/invalid-minimum"
if awk -F '\t' '$1 == "SIGN" && ($NF ~ /\/onecad$/ || $NF ~ /\.app$/) { found = 1 }
    END { exit !found }' "$TEST_LOG"; then
    echo 'invalid build minimum reached host or outer app signing' >&2; exit 1
fi
printf 'macOS signing mocks: policy, signing order, final worker digest, reuse and preflight passed\n'
