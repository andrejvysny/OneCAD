#!/usr/bin/env bash
# Compile the Bun assistant sidecar, stage it for Tauri bundling, and record
# what was actually built.
#
# Usage: scripts/build-assistant-host.sh [--target <rust-triple>] [--force]
#        ONECAD_ASSISTANT_TARGET=<rust-triple> scripts/build-assistant-host.sh
#
# Produces, in src-tauri/binaries/:
#
#   onecad-assistant-host-<triple>[.exe]                the sidecar
#   onecad-assistant-host-<triple>[.exe].manifest.json  what it was built from
#
# `<triple>[.exe]` is the name Tauri's bundle.externalBin expects, and the same
# name src-tauri/src/assistant/mod.rs looks up at runtime — both of them ask
# scripts/lib/assistant-host-target.sh, which is the one place the mapping
# lives.
#
# TARGET. There is NO fallback. This script used to infer the Rust host triple
# and, failing that, assume `aarch64-apple-darwin`; that does not fail on an
# unknown target, it mislabels a binary, and a mislabelled sidecar is one the
# bundle either cannot find or finds and cannot run. Now: --target, else
# ONECAD_ASSISTANT_TARGET, else the rustc host triple, else an error.
#
# CACHING (F15). A previous build is reused only when a manifest proves it was
# built from THIS source, THIS lockfile, THIS AgentKit dist, THIS Bun and THIS
# target, and the artifact on disk still has the digest that manifest recorded.
# Anything else rebuilds. The old test was "a dist file exists", which
# establishes none of that and can ship a stale artifact.
#
# SIZE. The output is about 99 MB, and essentially all of it is the Bun runtime
# that `--compile` embeds: a hello-world compiles to 99.3 MB, and this package's
# own code plus its dependencies add roughly 104 KB on top. There is nothing to
# shrink here short of not shipping Bun.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=lib/assistant-host-target.sh
source "${SCRIPT_DIR}/lib/assistant-host-target.sh"

TRIPLE="${ONECAD_ASSISTANT_TARGET:-}"
FORCE=0
while [ $# -gt 0 ]; do
    case "$1" in
    --target)
        TRIPLE="${2:?--target needs a Rust target triple}"
        shift 2
        ;;
    --force)
        FORCE=1
        shift
        ;;
    *)
        echo "build-assistant-host: unknown argument '$1'" >&2
        echo "usage: $0 [--target <rust-triple>] [--force]" >&2
        exit 2
        ;;
    esac
done

if ! command -v bun >/dev/null 2>&1; then
    echo "build-assistant-host: bun is required but not on PATH." >&2
    echo "                      Install it (https://bun.sh) — there is no other compiler" >&2
    echo "                      for this sidecar and no artifact to fall back on." >&2
    exit 1
fi
BUN_VERSION="$(bun --version)"

[ -n "${TRIPLE}" ] || TRIPLE="$(onecad_host_triple)"
BUN_TARGET="$(onecad_bun_target_for_triple "${TRIPLE}")"
STAGED_NAME="$(onecad_staged_name_for_triple "${TRIPLE}")"

DEST_DIR="${ROOT_DIR}/src-tauri/binaries"
DEST="${DEST_DIR}/${STAGED_NAME}"
MANIFEST="${DEST}.manifest.json"

sha256_stdin() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | cut -d' ' -f1
    else
        shasum -a 256 | cut -d' ' -f1
    fi
}

sha256_file() {
    [ -f "$1" ] || return 0
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

manifest_field() {
    [ -f "${MANIFEST}" ] || return 0
    sed -n "s/.*\"$1\": *\"\([^\"]*\)\".*/\1/p" "${MANIFEST}"
}

# One digest over everything that goes INTO the binary: the sidecar's sources,
# its manifest, its tsconfig and its lockfile. Not the tests, which do not.
source_digest() {
    (
        cd "${ROOT_DIR}/assistant-host"
        find src package.json tsconfig.json bun.lock -type f | LC_ALL=C sort |
            while IFS= read -r file; do
                printf '%s %s\n' "$(sha256_file "${file}")" "${file}"
            done
    ) | sha256_stdin
}

# ── The AgentKit dependency, verified rather than assumed ────────────────────
# Unconditionally: the bootstrap is what knows whether its own cached build is
# still valid, and "a dist file exists" is not that knowledge.
"${SCRIPT_DIR}/bootstrap-agentkit.sh"

AGENTKIT_PREFIX_DIR="${AGENTKIT_PREFIX:-${ROOT_DIR}/.agentkit-src}"
AGENTKIT_DIST="${AGENTKIT_PREFIX_DIR}/packages/agentkit/dist/index.js"
AGENTKIT_STAMP="${AGENTKIT_PREFIX_DIR}/.onecad-agentkit-build.json"
AGENTKIT_COMMIT_BUILT="$(sed -n 's/.*"commit": *"\([^"]*\)".*/\1/p' "${AGENTKIT_STAMP}")"
AGENTKIT_DIST_SHA="$(sha256_file "${AGENTKIT_DIST}")"

# Versions this artifact will claim on the wire. Recorded here so the manifest
# says which contract the SHIPPED binary speaks, not which one someone intended.
AGENTKIT_CONTRACT_VERSION="$(
    sed -n 's/.*CONTRACT_VERSION *= *"\([^"]*\)".*/\1/p' \
        "${AGENTKIT_PREFIX_DIR}/packages/contracts/src/version.ts"
)"
OCAK_PROTOCOL_VERSION="$(
    sed -n 's/.*OCAK_PROTOCOL_VERSION *= *\([0-9][0-9]*\).*/\1/p' \
        "${ROOT_DIR}/assistant-host/src/bridge/peer.ts"
)"
HOST_PACKAGE_VERSION="$(
    sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "${ROOT_DIR}/assistant-host/package.json" | head -1
)"
for pair in \
    "agentkit contract version:${AGENTKIT_CONTRACT_VERSION}" \
    "OCAK protocol version:${OCAK_PROTOCOL_VERSION}" \
    "assistant-host package version:${HOST_PACKAGE_VERSION}"; do
    if [ -z "${pair#*:}" ]; then
        echo "build-assistant-host: could not read the ${pair%%:*}." >&2
        echo "                      A manifest that cannot state it is a manifest that" >&2
        echo "                      proves nothing, so this is a failure, not a blank." >&2
        exit 1
    fi
