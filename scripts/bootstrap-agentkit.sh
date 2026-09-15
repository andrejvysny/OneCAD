#!/usr/bin/env bash
# Build AgentKit from source into a gitignored local prefix, so `assistant-host`
# has something to depend on — and record enough about what was built that the
# next run can tell whether it is still valid.
#
# WHY THIS EXISTS. AgentKit's documented consumer install is a GitHub tag:
#
#     "agentkit": "github:andrejvysny/AgentKit#v0.5.0"
#
# backed by a committed `packages/agentkit/dist/` that its release workflow
# produces. As of this writing that workflow has never run: the repository has
# no tags at all and `packages/agentkit/` holds only a README and a
# package.json. `github:...#<commit>` does not help either — it resolves to the
# private workspace root, which exports nothing.
#
# So until `v0.5.0` is cut, this script does locally what the release workflow
# does remotely: clone the pinned commit, build every package's dist, then
# assemble the umbrella dist. `assistant-host/package.json` depends on the
# result by path.
#
# WHEN THE TAG LANDS: replace assistant-host's dependency with the tag, delete
# this script, and drop its call sites (CI, docs/assistant/, CLAUDE.md).
# Nothing else in the tree depends on it.
#
# ── PROVENANCE (F15) ─────────────────────────────────────────────────────────
#
# The early exit used to be "HEAD is the pinned commit and a dist file exists",
# which establishes neither that the dist came FROM that commit nor that the
# source and lockfile have not moved since. A stale or mismatched dist could
# therefore be linked into a shipped binary. What this script now records, and
# re-checks before skipping any work, is: the commit, the worktree being clean,
# the lockfile digest, the dist digest, and the Bun that built it. Any
# disagreement rebuilds; nothing is assumed from a file merely existing.
#
# ── CLEANUP (F15) ────────────────────────────────────────────────────────────
#
# This script used to `rm -rf "${PREFIX}"` on a path the caller supplies through
# `AGENTKIT_PREFIX`. It now removes a directory ONLY when that directory carries
# the ownership marker this script wrote, and refuses to touch anything else.
set -euo pipefail

# The commit the specification package was reviewed against. Bump deliberately.
AGENTKIT_COMMIT="${AGENTKIT_COMMIT:-a450d6ff470fcfaacf3a37375d00d962ed2632e8}"
AGENTKIT_REMOTE="${AGENTKIT_REMOTE:-https://github.com/andrejvysny/agentkit}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${AGENTKIT_PREFIX:-${ROOT_DIR}/.agentkit-src}"

# Written by this script into a prefix it created, and the ONLY thing that
# authorises deleting one.
MARKER="${PREFIX}/.onecad-agentkit-prefix"
# What was built, and from what.
STAMP="${PREFIX}/.onecad-agentkit-build.json"
DIST="${PREFIX}/packages/agentkit/dist/index.js"
LOCKFILE="${PREFIX}/bun.lock"

if ! command -v bun >/dev/null 2>&1; then
    echo "bootstrap-agentkit: bun is required but not on PATH" >&2
    exit 1
fi
BUN_VERSION="$(bun --version)"

