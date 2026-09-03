---
name: repo-wpp-projection
description: WP-P projection facts a future implementer needs — where the guards live, which invariants bind, and the traps that are easy to reintroduce
metadata:
  type: project
---

WP-P projected sketch geometry (`src-tauri/src/sketch_projection.rs`, `api::project_to_sketch` /
`update_projection` / `detach_projection`, `onecad-core` `apply_sketch_ops`).

**Why:** re-association of a projected entity to its source is positional
(`source_element_id`, `source_ordinal`), which is only sound while the source's emission RUN is
unchanged. Getting it wrong is the H5-B silent-wrong-bind class the whole migration exists to remove.

**How to apply:**

- `sketch_upsert` (`api/mod.rs`) deserializes `Vec<SketchEditOp>` **straight from the frontend**.
  Any new `SketchEditOp` variant is therefore FE-reachable and needs its own guard — this is how the
  unscoped `SetEntityReferenceLocked{locked:false}` hole got in.
- Projected POINTS carry **no** `projections` row (`projected_sketch_content` only rows the curves);
  they are recognised through the curves that reference them. Any rule written as "has a provenance
  row" silently breaks detach and update.
- `add_sketch_on_face` seeds host boundary geometry `reference_locked` with **no** provenance rows
  (`_projections` is always empty there). Consequence: `squash_sketch_session`'s redo cannot remove
  it, so redo of a session on a plain sketch-on-face sketch is still broken. Pre-existing.
- Worker `faceOutline` rules that make counts exact (`worker/src/sketch/EdgeProjector.cpp`): shared
  `TopoDS_Edge` emitted once; a DEGENERATE boundary edge is SKIPPED, not refused (only a NAMED
  refusal fails the source); `appendPrimitives`/`lineExists` suppress a coincident duplicate line
  within one source's run — so a box cap seen edge-on is ONE line over two merged points, not two.
- The worker always emits `points`/`entities`/`refusals` as `json::array()`
  (`session/ProjectToSketchPlane.cpp`), so strict "present-but-not-an-array is an error" parsing is
  safe.
- `projection_stale` must be keyed by `SketchId`, never by timeline index — every rollback, insert
  or record delete moves an index and would re-attach the verdict to a different op.
