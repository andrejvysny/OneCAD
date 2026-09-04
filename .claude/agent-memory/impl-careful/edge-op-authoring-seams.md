---
name: edge-op-authoring-seams
description: The four seams a fillet/chamfer op param must be threaded through, and where the frontend's core-serde vs OCW1-wire boundary actually sits
metadata:
  type: project
---

Adding a field to a Fillet/Chamfer op touches four frontend seams and nowhere
else. Learned building the Chamfer reference face (kernel-hardening WP-F).

**Why:** each seam silently DROPS what it does not know about, so a field added
to only three of them previews correctly and commits wrong (or vice versa).

**How to apply:** read before adding any op-param field on the frontend.

- `src/ipc/types.ts` — the frontend domain type (`FilletParams`).
- `src/ipc/tauriCommandMap.ts` `filletParams()` — marshals to the **core serde**
  shape. `wireOperation` DROPS an op's top-level `inputs`, so typed `ElementRef`s
  must be synthesized INTO `params` here from `op.inputs`; Rust re-derives the
  op's `inputs[]` from the params. The OCW1 lowering (and any wire-only stripping)
  is `src-tauri/src/worker/wire.rs`'s job, not the frontend's.
- `src/ipc/previewOps.ts` `edgeOpBuilder` — the SAME builder produces the preview
  op and, via `localSolver.endPreview(…, true)`, the COMMITTED op. A field it
  drops is a field the fresh-author record never had.
- `ModelToolController.edgeOpParams()` / `edgeOpInputs()` — one derivation each,
  read by both the preview and the commit call site.

Re-edit is a different lane: `updateScalarParamsCommand(record, opType, base,
patch)` sends a WHOLE `Operation` (`UpdateOperationParams` carries no patch type
in Rust) built from a SHALLOW `{...base, ...patch}` merge — so removing a field
means `delete base.<key>`, never a patch entry.

`inputPathFor(opType, slot, params?, seedEdgeId?)` in `tauriCommandMap.ts` is the
frontend twin of Rust `wire_op_inputs`'s slot table; its test in
`tauriCommandMap.test.ts` is written to be read side by side with
`wire.rs::wire_op_inputs_slot_order_is_the_repair_slot_table`. Change one, change
both.

Re-edit is also where "required" is decided. Rust's
`validate_chamfer_reference_faces_required` only demands a typed field on the
edit that INTRODUCES the asymmetry (prior equal-leg or Fillet) or moves
`edgeIds`. A scalar edit on a LEGACY asymmetric record must send NEITHER key —
authoring one there persists an ordinal guess the user never made and makes the
§9 repair item disappear. And because the merge is `{...base, ...patch}`, an
empty array is NOT the same as omitting: Rust reads `referenceFaces: []` on a
record that has pairs as "strip them".

