---
name: repo-ocw1-driver
description: worker_harness --repl desyncs on event-emitting verbs; drive OCW1 directly to author a fixture
metadata:
  type: project
---

Authoring a `protocol/fixtures/*.ndjson` needs the REAL frames first, and the shipped
tool cannot get them for every verb.

**Why:** `worker_harness --repl` (`worker/tools/harness/main.cpp`, `run_repl`) reads
EXACTLY ONE frame per request line. `ExecutePlan` emits one `planStep` event per step
before its `resp`, so the repl desyncs immediately and every later response is silently
mis-attributed. It also needs one whole envelope per LINE — a pretty-printed request is a
parse error.

**How to apply:**

- Drive the worker directly instead. The frame is trivial: `b"OCW1"` +
  `struct.pack("<II", json_len, bin_len)` + JSON + bin, both directions
  (`worker/src/protocol/Frame.h`, 12-byte header). Read frames until the `resp` whose `id`
  matches the request. ~50 lines of Python; a working copy is in this session's scratchpad
  as `ocw1.py`.
- Param names that are easy to guess wrong: Extrude takes `booleanMode`
  (`NewBody`/`Add`/`Cut`/`Intersect`) and `extrudeMode` (`Blind`/`Symmetric`/...), plus
  `targetBodyId` for a boolean — NOT `operation`/`direction`. Getting it wrong silently
  makes a NEW body instead of fusing.
- `BindElementIds` rejects an `elementId` that does not look like one (`box_1` →
  `PROTOCOL_ERROR "malformed bodyId or elementId"`); use the `el_*` convention.
- `QueryBodyTopology` returns only `{faceCount, solidCount}`. To learn an edge's
  neighbours, bind `e:1..e:N` one at a time (a whole batch fails on the first bad key) and
  call `PrepareEdgeOp`, which reports `adjacentFaces`, `contour` and the anchor.
- A tangent pair (e.g. the two halves OCCT splits a cylinder/cylinder seam into) must be
  named in FULL in `edgeIds` when `tangentClosureVersion: 1`, or the step halts
  `EDGE_OP_TANGENT_CLOSURE_CHANGED`.
- The fixture matcher has no `$present` for objects but `"$present"` works as a leaf, and
  `$any` means "key present, any value". Numbers compare EXACTLY unless the file sets a
  tolerance, so make every OCCT measurement `$any` and pin the measured value in a ctest.
