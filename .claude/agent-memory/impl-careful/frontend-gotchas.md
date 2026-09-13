---
name: frontend-gotchas
description: OneCAD frontend implementation gotchas — engine test doubles, frozen contracts vs new tools, status-hint shape, preview-session frozen inputs, mock box adjacency, palette rows are registry commands
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
- `viewportStore`'s `StatusSeverity` is `"info" | "warn" | "error"` (the `warn`
  rung was added after the WP-P note that said it did not exist). `StatusBar`
  only styles `error` differently, so `warn` and `info` look the same on screen.
- `documentStore.applySnapshot` is a zustand `set` — it MERGES. A projection
  field that can go absent must therefore be `T | null`, never optional: an
  omitted key leaves the previous value in place instead of clearing it.
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
  to a file with `node:fs` under an env-var guard (or `expect(value).toBe(1)` and
  read the diff).
- `resetMockDocument()` does NOT reset the zustand `documentStore`, so
  `mockClient.getProjection()` still returns the PREVIOUS test's features while
  `nextFeatureId` has restarted — a "diff the projection before/after" trick to
  find a newly committed record's id silently finds nothing from the second test
  onwards. Read the id off the `ApplyOperationResult`'s own `features` instead.
- `tsconfig`'s lib target predates `Array.prototype.at`: `.at(-1)` type-errors
  (TS2550) even though vitest runs it fine. Use `rows[rows.length - 1]`.
- The ⌘K palette has no registry of its own: `buildPaletteItems` projects off
  `platform.commands/tools/workspaces`, so a new palette row is a registered
  CommandDefinition. For modeling that means three edits — `ModelingCommands` in
  `modules/modeling/ids.ts`, a `COMMAND_ACTIONS` entry (the map is
  `Record<ModelingCommandKey, …>`, so an id without an entry fails `tsc`), and
  `register.test.ts` asserts `platform.commands.size ===
  Object.keys(ModelingCommands).length`.
- `buildPaletteItems` COPIES `def.title` when it builds. A row whose title
  depends on live state needs a `get title()` accessor on the definition (legal
  against `readonly title: string`) AND `open` in `CommandPalette`'s `items`
  useMemo deps, or the memo serves whatever the last registry change captured.
- `api::undo` / `api::redo` answer with the `DocumentProjection`, never a label.
  `tauriClient` has to derive `opLabel` itself; it used to pass `undefined`, so
  "Undid X" only ever appeared on the mock lane.
- Every mutating mock verb settles through `withCursor()` in `mockClient.ts` —
  the one hook point for mirroring mock-only state into `documentStore` without
  touching each of the ~9 undo-stack push sites.
- The sketch chrome bar's ONLY channel to `SketchController` is the tool-mode
  flip (no controller singleton — it is constructed inside `ViewportRoot`), so a
  button that needs the controller to behave differently arms a store flag first
  and the controller consumes it in the mode subscription (`sketchStore.exitIntent`,
  WP-U7).
- An e2e spec that reads the live sketch entity count right after a tree
  double-click must use a reader that returns `-1` instead of throwing (see
  `sketch-multi-object.spec.ts`): "Editing …" becomes visible as soon as the tree
  sets `activeSketchId`, which is ~30 mock-latency ms BEFORE `sketchStore.session`
  lands, and `expect.poll` does not swallow a throw.
- A model-tool COMMIT test must seed every target body into `documentStore`:
  `activeToolPresentation`'s `missingRequiredTargetMessage` turns a target the
  projection does not know into `"… is no longer available"`, which sets
  `canConfirm: false`, and `armedConfirm()` gates the whole Enter table on
  `canConfirmActiveTool(...)` — so the commit silently no-ops. `seedMockDocument()`
  only carries `body1`, so any test arming on `body2`/`body3` (mesh-registry-only
  bodies) has to `documentStore.setState({ bodies: {…} })` as well.
- The SKETCH half of that same gate: a `profile`/`regions` context is refused
  with `"Profile sketch is no longer available"` unless `documentStore.sketches`
  carries it, and `seedMockDocument()` publishes `sketch2/4/5` only — so every
  extrude/revolve commit test arming on `"sk"` must `addSketch({id:"sk", …})`
  in its `beforeEach`, not just seed bodies.
