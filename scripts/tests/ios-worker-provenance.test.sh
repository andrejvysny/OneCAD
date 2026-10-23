#!/usr/bin/env bash
# Synthetic build tools exercise the real scripts; no kernel, Xcode build or network.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "${HERE}/../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
ROOT="${TMP}/repo"
mkdir -p "${ROOT}/scripts" "${ROOT}/platforms/ios/scripts" "${TMP}/bin" "${TMP}/sdk" "${TMP}/occt/share/onecad"
cp "${REPO}/scripts/"{build-ios-worker.sh,ios-worker-provenance.sh} "${ROOT}/scripts/"
cp "${REPO}/platforms/ios/scripts/build-rust.sh" "${ROOT}/platforms/ios/scripts/"
for path in worker/src/capi worker/cmake worker/third_party worker/tools/link_probe protocol crates/onecad-protocol crates/onecad-ios/include; do
    mkdir -p "${ROOT}/${path}"
    echo input >"${ROOT}/${path}/fixture"
done
for path in worker/CMakeLists.txt worker/src/capi/onecad_worker.h scripts/build-ios-occt.sh scripts/build-pinned-occt.sh; do
    echo input >"${ROOT}/${path}"
done
printf '#!/bin/sh\nexit 0\n' >"${ROOT}/scripts/check-worker-stdout-hygiene.sh"
chmod +x "${ROOT}/scripts/check-worker-stdout-hygiene.sh"
echo metadata >"${TMP}/occt/share/onecad/occt-build.cmake"
echo sdk >"${TMP}/sdk/SDKSettings.json"
mkdir -p "${TMP}/headers" "${TMP}/occt/lib"
echo dependency >"${TMP}/headers/header.h"
echo occt >"${TMP}/occt/lib/libTKFixture.a"
cat >"${TMP}/bin/tool" <<'TOOL'
#!/usr/bin/env bash
set -euo pipefail
case "${0##*/}" in
    xcrun)
        if [[ "${1:-}" == nm ]]; then exit 0; fi
        shift 2
        case "$1" in
            --show-sdk-path) echo "${FIXTURE}/sdk" ;;
            --show-sdk-version) echo "26.5" ;;
            --show-sdk-build-version) echo "fixture-sdk-build" ;;
            --find) echo "${FIXTURE}/bin/$2"; [[ "${FAIL_TOOL_LOOKUP:-0}" != 1 ]] ;;
            clang++) shift; "${FIXTURE}/bin/clang++" "$@" ;;
            *) exit 1 ;;
        esac ;;
    clang++)
        if [[ "${1:-}" == --version ]]; then echo fixture-clang; exit; fi
        [[ -z "${CPATH:-}${CPLUS_INCLUDE_PATH:-}${C_INCLUDE_PATH:-}${OBJC_INCLUDE_PATH:-}${LIBRARY_PATH:-}${COMPILER_PATH:-}${GCC_EXEC_PREFIX:-}" ]] || { echo 'unhashed compiler overrides leaked' >&2; exit 1; }
        while [[ $# -gt 0 ]]; do
            if [[ "$1" == -o ]]; then echo probe >"$2"; exit; fi
            shift
        done ;;
    cmake)
        if [[ "$1" == --version ]]; then echo fixture-cmake; exit; fi
        [[ -z "${CPATH:-}${CPLUS_INCLUDE_PATH:-}${C_INCLUDE_PATH:-}${OBJC_INCLUDE_PATH:-}${LIBRARY_PATH:-}${COMPILER_PATH:-}${GCC_EXEC_PREFIX:-}" ]] || { echo 'unhashed compiler overrides leaked' >&2; exit 1; }
        if [[ "$1" == --build ]]; then
            dir="$2"
            echo build >>"${FIXTURE}/builds"
            mkdir -p "${dir}/third_party/planegcs"
            echo worker >"${dir}/libonecad_worker_lib.a"
            echo solver >"${dir}/third_party/planegcs/libplanegcs.a"
            printf '# Object files:\n%s/lib/libTKFixture.a(member.o)\n# Sections:\n' "$ONECAD_IOS_OCCT_ROOT" >"${dir}/onecad_worker_link_probe.map"
            cat >"${dir}/onecad_worker_identity.txt" <<'IDENTITY'
abiVersion=1
protocolVersion=1
workerVersion=1.0
quantizationVersion=1
solverPolicyVersion=1
occtVersion=8.0.1
occtBuildId=fixture
fingerprintSeed=fixture
testHooks=0
IDENTITY
            if [[ "${MUTATE_DURING_BUILD:-0}" == 1 ]]; then echo changed >>"${FIXTURE}/repo/worker/src/capi/fixture"; fi
        fi ;;
    ninja) echo fixture-ninja ;;
    libtool)
        while [[ $# -gt 0 ]]; do
            if [[ "$1" == -o ]]; then printf 'bundle%s\n' "${WORKER_HOOK_LITERAL:-}" >"$2"; exit; fi
            shift
        done ;;
    otool)
        case "$1" in
            -L) printf '%s:\n\t/usr/lib/libc++.1.dylib (compatibility version 1.0.0)\n' "$2" ;;
            -l) printf 'cmd LC_BUILD_VERSION\nplatform IOSSIMULATOR\nminos 18.0\n' ;;
        esac ;;
    vtool|ld) : ;;
    *) exit 1 ;;
