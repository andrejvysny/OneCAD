# OneCAD Modeling UX Implementation & Hardening Specification — b022edf7

Repo-pinned, coding-agent-ready specification for unifying OneCAD modeling interactions, gizmos, numeric input, previews, publication, history, re-edit, and repair while preserving correctness invariants.

## 0. How to use this specification

# Purpose

This is the implementation brief for a coding agent working on OneCAD’s modeling UX and the correctness seams that make that UX trustworthy. It is not a visual moodboard and not permission to rewrite the app. Implement it as gated work packages against the exact baseline below.

**Repository:** `andrejvysny/OneCAD`  
**Pinned baseline:** [`b022edf7f96a2656f1e6e865bb34ad50ae023ab7`](https://github.com/andrejvysny/OneCAD/commit/b022edf7f96a2656f1e6e865bb34ad50ae023ab7) — `feat(modeling): add P3 publication policy; Preserve source identity for non-fused Pattern V2.`  
**Date inspected:** 2026-08-12  
**Evidence:** founder-operated modeling capture, Exercises 1–3; official Shapr3D, Plasticity, and Fusion documentation; direct GitHub inspection at the pinned commit.

# Coding-agent execution rule

Do not attempt this entire program as one diff. Execute the numbered work packages in dependency order. Before each package:

1. Re-read this section, Baseline, Invariants, and that package’s acceptance criteria.
2. Inspect the named production files and tests at the current working HEAD. If HEAD moved beyond `b022edf7`, produce a baseline-drift report before editing.
3. Write red tests first. Existing frozen contracts are evidence; do not edit them to make a refactor green.
4. Make the smallest coherent cross-layer change that closes the package.
5. Run the package gates and record exact commands/results in `TODO.md` using the repository’s gate style.
6. Stop at the gate. Do not auto-commit, push, pull, or begin the next package unless explicitly instructed.

# Required output from the coding agent at every gate

- Files changed, grouped by frontend / Rust / protocol / worker / tests.
- Behavior before and after.
- Tests added and why each is a valid red control.
- Commands run, pass/fail, and any gate not run with the concrete reason.
- Compatibility impact: legacy documents, protocol fixtures, Pattern V1/V2, persisted references.
- Residual risks and follow-up seams.
- No claim that a native gate passed unless it actually ran.

## 1. Product intent and UX principles

# Product intent

OneCAD is a parametric, history-based desktop CAD application. The target is not “look like Shapr3D.” The target is **Shapr3D-level directness with OneCAD-level history and fail-closed correctness**.

The capture showed that OneCAD already has useful primitives:

- select a face → Sketch → automatic orthographic alignment;
- direct Extrude drag with Add/Cut inference from direction;
- reverse Fillet drag to become Chamfer;
- real, uncommitted geometry before Apply;
- editable history and deterministic `NeedsRepair` rather than silent wrong rebinding.

The weakness is fragmentation: similar tools differ in focus, Enter, Escape, click-away, arrows, chips, previews, result selection, publication, and re-edit.

# Non-negotiable principles

1. **One interaction grammar.** Every modeling tool uses the same high-level states and keyboard semantics. Operation-specific geometry may differ; interaction rules may not drift accidentally.
2. **Preselect first, then act.** If valid geometry is already selected, invoking a tool starts immediately. If required input is missing, the tool names the exact next pick and filters the viewport accordingly.
3. **Type without clicking.** When a tool has one obvious primary numeric value, typing routes to that value without first focusing the field.
4. **Every keystroke previews.** A valid numeric edit updates the live result; blur must never be the only update mechanism.
5. **Preview is honest.** The viewport distinguishes committed geometry, exact kernel candidate, local exact transform, and approximate ghost. Apply is disabled when no publishable result is known.
6. **Commit semantics are universal.** Enter and the visible confirm control commit; Escape and the visible cancel control discard. Text labels versus glyphs are presentation, not different callback protocols.
7. **World-space UI remains legible.** Gizmos are constant-screen-size, depth-independent interaction overlays with adequate hit targets and collision-safe chips.
8. **Result semantics are explicit.** Before Apply, the user can tell whether the operation modifies a body, creates bodies, consumes tools, preserves sources, or fuses results.
9. **Every commit has history lineage.** A committed operation is represented by an editable history node with parameters, operands, and output lineage.
10. **Authoring errors are not history repair.** Invalid fresh input stays inside the active tool. `NeedsRepair` is reserved for previously valid persisted history broken by upstream change.
11. **Deterministic `NeedsRepair` beats silent rebinding.** UX improvements must not weaken resolver confidence/margin policy.
12. **Directness never outranks truth.** No silent clamp, stale preview, false success toast, duplicate seed, or unreported body overlap.

## 2. Baseline: current architecture and landed hardening

# Frontend architecture

- `src/tools/modelTools/modelToolMachine.ts` — pure reducers for Extrude, Revolve, Fillet/Chamfer, Boolean, Shell, OffsetFace, Linear Pattern, Circular Pattern, Mirror, and Transform. Shared state names exist, but recovery/confirmation behavior is not uniform.
- `src/tools/modelTools/ModelToolController.ts` — imperative coordinator for selection, pointer/keyboard input, preview sessions, chips, commits, re-edit, cancellation, and hydration. It contains large per-tool branches.
- `src/tools/modelTools/toolApplicability.ts` — shared but intentionally shallow applicability checks.
- `src/stores/toolChipStore.ts` — monolithic nullable-handler bridge between controller and React chip UI.
- `src/features/toolbar/ModelToolChips.tsx` — per-`kind` rendering. It has two confirmation vocabularies: shared ✓/✕ versus `Apply` only.
- `src/features/sketch/DimensionInput.tsx` — shared numeric field. `autoFocus` exists but is only used by some sketch flows. `onChange` does not notify model tools; commit happens on Enter/blur.
- `src/viewport/engine/DragHandle.ts` — screen-scaled single-axis handle.
- `src/viewport/engine/TransformGizmo.ts` — three translation arrows, three plane quads, three full torus rings; world-axis only; constant screen size; overlay materials use `depthTest:false`.
- `src/viewport/engine/ViewportEngine.ts` — owns gizmos, chip host/movement, screen scaling, picking, and render order.
- `src/ipc/previewOps.ts` — exact-kernel preview registry. Extrude/Revolve/Fillet/Chamfer/Shell/Boolean/Hole/OffsetFace are supported; Pattern/Mirror are ghost-only.
- `src/tools/preview/patternPreview.ts` and `src/viewport/engine/GhostLayer.ts` — local ghost transforms for duplicate-body tools.
- `src/features/inspector/HistoryList.tsx`, `sections.tsx` — history presentation and exact-`opType` re-edit routing.
- `src/features/repair/RepairPanel.tsx`, `src/stores/repairStore.ts`, `src/features/inspector/historyActions.ts` — repair candidates and rebinding.

# Backend/cross-layer architecture

- `src-tauri/crates/onecad-core/src/document/record.rs` — persisted typed operations, including Pattern `result_policy_version`.
- `src-tauri/src/document_runtime.rs` — regen/session publication, Pattern V2 child membership/metadata, repair candidate body provenance.
- `src-tauri/src/dto.rs` — projection types including exact `op_type`.
- `src/ipc/types.ts` and `src/ipc/tauriCommandMap.ts` — TypeScript DTOs and wire lowering.
- `protocol/SCHEMA.md` — normative wire contract and changelog authority.
- `worker/src/kernel/validation/ShapeAudit.{h,cpp}` — publication evidence/policy/decision.
- `worker/src/ops/OpCommon.{h,cpp}` — common `publication_decision` and Boolean result publication helpers.
- `worker/src/ops/BooleanOp.cpp`, `PatternOp.cpp`, `MirrorOp.cpp`, `TransformOp.cpp` — operation semantics relevant to this program.

# Landed changes that supersede screenshot-era conclusions

At `b022edf7`:

- Pattern V2 exists. New non-fused patterns preserve the source as instance zero and create only `count−1` child bodies. Do **not** re-fix the old screenshot by inventing another seed policy.
- Circular Pattern local preview uses `angle/count`, aligned with worker/protocol.
- Standalone Boolean validates empty results before body mutation and refuses recoverably.
- Boolean one/many-body publication is explicit.
- Mirror has Tier A/B publication validation.
- exact-preview failure propagation is structural and blocks commit.
- history re-edit routes on `opType`, not coarse folded `FeatureKind`.
- NeedsRepair has a real panel, candidate loading, markers, and snapshot/revision provenance.

# Remaining baseline defects this specification targets

- inconsistent confirmation, keyboard, click-away, and re-edit rules;
- no direct type-to-primary-value for most model tools;
- blur-gated Pattern spacing/angle updates;
- oversized/occluded Move gizmo and chip-over-pivot layout;
- Apply-only Boolean/Pattern/Mirror without visible cancel or Enter parity;
- resolved `ApplyOperationResult.errorMessage` can be treated as success in several paths;
- Pattern/Mirror post-commit selection is source-biased;
- Mirror re-edit drops `planePoint` and `fuseWithOriginal`;
- Circular Pattern re-edit drops `axisOrigin`;
- Repair candidate response body is fetched but ignored on rebind;
- same-revision older repair snapshots can overwrite newer ones;
- TransformBody bypasses shared publication validation;
- Pattern/Mirror have no real browser-level authoring/re-edit coverage;
- sketch UI can present contradictory `Under-constrained · DOF 0`.

## 3. Hard invariants and explicit non-goals

# Invariants

The coding agent must treat these as gates, not preferences:

- **No JavaScript on the kernel path.** Frontend receives projection DTOs only.
- **World is Z-up and right-handed.** Never rotate the scene, bodies root, or another root group to change visual orientation.
- **Worker stdout carries OCW1 frames only.** Logs go to stderr.
- **`protocol/SCHEMA.md` is normative.** Any wire change requires linked TS lowering, Rust record/DTO, C++ reading, §14 changelog entry, fixture update, and cross-track tests.
- **Rust is hash/identity authority.** Preserve deterministic BodyId policy and uniqueness validation.
- **Frozen behavior contracts under `src/test/contracts/` may not be edited to make a refactor pass.** Change production-side probes if architecture moves.
- **`src/styles/tokens.css` is the only source of design colors.** No raw hex in TS/TSX. Themes are token redeclarations, not Tailwind `dark:` variants.
- **`src/platform/**` must not depend on modeling.** Shared primitives for this program belong under modeling/features/tools/viewport unless they are genuinely platform-neutral and registered through the existing contribution system.
- **Use bun.** Ignore stale `package-lock.json`.
- **The worker must be staged before cargo commands that compile the app crate.** Use `ONECAD_REQUIRE_WORKER=1` for worker-backed gates.
- **Do not weaken resolver thresholds or the tie veto to make repair disappear.**

# Non-goals

- Loft, Sweep, assemblies, FEM, TechDraw, CAM, addon-defined modeling operations.
- Replacing Three.js with react-three-fiber.
- Rewriting the editor shell, toolbar contribution architecture, or inspector unrelated to modeling UX.
- Changing Transform to object-local axes. `TransformGizmo` is world-axis-only by persisted contract; local axes require a separate, versioned design.
- Generic “make it look like Shapr3D.” Use the reference for hierarchy and directness, not styling or branding.
- Adding Keep Tools to Boolean in the first UX package. There is no current `keepTools` field. Treat it as an optional, separately versioned protocol feature after target/tool UX and result classification are stable.
- Silently baking selected face/datum/axis geometry into vectors when the user expects parametric references. Reference expansion must be typed and persisted.

## 4. Unified modeling interaction contract

# Canonical states

Every model tool must map to this public interaction contract even if its internal reducer has additional states:

1. **Inactive** — no tool ownership.
2. **AwaitingSelection** — required input missing. The chip/status names the input; viewport filters valid pick classes.
3. **Editing** — all required references present; parameters may change.
4. **PreviewPending** — a newer parameter set is awaiting exact/local preview.
5. **PreviewValid** — latest parameters have a known valid candidate/result summary.
6. **PreviewInvalid** — latest parameters cannot publish. The reason is visible; confirm is disabled.
7. **Committing** — one fenced commit in flight; controls disabled.
8. **CommitFailed** — document unchanged or rolled back; tool remains open with references/parameters intact.
9. **Committed** — document/projected state updated; result selection applied; tool returns to Select unless re-edit remains open by design.
10. **Cancelled** — preview/ghost/chip/pick modes removed and pre-tool committed document restored.

Reducers may continue to use `idle | armed | dragging | committing`, but a declarative descriptor must expose the canonical state to UI and tests.

# Universal keyboard behavior

- Printable number start (`0–9`, `.`, `-` where legal) while Editing routes to the primary numeric parameter if focus is not already in another editable element.
- Typing replaces the selected/formatted value on the first character; subsequent input edits normally.
- Every parseable change triggers `onPreviewValue`; do not wait for blur.
- Enter commits current text, waits for PreviewValid/exact-candidate barrier where required, then confirms once.
- Tab / Shift-Tab move among parameters in deterministic visual order.
- Escape priority: cancel sub-pick → revert invalid field → cancel tool/re-edit. One press must never both close a nested selector and discard the whole tool.
- Alt modifies only documented gestures, e.g. Transform copy or symmetric Extrude. The chip must reflect modifier state.
- Inputs stop propagation only after the common command router has classified the key. Do not create double-confirm paths.

# Universal pointer behavior

- Preselected valid input arms immediately.
- Missing input enters a named pick phase; no apparently inert tool.
- Pointer release never commits a modeling operation.
- Click-away policy is **cancel focus, not commit the operation**. Do not use viewport click-away as a hidden confirmation. Existing Extrude/Revolve click-away confirmation must be removed or explicitly retained only behind a product decision and a cross-tool contract; this specification selects removal for consistency and safety.
- Direct drag changes the active parameter and updates preview. Apply/Enter commits; Cancel/Escape discards.

# Universal visible controls

Every editable operation presents:

- concise operation/phase label when context is not self-evident;
- primary value or selection summary;
- secondary options behind progressive disclosure where needed;
- visible confirm and cancel controls;
- inline validity state;
- result summary for body-lifecycle-changing operations.

`Apply`, ✓, and Enter call the same `confirm` callback. `Cancel`, ✕, and Escape call the same `cancel` callback. Labels are presentation only.

# Confirmation gates

Confirm is enabled only when:

- all required references exist and match the tool’s selection policy;
- numeric parameters are parseable and in domain;
- latest preview corresponds to latest parameters/epoch;
- exact-preview tools have a valid exact candidate;
- local-exact Transform has a valid placement matrix;
- ghost-only operations have passed the strongest available preflight and clearly disclose preview fidelity;
- commit is not already in flight.

# Success and failure

A success indication is emitted only after a central result classifier verifies a successful `ApplyOperationResult`, hydrates projection, and applies result selection. Promise resolution alone is not success.

## 5. Shared component and code refactor

# 5.1 Declarative tool interaction policy

Add a typed definition beside `modelToolMachine.ts`, for example `src/tools/modelTools/modelToolDefinition.ts`:

```ts
interface ModelToolDefinition<S, P> {
  id: Tool;
  primaryParameter?: NumericParameterDescriptor<S>;
  previewMode: "kernelExact" | "localExact" | "ghost" | "none";
  confirmPolicy: "explicit";
  successSelection: SuccessSelectionPolicy;
  canConfirm(state: S, preview: PreviewState): Confirmability;
  describeResult(state: S): ResultSummary;
  reeditCapabilities: ReeditCapabilities;
}
```

Do not put render components here. This is the behavior authority consumed by controller, chip UI, keyboard router, and contract tests.

# 5.2 Replace the nullable-handler bag

`ToolChipState` currently carries every field and every nullable callback for every tool. Replace it incrementally with a discriminated descriptor:

```ts
type ToolEditorDescriptor =
  | ExtrudeEditor
  | EdgeEditor
  | ShellEditor
  | BooleanEditor
  | LinearPatternEditor
  | CircularPatternEditor
  | MirrorEditor
  | TransformEditor
  | ...;

interface EditorCommon {
  state: "awaiting" | "pending" | "valid" | "invalid" | "committing";
  anchor: ChipAnchor;
  confirm: () => void;
  cancel: () => void;
  confirmLabel?: string;
  error?: ToolErrorPresentation;
}
```

Migration rule: keep existing `show*` façade functions temporarily so controller diffs stay bounded, but make each produce one descriptor. Delete `onApply` as a distinct protocol; pattern/boolean/mirror use `confirm`.

# 5.3 OperationHUD

Refactor `ModelToolChips.tsx` into a common frame plus operation-specific content:

- common border/tone for neutral, pending, valid, invalid;
- common confirm/cancel pair;
- common result summary slot;
- common `aria-live="polite"` error/phase text outside `aria-hidden` canvas decoration;
- common progressive-disclosure control;
- no content branch may omit cancel while an operation is armed;
- no branch may choose separate keyboard behavior.

Keep imperative positioning through the existing portal host. Do not move world-anchor updates into Zustand; the current no-remount rule protects input focus.

# 5.4 NumericRouter and live numeric field

Do not overload sketch constraint semantics blindly. Introduce a modeling wrapper or extend `DimensionInput` with explicit callbacks:

- `onPreview(value)` on every parseable change;
- `onCommit(value)` on Enter/explicit confirm;
- `onInvalid(text, reason)`;
- `autoFocus` or controller-level `beginPrimaryEntry()`;
- `commitOnBlur={false}` for armed model tools;
- optional `selectOnFocus`.

The document value remains mm/degrees. Reuse `src/units/format.ts` for display/parse. Invalid text remains editable and does not mutate the FSM. Never clamp silently.

A global/model-controller key router may focus the primary field, but it must ignore events from inputs, textareas, contenteditable nodes, command palette, and inspector editors.

# 5.5 GizmoOverlay contract

Do not merge `DragHandle` and `TransformGizmo` merely for code reuse. Share a small overlay contract and utilities:

- constant CSS-pixel sizing via `screenScale.ts`/`planePixelWorld()`;
- `depthTest:false`, deterministic render order, token colors;
- visible geometry separate from enlarged invisible hit geometry;
- hover/active state;
- world anchor and screen bounds reporting;
- disposal/theme refresh;
- optional projected bounds used by chip placement.

Add `ViewportEngine.getInteractionOverlayBounds(id)` or equivalent, and extend chip positioning to avoid those bounds and viewport safe areas.

# 5.6 Central result classifier

Create one function used by every fresh commit and re-edit, before `applyResult` and success status:

```ts
type ClassifiedApplyResult =
  | { kind: "success"; changed: BodyId[]; removed: BodyId[]; featureId?: string }
  | { kind: "recoverableFailure"; message: string; diagnostics: OperationDiagnostic[] }
  | { kind: "needsRepair"; evidence: unknown[] }
  | { kind: "stale"; message: string };
```

It must inspect `errorMessage`, changed/removed sets, structural evidence, and operation-specific valid lifecycle outcomes. Use it from `ModelToolController`, `historyActions`, `commitPattern`, `commitTransform`, and scalar re-edit paths. A resolved failure must re-arm/preserve the editor and must never announce success.



# 5.7 Preview scheduling and performance

“Every parseable change previews” does **not** mean one worker RPC per keydown. Preserve the controller’s latest-wins/coalescing architecture and operation-specific trailing floors. The numeric layer emits the latest parseable document-domain value immediately; the controller:

- updates cheap local visuals synchronously;
- coalesces kernel preview requests;
- fences responses by owner/session/epoch/base hash;
- discards superseded candidates;
- keeps `PreviewPending` visible until the latest candidate arrives;
- never confirms an older candidate because a newer value is still in flight.

Do not move preview scheduling into React components or Zustand.

# 5.8 Reuse existing anchor infrastructure

`toolChipStore.ts` already has `ChipAnchorOpts`, `anchorAxisFrom`, `anchorOffsetPx`, and `DEFAULT_CHIP_OFFSET_PX = 26`; the overlay host is intentionally stable to avoid blur/remount commits. Extend this mechanism for Transform and exclusion-bound placement. Do not create a second absolute-positioning system. Pixel values in this spec are starting hypotheses; final constants must be derived from rendered evidence and tests at supported viewports/device scales.


## 6. Move and gizmo specification

# Evidence-correct reference

The corrected comparison is:

- **OneCAD current:** blue-body screenshot; large RGB full-circle rings; horizontal options chip crosses the pivot.
- **Shapr3D reference:** cyan background; compact central widget; unobstructed pivot; translation arrows, plane handles, and small curved double-headed rotation handles; angle/Copy controls adjacent to the widget.

Borrow hierarchy and separation, not styling.

# Preserve current semantic contract

`TransformGizmo` is world-axis-only and the FSM stores world translation/rotation axis. Keep that invariant. This work changes interaction geometry and presentation, not persisted transform semantics.

# Required visual hierarchy

1. **Pivot:** always visible and never covered by the HUD. Use a small neutral/tokenized center disc/ring with adequate contrast.
2. **Translation arrows:** primary handles. Retain X/Y/Z token colors, strong shaft/arrow silhouette, and fat invisible hit cylinders.
3. **Plane handles:** secondary. Keep near pivot, lower opacity than arrows, and enlarge/brighten on hover.
4. **Rotation handles:** tertiary until hovered. Replace three full `TorusGeometry` rings with compact curved double-headed handles or partial arcs located near the pivot. Only the hovered/active rotation handle becomes prominent.
5. **HUD:** place outside the projected gizmo bounds. Connect with a subtle leader only if needed. Never cross the pivot, handles, or active drag path.

# Geometry changes in `TransformGizmo.ts`

Current constants `RING_R_PX = 82`, three full tori, and nearest-hit arbitration produce crossing/edge-on ambiguity. Replace the full-torus build with partial curves plus arrowheads and independent enlarged hit geometry.

Suggested authored ranges, to validate visually rather than copy literally:

- arrows retain approximately 50–65 CSS px reach;
- plane handles remain inside arrow reach;
- rotation handles occupy compact 30–55 px radial bands, not a full 164 px diameter;
- visible stroke 1.5–2.5 px equivalent;
- hit corridor at least 8–12 CSS px;
- inactive opacity below translation arrows; active opacity 1.

Use `THREE.TubeGeometry` over a partial `CurvePath` or line/arrow meshes; do not use one full torus and hide fragments with material tricks that leave full-circle hit geometry. Classification remains `{kind:"ring", axis}` for compatibility.

# Interaction

- Hover one handle: brighten and slightly enlarge only that handle; dim competing rotation handles.
- Drag: active handle stays visible when pointer leaves hit geometry.
- Rotation HUD shows angle as the primary value; Move HUD shows distance. The active axis is evident from handle color/orientation, so X/Y/Z chip segments become optional shortcuts rather than the only cue.
- Copy is visible and synchronized with Alt-drag.
- Align subflow temporarily hides or de-emphasizes the gizmo and changes the HUD to `Pick moving face` then `Pick target face`; cancel returns to the same transform state.
- Camera orbit/pan behavior remains unchanged.

# Chip placement

`toolChipStore.showTransform` currently anchors the chip at `worldPos` with no offset options. Extend Transform chip opts to accept `ChipAnchorOpts` and/or a screen-space exclusion rectangle from the gizmo. The overlay driver chooses among candidate placements around the projected bounds, scores viewport overflow/overlap, and keeps the last stable side during drag to avoid jitter.

# Acceptance criteria

- Pivot and all handle intersections remain visible at idle, hover, and drag.
- HUD never intersects the projected gizmo bounds at tested camera angles and viewport edges.
- All nine handles remain pickable with mouse and trackpad at 1× and 2× device scale.
- Rotation ring crossing ambiguity is eliminated because full circles no longer overlap.
- Constant screen size holds in perspective and orthographic views.
- Light/dark theme refresh preserves hover/active colors.
- Existing world-axis, frozen-pivot, fold, copy, and align semantics remain green.

## 7. Per-tool UX requirements

# Sketch and constraints

- Fix the presentation rule first: `DOF === 0` must never render Under-constrained. `constraintStatus.ts` and `InspectorPanel.tsx` must derive from one authority.
- Red test the captured `Under-constrained · DOF 0` state.
- Verify solver/status projection before “fixing copy.” If solver status says under with DOF 0, correct the projection contract; do not mask inconsistency only in React.
- Origin snapping must create a visible persisted coincident relation only when accepted. The snap glyph/label must distinguish hover candidate from committed relation.
- A rectangle whose corner is coincident with origin and whose width/height are dimensioned should become Fully constrained when the solver reports zero DOF.
- Do not auto-fix geometry merely because it was drawn near the origin.

# Extrude

- Keep direct arrow drag and Add/Cut inference.
- Primary distance supports type-to-enter and live preview.
- Remove hidden click-away commit; use explicit confirm/Enter.
- Keep advanced end condition, draft, symmetric, and Boolean options behind progressive disclosure.
- Maintain exact preview barrier and multi-region semantics.
- Resolve the cross-mode sketch→Extrude handoff so explicit Extrude intent does not arm then reset to Select.

# Fillet/Chamfer

- Preserve one shared FSM and direction-based switching where reliable geometry supports it.
- Add type-to-enter and live preview; keep explicit type toggle in HUD.
- Fresh authoring failures remain in-tool. They must not route to persisted NeedsRepair UI.
- Same-body/closure checks belong in prepare/applicability where possible; do not weaken descriptor fallback.
- Keep exact preview and candidate barrier.

# Shell

- Provide a thickness manipulation handle using the shared DragHandle/GizmoOverlay contract. A selected removal face gives a normal/direction anchor; the handle must not imply a valid direction where the operation cannot define one.
- Fresh and re-edit Shell use the same explicit confirm/cancel contract.
- Remove re-edit blur-commit. Value changes preview/edit state; Enter/confirm commits.
- Keep exact preview and Tier B publication validation.

# Boolean

- Tool activation with one body selected enters `AwaitingSelection: Pick tool body`; with two valid bodies preselected, assign roles deterministically and show them.
- HUD displays `Target`, `Tool(s)`, operation, Swap, result validity, confirm/cancel.
- Use viewport role badges/outlines; do not rely on color alone.
- Preview semantics: survivor/target, removed volume, and intersection must be visually distinguishable where the preview data supports it.
- Keep Tools is a later versioned option, not part of the first package.
- Enter/confirm parity; no Apply-only protocol.
- Preserve current empty-result refusal and one/many-body lifecycle policy.

# Linear Pattern

- Label count as `Total`; count includes the source.
- Default authoring is V2 non-fused: source remains once, children are instances `1…count−1`.
- Primary fields: axis/direction, total instances, spacing, result mode summary.
- Spacing updates preview on every valid change, not blur.
- Allow world axes initially; typed arbitrary reference expansion is a later package.
- Confirm/cancel/Enter parity.
- On success, select created child bodies for non-fused mode; select source for fused mode. Also expose/scroll the feature node.
- Do not change legacy absent-version semantics during re-edit.

# Circular Pattern

- Label count as `Total`; total angle is explicit.
- Do not add a separate radius parameter for axis-based patterning. Radius is seed-to-axis distance.
- Preserve the fixed `angle/count` preview rule.
- Preserve stored `axisOrigin` and `axisDirection` on re-edit. Current origin hardcoding is a defect.
- Support negative angle and direction clearly.
- Same result, confirmation, selection, and legacy-version rules as Linear Pattern.

# Mirror

- Immediate package: preserve stored `planePoint` and `fuseWithOriginal` during re-edit. Do not hardcode origin/false.
- Show `New body` versus `Fuse` result mode when backend semantics and exact validation are represented honestly.
- Current XY/XZ/YZ shortcuts remain.
- Planar face/datum selection requires typed parametric reference work described in Cross-layer Reference Expansion; do not bake a transient vector and pretend it will follow history.
- Non-fused commit selects the generated body; fused commit selects the source.

# Transform / Move

- Implement the Move gizmo section exactly.
- Type-to-enter targets the currently active Move/Rotate component.
- Preserve world-axis-only and frozen pivot semantics.
- `copy:true` success selects the created copies, not only sources.
- `copy:false` selects moved sources.
- Add shared result classification before success.
- TransformBody must pass shared Tier A publication validation in the worker before publication.



# Parameter-range policy

Frontend controls may offer a smaller ergonomic range than the worker’s safety maximum, but the distinction must be explicit and tested. Pattern authoring currently presents a 2–12 stepper while the worker accepts up to 128. Choose and document one policy:

- stepper remains 2–12 for fast common use, while direct numeric entry may accept up to the worker maximum; or
- UI hard maximum equals the normative authoring maximum across TS/Rust/worker.

Do not leave three silent ranges. Out-of-range input stays editable, displays the allowed range, does not clamp, and does not emit a preview/commit.


## 8. Body lifecycle, history, and re-edit contract

# History node rule

Every committed operation creates or updates exactly one history node. The node must expose:

- exact `opType`;
- human label;
- concise parameter summary;
- status (`ok`, `dirty`, `error`, `needsRepair`);
- input dependencies;
- output body lineage;
- editable parameters that preserve all stored semantics not exposed in the current UI.

Never route re-edit on folded `FeatureKind`; keep exact `opType` routing.

# Operation summaries

Examples:

- `Extrude · 25 mm · Add`
- `Chamfer · 3.56 mm · 2 edges`
- `Boolean · Cut · 1 tool`
- `Linear Pattern · 3 total · 20 mm · X · New bodies`
- `Circular Pattern · 3 total · 360° · Z · New bodies`
- `Mirror · Datum 1 · New body`
- `Move · +30 mm X`
- `Rotate · 30° Z · Copy`

# Source/output semantics

- Pattern V2 non-fused: source BodyId and body exist once; child outputs are deterministic and ordinal; no seed-position child.
- Pattern V2 fused: source BodyId modified in place; disconnected fused result refuses with no partial publication.
- Mirror non-fused: source remains; one generated body. Mirror fused: source modified only after valid connected result.
- Boolean: tool consumption occurs only after successful result classification.
- Transform copy: sources remain, outputs are selected and named; Transform move modifies/folds according to existing contract.

# Unique display names

Body identifiers and display names are different concerns. After operations that create bodies:

- allocate unique human labels without duplicates such as two `Body 3` rows;
- persist/reconstruct labels deterministically through regen, count edit, suppression, undo, save, and reopen;
- preserve user-assigned display metadata according to current Pattern V2 rules;
- do not infer body identity from display label.

# Re-edit preservation rule

A re-edit may change only fields the user changed. All other stored fields are deep-preserved.

Mandatory red tests:

- Mirror retains `planePoint` and `fuseWithOriginal` when only plane normal/shortcut changes.
- Circular Pattern retains `axisOrigin` when only angle/count changes.
- Pattern retains absent V1 versus explicit V2 and stored fuse mode.
- Extrude retains hidden end-condition/Boolean/draft fields unless explicitly changed.
- Transform re-edit/fold does not double-apply placement.

# Post-commit selection

Define this centrally:

- in-place modification → affected existing bodies;
- new-body operation → new bodies;
- non-fused pattern → generated children plus feature focus, not source-only;
- fused pattern/mirror → source;
- Transform Copy → copies;
- Transform Move → moved sources;
- split Boolean → all deterministic child outputs.

Selection is applied only after projection hydration confirms those outputs exist.

## 9. Preview, validation, diagnostics, and repair

# Preview fidelity taxonomy

The UI and tests must use these names:

- **Kernel exact:** worker-produced preview candidate for current parameters/base hash.
- **Local exact:** rigid matrix placement whose local math exactly matches persisted operation semantics.
- **Ghost:** transformed source mesh that illustrates placement but does not prove fused topology/publication.
- **No preview:** pick-only or unsupported state.

Do not call every translucent object an exact preview.

# Confirmation behavior by fidelity

- Kernel exact: confirm waits for the latest exact candidate and refuses stale/missing candidate.
- Local exact Transform: confirm requires valid matrix and worker-side publication validation on commit.
- Ghost Pattern/Mirror: confirm is allowed only with explicit ghost semantics and strong preflight; failures re-arm without claiming success. Fused modes should not be exposed as visually approved until an exact fused preview exists or the HUD clearly labels `Placement preview; fusion validated on Apply`.

# Error presentation

Use `PreviewFailure.structural` and diagnostics instead of flattening everything to one status hint:

- transient geometry invalid → inline parameter/geometry message;
- missing/bad fresh reference → selection prompt and preserved active tool;
- stale preview → automatic single retry, then inline stale message;
- persisted broken reference after upstream edit → NeedsRepair;
- worker diagnostics → expandable technical details in inspector/status, not discarded.

Confirm is visibly disabled in `PreviewPending` and `PreviewInvalid`. Clicking a disabled-looking control must not be the first time the user learns the result is invalid.

# Authoring versus repair

**Fresh authoring path:** selected edge/face/body cannot be prepared or previewed → remain in active tool; highlight offending input; allow reselection; document unchanged.

**Persisted history path:** an operation that was valid when stored cannot resolve after upstream edit → timeline stops; feature becomes NeedsRepair; repair panel opens on request.

Never create a persisted broken feature merely to enter repair UI.

# Repair provenance hardening

Current `RepairPanel` loads `ResolveRefResult.bodyId` into `CandidateLoad.bodyId` but `choose()` ignores it and `rebindCandidate()` promotes against `item.bodyId`. Fix the call chain so candidate promotion uses the authoritative echoed candidate-enumeration body.

Required API shape:

```ts
rebindCandidate({
  item,
  candidate,
  candidateBodyId: load.bodyId,
  revision: load.revision,
  snapshotId: load.snapshotId,
})
```

Reject missing/mismatched body provenance rather than falling back to the operated body for cross-body refs such as ToFace.

# Repair ordering

`repairStore.applyEvent` must compare `(revision, snapshotId)` lexicographically. Same-revision older snapshots cannot replace newer ones. Any accepted non-empty replacement event clears `hoveredWorldPos` and invalidates cached candidate loads from older provenance.

# Candidate copy truth

Do not show `5 candidates` and then `No candidates to choose from` without explanation. Prefer one of:

- collapsed row uses reason only until authoritative candidates load;
- or distinguish `5 potential matches` from `0 eligible rebind candidates` and explain why.

The final displayed count must derive from the same authoritative `ResolveRefs` response used to render selectable rows.

# Candidate inspection

Keep hover marker behavior. Add optional details for score, margin, and evidence contributions when available, but do not encourage a low-confidence silent bind. User choice is explicit repair, not auto-bind policy weakening.

## 10. Typed reference expansion for Mirror and Pattern

# Why vectors alone are insufficient

The capture request—Mirror about a model face or datum, Pattern about a picked axis—is correct UX. Persisting only the current plane point/normal or axis origin/direction would bake a snapshot. If the face/datum moves, the operation would not follow history. That violates the parametric product model.

# Staged implementation

## Stage A: preserve existing vectors

First fix re-edit loss for `planePoint`, `fuseWithOriginal`, and `axisOrigin`. Do not mix this with schema expansion.

## Stage B: additive typed references

Design additive, versioned fields in Rust records and `protocol/SCHEMA.md`, for example:

```text
MirrorPlaneSource =
  PrincipalPlane { plane: XY|XZ|YZ }
  DatumPlane { datumId }
  PlanarFace { faceRef: ElementRef }

PatternAxisSource =
  PrincipalAxis { axis: X|Y|Z }
  DatumAxis { datumId or typed axis id }
  LinearEdge { edgeRef: ElementRef }
  CylindricalAxis { faceRef: ElementRef }
```

Exact naming must follow repository conventions and existing datum representation. Unknown source variants must preserve forward-compatible data according to current `Operation::Opaque` policy; do not invent plugin execution.

# Resolution

- Rust persists typed source identity.
- Worker resolves semantic face/edge refs through the existing history/descriptor ladder.
- Datum references resolve in the document layer if datums are document-side; pass resolved vector plus provenance to the worker only if the normative architecture requires it.
- On ambiguity/failure after upstream edit, produce NeedsRepair with an addressable input path.
- Re-edit shows the referenced face/datum/axis name and supports reselection.
- Principal-plane/axis legacy vectors remain readable.

# Gates

- face/datum edit moves the Mirror result after regen;
- edge/cylindrical axis edit moves Circular Pattern after regen;
- deleted/ambiguous source produces deterministic NeedsRepair, never frozen-vector continuation;
- save/reopen retains source type;
- legacy vector-only documents reproduce byte-identically when untouched;
- protocol changelog, Rust/C++ fixtures, and cross-track tests updated together.

## 11. Worker publication hardening

# TransformBody

`worker/src/ops/TransformOp.cpp` must validate every produced transformed shape with shared Tier A `publication_decision` before any body/lifecycle mutation.

Acceptance:

- one solid, finite positive volume, validity/tolerance evidence according to Tier A;
- multi-target transaction fails without partial publication if any target fails;
- duplicate/128-target/cancellation rules remain;
- negative-control test proves validator invocation rather than only success behavior.

# Pattern/Mirror

Preserve current P3/Pattern V2 semantics. Add missing evidence, not a new result policy.

- Circular Pattern V2 fused/non-fused C++ and real-worker Rust coverage.
- Mirror fused-disjoint negative test proving no body/event/element-map mutation.
- Stable refusal codes recorded in normative error taxonomy.
- If fuse modes become user-visible, exact preview or explicit fidelity disclosure is required.

# Publication policy scope

Do not expand `PublicationPolicy` opportunistically inside a frontend package. If the UX program requires a richer machine-readable operation contract—manifold/edge-use, sliver/micro-edge evidence, calibrated tolerance ceiling, operation semantic callbacks—open a separate hardening package with red negative controls.

# Known exception

Hole’s multi-solid residual is explicitly accepted and version-sensitive. Do not “normalize” it inside UX work without a dedicated compatibility decision.

## 12. Test strategy and required red controls

# Contract layer

Add a new immutable contract/probe for the modeling interaction matrix. The contract should list, per tool:

- primary parameter;
- preview fidelity;
- Enter support;
- visible cancel;
- click-away policy;
- required selections;
- success-selection policy;
- re-edit capabilities.

Production must conform to the contract. Once approved, do not edit expected rows to make implementation pass.

# Unit/component tests

## Numeric input

- typing without prior click focuses primary model value;
- every valid change calls preview callback;
- invalid partial text does not mutate FSM;
- Enter previews latest value then confirms exactly once;
- blur does not commit armed model tools;
- Escape restores/cancels according to state;
- unit conversion remains document-domain mm/degrees.

## OperationHUD

- every armed operation exposes confirm and cancel;
- pending/invalid disables confirm;
- Boolean/Pattern/Mirror no longer use a separate `onApply` protocol;
- result summary text matches operation state;
- accessible status exists outside the canvas `aria-hidden` subtree.

## TransformGizmo

Extend `TransformGizmo.test.ts`:

- partial rotation-handle classification from representative cameras;
- no full-ring crossing cases;
- invisible hit corridor is larger than visible geometry;
- hover/active enlargement affects only one handle;
- world-axis and constant-screen-size invariants;
- dispose/theme refresh.

Add chip/gizmo collision tests at center, corners, perspective, orthographic, and high-DPI.

## Controller/reducers

- shared Enter/cancel scripts for every tool;
- no click-away commit;
- commit-failed re-arms with state preserved;
- resolved `errorMessage` never reaches success path;
- success selection per operation/result mode;
- sketch→Extrude explicit handoff remains armed;
- Shell re-edit requires explicit confirm;
- Mirror/Circular re-edit preserve hidden stored fields.

## Repair

- candidate body differs from operated body and authoritative response body is used;
- same revision/newer snapshot wins over late older snapshot;
- accepted replacement clears hover/caches;
- displayed candidate count and selectable rows cannot contradict;
- stale load/click remains blocked.

# Worker/Rust tests

- Transform Tier A validation negative control;
- Circular Pattern V2 fused/non-fused lifecycle and persistence;
- Mirror fused-disjoint no-publication;
- real-worker Boolean Intersect success/empty/split/re-edit parity;
- typed reference expansion tests when that package lands;
- legacy document round trips.

# Playwright/e2e

Required real flows with `retries: 0`:

1. type-to-enter Extrude; Apply/Discard; undo.
2. Fillet→Chamfer drag switch; fresh invalid reference remains in tool.
3. Shell handle + typed thickness + cancel.
4. Boolean target/tool role assignment, swap, empty refusal, re-arm.
5. Linear Pattern V2: source once, `count−1` children, edit count, undo/save/reopen.
6. Circular Pattern: total count/angle, negative angle, axis origin preserved on re-edit.
7. Mirror: new body/fuse, plane point preserved; later face/datum reference.
8. Move: all handle classes, chip outside gizmo, numeric/drag parity, Copy selection, Align cancel/return.
9. real Tauri worker-generated NeedsRepair through resolve, rebind, republish.

# Coverage manifest

Strengthen `scripts/verify-modeling-coverage.mjs` beyond row shape:

- validate referenced paths exist;
- require non-empty evidence for supported rows;
- associate declared tests with existing files/test names;
- correct Circular Pattern V2 evidence if currently overstated;
- require Playwright evidence for supported UI authoring flows.

## 13. Work packages and dependency order

# WP0 — Baseline and red evidence

**Goal:** freeze behavior before refactor.  
**Changes:** tests/contracts only, plus no-op probes if needed.  
**Must include:** interaction matrix; DOF 0 contradiction; direct typing; blur-gated Pattern; Apply-only tools; Shell blur-commit; Move overlap bounds; result-error false success; Pattern/Mirror result selection; repair provenance; Mirror/Circular re-edit loss.

**Gate:** all new tests fail for the intended reason; existing suite remains unchanged except expected new reds on the working branch.

# WP1 — Central result classification

**Goal:** no false success or lost editor state.  
**Dependencies:** WP0.  
**Scope:** classifier, all commit/re-edit paths, rollback/re-arm policy, success selection descriptor.

**DoD:** resolved error results never announce success; operation remains editable; tests cover fresh and re-edit paths.

# WP2 — Unified editor descriptor and confirmation contract

**Goal:** one confirm/cancel/Enter/Escape policy.  
**Scope:** discriminated chip descriptor; `OperationHUD`; remove `onApply`; remove hidden click-away commit; Shell explicit re-edit confirmation; cross-mode Extrude handoff.

**DoD:** every armed model tool has visible cancel and Enter parity; contract test green.

# WP3 — Live numeric router

**Goal:** type without clicking and preview per valid change.  
**Scope:** model numeric wrapper/router, no blur commit, Tab order, validation tone, unit behavior.

**DoD:** Extrude, Fillet/Chamfer, Shell, Pattern spacing/angle, and Transform obey one numeric contract.

# WP4 — GizmoOverlay and Move redesign

**Goal:** correct visual hierarchy and non-overlapping HUD.  
**Scope:** overlay bounds contract, partial rotation handles, hit geometry, hover/active scaling, collision-safe chip placement, light/dark/high-DPI tests.

**DoD:** Move reference acceptance criteria green; no semantic transform changes.

# WP5 — Tool entry, result summaries, and selection

**Goal:** no inert tool and no uncertain result.  
**Scope:** Boolean AwaitingSelection/roles/swap; Pattern total-instance copy; Mirror mode summary; post-commit selection; unique display labels.

**DoD:** user can state inputs and result before Apply; all outputs selected correctly afterward.

# WP6 — Re-edit preservation and repair correctness

**Goal:** edits preserve unexposed semantics and repair uses authoritative provenance.  
**Scope:** Mirror `planePoint`/fuse, Circular `axisOrigin`, candidate body, snapshot ordering, candidate copy truth, diagnostics surfacing.

**DoD:** red tests from WP0 green; no stale/cross-body rebind.

# WP7 — Worker and evidence hardening

**Goal:** Transform and remaining operation evidence align with public UX claims.  
**Scope:** Transform Tier A validation, Circular Pattern V2 tests, Mirror disjoint refusal, coverage-manifest verifier.

# WP8 — Typed face/datum/axis references

**Goal:** Mirror/Pattern accept real model references without baking geometry.  
**Scope:** separately approved protocol/Rust/C++/TS expansion, history resolution, NeedsRepair, re-edit, persistence.

**Dependency:** WP6 and protocol design review. Do not fold into WP5.

# Package boundaries

Each WP is a gate/commit boundary. Do not combine WP1–WP8 into a single change. If a package exposes an architectural blocker, stop and revise this spec rather than bypassing the blocker locally.

## 14. Verification commands and gates

# Fast frontend loop

```bash
bun install --frozen-lockfile
bun run test
bun run build
node scripts/verify-modeling-coverage.mjs
scripts/tests/verify-modeling-coverage.test.sh
```

Use targeted Vitest files while developing, then full `bun run test` at gate.

# Browser gates

```bash
bun run e2e -- --project=chromium
bun run e2e -- --project=webkit
```

Keep `retries: 0`. Do not “fix” a race with retries.

# Worker gates

With pinned OCCT 8.0.1 configured according to repo scripts:

```bash
scripts/check-worker-stdout-hygiene.sh
cmake --build <worker-build-dir> --parallel 3
ctest --test-dir <worker-build-dir> --output-on-failure
```

Run operation-specific test binaries twice when determinism is part of the package.

# Rust/full workspace gates

Stage the worker binary first. Then:

```bash
cd src-tauri
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
ONECAD_WORKER_PATH=<absolute-worker-path> ONECAD_REQUIRE_WORKER=1 cargo test --workspace
```

# Kernelbench

Do not claim a kernelbench gate if the environment lacks the pinned worker/OCCT. For worker/publication changes, run the repository’s T0 semantics path from CI. Digests are same-machine tripwires; do not compare across machines.

# Manual gate

At the final UX integration gate, perform real Tauri smoke on a machine with the native stack:

1. open/create project;
2. sketch from origin and fully constrain;
3. Extrude by drag and direct typing;
4. Fillet/Chamfer; cancel and reapply;
5. Shell;
6. Boolean with target/tool swap and invalid empty result;
7. Pattern and Mirror, edit history, undo;
8. Move translation/plane/rotation/Copy/Align at multiple camera angles;
9. induce upstream reference break, repair, save, reopen;
10. verify light and dark themes.

Record screenshots/video and exact commit. This sandbox cannot run the native gate, so the coding agent or user must.

## 15. Definition of Done

The program is complete only when all statements are true:

## Interaction

- Every model tool follows one visible confirm/cancel and Enter/Escape contract.
- No model operation commits on blur or hidden click-away.
- Primary numeric values can be typed without first clicking the field.
- Every parseable edit updates preview immediately.
- Tool activation never appears inert; missing input is named.

## Gizmos

- Extrude/edge/shell/offset handles and Move use shared overlay sizing/hit/contrast/placement rules.
- Move pivot is unobstructed.
- Rotation uses compact curved handles, not three overlapping full rings.
- HUD does not overlap gizmo or leave viewport safe bounds.
- Light/dark and high-DPI behavior are tested.

## Correctness and publication

- No resolved operation failure is treated as success.
- Transform results pass shared publication validation.
- Pattern V2 source exists exactly once; outputs/selection/history match the persisted policy.
- Mirror/Circular re-edit preserves all stored semantics.
- Body display labels are unique without becoming identity.

## History and repair

- Every commit/re-edit has one exact-`opType` history node and truthful parameter summary.
- Fresh authoring failures stay in-tool.
- NeedsRepair is post-edit history recovery only.
- Candidate body/snapshot provenance is authoritative end-to-end.
- Candidate counts/copy cannot contradict selectable results.
- Real-worker repair e2e passes.

## Evidence

- New frozen interaction contract is green.
- Unit/controller/component/worker/Rust/e2e tests listed in this spec pass.
- Coverage manifest refers to real evidence.
- Required manual Tauri smoke is recorded.
- `TODO.md` and `CURRENT_STATE.md` accurately record gates and residuals.

## Compatibility

- Untouched legacy documents remain byte-stable where the architecture promises it.
- Pattern V1/V2 behavior remains version-correct.
- Any wire extension has full schema/changelog/fixture/Rust/C++/TS coverage.
- No hard invariant is violated.

## 16. References

# Repository references

- [Pinned baseline commit](https://github.com/andrejvysny/OneCAD/commit/b022edf7f96a2656f1e6e865bb34ad50ae023ab7)
- `src/features/toolbar/ModelToolChips.tsx`
- `src/stores/toolChipStore.ts`
- `src/features/sketch/DimensionInput.tsx`
- `src/tools/modelTools/modelToolMachine.ts`
- `src/tools/modelTools/ModelToolController.ts`
- `src/viewport/engine/TransformGizmo.ts`
- `src/viewport/engine/DragHandle.ts`
- `src/ipc/previewOps.ts`
- `src/tools/preview/patternPreview.ts`
- `src/features/repair/RepairPanel.tsx`
- `src/stores/repairStore.ts`
- `src/features/inspector/historyActions.ts`
- `src-tauri/src/document_runtime.rs`
- `src-tauri/crates/onecad-core/src/document/record.rs`
- `worker/src/kernel/validation/ShapeAudit.{h,cpp}`
- `worker/src/ops/OpCommon.{h,cpp}`
- `protocol/SCHEMA.md`

# UX benchmark references

- [Shapr3D — Extrude](https://support.shapr3d.com/hc/en-us/articles/7874453786908-Extrude)
- [Shapr3D — Advanced Part Design](https://support.shapr3d.com/hc/en-us/articles/14030468016540-Advanced-Part-Design)
- [Shapr3D — Boolean operations](https://support.shapr3d.com/hc/en-us/articles/10565066254108-Boolean-operations)
- [Plasticity — Boolean](https://doc.plasticity.xyz/solid/boolean.en)
- [Autodesk Fusion — Mirrors and patterns](https://help.autodesk.com/cloudhelp/ENU/Fusion-Model/files/SLD-PATTERNS.htm)
- [Autodesk Fusion — Mirror reference](https://help.autodesk.com/cloudhelp/ENU/Fusion-Model/files/SLD-REF-MIRROR-DIALOG.htm)

# Evidence caveat

The founder capture covered startup and Exercises 1–3, not the complete proposed capture inventory. Claims about file lifecycle and every remaining operation dialog are repo-derived rather than directly observed. The specification therefore requires additional manual smoke and browser/native evidence before declaring the UX program complete.
