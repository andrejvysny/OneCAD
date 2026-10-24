#!/usr/bin/env bash
# Controlled tool outputs exercise refusal paths without rewriting real Mach-O code.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/onecad-minimum-test.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
APP="$SCRATCH/test app.app"
export MINIMUM_TEST_METADATA="$SCRATCH/metadata"
mkdir -p "$SCRATCH/bin" "$MINIMUM_TEST_METADATA" "$APP/Contents/MacOS" \
    "$APP/Contents/Frameworks" "$APP/Contents/Resources"

cat > "$SCRATCH/bin/file" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "${MINIMUM_TEST_FAILURE:-}" != file ]] || exit 71
[[ $# -eq 2 && "$1" == -b ]] || exit 72
if grep -qx MACHO "$2"; then echo 'Mach-O 64-bit executable arm64'; else echo 'ASCII text'; fi
SH
cat > "$SCRATCH/bin/xcrun" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
tool="$1"
shift
[[ "${MINIMUM_TEST_FAILURE:-}" != "$tool" ]] || exit 71
case "$tool" in
    lipo)
        [[ $# -eq 3 && "$2" == -verify_arch && "$3" == arm64 ]] || exit 72
        [[ "${MINIMUM_TEST_ARCH:-arm64}" == arm64 ]] ;;
    vtool)
        [[ $# -eq 4 && "$1" == -arch && "$2" == arm64 && "$3" == -show-build ]] || exit 72
        cat "$MINIMUM_TEST_METADATA/$(basename "$4")" ;;
    *) exit 72 ;;
esac
SH
chmod +x "$SCRATCH/bin/"*
export PATH="$SCRATCH/bin:$PATH"

metadata() {
    printf 'Load command 9\ncmd LC_BUILD_VERSION\nplatform %s\nminos %s\nsdk 26.5\n' \
        "${3:-MACOS}" "$2" > "$MINIMUM_TEST_METADATA/$1"
}

reset_fixture() {
    unset MINIMUM_TEST_FAILURE MINIMUM_TEST_ARCH
    printf 'MACHO\n' > "$APP/Contents/MacOS/onecad"
    printf 'MACHO\n' > "$APP/Contents/MacOS/onecad-worker"
    printf 'MACHO\n' > "$APP/Contents/Frameworks/libOCCT.dylib"
    printf 'ordinary resource\n' > "$APP/Contents/Resources/readme.txt"
    metadata onecad 11.0
    metadata onecad-worker 15.0
    metadata libOCCT.dylib 26.0
}

expect_minimum() {
    local actual
    actual="$("$ROOT/scripts/macos-minimum.sh" "$APP")"
    [[ "$actual" == "$1" ]] || { echo "expected $1, received $actual" >&2; exit 1; }
}

refusals=0
expect_refusal() {
    if "$ROOT/scripts/macos-minimum.sh" "$APP" > "$SCRATCH/output" 2>&1; then
        echo "unexpected acceptance: $1" >&2; exit 1
    fi
    grep -qF "$1" "$SCRATCH/output" || { cat "$SCRATCH/output" >&2; exit 1; }
    refusals=$((refusals + 1))
}

reset_fixture
worker_before="$(shasum -a 256 "$APP/Contents/MacOS/onecad-worker")"
expect_minimum 26.0.0
[[ "$worker_before" == "$(shasum -a 256 "$APP/Contents/MacOS/onecad-worker")" ]]
metadata onecad 26.10.3
metadata onecad-worker 26.9.255
metadata libOCCT.dylib 26.10.2
expect_minimum 26.10.3
metadata libOCCT.dylib 26.10.4
expect_minimum 26.10.4

reset_fixture
: > "$MINIMUM_TEST_METADATA/onecad-worker"
expect_refusal 'invalid or ambiguous macOS build version'
for invalid in 26 26.0beta 26.0.0.1 0.0 26.256; do
    metadata onecad-worker "$invalid"
    expect_refusal 'invalid or ambiguous macOS build version'
done
reset_fixture
printf 'minos 26.0\n' >> "$MINIMUM_TEST_METADATA/onecad-worker"
expect_refusal 'invalid or ambiguous macOS build version'
reset_fixture
printf 'cmd LC_BUILD_VERSION\nplatform MACOS\nminos 26.0\n' >> "$MINIMUM_TEST_METADATA/onecad-worker"
expect_refusal 'invalid or ambiguous macOS build version'
reset_fixture
printf 'cmd LC_BUILD_VERSION extra\nplatform MACOS\nminos 26.0\n' > "$MINIMUM_TEST_METADATA/onecad-worker"
expect_refusal 'invalid or ambiguous macOS build version'
reset_fixture
metadata libOCCT.dylib 18.0 IOSSIMULATOR
expect_refusal 'invalid or ambiguous macOS build version'
reset_fixture
export MINIMUM_TEST_ARCH=x86_64
expect_refusal 'missing arm64 slice'
for failed in file lipo vtool; do
    reset_fixture
    export MINIMUM_TEST_FAILURE="$failed"
    case "$failed" in
        file) expect_refusal 'cannot identify' ;;
        lipo) expect_refusal 'missing arm64 slice' ;;
        vtool) expect_refusal 'cannot inspect' ;;
    esac
done
reset_fixture
printf 'not code\n' > "$APP/Contents/Frameworks/libOCCT.dylib"
expect_refusal 'expected Mach-O code'
reset_fixture
printf 'not code\n' > "$APP/Contents/MacOS/onecad"
printf 'not code\n' > "$APP/Contents/MacOS/onecad-worker"
printf 'not code\n' > "$APP/Contents/Frameworks/libOCCT.dylib"
expect_refusal 'expected Mach-O code'
reset_fixture
ln -s ../MacOS/onecad "$APP/Contents/Resources/alias"
expect_refusal 'unsupported bundle symlink'
rm "$APP/Contents/Resources/alias" "$APP/Contents/MacOS/onecad"
expect_refusal 'missing onecad'
printf 'macOS minimum: 3 positive comparisons, %s refusal cases, unchanged worker passed\n' "$refusals"