# sha256 of a file, on either CI platform. Empty when the file is absent, which
# is a digest that can never match a recorded one.
sha256_file() {
    [ -f "$1" ] || return 0
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# One field out of the stamp. The stamp is written one field per line, so this
# needs no JSON parser and therefore no dependency.
stamp_field() {
    [ -f "${STAMP}" ] || return 0
    sed -n "s/.*\"$1\": *\"\([^\"]*\)\".*/\1/p" "${STAMP}"
}

write_stamp() {
    cat >"${STAMP}" <<JSON
{
  "commit": "${AGENTKIT_COMMIT}",
  "remote": "${AGENTKIT_REMOTE}",
  "lockfileSha256": "$(sha256_file "${LOCKFILE}")",
  "distSha256": "$(sha256_file "${DIST}")",
  "bunVersion": "${BUN_VERSION}",
  "builtAt": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
}

# ── Is the cached build still the build we want? ─────────────────────────────
if [ -d "${PREFIX}/.git" ]; then
    head="$(git -C "${PREFIX}" rev-parse HEAD)"
    # This script's own marker and stamp live inside the prefix and are not
    # tracked by AgentKit, so they are not what "dirty" is about.
    dirty="$(git -C "${PREFIX}" status --porcelain |
        grep -v -e '\.onecad-agentkit-build\.json$' -e '\.onecad-agentkit-prefix$' || true)"

    if [ -n "${dirty}" ] && [ "${ONECAD_AGENTKIT_ALLOW_DIRTY:-0}" != "1" ]; then
        echo "bootstrap-agentkit: ${PREFIX} has local modifications:" >&2
        echo "${dirty}" >&2
        echo "bootstrap-agentkit: this is generated, pinned source — a fix belongs in AgentKit" >&2
        echo "                    and a new pin here. Rebuilding over it would discard the edit" >&2
        echo "                    silently and ship a dist nothing can reproduce." >&2
        echo "                    Set ONECAD_AGENTKIT_ALLOW_DIRTY=1 to build anyway." >&2
        exit 1
    fi

    reason=""
    [ "${head}" = "${AGENTKIT_COMMIT}" ] || reason="prefix is at ${head}, want ${AGENTKIT_COMMIT}"
    [ -n "${reason}" ] || [ -f "${STAMP}" ] || reason="no build stamp: nothing records what this dist was built from"
    [ -n "${reason}" ] || [ -f "${DIST}" ] || reason="the umbrella dist is missing"
    [ -n "${reason}" ] || [ "$(stamp_field commit)" = "${AGENTKIT_COMMIT}" ] ||
        reason="the dist was built from $(stamp_field commit), not ${AGENTKIT_COMMIT}"
    [ -n "${reason}" ] || [ "$(stamp_field distSha256)" = "$(sha256_file "${DIST}")" ] ||
        reason="the dist has changed since it was built"
    [ -n "${reason}" ] || [ "$(stamp_field lockfileSha256)" = "$(sha256_file "${LOCKFILE}")" ] ||
        reason="the lockfile has changed since the dist was built"

    if [ -z "${reason}" ]; then
        echo "bootstrap-agentkit: verified cached build at ${AGENTKIT_COMMIT}"
        echo "                    dist $(stamp_field distSha256)"
        exit 0
    fi
    echo "bootstrap-agentkit: rebuilding — ${reason}"

    if [ "${head}" != "${AGENTKIT_COMMIT}" ]; then
        git -C "${PREFIX}" fetch --depth 1 origin "${AGENTKIT_COMMIT}"
        git -C "${PREFIX}" checkout --detach FETCH_HEAD
    fi
else
    # A prefix this script did not create is not a prefix this script deletes.
    if [ -e "${PREFIX}" ]; then
        if [ -f "${MARKER}" ]; then
            echo "bootstrap-agentkit: replacing the generated prefix at ${PREFIX}"
            rm -rf "${PREFIX}"
        elif [ -d "${PREFIX}" ] && [ -z "$(ls -A "${PREFIX}")" ]; then
            : # An empty directory is safe to build into.
        else
            echo "bootstrap-agentkit: ${PREFIX} exists, is not a git checkout, and carries no" >&2
            echo "                    ${MARKER##*/} marker, so this script did not create it." >&2
            echo "                    Refusing to delete it. Remove it yourself, or point" >&2
            echo "                    AGENTKIT_PREFIX somewhere this script owns." >&2
            exit 1
        fi
    fi
    mkdir -p "${PREFIX}"
    printf 'Generated by scripts/bootstrap-agentkit.sh. Safe to delete.\n' >"${MARKER}"
    # Fetch exactly the pinned commit; no history, no LFS.
    git -C "${PREFIX}" init -q
    git -C "${PREFIX}" remote add origin "${AGENTKIT_REMOTE}"
    GIT_LFS_SKIP_SMUDGE=1 git -C "${PREFIX}" fetch --depth 1 origin "${AGENTKIT_COMMIT}"
    git -C "${PREFIX}" checkout -q --detach FETCH_HEAD
fi

# The pin is a pin: whatever the remote answered with, this must be it.
actual="$(git -C "${PREFIX}" rev-parse HEAD)"
if [ "${actual}" != "${AGENTKIT_COMMIT}" ]; then
    echo "bootstrap-agentkit: checked out ${actual}, expected ${AGENTKIT_COMMIT}" >&2
    exit 1
fi
[ -f "${MARKER}" ] || printf 'Generated by scripts/bootstrap-agentkit.sh. Safe to delete.\n' >"${MARKER}"

echo "bootstrap-agentkit: installing workspace dependencies (frozen)"
(cd "${PREFIX}" && bun install --frozen-lockfile)

echo "bootstrap-agentkit: building package dists"
(cd "${PREFIX}" && bun run build)

echo "bootstrap-agentkit: assembling the umbrella dist"
(cd "${PREFIX}" && bun run build:umbrella)

if [ ! -f "${DIST}" ]; then
    echo "bootstrap-agentkit: umbrella build produced no dist/index.js" >&2
    exit 1
fi

write_stamp
echo "bootstrap-agentkit: ready at ${PREFIX}/packages/agentkit (commit ${AGENTKIT_COMMIT})"
echo "                    dist $(sha256_file "${DIST}")"
