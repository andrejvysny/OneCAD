#!/usr/bin/env bash
# check_prism_profile_bytes.sh — SCHEMA §7.8 `ExtractPrismProfile` determinism,
# ACROSS PROCESSES (Component Library WP-C, review finding F6).
#
# `test_extract_prism_profile` bakes twice inside ONE process and compares the
# bytes. That cannot see the failure this gate exists for: a canonical frame that
# depends on anything process-scoped — a static initialised in load order, an
# OCCT session tolerance, an uninitialised read that happens to be stable within
# a run. §7.8 claims the written bytes are byte-identical "across fresh worker
# processes", and only two fresh processes can falsify that claim.
#
# So: replay the canonical fixture twice, each replay spawning its own worker,
# keep the file each one wrote, and `cmp` them. The fixture is the SAME one the
# `canonical_place_component_profile` test drives, so the geometry under test is
# the contract's own and cannot drift away from it.
#
# NO DIGEST LITERAL is pinned here or in the fixture, deliberately. libm differs
# between macOS and Linux, so a frozen SHA-256 would gate the platform rather than
# the determinism; what is portable is that one platform agrees with ITSELF.
#
# Usage: check_prism_profile_bytes.sh <harness> <worker> <fixture> <written_path>
set -euo pipefail

HARNESS="${1:?harness binary path required}"
WORKER="${2:?worker binary path required}"
FIXTURE="${3:?fixture path required}"
WRITTEN="${4:?the path the fixture tells the worker to write required}"

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

for run in 1 2; do
    rm -f "${WRITTEN}"
    echo "prism-bytes: replay ${run} of 2 (fresh worker process)"
    "${HARNESS}" --worker "${WORKER}" --fixture "${FIXTURE}"
    if [ ! -s "${WRITTEN}" ]; then
        echo "prism-bytes: replay ${run} wrote nothing to ${WRITTEN} — refusing to pass vacuously"
        exit 1
    fi
    cp "${WRITTEN}" "${work}/run${run}.brep"
done

if cmp -s "${work}/run1.brep" "${work}/run2.brep"; then
    echo "prism-bytes: two fresh worker processes wrote byte-identical canonical profiles ($(wc -c < "${work}/run1.brep") bytes)"
    exit 0
fi

echo "prism-bytes: the two runs DIFFER — the canonical frame is not process-independent"
cmp "${work}/run1.brep" "${work}/run2.brep" || true
exit 1
