#!/usr/bin/env bash
# Build the OneCAD C++ sidecar worker and stage it for Tauri bundling.
#
# Usage: scripts/build-worker.sh [Debug|Release]   (default: Release)
#
# Optional environment:
#   ONECAD_OCCT_ROOT          explicit OCCT installation prefix
#   ONECAD_OCCT_VERSION       exact OCCT version (default: 8.0.1)
#   ONECAD_OCCT_BUILD_ID      stable build artifact id (default: local)
#   ONECAD_WORKER_BUILD_DIR   isolated worker build directory
#
# Produces src-tauri/binaries/onecad-worker-<rust-host-triple>, the name Tauri's
# bundle.externalBin expects, plus a manifest binding that exact binary to its
# hello compatibility axes. Run from anywhere; paths resolve to the repo root.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

echo "==> Checking worker stdout hygiene"
"${SCRIPT_DIR}/check-worker-stdout-hygiene.sh"

BUILD_TYPE="${1:-Release}"
OCCT_VERSION="${ONECAD_OCCT_VERSION:-8.0.1}"
OCCT_BUILD_ID="${ONECAD_OCCT_BUILD_ID:-local}"
BUILD_DIR="${ONECAD_WORKER_BUILD_DIR:-${ROOT_DIR}/worker/build}"

CMAKE_ARGS=(
    -DCMAKE_BUILD_TYPE="${BUILD_TYPE}"
    -DONECAD_OCCT_VERSION="${OCCT_VERSION}"
    -DONECAD_OCCT_BUILD_ID="${OCCT_BUILD_ID}"
    -DONECAD_WORKER_BUILD_DIR="${BUILD_DIR}"
)
if [ -n "${ONECAD_OCCT_ROOT:-}" ]; then
    CMAKE_ARGS+=(-DONECAD_OCCT_ROOT="${ONECAD_OCCT_ROOT}")
fi

# Rust host triple names the sidecar binary; fall back to Apple Silicon if
# rustc is not on PATH.
if command -v rustc >/dev/null 2>&1; then
    TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
fi
if [ -z "${TRIPLE:-}" ]; then
    TRIPLE="aarch64-apple-darwin"
fi

echo "==> Building onecad-worker (${BUILD_TYPE}) for triple ${TRIPLE}"

cmake -S "${ROOT_DIR}/worker" -B "${BUILD_DIR}" "${CMAKE_ARGS[@]}"
cmake --build "${BUILD_DIR}" -j

DEST_DIR="${ROOT_DIR}/src-tauri/binaries"
mkdir -p "${DEST_DIR}"
DEST="${DEST_DIR}/onecad-worker-${TRIPLE}"
cp "${BUILD_DIR}/onecad-worker" "${DEST}"
chmod +x "${DEST}"

HELLO_FRAME="$(mktemp)"
trap 'rm -f "${HELLO_FRAME}"' EXIT
"${DEST}" </dev/null >"${HELLO_FRAME}"

MANIFEST="${DEST_DIR}/onecad-worker-manifest.json"
bun -e '
const [framePath, binaryPath, manifestPath] = process.argv.slice(1);
const frame = new Uint8Array(await Bun.file(framePath).arrayBuffer());
if (frame.length < 12 || new TextDecoder().decode(frame.slice(0, 4)) !== "OCW1") {
  throw new Error("worker did not emit an OCW1 hello frame");
}
const header = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
const jsonLength = header.getUint32(4, true);
const binaryLength = header.getUint32(8, true);
if (binaryLength !== 0 || frame.length !== 12 + jsonLength) {
  throw new Error("worker hello frame has an invalid length or binary tail");
}
const envelope = JSON.parse(new TextDecoder().decode(frame.slice(12)));
if (envelope.t !== "hello" || envelope.seq !== 0 || !envelope.result) {
  throw new Error("worker first frame is not the required hello");
}
const binary = new Uint8Array(await Bun.file(binaryPath).arrayBuffer());
const binarySha256 = new Bun.CryptoHasher("sha256").update(binary).digest("hex");
const hello = envelope.result;
const manifest = {
  manifestVersion: 1,
  binarySha256,
  protocolVersion: hello.protocolVersion,
  workerVersion: hello.workerVersion,
  quantizationVersion: hello.quantizationVersion,
  solverPolicyVersion: hello.solverPolicyVersion,
  occt: hello.occt,
};
await Bun.write(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
' "${HELLO_FRAME}" "${DEST}" "${MANIFEST}"

echo "==> Staged sidecar: ${DEST}"
echo "==> Staged manifest: ${MANIFEST}"
