#!/usr/bin/env bash
# check_interop.sh — C++ worker <-> Rust-authored contract smoke (W-WP3c).
#
# The canonical fixtures in protocol/fixtures/ are authored by the RUST track
# (they are the executable form of protocol/SCHEMA.md that the Rust
# ProtocolClient runs in its own CI). Replaying them against the C++
# ./onecad-worker via the harness proves the worker speaks the EXACT SCHEMA
# envelope the Rust core expects — byte-for-byte, unsolicited hello first,
# OpenSession/GetWorkerHead/Shutdown lifecycle, and the well-framed
# PROTOCOL_ERROR path — WITHOUT compiling any Rust here.
#
# BOUNDARY: this is the static-contract smoke. Full LIVE Rust<->C++ interop
# (spawning the real Rust ProtocolClient process against the C++ worker binary
# and vice-versa) is intentionally OUT OF SCOPE for W-WP3c and lands in
# W-WP4 / R-WP11. Until then, "the worker passes the canonical fixtures" is the
# interop guarantee, and it is enforced on every build by this test.
#
# Usage: check_interop.sh <harness> <worker> <protocol_fixtures_dir>
set -euo pipefail

HARNESS="${1:?harness binary path required}"
WORKER="${2:?worker binary path required}"
FIXDIR="${3:?protocol/fixtures dir required}"

# ENUMERATED, never listed: a hardcoded set is how a canonical fixture escapes
# this lane (boolean_empty_refusal.ndjson did exactly that). Every *.ndjson in
# protocol/fixtures/ is part of the contract, so every one of them is replayed.
rc=0
shopt -s nullglob
fixtures=("${FIXDIR}"/*.ndjson)
if [ "${#fixtures[@]}" -eq 0 ]; then
    echo "interop: NO fixtures found in ${FIXDIR} — refusing to pass vacuously"
    exit 1
fi
for path in "${fixtures[@]}"; do
    fx="$(basename "${path}")"
    echo "interop: replaying Rust-authored ${path} against ${WORKER}"
    if "${HARNESS}" --worker "${WORKER}" --fixture "${path}"; then
        echo "interop: ${fx} OK"
    else
        echo "interop: ${fx} FAILED"
        rc=1
    fi
done

if [ "${rc}" -eq 0 ]; then
    echo "interop: all canonical fixtures pass (static contract OK; live interop = W-WP4/R-WP11)"
fi
exit "${rc}"
