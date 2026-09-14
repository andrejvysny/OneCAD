#!/usr/bin/env bash
# Compile the Bun assistant sidecar and stage it for Tauri bundling.
#
# Usage: scripts/build-assistant-host.sh
#
# Produces src-tauri/binaries/onecad-assistant-host-<rust-host-triple>, the name
# Tauri's bundle.externalBin expects — the same staging shape
# scripts/build-worker.sh uses for the C++ geometry worker, so the two sidecars
# are bundled by one mechanism. Run from anywhere; paths resolve to the repo
# root.
#
# SIZE. The output is about 99 MB, and essentially all of it is the Bun runtime
# that `--compile` embeds: a hello-world compiles to 99.3 MB, and this package's
# own code plus its dependencies add roughly 104 KB on top. There is nothing to
# shrink here short of not shipping Bun.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

# AgentKit is built from source into a gitignored prefix until its release tag
# exists; see scripts/bootstrap-agentkit.sh for why.
if [ ! -f "${ROOT_DIR}/.agentkit-src/packages/agentkit/dist/index.js" ]; then
    echo "==> AgentKit dist missing; bootstrapping"
    "${SCRIPT_DIR}/bootstrap-agentkit.sh"
fi

# Rust host triple names the sidecar binary; fall back to Apple Silicon if
# rustc is not on PATH.
if command -v rustc >/dev/null 2>&1; then
    TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
fi
if [ -z "${TRIPLE:-}" ]; then
    TRIPLE="aarch64-apple-darwin"
fi

echo "==> Installing assistant-host dependencies"
(cd "${ROOT_DIR}/assistant-host" && bun install)

DEST_DIR="${ROOT_DIR}/src-tauri/binaries"
mkdir -p "${DEST_DIR}"
DEST="${DEST_DIR}/onecad-assistant-host-${TRIPLE}"

echo "==> Compiling onecad-assistant-host for triple ${TRIPLE}"
(cd "${ROOT_DIR}/assistant-host" && bun build --compile src/main.ts --outfile "${DEST}")
chmod +x "${DEST}"

echo "==> Staged sidecar: ${DEST}"
ls -la "${DEST}"
