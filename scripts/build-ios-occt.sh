#!/usr/bin/env bash
# Build the checksum-pinned OCCT 8.0.1 as static libraries for one iPadOS
# destination (device or Apple-Silicon simulator).
#
# Usage: scripts/build-ios-occt.sh --platform <iphoneos|iphonesimulator> \
#            --prefix <abs path> --build-dir <abs path>
#
# Device and simulator ARM64 are different Mach-O platforms; each gets its own
# build directory and prefix and they are never merged into one lipo slice.
# The source archive and its checksum are shared with build-pinned-occt.sh.
set -euo pipefail

PLATFORM=""
PREFIX=""
BUILD_DIR=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --platform) PLATFORM="${2:-}"; shift 2 ;;
        --prefix) PREFIX="${2:-}"; shift 2 ;;
        --build-dir) BUILD_DIR="${2:-}"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done
case "$PLATFORM" in
    iphoneos | iphonesimulator) ;;
    *) echo "--platform must be iphoneos or iphonesimulator" >&2; exit 2 ;;
esac
if [[ "$PREFIX" != /* || "$BUILD_DIR" != /* ]]; then
    echo "--prefix and --build-dir must be absolute paths" >&2
    exit 2
fi

readonly VERSION="8.0.1"
readonly TAG="V8_0_1"
readonly COMMIT="b8f597c677811d1f9f4d8a97f5ae2825c0353a42"
readonly ARCHIVE_SHA256="0d6913eae4bcc09a3653ceced6dda1aec11c35a1513d4c06762c9b002092c68a"
readonly DEPLOYMENT_TARGET="${ONECAD_IOS_DEPLOYMENT_TARGET:-18.0}"
readonly KERNEL_POLICY_VERSION="1"
# Static libraries are required on iPadOS (no loose dylibs inside the app); the
# remaining options match the desktop policy so kernel behavior is comparable.
readonly NORMALIZED_OPTIONS="BUILD_LIBRARY_TYPE=Static,BUILD_MODULE_Draw=OFF,BUILD_USE_PCH=OFF,CMAKE_BUILD_TYPE=Release,USE_DRACO=OFF,USE_FFMPEG=OFF,USE_FREEIMAGE=OFF,USE_FREETYPE=OFF,USE_OPENGL=OFF,USE_RAPIDJSON=OFF,USE_TBB=OFF,USE_TCL=OFF,USE_TK=OFF,USE_VTK=OFF"
readonly BUILD_ID="onecad-occt-${VERSION}-${COMMIT:0:12}-kp${KERNEL_POLICY_VERSION}-${PLATFORM}-arm64-ios${DEPLOYMENT_TARGET}"
readonly DOWNLOAD_DIR="${ONECAD_OCCT_DOWNLOAD_DIR:-$HOME/.onecad-occt-build/${VERSION}-downloads}"
readonly ARCHIVE="${DOWNLOAD_DIR}/OCCT-${TAG}.tar.gz"
readonly SOURCE_DIR="${BUILD_DIR}-source"
readonly METADATA="${PREFIX}/share/onecad/occt-build.json"

if [[ -f "$METADATA" ]] && grep -Fq "\"buildId\": \"${BUILD_ID}\"" "$METADATA"; then
    echo "==> Reusing iPadOS OCCT ${PLATFORM}: ${PREFIX}"
    exit 0
fi

mkdir -p "$DOWNLOAD_DIR" "$BUILD_DIR" "$PREFIX"
if [[ ! -f "$ARCHIVE" ]]; then
    curl --fail --location --retry 3 --output "${ARCHIVE}.part" \
        "https://github.com/Open-Cascade-SAS/OCCT/archive/refs/tags/${TAG}.tar.gz"
    mv "${ARCHIVE}.part" "$ARCHIVE"
fi
printf '%s  %s\n' "$ARCHIVE_SHA256" "$ARCHIVE" | shasum -a 256 -c -
if [[ ! -f "${SOURCE_DIR}/CMakeLists.txt" ]]; then
    mkdir -p "$SOURCE_DIR"
    tar -xzf "$ARCHIVE" --strip-components=1 -C "$SOURCE_DIR"
fi

SDK_PATH="$(xcrun --sdk "$PLATFORM" --show-sdk-path)"
SDK_VERSION="$(xcrun --sdk "$PLATFORM" --show-sdk-version)"
echo "==> Configuring OCCT ${VERSION} for ${PLATFORM} ${SDK_VERSION} (arm64, iPadOS ${DEPLOYMENT_TARGET})"
cmake -S "$SOURCE_DIR" -B "$BUILD_DIR" -G Ninja \
    -DCMAKE_SYSTEM_NAME=iOS \
    -DCMAKE_OSX_SYSROOT="$SDK_PATH" \
    -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DINSTALL_DIR="$PREFIX" \
    -DBUILD_LIBRARY_TYPE=Static \
    -DBUILD_MODULE_Draw=OFF \
    -DBUILD_DOC_Overview=OFF \
    -DBUILD_USE_PCH=OFF \
    -DUSE_DRACO=OFF \
    -DUSE_FFMPEG=OFF \
    -DUSE_FREEIMAGE=OFF \
    -DUSE_FREETYPE=OFF \
    -DUSE_OPENGL=OFF \
    -DUSE_GLES2=OFF \
    -DUSE_RAPIDJSON=OFF \
    -DUSE_TBB=OFF \
    -DUSE_TCL=OFF \
    -DUSE_TK=OFF \
    -DUSE_VTK=OFF \
    -DUSE_XLIB=OFF

cmake --build "$BUILD_DIR" --parallel "${ONECAD_OCCT_JOBS:-6}"
cmake --install "$BUILD_DIR"

mkdir -p "${PREFIX}/share/onecad"
{
    printf '{\n'
    printf '  "schemaVersion": 1,\n'
    printf '  "version": "%s",\n' "$VERSION"
    printf '  "tag": "%s",\n' "$TAG"
    printf '  "sourceCommit": "%s",\n' "$COMMIT"
    printf '  "archiveSha256": "%s",\n' "$ARCHIVE_SHA256"
    printf '  "buildId": "%s",\n' "$BUILD_ID"
    printf '  "kernelPolicyVersion": %s,\n' "$KERNEL_POLICY_VERSION"
    printf '  "platform": "%s",\n' "$PLATFORM"
    printf '  "sdkVersion": "%s",\n' "$SDK_VERSION"
    printf '  "deploymentTarget": "%s",\n' "$DEPLOYMENT_TARGET"
    printf '  "normalizedBuildOptions": "%s"\n' "$NORMALIZED_OPTIONS"
    printf '}\n'
} >"$METADATA"
echo "==> Installed iPadOS OCCT ${PLATFORM}: ${PREFIX}"
