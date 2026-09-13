---
name: regen-provenance-in-tests
description: Real-worker integration tests must pick RevertToEnd vs ToEnd{from} deliberately — resolver v6 blanket-refuses anchor-only refs downstream of editedFrom
metadata:
  type: project
---

A `src-tauri/tests/*.rs` real-worker test that calls `rt.run_regen(...)` is
CHOOSING the SCHEMA §7.2 `editedFrom` claim, and that choice is load-bearing.
Split the helper the way `feature_pattern_integration.rs` does:

- `clean_regen(rt)` → `RegenRequest::RevertToEnd { from: 0 }` — first build, a
  record merely pushed onto the timeline, save → reopen, and after `rt.undo()`.
  Makes NO `editedFrom` claim.
- `regen_from(rt, k)` → `RegenRequest::ToEnd { from: k }` — a real
  `UpdateOperationParams` / `EditOperationInput` / `SetOperationSuppression` /
  `RemoveOperation` / variable edit, with `k` the edited step (a `RemoveOperation`
  uses the POST-removal index; a variable edit dirties from 0).

**Why:** production changed in `ee5b449` —
`api::enqueue_initial_regen_if_current` mints `RevertToEnd { from: 0 }` for
new/open/import/restart, so a blind `ToEnd { from: 0 }` helper models nothing a
user can reach. The same commit landed resolver v6
(`worker/src/elementmap/Ladder.cpp`, EDIT-SCOPED DESCRIPTOR-TIE VETO), which for
any step with `stepIndex > editedFrom` refuses a candidate whose DESCRIPTOR-only
separation from its best rival is ~0 — even at score 1.0 with a wide blended
margin. An ANCHOR-ONLY `ElementRef` (`primary` + `anchor`, no `intent`) scores 0
in descriptor space against every candidate, so post-edit it is refused
unconditionally: v6 removed the v5 anchor-exact carve-out. A symmetric box's
face/edge picks are exactly that shape.

**How to apply:** when a real-worker test reports `NeedsRepair` where an
auto-bind was expected and the `RepairItem` shows `ladder_failed: Descriptor`,
`reason: Ambiguous`, `scoring_version: Some(6)` and `featureContributions` with
only an `anchor` term — the lane is wrong, not the geometry. Check the helper
before touching anything else. Conversely, a ref promoted in the LIVE session
resolves at rung 1 and never sees v6; a promotion does NOT survive
`DocumentRuntime::open` (a reopened document has no element map), so reopened
refs always go through the ladder.

Provenance does NOT fix an anchor-only ref on a genuine edit lane — v6 refuses it
by design (decision D-3, 2026-09-13: v6 is the normative contract). Those tests
now assert the refusal and then walk the repair round-trip. Two core refusals
shape what a repair may look like on an ASYMMETRIC chamfer, and both are by name
in `edit/session.rs`: `EditOperationInput{FilletEdges}` alone is refused because
moving `edgeIds[0]` either introduces an uncovered asymmetry
(`validate_chamfer_reference_faces_required`) or orphans the `referenceFaces`
pair keyed by the old id ("not in edgeIds"). The contour and its reference face
move TOGETHER, in one `UpdateOperationParams` authoring the complete typed set.

Also: a repair's own regen must claim the REPAIRED record's step, not the
original edit's. Repairing a revolve axis at step 3 and then regenerating with
`ToEnd { from: 1 }` re-runs the axis edge's PRODUCER, which throws away the
promotion the repair just made and re-refuses it; `ToEnd { from: 3 }` keeps it.
