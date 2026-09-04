---
name: frontend-gotchas
description: OneCAD frontend implementation gotchas — engine test doubles, frozen contracts vs new tools, status-hint shape, preview-session frozen inputs, mock box adjacency
metadata:
  type: project
---

Durable facts for anyone adding a frontend feature to OneCAD. Learned while
building the Project-edges tool (WP-P).

**Why:** each of these cost a full test cycle to discover; none is stated in
CLAUDE.md and none is derivable without running the suite.

**How to apply:** read before adding a viewport method, a tool, or a status hint.

- Adding a method to `ViewportEngine` breaks ~25 hand-rolled engine doubles in
  `src/**/*.test.ts` (they are cast `as unknown as ViewportEngine`, so `tsc` is
  blind to the gap). Grep the doubles by an existing sibling method name and add
  the new key to each; there is no shared factory.
- `viewportStore`'s `StatusSeverity` is only `"info" | "error"` — there is no
  `warning` rung. A partial refusal has to be reported as `error`.
- `sketchService.ts` reaches the viewport through the module singleton
  `getViewportEngine()` (engine BRIDGE), not through `SketchController`'s
  `deps.engine`. A controller test that asserts on an engine double must also
  register it via `setViewportEngine()` or the write goes nowhere.
- A new sketch/model tool must be added in five places or something breaks:
  the `SketchTool`/`ModelTool` union, `ModelingSketchTools`/`ModelingModelTools`
  in `modules/modeling/ids.ts` (`satisfies Record<Tool, string>` — exhaustive),
  `SKETCH_TOOL_DESCRIPTORS`/`MODEL_TOOL_DESCRIPTORS`, `MODELING_BINDINGS`, and
  `activateTool`'s `SKETCH_ONLY`/`MODEL_ONLY`.
- `toolbarHidden: true` on a descriptor is the supported "registered but no
  toolbar button" split — it keeps the frozen `toolbarContract.ts` intact while
  the tool still resolves its shortcut and mirrors into `platform.toolHost`.
  Skipping the descriptor entirely instead makes `register.test.ts`'s
  "every tool binding reaches its registered tool" probe fail.
- `useShortcuts.test.tsx`'s contributed-chord block needs a key no built-in
  binding owns; it currently uses `q` (it was `j` until Project-edges took it).
- Adding a registered PANEL requires amending the frozen
  `src/test/contracts/shellContract.ts`. To avoid that, mount new chrome as a
  plain child of an existing registered panel — the documented precedent is
  `ConstraintMenu`/`SketchErrorPulse` inside `SketchChromeBar`.
- `beginPreview` FREEZES a draft's `inputs[]`; only `params` flow through
  `updatePreview`. Any edit that changes the op's input SLOTS (e.g. a chamfer
  gaining a reference-face ref) must `closePreviewSessions()` + re-open, or the
  committed op — built by `buildPreviewOp` from the session's frozen inputs at
  `endPreview(…, true)` — will not match the previewed params.
- Adding an `await` on the FRESH-commit path of `ModelToolController` breaks the
  preview-epoch specs (`edgePrepare`, `edgeShellPreview`), which assume a ✓
  commits in the same turn after one `await flush()`. Gate any new await behind
  the narrow condition that needs it.
- `viewportStore.statusHint` is `{message, severity, sticky}`, not a string —
  assert on `hint?.message`.
- The mock lane CAN answer edge→face adjacency for the seed box (`body1`):
  `BOX_EDGE_PAIRS` + `BOX_FACES` in `mockMeshes.ts` are the same tables
  `makeBoxMesh` renders, so `mockFaceGeometry.mockAdjacentFaces` derives it. Any
  other body has no analytic topology — omit the field, never fabricate one.
- `mockClient.applyEditCommand`'s `editOperationInput` arm is a structural NO-OP
  (revision bump only): it does not update `featureParams`, so an input rebind is
  not observable through `getOperationParams` in the mock lane.
- A repair candidate's `worldPos` is the anchor of the thing the ITEM is about,
  not of the candidate. For an item naming an empty slot (a chamfer reference
  face) it is the seed EDGE's point, which sits on both adjacent faces — read
  `elementInfo(...).center` for the face instead before building the ref.
- `vitest run --reporter=basic` does not exist in this repo's Vitest 4 setup, and
  `console.log` from a test is swallowed. To get a value out of a test, write it
  to a file with `node:fs` under an env-var guard.