- `toolChipStore.showXxx(…)` re-seats the chip from `CLEARED`, which NULLS
  `context`, and `setContext(tool, …)` silently drops unless `state.kind` already
  equals `tool`. So every `showXxx` call site must be followed by its own
  `setContext`/`publishXxxContext` — a missed one makes `canConfirm` false
  forever, which disables the chip's ✓ (`ModelToolChips.tsx`) AND the Enter table
  (`armedConfirm`). A test that calls `toolChipStore.getState().onConfirm?.()`
  directly does NOT catch it; assert `canConfirmActiveTool(...)` instead.
- `__extrudePreview` (the `?vpdebug` surface) is only ever republished by an
  explicit `this.updateDebug()`. `sendPreview()` does NOT publish it for the
  extrude/revolve owners: `markPreviewPending` returns early for those two before
  its own `updateDebug()`, so an arm path that forgets the call leaves the whole
  e2e/jsdom debug surface reporting the previous phase.
- A RE-EDIT entry point (`editXxxFeature`) arms with NO live picks on purpose, so
  any chip context built from the live pick arrays publishes an EMPTY one, which
  the confirm gate reads as "Affected body is no longer available" — same dead ✓
  as a missing context. The shell re-edit's `storedFaces` fallback
  (`ModelToolController.ts`, `armShell`) is the pattern: publish the record's
  stored typed refs when the live list is empty.
- The e2e specs are NOT type-checked by anything in CI or by `bunx tsc --noEmit`:
  `tsconfig.json`'s `include` is `["src"]` and there is no `e2e/tsconfig.json`. A
  spec-only change can be tsc-green and still not compile. To check one, point an
  ad-hoc tsconfig at `e2e/**/*.ts` with `lib`/`target` ES2022 (the repo's ES2020
  makes every `.at(-1)` in the specs a TS2550) and `types: ["node"]`.
- **The armed model tool is split across two hosts.** `ModelToolChips` (portalled,
  `model-tool-chip`) carries ONLY the primary value input, the operation badge,
  `chip-confirm`/`chip-cancel`; every secondary control — `chip-edgeop-*`,
  `chip-chamfer-*`, `chip-draft-input`, `chip-symmetric`, `chip-end-*`,
  `chip-bool-*`, `chip-offset-type-*`, `chip-mirror-fuse`, `chip-transform-*` —
  is rendered by `features/inspector/ActiveToolInspector.tsx` inside
  `inspector-drawer-content` (open by default). The per-chip `⋯` overflows
  (`ChipOverflow`, `EdgeOpOverflow`, `ExtrudeOverflow`, the whole
  `RevolveChipControls.tsx`, and testids `chip-*-overflow*`, `chip-mode-readout`,
  `chip-draft`) are DEAD — unreferenced by any render path, still exported.
- `ActiveToolInspector`'s read-only "last operation" summary is gated on
  `state.kind === "none"` ALONE. A settled `completed` attempt must never suppress
  a live tool: `operationAttemptStore` gives `completed` no authority (only
  `applying` blocks `begin`, `authoringEntryBlocked` and `toolStore.setTool`), and
  a re-edit entered from a history row (`editFeature` → `editXxxFeature`) never
  reaches `activateTool`, the one caller of `clear()`. Gating the recap on the
  attempt instead cost every re-edit its whole secondary-control section (fixed
  2026-09-13). `tool-chip-dock` is the discriminator between the two: the terminal
  branch returns before it, every armed branch renders it.
- The chamfer ANGLE / SECOND-DISTANCE fields promise "Enter applies the value THEN
  confirms", but authoring either makes the chamfer asymmetric, which moves the
  reference-face pair set, which makes `syncChamferReferenceFaces` close and
  REOPEN the preview session. `commitFillet` has an explicit gate for that same
  turn (`ModelToolController.ts` ~:9501) — it awaits `this.chamferSync`, then
  returns SILENTLY if `armGen` moved or the FSM left `armed`. That silent return
  fires intermittently: the four e2e tests that type into `chip-chamfer-angle` /
  `chip-chamfer-d2` and press Enter pass alone and fail in-file, with a different
  member of the set failing each run, and the FE log shows the arm, two
  "Computing preview…" cycles, then no `edgeOp: armed → committing` and no hint.
