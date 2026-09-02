#!/usr/bin/env bash
# check_project_to_sketch_plane_frames.sh — SCHEMA §7.6 `ProjectToSketchPlane`
# determinism, ACROSS PROCESSES (WP-P).
#
# `test_edge_projector` projects twice inside ONE process and compares. That
# cannot see the failure this gate exists for: an answer that depends on anything
# process-scoped — a static initialised in load order, an OCCT session tolerance,
# an uninitialised read that happens to be stable within a run. §7.6 claims the
# response is byte-identical across FRESH worker processes, and only two fresh
# processes can falsify that claim.
#
# So: replay the CANONICAL FIXTURE's own request stream twice through the
# harness's `--repl` mode, each replay spawning its own worker, and `cmp` the two
# transcripts. The requests are extracted from the fixture itself, so the geometry
# under test is the contract's own and cannot drift away from it. `--repl` reads
# exactly one frame per request, so the nine `planStep` events the two plans emit
# would leave the tail of the stream unread; the drain requests appended below
# pull it through, and the `Shutdown` line is dropped because EOF on stdin is
# already a clean shutdown.
#
# NO DIGEST LITERAL is pinned here or in the fixture, deliberately. The transcript
# carries `projectedHash` values computed from OCCT doubles, and libm differs
# between macOS and Linux; a frozen digest would gate the platform rather than the
# determinism. What is portable is that one platform agrees with ITSELF.
#
# Usage: check_project_to_sketch_plane_frames.sh <harness> <worker> <fixture>
set -euo pipefail

HARNESS="${1:?harness binary path required}"
WORKER="${2:?worker binary path required}"
FIXTURE="${3:?fixture path required}"

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

# Every `send` envelope, in order, minus Shutdown.
sed -n 's/^{"send"://p' "${FIXTURE}" | sed 's/}$//' \
    | grep -v '"verb":"Shutdown"' > "${work}/requests.ndjson"

if [ ! -s "${work}/requests.ndjson" ]; then
    echo "project-frames: extracted no requests from ${FIXTURE} — refusing to pass vacuously"
    exit 1
fi
for _ in 1 2 3 4 5 6 7 8 9; do
    echo '{"v":1,"t":"req","id":900,"verb":"GetWorkerHead","args":{}}' >> "${work}/requests.ndjson"
done

for run in 1 2; do
    echo "project-frames: replay ${run} of 2 (fresh worker process)"
    "${HARNESS}" --repl --worker "${WORKER}" < "${work}/requests.ndjson" > "${work}/run${run}.txt"
    if ! grep -q '"type":"Ellipse"' "${work}/run${run}.txt"; then
        echo "project-frames: replay ${run} produced no Ellipse — the transcript is not the contract's"
        exit 1
    fi
done

if cmp -s "${work}/run1.txt" "${work}/run2.txt"; then
    echo "project-frames: two fresh worker processes produced byte-identical transcripts ($(wc -l < "${work}/run1.txt") frames)"
    exit 0
fi

echo "project-frames: the two runs DIFFER — the projection is not process-independent"
diff "${work}/run1.txt" "${work}/run2.txt" | head -20 || true
exit 1