esac
TOOL
chmod +x "${TMP}/bin/tool"
for tool in xcrun clang++ cmake ninja libtool otool vtool ld; do ln -s tool "${TMP}/bin/${tool}"; done
FIXTURE_REAL_SHASUM="$(command -v shasum)"
FIXTURE_REAL_FIND="$(command -v find)"
export FIXTURE_REAL_SHASUM FIXTURE_REAL_FIND
cat >"${TMP}/bin/shasum" <<'HASH'
#!/usr/bin/env bash
"$FIXTURE_REAL_SHASUM" "$@" || exit $?
[[ "${FAIL_HASH:-0}" != 1 ]] || exit 7
HASH
cat >"${TMP}/bin/find" <<'FIND'
#!/usr/bin/env bash
"$FIXTURE_REAL_FIND" "$@" || exit $?
[[ "${FAIL_TRAVERSAL:-0}" != 1 ]] || exit 8
FIND
chmod +x "${TMP}/bin/shasum" "${TMP}/bin/find"
export FIXTURE="$TMP" PATH="${TMP}/bin:${PATH}"
export ONECAD_IOS_OCCT_ROOT="${TMP}/occt" ONECAD_HEADER_PREFIXES="${TMP}/headers"
export ONECAD_EMBEDDED_BUILD_DIR="${TMP}/build"
OUT="${TMP}/out"
BUILD="${ROOT}/scripts/build-ios-worker.sh"
run_builder() {
    "$BUILD" --platform iphonesimulator --config Debug --out "$OUT" --reuse-if-valid >"${TMP}/log" 2>&1
}
expect_rebuild() {
    run_builder
    grep -q 'rebuilding' "${TMP}/log"
    echo "ok: $1 rebuilds"
}
expect_rebuild 'missing provenance'
run_builder
grep -q 'verified embedded worker cache' "${TMP}/log"
[[ "$(wc -l <"${TMP}/builds" | tr -d ' ')" == 1 ]]
echo 'ok: unchanged cache reused without build'
cp "${OUT}/onecad-worker-provenance.txt" "${TMP}/valid-provenance"
for failure in FAIL_HASH=1 FAIL_TRAVERSAL=1 FAIL_TOOL_LOOKUP=1; do
    if (export "${failure?}"; run_builder); then
        echo "FAIL: ${failure} accepted" >&2; exit 1
    fi
    grep -q 'Cannot validate worker inputs' "${TMP}/log"
    if grep -q 'verified embedded worker cache' "${TMP}/log"; then exit 1; fi
    cmp "${TMP}/valid-provenance" "${OUT}/onecad-worker-provenance.txt"
    echo "ok: ${failure} fails before cache acceptance/publication"
done

echo source >>"${ROOT}/worker/src/capi/fixture"
expect_rebuild 'source content change'
echo future >"${ROOT}/worker/src/new-source.cpp"
expect_rebuild 'new source input'
echo protocol >>"${ROOT}/protocol/fixture"
expect_rebuild 'protocol change'
echo abi >>"${ROOT}/crates/onecad-ios/include/fixture"
expect_rebuild 'ABI change'
echo occt >>"${TMP}/occt/share/onecad/occt-build.cmake"
expect_rebuild 'dependency identity change'
echo headers >>"${TMP}/headers/header.h"
expect_rebuild 'dependency header change'
echo tampered >>"${OUT}/libonecad_worker_bundle.a"
expect_rebuild 'archive tamper'
echo tampered >>"${OUT}/onecad-embedded-kernel.json"
expect_rebuild 'identity tamper'
echo sdk >>"${TMP}/sdk/SDKSettings.json"
expect_rebuild 'SDK content change'
CXXFLAGS=-DFIXTURE expect_rebuild 'build options change'
CPATH=/unhashed CPLUS_INCLUDE_PATH=/unhashed C_INCLUDE_PATH=/unhashed OBJC_INCLUDE_PATH=/unhashed \
    LIBRARY_PATH=/unhashed COMPILER_PATH=/unhashed GCC_EXEC_PREFIX=/unhashed \
    expect_rebuild 'ambient compiler overrides ignored'

