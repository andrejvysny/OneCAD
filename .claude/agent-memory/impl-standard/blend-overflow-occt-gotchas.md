---
name: blend-overflow-occt-gotchas
description: OCCT 8.0.1 gotchas hit building worker/src/kernel/fillet/BlendOverflow
metadata:
  type: project
---

- MapShapesAndAncestors VERTEX->FACE lists a face once per incident edge: dedupe with IsSame before counting.
- BRepAlgoAPI_Cut::Modified is non-const; use `ex.what()`, `true/false` (Standard_True/GetMessageString deprecated); worker/.clang-format is DisableFormat.
- `scripts/build-worker.sh` exceeds 120s: run with timeout 600000 (a timed-out run leaves a stale test binary).
