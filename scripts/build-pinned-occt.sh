#!/usr/bin/env bash
# Build one of OneCAD's checksum-pinned OCCT releases into an isolated prefix.
set -euo pipefail

usage() {
    echo "usage: $0 --version <8.0.1|7.9.3> --prefix <path> --build-dir <path> --build-id <id>" >&2
}

VERSION=""
PREFIX=""
BUILD_DIR=""
BUILD_ID=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) VERSION="${2:-}"; shift 2 ;;
        --prefix) PREFIX="${2:-}"; shift 2 ;;
        --build-dir) BUILD_DIR="${2:-}"; shift 2 ;;
        --build-id) BUILD_ID="${2:-}"; shift 2 ;;
        -h | --help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage; exit 2 ;;
    esac
done

if [[ -z "$VERSION" || -z "$PREFIX" || -z "$BUILD_DIR" || -z "$BUILD_ID" ]]; then
    usage
    exit 2
fi

case "$VERSION" in
    8.0.1 | V8_0_1)
        VERSION="8.0.1"
        TAG="V8_0_1"
        COMMIT="b8f597c677811d1f9f4d8a97f5ae2825c0353a42"
        ARCHIVE_SHA256="0d6913eae4bcc09a3653ceced6dda1aec11c35a1513d4c06762c9b002092c68a"
        ;;
    7.9.3 | V7_9_3)
        VERSION="7.9.3"
        TAG="V7_9_3"
        COMMIT="a016080bf6738d6aeae020badee4e888ad1540a5"
        ARCHIVE_SHA256="5ecf094ec6b12d5413dfb851d8c3590c354058aee556e32e408bdfbf8c357d57"
        ;;
    *)
        echo "unsupported pinned OCCT version: $VERSION" >&2
        exit 2
        ;;
esac

