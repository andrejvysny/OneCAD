#!/usr/bin/env bash
# Build AgentKit from source into a gitignored local prefix, so `assistant-host`
# has something to depend on.
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
set -euo pipefail

# The commit the specification package was reviewed against. Bump deliberately.
AGENTKIT_COMMIT="${AGENTKIT_COMMIT:-a450d6ff470fcfaacf3a37375d00d962ed2632e8}"
AGENTKIT_REMOTE="${AGENTKIT_REMOTE:-https://github.com/andrejvysny/agentkit}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${AGENTKIT_PREFIX:-${ROOT_DIR}/.agentkit-src}"

if ! command -v bun >/dev/null 2>&1; then
    echo "bootstrap-agentkit: bun is required but not on PATH" >&2
    exit 1
fi

if [ -d "${PREFIX}/.git" ]; then
    have="$(git -C "${PREFIX}" rev-parse HEAD)"
    if [ "${have}" = "${AGENTKIT_COMMIT}" ] && [ -f "${PREFIX}/packages/agentkit/dist/index.js" ]; then
        echo "bootstrap-agentkit: already at ${AGENTKIT_COMMIT} with a built dist; nothing to do"
        exit 0
    fi
    echo "bootstrap-agentkit: prefix is at ${have}, want ${AGENTKIT_COMMIT} — refetching"
    git -C "${PREFIX}" fetch --depth 1 origin "${AGENTKIT_COMMIT}"
    git -C "${PREFIX}" checkout --detach FETCH_HEAD
else
    rm -rf "${PREFIX}"
    # Fetch exactly the pinned commit; no history, no LFS.
    mkdir -p "${PREFIX}"
    git -C "${PREFIX}" init -q
    git -C "${PREFIX}" remote add origin "${AGENTKIT_REMOTE}"
    GIT_LFS_SKIP_SMUDGE=1 git -C "${PREFIX}" fetch --depth 1 origin "${AGENTKIT_COMMIT}"
    git -C "${PREFIX}" checkout -q --detach FETCH_HEAD
fi

echo "bootstrap-agentkit: installing workspace dependencies"
(cd "${PREFIX}" && bun install --frozen-lockfile)

echo "bootstrap-agentkit: building package dists"
(cd "${PREFIX}" && bun run build)

echo "bootstrap-agentkit: assembling the umbrella dist"
(cd "${PREFIX}" && bun run build:umbrella)

if [ ! -f "${PREFIX}/packages/agentkit/dist/index.js" ]; then
    echo "bootstrap-agentkit: umbrella build produced no dist/index.js" >&2
    exit 1
fi

echo "bootstrap-agentkit: ready at ${PREFIX}/packages/agentkit (commit ${AGENTKIT_COMMIT})"
