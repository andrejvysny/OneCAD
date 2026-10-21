#!/usr/bin/env bash
# Build the in-process OCCT worker (onecad_worker_lib) for one Apple platform and
# stage it for crates/onecad-embedded-worker (ONECAD_EMBEDDED_WORKER_DIR).
#
# Usage: scripts/build-ios-worker.sh --platform <iphonesimulator|iphoneos|macosx>
#            --config <Debug|Release> [--test-hooks] --out <absolute dir>
#
# Emits into --out (contract: docs/ios/KERNEL.md):
#   libonecad_worker_bundle.a    onecad_worker_lib + planegcs + the OCCT toolkits
#                                the link probe really pulls (iOS, static OCCT);
#                                worker + planegcs only for macosx (shared OCCT)
#   onecad_worker.h              the C ABI header
#   onecad-embedded-kernel.json  identity manifest v2
#   onecad-embedded-link.txt     link directives for build.rs (kind=value lines)
#   audit.txt                    otool/vtool audit, every command recorded
#
# Optional environment:
#   ONECAD_IOS_OCCT_ROOT            iOS OCCT prefix (default ~/.onecad-occt/8.0.1-<platform>)
#   ONECAD_OCCT_ROOT                desktop OCCT prefix for macosx (default ~/.onecad-occt/8.0.1)
#   ONECAD_IOS_DEPLOYMENT_TARGET    default 18.0
#   ONECAD_MACOS_DEPLOYMENT_TARGET  default 14.0
#   ONECAD_EMBEDDED_BUILD_DIR       default worker/build-embedded-<platform>-<config>[-hooks]
#   ONECAD_HEADER_PREFIXES          Eigen/Boost CMake prefixes (default Homebrew's)
#   CMAKE_BUILD_PARALLEL_LEVEL      default 5
set -euo pipefail
# The linker map and the archives hold non-UTF-8 bytes; byte-wise text tools.
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly OCCT_VERSION="8.0.1"

PLATFORM=""
CONFIG=""
TEST_HOOKS=0
OUT_DIR=""