if [[ "$PREFIX" != /* || "$BUILD_DIR" != /* ]]; then
    echo "--prefix and --build-dir must be absolute paths" >&2
    exit 2
fi
mkdir -p "$PREFIX" "$BUILD_DIR"
PREFIX="$(cd "$PREFIX" && pwd -P)"
BUILD_DIR="$(cd "$BUILD_DIR" && pwd -P)"
if [[ "$PREFIX" == "$BUILD_DIR" || "$PREFIX" == "$BUILD_DIR"/* ||
      "$BUILD_DIR" == "$PREFIX"/* ]]; then
    echo "--prefix and --build-dir must be disjoint" >&2
    exit 2
fi
if [[ ! "$BUILD_ID" =~ ^[A-Za-z0-9._+-]+$ ]]; then
    echo "--build-id must contain only letters, digits, dot, underscore, plus, or hyphen" >&2
    exit 2
fi

readonly KERNEL_POLICY_VERSION="1"
readonly SOURCE_URL="https://github.com/Open-Cascade-SAS/OCCT/archive/refs/tags/${TAG}.tar.gz"
readonly DOWNLOAD_DIR="${ONECAD_OCCT_DOWNLOAD_DIR:-${BUILD_DIR}-downloads}"
readonly SOURCE_DIR="${BUILD_DIR}-source"
readonly ARCHIVE="${DOWNLOAD_DIR}/OCCT-${TAG}.tar.gz"
readonly METADATA_DIR="${PREFIX}/share/onecad"
readonly CMAKE_METADATA="${METADATA_DIR}/occt-build.cmake"
readonly JSON_METADATA="${METADATA_DIR}/occt-build.json"

# Keep this sorted. The configured cache is checked against this string before
# the build; metadata never records unverified requested values.
readonly OPTION_KEYS=(
    BUILD_LIBRARY_TYPE BUILD_MODULE_Draw BUILD_USE_PCH CMAKE_BUILD_TYPE
    USE_DRACO USE_FFMPEG USE_FREEIMAGE USE_FREETYPE USE_OPENGL USE_RAPIDJSON
    USE_TBB USE_TCL USE_TK USE_VTK
)
readonly NORMALIZED_OPTIONS="BUILD_LIBRARY_TYPE=Shared,BUILD_MODULE_Draw=OFF,BUILD_USE_PCH=OFF,CMAKE_BUILD_TYPE=Release,USE_DRACO=OFF,USE_FFMPEG=OFF,USE_FREEIMAGE=OFF,USE_FREETYPE=OFF,USE_OPENGL=OFF,USE_RAPIDJSON=OFF,USE_TBB=OFF,USE_TCL=OFF,USE_TK=OFF,USE_VTK=OFF"

metadata_matches() {
    [[ -f "$CMAKE_METADATA" ]] &&
        grep -Fq "set(ONECAD_OCCT_SOURCE_COMMIT \"${COMMIT}\")" "$CMAKE_METADATA" &&
        grep -Fq "set(ONECAD_OCCT_NORMALIZED_BUILD_OPTIONS \"${NORMALIZED_OPTIONS}\")" "$CMAKE_METADATA" &&
        grep -Fq "set(ONECAD_OCCT_BUILT_ID \"${BUILD_ID}\")" "$CMAKE_METADATA"
}

if metadata_matches; then
    echo "==> Reusing pinned OCCT ${VERSION}: ${PREFIX}"
    exit 0
fi
if [[ -e "$PREFIX" ]] && [[ -n "$(find "$PREFIX" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null)" ]]; then
    echo "prefix exists but does not match requested pinned build: $PREFIX" >&2
    echo "choose an empty versioned prefix" >&2
    exit 1
fi

mkdir -p "$DOWNLOAD_DIR"
if [[ ! -f "$ARCHIVE" ]] || ! printf '%s  %s\n' "$ARCHIVE_SHA256" "$ARCHIVE" | shasum -a 256 -c - >/dev/null 2>&1; then
    echo "==> Downloading OCCT ${TAG} (${COMMIT})"
    curl --fail --location --retry 3 --output "${ARCHIVE}.part" "$SOURCE_URL"
    printf '%s  %s\n' "$ARCHIVE_SHA256" "${ARCHIVE}.part" | shasum -a 256 -c -
    mv "${ARCHIVE}.part" "$ARCHIVE"
fi
printf '%s  %s\n' "$ARCHIVE_SHA256" "$ARCHIVE" | shasum -a 256 -c -

if [[ ! -f "${SOURCE_DIR}/CMakeLists.txt" ]]; then
    mkdir -p "$SOURCE_DIR"
    tar -xzf "$ARCHIVE" --strip-components=1 -C "$SOURCE_DIR"
fi

echo "==> Configuring OCCT ${VERSION}"
cmake -S "$SOURCE_DIR" -B "$BUILD_DIR" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DINSTALL_DIR="$PREFIX" \
    -DBUILD_LIBRARY_TYPE=Shared \
    -DBUILD_MODULE_Draw=OFF \
    -DBUILD_USE_PCH=OFF \
    -DUSE_DRACO=OFF \
    -DUSE_FFMPEG=OFF \
    -DUSE_FREEIMAGE=OFF \
    -DUSE_FREETYPE=OFF \
    -DUSE_OPENGL=OFF \
    -DUSE_RAPIDJSON=OFF \
    -DUSE_TBB=OFF \
    -DUSE_TCL=OFF \
    -DUSE_TK=OFF \
    -DUSE_VTK=OFF

actual_options=""
for key in "${OPTION_KEYS[@]}"; do
    value="$(sed -n "s/^${key}:[^=]*=//p" "$BUILD_DIR/CMakeCache.txt" | tail -n 1)"
    if [[ -z "$value" ]]; then
        echo "configured OCCT cache omitted required option: $key" >&2
        exit 1
    fi
    [[ -n "$actual_options" ]] && actual_options+=","
    actual_options+="${key}=${value}"
done
if [[ "$actual_options" != "$NORMALIZED_OPTIONS" ]]; then
    echo "configured OCCT options differ from fingerprint policy" >&2
    echo "expected: $NORMALIZED_OPTIONS" >&2
    echo "actual:   $actual_options" >&2
    exit 1
fi

echo "==> Building OCCT ${VERSION}"
cmake --build "$BUILD_DIR" --parallel "${ONECAD_OCCT_JOBS:-3}"
cmake --install "$BUILD_DIR"

mkdir -p "$METADATA_DIR"
{
    printf 'set(ONECAD_OCCT_SOURCE_COMMIT "%s")\n' "$COMMIT"
    printf 'set(ONECAD_OCCT_NORMALIZED_BUILD_OPTIONS "%s")\n' "$NORMALIZED_OPTIONS"
    printf 'set(ONECAD_OCCT_BUILT_ID "%s")\n' "$BUILD_ID"
} >"$CMAKE_METADATA"
{
    printf '{\n'
    printf '  "schemaVersion": 1,\n'
    printf '  "version": "%s",\n' "$VERSION"
    printf '  "tag": "%s",\n' "$TAG"
    printf '  "sourceCommit": "%s",\n' "$COMMIT"
    printf '  "archiveSha256": "%s",\n' "$ARCHIVE_SHA256"
    printf '  "buildId": "%s",\n' "$BUILD_ID"
    printf '  "kernelPolicyVersion": %s,\n' "$KERNEL_POLICY_VERSION"
    printf '  "normalizedBuildOptions": "%s"\n' "$NORMALIZED_OPTIONS"
    printf '}\n'
} >"$JSON_METADATA"

echo "==> Installed pinned OCCT ${VERSION}: ${PREFIX}"