# Use the shared helper to isolate metadata axes without pretending fake tools
# qualify a different platform or acceptance hooks.
source "${ROOT}/scripts/ios-worker-provenance.sh"
if (set +o pipefail; export FAIL_HASH=1; provenance_tree "${ROOT}/worker/src" >/dev/null); then
    echo 'FAIL: failed source hash accepted without caller pipefail' >&2; exit 1
fi
if (export FAIL_HASH=1; provenance_sha256 "${OUT}/libonecad_worker_bundle.a" >/dev/null); then
    echo 'FAIL: failed archive hash accepted' >&2; exit 1
fi
echo 'ok: hash failures propagate inside conditionals without caller pipefail'
ROOT_DIR="$ROOT" PLATFORM=iphonesimulator CONFIG=Debug TEST_HOOKS=0
DEPLOYMENT_TARGET=18.0 SDK_PATH="${TMP}/sdk" SDK_VERSION=26.5 OCCT_VERSION=8.0.1 OCCT_ROOT="$ONECAD_IOS_OCCT_ROOT" OUT_DIR="$OUT"
base="$(provenance_collect "${TMP}/snapshot")"
for setting in PLATFORM=iphoneos CONFIG=Release TEST_HOOKS=1 DEPLOYMENT_TARGET=19.0 SDK_VERSION=27.0; do
    changed="$(export "${setting?}"; provenance_collect "${TMP}/snapshot")"
    [[ "$base" != "$changed" ]]
    echo "ok: ${setting} changes provenance"
done
echo rebuild-trigger >>"${ROOT}/worker/src/capi/fixture"
if MUTATE_DURING_BUILD=1 run_builder; then echo 'FAIL: changing build inputs accepted' >&2; exit 1; fi
grep -q 'Worker inputs changed during build' "${TMP}/log"
[[ ! -e "${OUT}/onecad-worker-provenance.txt" ]]
echo 'ok: mid-build mutation publishes no provenance'
if "$BUILD" --platform iphoneos --config Release --test-hooks --out "$OUT" >"${TMP}/log" 2>&1; then
    echo 'FAIL: Release worker hooks accepted' >&2; exit 1
fi
grep -q 'Release worker must not enable test hooks' "${TMP}/log"
echo 'ok: Release worker hooks refused'
for hook in __testSolverDelayMs __testSolverIgnoreCancel; do
    if WORKER_HOOK_LITERAL="$hook" "$BUILD" --platform iphonesimulator --config Release --out "$OUT" >"${TMP}/log" 2>&1; then
        echo "FAIL: Release worker contains ${hook}" >&2; exit 1
    fi
    grep -q 'production worker contains test-input/fault hooks' "${TMP}/log"
    [[ ! -e "${OUT}/onecad-worker-provenance.txt" ]]
    echo "ok: Release archive containing ${hook} refused"
done

mkdir -p "${TMP}/stubs"
echo app >"${TMP}/stubs/libonecad_ios.a"
echo worker >"${TMP}/stubs/libonecad_worker_bundle.a"
export PLATFORM_NAME=iphonesimulator ARCHS=arm64 SRCROOT="${ROOT}/platforms/ios"
export ONECAD_IOS_STUB_LIB_DIR="${TMP}/stubs" ONECAD_RUST_LIB_DIR="${TMP}/staged"
for CONFIGURATION in Release Acceptance; do
    export CONFIGURATION
    if "${ROOT}/platforms/ios/scripts/build-rust.sh" >"${TMP}/log" 2>&1; then echo "FAIL: ${CONFIGURATION} stub accepted" >&2; exit 1; fi
    grep -q "permitted only in explicit Debug builds; refused ${CONFIGURATION}" "${TMP}/log"
    [[ ! -e "$ONECAD_RUST_LIB_DIR" ]]
    echo "ok: ${CONFIGURATION} stub refused before staging"
done
CONFIGURATION=Debug "${ROOT}/platforms/ios/scripts/build-rust.sh" >"${TMP}/log" 2>&1
cmp "${TMP}/stubs/libonecad_ios.a" "${TMP}/staged/libonecad_ios.a"
echo 'ok: explicit Debug stub remains available'
echo 'All build provenance contract tests passed (synthetic tools only).'
