---
name: repo-rust-lane-gotchas
description: Non-obvious mechanics of the OneCAD Rust lane an implementer hits when adding a field to an op's params or a repair reason
metadata:
  type: project
---

Adding a field to a `KnownOperation` params struct in `onecad-core` touches ~19
struct-literal sites across `src/`, `crates/`, and `tests/` (no `..Default`
anywhere) — `cargo check --workspace --all-targets` enumerates them all.

**Why:** the params structs are exhaustively constructed on purpose, so a new
field cannot be silently defaulted into a record the author did not think about.

**How to apply:** budget for the fan-out; never bulk-edit these with a
line-offset script (a mis-count silently DROPS a `#[serde(...)]` attribute and
the schema-freeze snapshots then move) — patch by matching text, and audit
`git diff | grep '^-'` for unintended deletions before running the gate.

Related mechanics worth knowing:

- `RepairReason` has a HAND-WRITTEN `Deserialize` (`document/repair.rs`) that
  degrades unknown tokens to `Unknown`; a new variant needs an arm there AND, if
  its wire token is not kebab-case, a `#[serde(rename = "...")]` for the derived
  `Serialize`. Three more places enumerate the reasons: `dto::repair_reason_str`,
  `crates/onecad-regen/src/main.rs::repair_reason_token`, and the TS
  `NeedsRepairItem.reason` doc.
- `DocumentRuntime` holds only `GeometryEngine` + `MeshProvider` + `SolverEngine`.
  The other worker facets (`FaceBoundaryProjection`, `ElementQuery`, …) live on
  `AppState` and are reached by the `#[tauri::command]` wrappers, so a runtime
  method that needs one takes it as a parameter rather than a new field.
- `RegenRequest::ToEnd { from: 0 }` DOES claim SCHEMA §7.2 `editedFrom: 0`
  (kernel-hardening WP-A). `RevertToEnd` is the only full-timeline replay lane
  that makes NO edit claim — use it when a test needs a genuine no-edit replay.
  A worker guard must NOT be gated on that claim: it would fire on open and
  vanish on redo (WP-F review, 2026-09-04 — the chamfer reference-face guard now
  halts in every lane instead).
- A repair item that names an EMPTY op-input slot cannot address its seed element
  by id: the halted step never committed, so the binding it resolved on the
  scratch state died with it and `PrepareEdgeOp{elementId}` answers
  `REF_UNRESOLVED`. Re-bind through the ladder from the record's own stored typed
  ref first, then address by the `{bodyId, topoKey}` the ladder answers with.
- `ElementInfoDto::magnitude` is the exact `GProp` quantity (face AREA, edge arc
  length, solid volume) — verified on a plain box, 2026-09-04. `normal` is the
  OCCT SURFACE normal, NOT outward-oriented: a box's two opposite walls report
  opposite signs and its top and bottom share `+Z`, so identify a face by axis
  PLUS its bbox `center`, never by the normal's sign alone.

- `edit_operation_input` (`edit/session.rs`) historically ran only
  `validate_shell_lockstep`, so an `EditOperationInput` could write params that
  `UpdateOperationParams` then refuses — the edit succeeds and SOFT-BRICKS the
  record. Any new `InputPath` must add its op's validation there too.
- A repair candidate's `world_pos` is where the panel HIGHLIGHTS it and where the
  ref it creates is ANCHORED. Never give two candidates one shared point (e.g. an
  edge anchor that lies on both adjacent faces): it stacks them in the UI and
  leaves an anchor the ladder can never use to separate them. Measure each
  candidate (`query_element_by_topo_key` → `center`) and drop one you cannot.