- `history-row-<id>` is ONLY the clickable header now. The inline value editor is a
  SIBLING under `history-details-<id>`; both live under the `history-item-<id>`
  wrapper. A locator scoped to `history-row-…` can never reach the editor.
- The M3 semantic aria-labels on `DimensionInput` interpolate the LIVE display unit
  (`Depth (mm)` / `Offset (in)` / `Angle (°)`; the unlabelled default is still
  `Dimension value`, which is what the history rows use). A spec that switches units
  under one armed chip must use a regex locator — `/^Offset \((mm|in)\)$/`.
- The MOCK LANE had no mesh publication at all until `currentMockPublication()`:
  `seedMockDocument()` writes the projection straight into the store and the
  e2e boot then calls `newDocument()`, which does NOT replace `documentStore`.
  Anything gated on `promote.ts`'s `installedProofIsCurrent` (Measure, every
  proof-carrying pick) is dead without a retained publication AND without
  provenance on the installed mesh — `meshSync.reconcile()` must pass the
  publication's generation or `buildBodyObjects` records none.
- `?vpdemo` is the one mock-lane flag that emits a real `document-changed`, so a
  spec on `?vpdebug` alone and one on `?vpdemo` exercise different publication
  states — a useful control when a pick is refused as stale.
- A committed sketch leg can be MOVED by its own auto-inferred `Horizontal`
  (`mockEnforce` projects both endpoints onto their mid-y; PlaneGCS does the
  same), so the draw machine's anchors go stale. `SketchController.reanchorChain`
  re-seats them after `applySketchSolveResult`; without it the next chain leg
  starts where the user clicked and `autoConstrain` welds nothing (it only infers
  `Coincident` between points already equal to 1e-6).
- Snap resolution is ASYMMETRIC between the first and second click of a gesture:
  the first has no live-dimension frame (spatial ladder only — a grid node up to
  a grid reach away), the second has one, and cursor rounding costs a fraction of
  a pixel so it wins. "The same client pixel twice" is therefore NOT the same
  plane point. `setSnapPref` (helpers.ts) is the sanctioned way for a spec that
  needs raw placement to turn off `grid`/`dimensionRound`/`polarTracking`/
  `sketchGuideLines`.
- The compact-chip/inspector split moved settings OUT of `model-tool-chip`:
  transform mode + axis + Copy + Align are in `active-tool-inspector`
  (`TransformInspector`), and Extrude's boolean segments too — the chip keeps the
  value, a `chip-*-badge` and ✓/✕. `chip-mode-readout` → `chip-mode-badge`.
- The left history rail is a 220px `z-20` overlay over the canvas: any e2e click
  computed from a world point can land under it (and under the right inspector /
  top toolbar) and never reach the viewport, with NO error — the pick just
  silently does nothing.
- `e2e/zzdebug.spec.ts` is a TRACKED empty file. Scratch diagnostics go in it,
  but restore it with `git checkout --` when done; deleting it is a worktree
  change.
- Playwright's `webServer` is killed by whichever run STARTED it, so two
  concurrent agents on port 4177 tear each other's server down mid-run
  (`net::ERR_CONNECTION_REFUSED`). Start one long-lived `bun run dev -- --port
  4177 --strictPort` first and both runs reuse it.
- `mockClient.prepareOffsetFace` CLASSIFIES the picked face instead of seeding
  fixed numbers: `mockOffsetFaceDims` (`mockFaceGeometry.ts`) answers `thickness`
  + an opposite for a planar face, `radius` for the cylinder's curved side, and
  `{}` for anything it cannot classify (a synthesized body, a multi-face closure).
  Never both keys at once — `offsetAllowedTypes` tests `radius` FIRST, so the old
  `{ radius: 10, thickness: 10 }` made every mock-lane face read as curved and
  removed `Total` from the UI entirely.