done

echo "==> Installing assistant-host dependencies (frozen)"
(cd "${ROOT_DIR}/assistant-host" && bun install --frozen-lockfile)

SOURCE_SHA="$(source_digest)"
ONECAD_REVISION="$(git -C "${ROOT_DIR}" rev-parse HEAD 2>/dev/null || echo unknown)"
ONECAD_DIRTY=clean
if [ -n "$(git -C "${ROOT_DIR}" status --porcelain -- assistant-host scripts 2>/dev/null)" ]; then
    ONECAD_DIRTY=dirty
fi

# ── Is the staged artifact the one this input would produce? ─────────────────
reuse=0
if [ "${FORCE}" = "0" ] && [ -f "${DEST}" ] && [ -f "${MANIFEST}" ]; then
    stale=""
    [ "$(manifest_field sourceSha256)" = "${SOURCE_SHA}" ] || stale="the sidecar source or lockfile changed"
    [ -n "${stale}" ] || [ "$(manifest_field agentkitCommit)" = "${AGENTKIT_COMMIT_BUILT}" ] ||
        stale="the AgentKit pin changed"
    [ -n "${stale}" ] || [ "$(manifest_field agentkitDistSha256)" = "${AGENTKIT_DIST_SHA}" ] ||
        stale="the AgentKit dist changed"
    [ -n "${stale}" ] || [ "$(manifest_field bunVersion)" = "${BUN_VERSION}" ] ||
        stale="a different Bun built it ($(manifest_field bunVersion))"
    [ -n "${stale}" ] || [ "$(manifest_field bunTarget)" = "${BUN_TARGET}" ] ||
        stale="it was built for $(manifest_field bunTarget)"
    [ -n "${stale}" ] || [ "$(manifest_field rustTriple)" = "${TRIPLE}" ] ||
        stale="it was built for $(manifest_field rustTriple)"
    [ -n "${stale}" ] || [ "$(manifest_field artifactSha256)" = "$(sha256_file "${DEST}")" ] ||
        stale="the staged binary is not the one the manifest recorded"
    if [ -z "${stale}" ]; then
        reuse=1
    else
        echo "==> Rebuilding: ${stale}"
    fi
fi

mkdir -p "${DEST_DIR}"
if [ "${reuse}" = "1" ]; then
    echo "==> Verified staged sidecar, nothing to rebuild: ${DEST}"
else
    echo "==> Compiling onecad-assistant-host for ${TRIPLE} (${BUN_TARGET})"
    (
        cd "${ROOT_DIR}/assistant-host" &&
            bun build --compile --target="${BUN_TARGET}" src/main.ts --outfile "${DEST}"
    )
    chmod +x "${DEST}"

    cat >"${MANIFEST}" <<JSON
{
  "schema": 1,
  "onecadRevision": "${ONECAD_REVISION}",
  "onecadWorktree": "${ONECAD_DIRTY}",
  "sourceSha256": "${SOURCE_SHA}",
  "agentkitCommit": "${AGENTKIT_COMMIT_BUILT}",
  "agentkitDistSha256": "${AGENTKIT_DIST_SHA}",
  "agentkitContractVersion": "${AGENTKIT_CONTRACT_VERSION}",
  "ocakProtocolVersion": "${OCAK_PROTOCOL_VERSION}",
  "hostPackageVersion": "${HOST_PACKAGE_VERSION}",
  "bunVersion": "${BUN_VERSION}",
  "bunTarget": "${BUN_TARGET}",
  "rustTriple": "${TRIPLE}",
  "artifact": "${STAGED_NAME}",
  "artifactSha256": "$(sha256_file "${DEST}")",
  "builtAt": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
fi

echo "==> Staged sidecar: ${DEST}"
ls -la "${DEST}"
echo "==> Manifest: ${MANIFEST}"
cat "${MANIFEST}"