usage() {
    echo "usage: $0 --platform <iphonesimulator|iphoneos|macosx> --config <Debug|Release> [--test-hooks] --out <dir>" >&2
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --platform) PLATFORM="${2:-}"; shift 2 ;;
            --config) CONFIG="${2:-}"; shift 2 ;;
            --test-hooks) TEST_HOOKS=1; shift ;;
            --out) OUT_DIR="${2:-}"; shift 2 ;;
            -h | --help) usage; exit 0 ;;
            *) echo "unknown argument: $1" >&2; usage; exit 2 ;;
        esac
    done
    case "$PLATFORM" in iphonesimulator | iphoneos | macosx) ;; *) usage; exit 2 ;; esac
    case "$CONFIG" in Debug | Release) ;; *) usage; exit 2 ;; esac
    if [[ "$OUT_DIR" != /* ]]; then
        echo "--out must be an absolute path" >&2
        exit 2
    fi
}

# Platform facts: SDK, deployment target, OCCT prefix, the Mach-O platform the
# objects must carry, and whether OCCT is linked static (iOS) or shared (macOS).
resolve_platform() {
    SDK_PATH="$(xcrun --sdk "$PLATFORM" --show-sdk-path)"
    SDK_VERSION="$(xcrun --sdk "$PLATFORM" --show-sdk-version)"
    case "$PLATFORM" in
        iphonesimulator) MACHO_PLATFORM="IOSSIMULATOR" ;;
        iphoneos) MACHO_PLATFORM="IOS" ;;
        macosx) MACHO_PLATFORM="MACOS" ;;
    esac
    if [[ "$PLATFORM" == "macosx" ]]; then
        DEPLOYMENT_TARGET="${ONECAD_MACOS_DEPLOYMENT_TARGET:-14.0}"
        OCCT_ROOT="${ONECAD_OCCT_ROOT:-$HOME/.onecad-occt/${OCCT_VERSION}}"
        SHARED_OCCT=1
    else
        DEPLOYMENT_TARGET="${ONECAD_IOS_DEPLOYMENT_TARGET:-18.0}"
        OCCT_ROOT="${ONECAD_IOS_OCCT_ROOT:-$HOME/.onecad-occt/${OCCT_VERSION}-${PLATFORM}}"
        SHARED_OCCT=0
    fi
    if [[ ! -f "${OCCT_ROOT}/share/onecad/occt-build.cmake" ]]; then
        echo "OCCT prefix ${OCCT_ROOT} has no share/onecad/occt-build.cmake;" \
            "build it with scripts/build-ios-occt.sh / build-pinned-occt.sh" >&2
        exit 1
    fi
    local suffix=""
    [[ "$TEST_HOOKS" == 1 ]] && suffix="-hooks"
    local lower_config
    lower_config="$(printf '%s' "$CONFIG" | tr '[:upper:]' '[:lower:]')"
    BUILD_DIR="${ONECAD_EMBEDDED_BUILD_DIR:-${ROOT_DIR}/worker/build-embedded-${PLATFORM}-${lower_config}${suffix}}"
}

configure_and_build() {
    local hooks=OFF
    [[ "$TEST_HOOKS" == 1 ]] && hooks=ON
    local args=(
        -S "${ROOT_DIR}/worker" -B "$BUILD_DIR" -G Ninja
        -DCMAKE_BUILD_TYPE="$CONFIG"
        -DONECAD_OCCT_ROOT="$OCCT_ROOT"
        -DONECAD_OCCT_VERSION="$OCCT_VERSION"
        -DONECAD_WORKER_BUILD_DIR="$BUILD_DIR"
        -DONECAD_WORKER_LIBRARY_ONLY=ON
        -DONECAD_WORKER_LIB_TEST_HOOKS="$hooks"
        -DCMAKE_OSX_ARCHITECTURES=arm64
        -DCMAKE_OSX_SYSROOT="$SDK_PATH"
        -DCMAKE_OSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET"
        -DCMAKE_PREFIX_PATH="${ONECAD_HEADER_PREFIXES:-/opt/homebrew/opt/eigen;/opt/homebrew/opt/boost}"
    )
    if [[ "$PLATFORM" != "macosx" ]]; then
        # Eigen and Boost are header-only host packages: let the cross build find
        # their CMake configs outside the iOS sysroot. Nothing is linked from them.
        args+=(-DCMAKE_SYSTEM_NAME=iOS
            -DCMAKE_FIND_ROOT_PATH_MODE_PACKAGE=BOTH
            -DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=BOTH
            -DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=BOTH)
    fi
    echo "==> Configuring onecad_worker_lib for ${PLATFORM} ${SDK_VERSION} (${CONFIG}, deployment ${DEPLOYMENT_TARGET}, test hooks ${hooks})"
    cmake "${args[@]}"
    cmake --build "$BUILD_DIR" --target onecad_worker_lib onecad_worker_link_probe \
        --parallel "${CMAKE_BUILD_PARALLEL_LEVEL:-5}"
}

# The OCCT archives the probe link pulled at least one object from (ld map,
# "# Object files:" section). That is the linker's own dependency closure.
derive_static_closure() {
    local map="${BUILD_DIR}/onecad_worker_link_probe.map"
    [[ -f "$map" ]] || { echo "missing link map ${map}" >&2; exit 1; }
    NEEDED_ARCHIVES=()
    local archive
    while IFS= read -r archive; do
        NEEDED_ARCHIVES+=("$archive")
    done < <(sed -n '/^# Object files:/,/^# Sections:/p' "$map" |
        grep -oE "${OCCT_ROOT}/lib/libTK[A-Za-z0-9_]+\.a" | sort -u)
    if [[ ${#NEEDED_ARCHIVES[@]} -eq 0 ]]; then
        echo "link map names no OCCT archive; refusing to emit a bundle without OCCT" >&2
        exit 1
    fi
}

# The OCCT dylibs the probe still references after -dead_strip_dylibs.
derive_shared_closure() {
    NEEDED_DYLIBS=()
    local name
    while IFS= read -r name; do
        NEEDED_DYLIBS+=("$name")
    done < <(otool -L "${BUILD_DIR}/onecad_worker_link_probe" |
        sed -nE 's#^[[:space:]]*@rpath/lib(TK[A-Za-z0-9_]+)\.[0-9.]+dylib .*#\1#p' | sort -u)
    if [[ ${#NEEDED_DYLIBS[@]} -eq 0 ]]; then
        echo "link probe references no OCCT dylib; refusing to emit a bundle" >&2
        exit 1
    fi
}

merge_bundle() {
    BUNDLE="${OUT_DIR}/libonecad_worker_bundle.a"
    local inputs=("${BUILD_DIR}/libonecad_worker_lib.a" "${BUILD_DIR}/third_party/planegcs/libplanegcs.a")
    if [[ "$SHARED_OCCT" == 0 ]]; then
        inputs+=("${NEEDED_ARCHIVES[@]}")
    fi
    rm -f "$BUNDLE"
    # -no_warning_for_no_symbols: OCCT ships members with no symbols; duplicate
    # member NAMES across archives are expected and harmless (the TOC addresses
    # members by offset).
    ZERO_AR_DATE=1 libtool -static -no_warning_for_no_symbols -o "$BUNDLE" "${inputs[@]}" 2>&1 |
        grep -v "same member name" || true
    [[ -s "$BUNDLE" ]] || { echo "libtool produced no bundle" >&2; exit 1; }
    ARCHIVE_SHA256="$(shasum -a 256 "$BUNDLE" | cut -d' ' -f1)"
}

# hex16(fnv1a64(seed)): the worker's own rule (WorkerRuntime.cpp occt_fingerprint).
# Bash arithmetic is 64-bit two's complement, so the multiply wraps mod 2^64
# exactly like the C++ uint64_t; printf %016x prints the same bits.
fnv1a_hex16() {
    local text="$1" h=$((0xcbf29ce484222325)) i byte
    for ((i = 0; i < ${#text}; i++)); do
        printf -v byte '%d' "'${text:i:1}"
        h=$(((h ^ byte) * 0x100000001b3))
    done
    printf '%016x' "$h"
}

identity_value() {
    local key="$1" line
    line="$(grep -m1 "^${key}=" "${BUILD_DIR}/onecad_worker_identity.txt")" ||
        { echo "identity key ${key} missing" >&2; exit 1; }
    printf '%s' "${line#*=}"
}

# Every string interpolated into the JSON must be plain: no quote, backslash or
# control character can reach the manifest unescaped.
json_safe() {
    [[ "$1" =~ ^[A-Za-z0-9._+:/=,|-]*$ ]] || { echo "unsafe manifest value: $1" >&2; exit 1; }
    printf '%s' "$1"
}

write_identity() {
    local seed fingerprint hooks_json=false
    seed="$(identity_value fingerprintSeed)"
    fingerprint="$(fnv1a_hex16 "$seed")"
    [[ "$(identity_value testHooks)" == 1 ]] && hooks_json=true
    MANIFEST="${OUT_DIR}/onecad-embedded-kernel.json"
    {
        printf '{\n'
        printf '  "manifestVersion": 2,\n'
        printf '  "kind": "embedded",\n'
        printf '  "abiVersion": %s,\n' "$(json_safe "$(identity_value abiVersion)")"
        printf '  "protocolVersion": %s,\n' "$(json_safe "$(identity_value protocolVersion)")"
        printf '  "workerVersion": "%s",\n' "$(json_safe "$(identity_value workerVersion)")"
        printf '  "quantizationVersion": %s,\n' "$(json_safe "$(identity_value quantizationVersion)")"
        printf '  "solverPolicyVersion": %s,\n' "$(json_safe "$(identity_value solverPolicyVersion)")"
        printf '  "occt": {\n'
        printf '    "version": "%s",\n' "$(json_safe "$(identity_value occtVersion)")"
        printf '    "buildId": "%s",\n' "$(json_safe "$(identity_value occtBuildId)")"
        printf '    "fingerprint": "%s"\n' "$fingerprint"
        printf '  },\n'
        printf '  "platform": "%s",\n' "$PLATFORM"
        printf '  "sdkVersion": "%s",\n' "$(json_safe "$SDK_VERSION")"
        printf '  "deploymentTarget": "%s",\n' "$(json_safe "$DEPLOYMENT_TARGET")"
        printf '  "archiveSha256": "%s",\n' "$ARCHIVE_SHA256"
        printf '  "testHooks": %s\n' "$hooks_json"
        printf '}\n'
    } >"$MANIFEST"
}

# System libraries for a link of the bundle, DERIVED: start from libc++ alone
# and add the frameworks the probe's own link line used only if the minimal
# link leaves symbols undefined.
link_bundle_probe() {
    local out="$1"
    shift
    local target
    case "$PLATFORM" in
        iphonesimulator) target="arm64-apple-ios${DEPLOYMENT_TARGET}-simulator" ;;
        iphoneos) target="arm64-apple-ios${DEPLOYMENT_TARGET}" ;;
        macosx) target="arm64-apple-macos${DEPLOYMENT_TARGET}" ;;
    esac
    local extra=()
    if [[ "$SHARED_OCCT" == 1 ]]; then
        extra+=(-L"${OCCT_ROOT}/lib" -Wl,-rpath,"${OCCT_ROOT}/lib")
        local name
        for name in "${NEEDED_DYLIBS[@]}"; do extra+=(-l"$name"); done
    fi
    xcrun --sdk "$PLATFORM" clang++ -target "$target" -isysroot "$SDK_PATH" \
        -I"${ROOT_DIR}/worker/src" "${ROOT_DIR}/worker/tools/link_probe/link_probe.cpp" \
        "$BUNDLE" ${extra[@]+"${extra[@]}"} -lc++ "$@" -o "$out"
}

derive_system_libs() {
    SYSTEM_LIBS=()
    local log="${BUILD_DIR}/bundle_probe_link.log"
    if link_bundle_probe "${BUILD_DIR}/bundle_probe" >"$log" 2>&1; then
        return
    fi
    local fw
    while IFS= read -r fw; do
        SYSTEM_LIBS+=("-framework" "$fw")
    done < <(ninja -C "$BUILD_DIR" -t commands onecad_worker_link_probe | tail -n 1 |
        grep -oE '[A-Za-z0-9_]+\.framework' | sed 's/\.framework$//' | sort -u)
    if ! link_bundle_probe "${BUILD_DIR}/bundle_probe" "${SYSTEM_LIBS[@]}" >"$log" 2>&1; then
        echo "the bundle does not link even with the probe's frameworks; see ${log}" >&2
        exit 1
    fi
}

write_link_file() {
    local link="${OUT_DIR}/onecad-embedded-link.txt"
    {
        printf 'static=onecad_worker_bundle\n'
        printf 'lib=c++\n'
        local i
        for ((i = 0; i < ${#SYSTEM_LIBS[@]}; i += 2)); do
            printf 'framework=%s\n' "${SYSTEM_LIBS[i + 1]}"
        done
        if [[ "$SHARED_OCCT" == 1 ]]; then
            printf 'dylib-dir=%s\n' "${OCCT_ROOT}/lib"
            local name
            for name in "${NEEDED_DYLIBS[@]}"; do printf 'dylib=%s\n' "$name"; done
            printf 'rpath=%s\n' "${OCCT_ROOT}/lib"
        fi
    } >"$link"
}

# Runs one audit command, records it and its output in audit.txt.
audit_cmd() {
    printf '$ %s\n' "$*" >>"$AUDIT"
    "$@" >>"$AUDIT" 2>&1
    printf '\n' >>"$AUDIT"
}

audit() {
    AUDIT="${OUT_DIR}/audit.txt"
    : >"$AUDIT"
    local probe="${BUILD_DIR}/bundle_probe"
    audit_cmd vtool -show-build "$probe"
    audit_cmd otool -L "$probe"
    audit_cmd otool -l "$probe"
    # Every object in the bundle must carry the target platform and minos.
    local platforms minos
    platforms="$(otool -l "$BUNDLE" | awk '/cmd LC_BUILD_VERSION/{f=1} f&&/platform/{print $2; f=0}' | sort | uniq -c)"
    minos="$(otool -l "$BUNDLE" | awk '/cmd LC_BUILD_VERSION/{f=1} f&&/minos/{print $2; f=0}' | sort | uniq -c)"
    printf '$ otool -l %s | LC_BUILD_VERSION platform counts\n%s\n\n' "$BUNDLE" "$platforms" >>"$AUDIT"
    printf '$ otool -l %s | LC_BUILD_VERSION minos counts\n%s\n\n' "$BUNDLE" "$minos" >>"$AUDIT"
    check_platforms "$platforms" "$minos"
    check_probe_loads "$probe"
    local homebrew_strings
    homebrew_strings="$(strings -a "$BUNDLE" | grep -c '/opt/homebrew' || true)"
    printf '$ strings -a %s | grep -c /opt/homebrew\n%s (informational: debug info / __FILE__ text, not a load path)\n' \
        "$BUNDLE" "$homebrew_strings" >>"$AUDIT"
}

check_platforms() {
    local platforms="$1" minos="$2" expected_number
    case "$MACHO_PLATFORM" in IOS) expected_number=2 ;; IOSSIMULATOR) expected_number=7 ;; MACOS) expected_number=1 ;; esac
    # otool prints the platform by name (IOSSIMULATOR) or number (7) by version.
    if grep -vqE "[[:space:]]($MACHO_PLATFORM|$expected_number)$" <<<"$platforms"; then
        echo "audit: bundle objects carry a platform other than ${MACHO_PLATFORM}:" >&2
        echo "$platforms" >&2
        exit 1
    fi
    if [[ "$SHARED_OCCT" == 0 ]] && grep -vqE "[[:space:]]${DEPLOYMENT_TARGET}$" <<<"$minos"; then
        echo "audit: bundle objects carry a minos other than ${DEPLOYMENT_TARGET}:" >&2
        echo "$minos" >&2
        exit 1
    fi
}

# An iOS link of the bundle must load nothing outside the SDK: no Homebrew, no
# desktop OCCT dylib, no LC_RPATH.
check_probe_loads() {
    local probe="$1"
    [[ "$SHARED_OCCT" == 1 ]] && return
    if otool -L "$probe" | tail -n +2 | grep -vqE '^[[:space:]]*/(usr/lib|System/Library)/'; then
        echo "audit: the iOS probe loads a library outside the SDK" >&2
        otool -L "$probe" >&2
        exit 1
    fi
    if otool -l "$probe" | grep -q LC_RPATH; then
        echo "audit: the iOS probe carries an LC_RPATH" >&2
        exit 1
    fi
}

main() {
    parse_args "$@"
    "${SCRIPT_DIR}/check-worker-stdout-hygiene.sh"
    resolve_platform
    configure_and_build
    mkdir -p "$OUT_DIR"
    if [[ "$SHARED_OCCT" == 0 ]]; then derive_static_closure; else derive_shared_closure; fi
    merge_bundle
    derive_system_libs
    write_identity
    write_link_file
    cp "${ROOT_DIR}/worker/src/capi/onecad_worker.h" "${OUT_DIR}/onecad_worker.h"
    audit
    echo "==> Embedded worker for ${PLATFORM} (${CONFIG}): ${OUT_DIR}"
    echo "    bundle sha256 ${ARCHIVE_SHA256}"
    if [[ "$SHARED_OCCT" == 0 ]]; then
        echo "    OCCT toolkits bundled: ${#NEEDED_ARCHIVES[@]}"
    else
        echo "    OCCT dylibs linked: ${#NEEDED_DYLIBS[@]}"
    fi
}

main "$@"
