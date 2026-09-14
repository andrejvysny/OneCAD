# CURRENT STATUS: INCOMPLETE — UX HARDENING HANDOFF 2026-09-12

Acceptance is open. The original `status: done` below is historical smaller-plan text only and must not compete with this current status.

## RESUMED IMPLEMENTATION — 2026-09-12

The handoff draft remains retained; implementation is active. ui_plan Sol owns typed presentation contract work; tools_plan Sol owns the general-pattern contract audit pending main approval. Luna= simple docs/UI, Terra=moderate UI, Sol=complex work, Astra=final review. No tests or gates are implied by this documentation note.

# CURRENT APPROVED UX HARDENING PLAN — 2026-09-12

This pointer is authoritative for the current larger hardening program. The older WP-U1..U12 plan below is historical and its “done” labels do not close the current program. Preserve all historical entries and use this section for new work.

## Scope and status

Phase 0 baseline evidence/inventory: partial/done for durable evidence, review inventory, normative wire/schema/fixtures and verifier manifests; current dirty-tree ownership and scope reruns remain owed.

Phase 1A native stalls/locks/publication: partial. Runtime session/manager/epoch fencing, restart fences, latest-event coalescing, mesh identity fences, appStore success-boundary reset/single-flight and subscriber-failure isolation are implemented; scheduler fence, queued-old-runtime app-driver and restart-owner automation are 1/1. Tracing reproduced the webview-lock stall and pinned versions/narrow Tauri tracing feature are documented. Native stress and publication-to-render completion remain owed.

Phase 1B interaction boundary: bounded/automated accepted; native pointer/AX/Enter equivalence and physical input behavior remain owed.

Phase 1C solver/currentness/reference safety: partial. Solver currentness, persistent identity and ambiguity handling are bounded; native validation and fail-closed reference evidence remain open. Typed targets/reference consistency is tracked under 2C/2D.

Phase 1D removed selection/inspector trust: partial. Removed selection/hover cleanup, currentness and ranking are bounded; inspector recovery/history, tree/canvas hover + Reveal, native recovery/undo and persistence remain open.

Phase 2A atomic operation/undo/visibility: partial. Atomic operation, consumed-sketch visibility and runtime/revision/exact-entry rollback receipts with cap 200 are bounded; native rollback, 100 stress, save/autosave isolation remain open.

Phase 2B palette/semantic labels/dirty behavior: partial/bounded. Palette ranking, semantic labels, no-op clean/autosave behavior and field-local undo boundaries are bounded; invalid-mode/tool-first/constraint matrix and every report-tool variant remain open.

Phase 2C shared presentation/controller contract: partial. Typed targets/references and one-parameter authority are bounded; full cross-tool consistency and native window matrix remain open. Annotation work is tracked under 3B.

Phase 2D chip/inspector rendering, accessibility and layout: partial. All tool families already render through the frame; typed contract/targets/reference consistency, AX labels, docking edge cases and stable full-parameter inspector remain open.

Phase 2E solver-backed drag/snap/rollback: partial. Solver-backed drag, snap ordering and rollback/currentness seams are bounded; constraint/snap/trim/both-cancel, native rollback stress and complex dimensioned cases remain open.

Phase 2F sketch wording/indicators: partial. Sketch wording, snap/constraint indicators and semantic labels are bounded; all variants and native constraint/snap evidence remain open.

Phase 3A candidate API/reference picking: bounded partial. Candidate API, reference picking, currentness and fail-closed identity are bounded; native provenance breadth, all tool reference contracts and ambiguity handling remain open.

Phase 3B candidate UI/tree hover/Reveal/framing/section/measurement/history: partial. Candidate chooser/filter/keyboard has fresh 2/2 browser proof; annotations, hover, framing, section, measurement, history and native selection trust remain open.

Phase 3C FeaturePattern contracts/core/worker: restricted partial. Canonical selected IDs, shared host lineage and fail-closed validation are bounded for selected families; general mixed chains, dependency/source edit, suppression, repair, persistence and native breadth remain open. No opaque extensions or Loft/Sweep claims.

Phase 3D FeaturePattern UI/tests: open/partial. Backend-hidden broad UI and tests remain incomplete; do not add registry rows to imply support.

Phase 4 docs/review fixes: active. Documentation carries evidence and boundaries; Astra/main owns all-phase acceptance, not Phase 4. The recorded Rust workspace gate passed for the prior frozen source; any changed tree requires a fresh serial rerun. Full browser/native, stress, accessibility, window, recovery/currentness, pattern, save/autosave and report-tool gates remain open.

## Locked decisions and constraints

Small movable chip + stable inspector; invalid typed values stay visible and block confirmation; `Use maximum` is explicit and non-committing; snap occurs before constraint solve and Alt bypasses snap only; FeaturePattern uses selected canonical record IDs, shared parameters, source included in count, no per-instance override/suppression, one-host lineage and reject-all invalid input; BodyPattern V2 preserved. Normative wire/schema/fixtures stay lockstep. No speculative framework upgrades or registry edits. Physical unsupported interactions are human-blocked, never fabricated as passes. Isolated QA files are append-only and must not overwrite prior evidence.

## Evidence boundary

Durable evidence: Vitest 332 files/5880 pass/78 skip; worker Rust 1646/0/0/0 across exactly 100 result lines; CTest 196/196; post-DTO fmt/clippy warning-free; build tsc+Vite pass with >500kB warning. Historical Vitest RED is a record without a durable RED artifact; Rust RED is durable. Full browser is not current after final implementation; native GUI was last known closed at the prior checkpoint; full acceptance remains open.

## Acceptance checklist — all-phase Astra/main gate

1. Annulus: pointer/AX/Enter, exactly one hollow commit and one operation.
2. At least 100 commits each for fillet, hole and offset families across three fresh native launches, with render completion.
3. Invalid rapid typing, delayed cancel, tool-first and every invalid-mode path.
4. Constraint/snap/Alt/trim and both-cancel matrix; physical unsupported input is human-blocked, never fabricated.
5. Removed selection, recovery and currentness across changed/removed references.
6. Fully dimensioned housing and revolved flange, including solver-backed drag/rollback.
7. General FeaturePattern: atomic refs, twins, source edits, dependencies, suppression, repair, persistence, BodyPattern V2 unchanged.
8. Isolated native save/open/autosave and currentness.
9. Exact 1024×768, 1156×768 and 1440×900 normal/Retina/enlarged window matrix.
10. Every report-tool variant: explicit pass, defect, or blocked reason.

FeaturePattern acceptance additionally requires selected records rather than copied payloads; internal refs within an instance; host refs retaining host; external refs unique; cycle, multi-host and unselected-operand rejection; deterministic provenance per pattern/instance/source; whole-pattern suppression; and no opaque extension or Loft/Sweep claims.

## Execution order

Resume with AGENTS/CLAUDE → CURRENT_STATE/HANDOFF → this section → TODO/matrix. Recheck identity, dirty tree, processes and ports. Use the cheapest capable model; Claude Code must verify available equivalents and never assume GPT models/tools or extra GPT-call authority. Choose typed-target or D-camera bounded work, then pattern contracts serially, then UI. Build worker before Cargo, use Bun and `npx tsc`, read native Computer-Use skill before native validation, restart Vite and verify served modules, and keep heavy gates serial on main. Unresolved questions: none product-level; engineering validation remains open.

# PLAN: Fix the findings of the 2026-09-11 native UX review (`OneCAD-UX-Review-2026-09-11.md`)
status: done (uncommitted; ledger in TODO.md)

## Context

Source: `OneCAD-UX-Review-2026-09-11.md` (manual Computer-Use review, two live passes, three addenda). The
reviewer's verdict: the gaps are trust in recovery, history visibility, selection clarity, and numeric-input
confidence — not missing commands. This plan fixes every finding that has a code-side cause, records the ones
that do not, and adds the acceptance tests the review asked for.

Evidence added this session from `logs/dev.jsonl` (the Addendum C session, pid 48299, 11:46–11:54 UTC):

- **Undo never reached the backend.** Both ⌘Z / ⌃Z presses after the Revolve landed as `ipc://localhost/redo`
  (11:48:52, 11:49:02); `undo` was never invoked. The FE handler (`src/shortcuts/useShortcuts.ts:279-291`) maps
  `mod+Z` → undo and `mod+Shift+Z` → redo, so the injected key carried Shift (uppercase `Z`). Independent of that:
  there is **no native menu** (`src-tauri/src/lib.rs` never builds one; Tauri's implicit macOS default menu's
  Edit → Undo is NSResponder text undo), the palette has **no Undo/Redo entry** (`src/features/palette/paletteItems.ts`),
  a no-op undo/redo gives **no status hint** (`ModelToolController.ts:9302-9311` hints only when `res.opLabel` is
  set), and the undo chord fires **before** the `isEditableTarget` guard (`useShortcuts.ts:327`), so ⌘Z inside a
  number field undoes the document.
- **"Selection is out of date — pick again"** = `RefUnresolved` from `document_runtime.rs:4179-4187`
  (`promote_selection`: "element promotion returned an incomplete or mismatched batch") at 11:50:28, 24 s after the
  hole regen published snapshot 6, with the pick addressed **to snapshot 6** (`gate_stale_pick` passed). Worker
  `acquire_ids` (`worker/src/session/ElementIdentity.cpp:136-163`) omitted the pick: `resolve_pick` (`:62-99`)
  matched neither the topoKey nor the anchor. One second later `face_sketch_plane` + `add_sketch_on_face` on the
  same selection succeeded (`QueryElement` ids 96/97). Root cause not pinned. The only FE code that rewrites a
  ref's topoKey is `src/viewport/mesh/rebindPick.ts` (geometric best-effort rebind on every mesh swap,
  `REBIND_TOL_FRAC=1e-3`, `REBIND_DIR_MIN=0.9`, called from `meshSync.ts:391-396`). The Rust mismatch message drops
  the requested/returned keys, so the log cannot say which key was sent.
- **Displaced highlight after chamfer** matches `rebindSelectionForBody` (`rebindPick.ts:342-355`) re-pointing the
  consumed edge's ref to the nearest surviving edge; `HighlightLayer` draws whatever `ordinalForRef` resolves
  (`HighlightLayer.ts:184-227`).
- **"12 → 2 mm"**: type-to-enter seeds `DimensionInput` with `initialText="1"` (`ModelToolController.ts:9739-9749`
  → `toolChipStore.beginPrimaryEntry`), and the mount effect does `focus()` **+ `select()`**
  (`DimensionInput.tsx:261-266`), so the second digit replaces the selected first digit. Deterministic.
- **Fillet 100 → 11.99 silently**: `guardEdgeOpValue` (`ModelToolController.ts:3363-3371`) discards
  `clampToEdgeOpRange(...).clamped/.reason` (`src/tools/preview/filletRadius.ts:148-243`).
- **Sketch Cancel keeps geometry**: FE `exit()` always does `cancelSketch` then `finishSketch`
  (`SketchController.ts:1211-1262`); Rust `cancel_sketch` is squash-only (`document_runtime.rs:3971-3987`).
  `SketchSession { prior, undo_watermark, evicted_at_enter }` (`:296-312`) and `squash_sketch_session`
  (`:5095-5121`) already collapse the session into one net undo step, so a discard is one extra `session.undo()`.
- **Construction mode sticky across sketches** by design (`src/stores/sketchStore.ts:68-73`; reset only on document
  close).
- **History rows missing**: `HistorySelectionSection` (`src/features/inspector/sections.tsx:349-364`) shows
  `features.slice(0, 3)` for a body and the first extrude otherwise. Hole / Pattern / Chamfer rows exist in
  `HistoryList.tsx` (`OPTYPE_ICON`); they are cut off. No "Show all".
- **Sketch intercepts solid edge**: `refFromModelHits` (`src/viewport/ViewportRoot.tsx:81-95`) lets a sketch hit win
  any numeric tie against a body hit unless Alt (pick-through) is held; an edge-vs-sketch tie is treated like a
  face-vs-sketch tie.
- **Pattern disabled on a Hole**: patterns are body-only (`toolApplicability.ts:176-179`, reason "Select a body to
  pattern"; `worker/src/ops/PatternOp.h`; SCHEMA §7.3 body-copy semantics). The reason is tooltip-only.
- **Measure circular edge**: `ElementInfoDto` (`src-tauri/src/dto.rs:990-1016`) carries `magnitude` (arc length),
  no radius; `MeasureOverlay.tsx:34-37` prints "Length". The worker already computes circle/cylinder radius in
  `ClassifyElement.cpp:126,178` (different verb).
- **Std hole picker cells** render a literal `·`, never the mm value (`HoleChipCluster.tsx:315-343`).
- **Chip accessibility**: overlay root is `aria-hidden` (`ModelToolChips.tsx:79-84`); `DimensionInput` hardcodes
  `aria-label="Dimension value"` (`DimensionInput.tsx:409`); hole/chamfer fields have testids only.
- **Status verbs**: `finishExtrude` (`ModelToolController.ts:8413-8452`) says "Extruded" for Union/NewBody/Cut with
  results; "Cut completed" only on the zero-body branch.
- **Empty / projected sketch message**: `emptySketchCard()` exists (`constraintStatus.ts:64`); a face sketch holding
  only projected entities (11 from `add_sketch_on_face`) still reads "Fully constrained · DOF 0".
- **Snap kind** is resolved (`decision.primaryKind`, `snapEngine.ts:265`) and drawn as a glyph only
  (`SnapIndicator.ts:367`).
- **Shell / New-project freeze** (Addenda A/B): recurred with the old bundle before any shell op; rebuild fixed it; the
  earlier log was truncated by the relaunch. No evidence survives. Not fixable from code here.
- **Sketch vertex drag** has a green mock-lane spec (`e2e/sketch-drag.spec.ts`); the review's failure is
  unattributed (gesture injection suspected).

Conventions / commands (CLAUDE.md, verified): bun + vitest (`bun run test`), Playwright mock lane (`bun run e2e`,
port 4177, `retries: 0`), Rust from `src-tauri/` with `ONECAD_REQUIRE_WORKER=1`, ctest, hex gate, QA verifiers.
`*.golden.test.ts` and `src/test/contracts/` are frozen; `keymapContract.ts` has no undo/redo rows.

## Scope

In: WP-U1 … WP-U12 below. Out (Follow-ups): feature-level (hole) patterns, the freeze root cause, ViewCube orbit
semantics, measurement-chip de-overlap, pure hover QA, a docked full-parameter inspector.

## Decisions

- **D-1 Sketch Cancel discards** (user, 2026-09-11): Cancel reverts to the state at sketch entry and deletes a
  sketch minted this session; Esc keeps today's exit-and-keep.
- **D-2 Construction mode resets per sketch** and shows a persistent banner while on (user).
- **D-3 Native Edit menu is wired** to document undo/redo; FE re-dispatches text undo when an input is focused (user).
- **D-4 One Astra `break` (xhigh) on WP-U4** after implementation; findings are red-first fixes (user).
- **D-5 Selection never guesses.** `rebindPick` stops geometric re-pointing. A ref survives a regen only when the
  backend confirms it (promoted ElementId resolved by `ResolveRefs`); otherwise it is dropped silently. This is the
  same "deterministic NeedsRepair beats a silent wrong bind" law as the identity ladder, applied to the viewport.
- **D-6 Clamped values are disclosed, not blocked.** A typed fillet/chamfer value above the measured range is
  applied at the limit and the status hint says so with the limit and the reason.
- **D-7 Reviewer P1 acceptance tests become gated e2e specs** (mock lane) where the mock lane can express them;
  the real-stack items are recorded as owed user-run checks in `TODO.md`.

## Design

### WP-U1 Undo/Redo reach, guard, feedback, menu, palette (FE + Rust, careful)
- `useShortcuts.ts`: move the `isEditableTarget` bail ahead of the ⌘Z/⌘Y block so a focused input keeps native
  text undo. ⌘K stays the one documented exception.
- New `src/features/shell/undoActions.ts`: `runUndo()` / `runRedo()` = the existing mode branch (sketch →
  `undoSketch/redoSketch`, model → `ModelToolController.undo/redo`) extracted so keydown, palette, and menu share
  one router. If `document.activeElement` is editable, call `document.execCommand("undo"|"redo")` instead
  (menu accelerators reach the FE only through this router).
- `ModelToolController.undo/redo`: when `res.opLabel` is absent, hint "Nothing to undo" / "Nothing to redo".
- Projection DTO (`dto.rs`, `types.ts`, mock + tauri clients, lockstep): add `undoDepth`, `redoDepth`,
  `undoLabel?`, `redoLabel?` from `DocumentRuntime` (`undo_depth()` exists at `document_runtime.rs:894`; add the
  redo twin and top-of-stack labels via `DocumentSession`). Palette items "Undo <label>" / "Redo <label>" enabled by
  depth; keywords `undo`, `redo`, `revert`.
- Rust menu (`src-tauri/src/menu.rs`, new; wired in `lib.rs` `setup`): App submenu (predefined), File (New/Open/
  Save/Save As → `MENU_ACTION` events reusing the existing FE `fileActions` bridge), Edit (Undo `CmdOrCtrl+Z`, Redo
  `Shift+CmdOrCtrl+Z`, then predefined Cut/Copy/Paste/Select All), Window (predefined). `on_menu_event` emits
  `events::MENU_ACTION = "menu-action"` `{ action: "undo" | "redo" | "new" | "open" | "save" | "saveAs" }`.
  FE `tauriClient` subscribes (EVT lockstep) and calls the router / file bridge. Mock lane: no menu.
- Rust: `DocumentRuntime::undo/redo` unchanged.

### WP-U2 Type-to-enter seed (FE, standard)
- `DimensionInput.tsx` mount effect: when `initialText !== undefined`, `focus()` and place the caret at the end
  (`setSelectionRange(len, len)`), never `select()`. Unit test: seed "1", type "2" → text "12", preview 12.
- `useShortcuts`/controller unchanged. Also cover: seed "-" then digits, seed "." then digits (existing parse rules).

### WP-U3 Clamp disclosure (FE, standard)
- `guardEdgeOpValue` returns the full `clampToEdgeOpRange` result; the typed-commit path
  (`ModelToolController.ts:3340-3348`) sets the chip to the clamped value and hints
  `Radius limited to <max> mm — largest that fits the selected edges` (or the `nonMonotonic` wording) with
  `severity: "warn"`, cleared on the next edit. Drag path unchanged (drag already shows the live value).

### WP-U4 Selection integrity across regen (FE + Rust, **critical**)
1. Rust diagnostics first: `promote_selection` mismatch error message carries `requested=[…] returned=[…]`;
   `acquire_ids` logs the unresolved pick at `WLOG_WARN` with topoKey, kind and anchor.
2. Red-first Rust integration test (`src-tauri/tests/topology_rebind.rs` or new `selection_promote.rs`,
   `real_worker()` pattern): revolve a stepped profile, add four `Hole`s on the flange face, and after **each**
   publish tessellate the body, read every face/edge id off the MESH1 id table, and `promote_selection` each one at
   the head snapshot — all must resolve. Same for extrude+chamfer. If this is red, the bug is worker/Rust and gets
   fixed there before step 3; if green, the bug is FE-side and step 3 is the fix.
3. `rebindPick.ts`: delete the geometric rebind. New behaviour on a body mesh swap (`meshSync.ts:391-396`):
   for each selected/hovered ref on that body whose topoKey no longer resolves — if it has an `elementId`, call
   `client.resolveRefs` (existing lane, `src/ipc/promote.ts` neighbourhood) and adopt the returned topoKey when the
   outcome is `resolved`; otherwise remove the ref from `selectionStore` (no hint; the highlight simply goes with
   the geometry). Hover refs are always dropped.
4. `promotePick` (`ViewportRoot.tsx:126-149`) sends the pick-time topoKey and the current snapshot id from the
   store, never a rewritten one.
5. After an edge-op / shell / hole commit, the controller clears the refs the op consumed (its own inputs) before
   the regen result lands, so the next pick starts clean (reviewer P1 "correct selection after geometry edits").
6. Tests: vitest for the new swap policy (kept via elementId, dropped without), the e2e `filletChamfer.spec.ts`
   gains "after chamfer commit, select the new boundary edge, start Fillet, no stale hint".
7. Astra `break` (D-4) on this section + the diff; local `adversarial-reviewer` on the diff.

### WP-U5 Pick precedence (FE, standard)
- `refFromModelHits`: a body **edge** hit within tie tolerance beats a sketch curve/fill of a sketch that is not
  the active sketch; a sketch fill still beats a body **face** (keeps the flush-sketch-region target). Alt inverts as
  today. Unit test for the three cases; e2e `sketch-hole-extrude.spec.ts` or new spec: with a finished visible sketch
  on the top face, click the plate edge → Body edge selected, Fillet enabled.

### WP-U6 Complete, understandable history (FE, standard)
- `HistorySelectionSection`: for a body, list every feature whose result or input includes that body
  (`FeatureRecord` body refs; fall back to the full timeline when the record has none) in timeline order; for a
  sketch, the ops consuming it. Header shows `History · <n> of <total>` and a "Show all" toggle (local UI state)
  that switches to the full timeline. No slice.
- e2e: after hole + linear pattern + chamfer, the body inspector lists Hole, Linear pattern, Chamfer rows.

### WP-U7 Sketch Cancel = discard (Rust + FE, careful)
- Rust: `cancel_sketch(sketch_id, discard: bool)`. `discard=true`: squash as today, then if the squash produced a
  net step (`undo_depth()` moved), `session.undo()` it and clear the redo entry it pushed (revert must not be
  redoable back into a cancelled sketch); if the squash was refused (eviction), return `EngineError` "cannot discard —
  history was trimmed; changes kept" and keep today's behaviour. If the sketch record was minted in this session
  (FE passes `createdInSession`), delete it afterwards via the existing delete-sketch command.
- API: `api::cancel_sketch` gains `discard: bool` (default false for the old callers); `CMD`/DTO lockstep; mock
  client mirrors with its snapshot restore.
- FE: the chrome **Cancel** button and its palette command call `exit({ discard: true })` → `cancelSketch(id, true)`
  and skip `finishSketch`; Esc / Finish keep the current cancel-then-finish sequence. Hint "Sketch changes
  discarded". Tooltip on Cancel: "Discard changes since entering the sketch".
- Tests: Rust unit (cancel with discard restores `prior`, undo depth back to the watermark, redo empty), vitest for
  `exit({discard})`, e2e `sketch-undo.spec.ts` gains "draw two lines, Cancel → sketch gone / unchanged".

### WP-U8 Construction mode per sketch (FE, standard)
- `sketchStore`: `constructionMode` reset to `false` in `SketchController.openSession`; doc comment updated (W1-B
  note amended). Persistent banner chip in the sketch chrome bar while on ("Construction geometry — new entities are
  reference only"), status hint on toggle. e2e `construction.spec.ts` gains "toggle on, finish, new sketch → off".

### WP-U9 Verbs and reasons (FE, standard)
- `finishExtrude`: hint by boolean mode — "Cut created" / "Joined" / "New body created" (+ "N bodies"); when a Cut
  changes no body and no error → "Cut did not intersect any body". Revolve mirrors the same map.
- `toolApplicability` pattern reason: "Select a body to pattern — feature patterns are not supported yet". Disabled
  tool reason also appears in the chip/status line when the user activates a disabled tool via shortcut or palette
  (not just the tooltip).

### WP-U10 Chip labels, units, Std picker (FE, standard)
- `DimensionInput` gains `label` (aria-label, required at the model-chip call sites) and shows the unit suffix;
  hole fields "Hole diameter", "Counterbore diameter", "Counterbore depth", "Hole depth"; chamfer "Distance",
  "Distance 2", "Angle"; extrude "Depth"; pattern "Spacing"/"Count"/"Angle".
- Remove `aria-hidden` from the chip subtree only (keep it on the decorative overlay); testids stay.
- `HoleChipCluster` Std table: cells print the value ("Ø3.4") instead of `·`; header keeps fit names.
- Golden/contract check: `src/test/contracts/` inspector/toolbar contracts must not need edits; if a probe reads the
  aria tree, update the probe, never the contract.

### WP-U11 Measure: diameter for circular edges (FE only — revised after the protocol audit)
- **Audit outcome (2026-09-11):** `element_info` is backed by `QueryElement`, whose `elementId` rung normatively never
  reaches a `TopoDS_Shape` (SCHEMA §7.5 lines 3731-3734, 6753-6756), so an additive `radius` there would be present on
  one addressing rung and absent on the other. `ClassifyElement` (SCHEMA §7.5 :3529-3543) already returns `radius`
  for cylinder faces and circle edges and is plumbed to `ClassifyFrame.radius` (`src/ipc/types.ts:670`). **No wire
  change.**
- FE: after `elementInfo` in the measure pick (`ModelToolController.ts:1255-1265`) call
  `client.classifyElement(bodyId, elementId ?? "", topoKey)` (catch → null); `MeasurePick.radius: number | null`
  (advisory, head-read — `present:false` renders no Ø, never an error); `MeasureOverlay` prints
  "Ø 6.6 mm · Length 20.735 mm" for a circle edge and "R 20 mm · Area …" for a cylindrical face.
- Mock: `mockClient.classifyElement` classifies every non-straight edge as `other` with no frame — teach it circle
  edges (radius from the mock geometry) so the e2e `measure.spec.ts` can assert the Ø.

### WP-U12 Sketch guidance + snap text (FE, standard)
- `constraintStatus`: callers pass entity counts split by origin; "Projected geometry only — draw to begin" when every
  entity is projected/fixed; "Size fixed; position still free" when DOF equals the rigid-body freedom of the profile
  (translation 2 / +rotation 1) — computed from the constraint set, gated to the rectangle/circle cases the
  auto-constrain produces.
- Snap: the DOF/status badge shows the winning rule text while a draw tool is placing ("Snap: grid", "Snap: endpoint",
  "Length rounded"), from `decision.primaryKind` plus a `numeric` co-candidate flag.

## Tasks

| # | Package | Tier | Files (representative) | Depends |
|---|---|---|---|---|
| T1 | WP-U2 seed caret | standard | `DimensionInput.tsx`, `DimensionInput.test.tsx` | — |
| T2 | WP-U3 clamp hint | standard | `ModelToolController.ts` (3340-3371), `filletRadius.ts`, tests | — |
| T3 | WP-U9 verbs/reasons | standard | `ModelToolController.ts` (8413-8455, 3104), `toolApplicability.ts`, `activateTool.ts` | — |
| T4 | WP-U8 construction | standard | `sketchStore.ts`, `SketchController.ts` (openSession), sketch chrome bar, `e2e/construction.spec.ts` | — |
| T5 | WP-U5 pick precedence | standard | `ViewportRoot.tsx` (81-95), `Picker.ts`, tests, e2e | — |
| T6 | WP-U6 history | standard | `sections.tsx` (349-364), `HistoryList.tsx`, e2e | — |
| T7 | WP-U10 labels/Std | standard | `DimensionInput.tsx`, `ModelToolChips.tsx`, `HoleChipCluster.tsx`, `EdgeOpChipControls.tsx`, `ExtrudeChipControls.tsx` | T1 |
| T8 | WP-U12 guidance/snap | standard | `constraintStatus.ts`, sketch status badge, `SnapIndicator.ts` consumer | — |
| T9 | WP-U1 undo router + guard + hints + palette + projection depth | careful | `useShortcuts.ts`, new `undoActions.ts`, `paletteItems.ts`, `dto.rs`, `types.ts`, both clients, `document_runtime.rs` | — |
| T10 | WP-U1 native menu | careful | new `src-tauri/src/menu.rs`, `lib.rs`, `events.rs`, `tauriClient.ts` EVT | T9 |
| T11 | WP-U7 cancel discard | careful | `document_runtime.rs` (3971, 5095), `api/mod.rs` cancel_sketch, `SketchController.ts` exit, clients, tests | — |
| T12 | WP-U4 diagnostics + red-first Rust test | critical | `document_runtime.rs` (4179), `ElementIdentity.cpp` (136-163), new integration test | — |
| T13 | WP-U4 FE policy | critical | `rebindPick.ts`, `meshSync.ts`, `ViewportRoot.tsx`, `HighlightLayer.ts`, controller commit paths, tests, e2e | T12 |
| T14 | WP-U11 measure radius (FE only) | standard | `ModelToolController.ts` (~1255), `measureTool.ts`, `MeasureOverlay.tsx`, `mockClient.ts` classifyElement, `e2e/measure.spec.ts` | — |

Parallel waves (max three implementers at once, disjoint files):
wave A = T1, T3, T4 · wave B = T5, T6, T8 · wave C = T9, T11, T12 · wave D = T2, T7, T10 · wave E = T13 (alone,
after T12 verdict) · wave F = T14. Every diff orchestrator-reviewed; T11/T12/T13 → `adversarial-reviewer`; T13 →
Astra `break`; T14 → `protocol-auditor` before and after.

## Edge cases and failure modes

- Undo while a drag gesture is open: runtime already refuses (`revert_blocked_by_gesture`); the hint must say
  "Finish the drag first", not "Nothing to undo" — distinguish by `undoDepth > 0`.
- Menu accelerator with a focused `contenteditable`/input: router must re-dispatch text undo; verify on macOS by hand
  (owed user gate). WebKit `execCommand` returning false → fall through to document undo is **wrong**; do nothing.
- Cancel-discard when the squash was refused (eviction) → error hint, geometry kept, sketch stays.
- Cancel-discard on a sketch created from a face pick: also delete the datum/plane record if one was minted.
- Selection drop on regen must not clear a `feature` or `sketch` ref (only body sub-element refs on changed bodies).
- ResolveRefs `needsRepair` for a selected ref → drop it and surface the existing repair banner, never rebind.
- Pick precedence: an edge exactly on a sketch region boundary with Alt held → body wins outright (unchanged).
- Std picker values must come from `HOLE_STANDARDS` only; never persist the label.
- `radius` absent (line edge) → overlay omits Ø; old worker without the field → Rust deserialises `None`.

## Verification

- L0 per task: `bunx tsc --noEmit`; Rust tasks add `cargo fmt --all --check` + `cargo clippy --workspace --all-targets -- -D warnings`.
- L1: `bunx vitest run <file>` for each touched test; `cargo test -p onecad --test <target>`; ctest for the worker change.
- L2 per wave: `bun run test`; `bunx playwright test e2e/{model-undo,sketch-undo,filletChamfer,construction,measure,hole,sketch-hole-extrude}.spec.ts --project=chromium`.
- L3 at commit (orchestrator re-runs, main thread, no concurrent heavy job): ctest full · `ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker ONECAD_REQUIRE_WORKER=1 cargo test --workspace` · `bun run test` · `bun run e2e` (both projects) · fmt/clippy/hex · `node scripts/verify-modeling-coverage.mjs` · `node scripts/verify-modeling-contracts.mjs` · `scripts/check-worker-stdout-hygiene.sh`.
- Owed user-run gates (record in `TODO.md`): native Edit → Undo on the bundled app removes the pattern and restores on Redo; ⌘Z in a focused number field undoes the field only; reviewer scenario "revolve + 4 holes + face pick" shows no stale hint.

## Constraints

- No `git stash`, no branch switching, no commits without an explicit go; preserve unrelated worktree changes
  (`src-tauri/.claude/` untracked, never staged).
- Frozen contracts and goldens untouched; probes may change.
- No raw hex in TS/TSX; new colors via `tokens.css` in both theme blocks.
- No OCW1 change outside T14; T14 follows the SCHEMA change protocol.
- Subagents never run L3; the orchestrator re-runs every gate before it is written to `TODO.md`.

## Open questions

(none — D-1…D-4 answered by the user on 2026-09-11)

## Run log

- **T1 (WP-U2) done, inline.** `DimensionInput.tsx` mount effect: a seeded field places the caret at the end instead of `select()`; two new tests. `bunx vitest run src/features/sketch/DimensionInput.test.tsx` → 44 passed / 0 failed.
- **T5 (WP-U5) done, inline.** `refFromModelHits`: a body edge wins a tie against a sketch hit; a sketch clearly in front still wins; Alt unchanged. `bunx vitest run src/viewport/ViewportRoot.test.tsx` → 12 passed. `bunx tsc --noEmit` clean. **Deviation:** no new e2e — the mock lane has no fixture with a finished sketch coincident with a box edge; the arbitration is unit-tested and the engine hit-test path is unchanged. Follow-up recorded.
- **T6 (WP-U6) done, inline.** `HistorySelectionSection` lists the full timeline with a "N features · nothing filtered" line (`data-testid="history-count"`). **Deviation:** the projection has no feature→body lineage, so there is no per-body filter and therefore no "Show all" toggle; the count line makes the absence of filtering explicit. Golden `InspectorPanel.golden.test.tsx` untouched (count moved out of the section label to keep it green); `InspectorPanel.test.tsx` three `getByText("Sketch 2")` assertions relaxed to `getAllByText` because the selection header and the history row now both show the name (ordinary test, not a contract). `bunx vitest run src/features/inspector` → 190 passed / 0 failed.
- **T4 (WP-U8) done, delegated (impl-standard), diff reviewed.** `sketchStore.resetConstructionMode()` called from `SketchController.openSession`; shared `toggleConstructionModeWithHint()` in `sketchService.ts` used by the `X` shortcut and the toolbar button; persistent "Construction" chip (`data-testid="sketch-construction-banner"`) in `FloatingToolbar.tsx` (the toggle lives there, not in `SketchChromeBar` — deviation from the brief's wording, not the design). Agent-reported: vitest `src/stores src/tools/sketch` 1412 passed; `FloatingToolbar`+`src/shortcuts` 98 passed; `e2e/construction.spec.ts` chromium 4 passed; hex gate empty. Orchestrator re-run pending in the wave L2.
- **T8 (WP-U12) part 1 done, inline.** `projectedOnlySketchCard()` in `constraintStatus.ts`; `InspectorPanel` passes `projectedCount` (entities with `referenceLocked`) and shows "Projected geometry only · N projected reference edges · draw geometry to begin." instead of "Fully constrained". Tests: `bunx vitest run src/features/inspector src/features/sketch/constraintStatus.test.ts` → 200 passed. **Deviation:** the "Size fixed; position still free" sentence was not implemented — the projection exposes no per-entity constraint decomposition that makes the claim honest; recorded as a follow-up. Snap-rule text (part 2) still open.
- **T8 (WP-U12) part 2 done, inline.** Cursor numeric rounding now labels itself "Rounded" (`snapArbitration.ts` numeric candidate; typed values stay unlabelled) and the decision label appends numeric labels after the visible ones (`withNumericLabels`), so the existing snap hint chip reads "Grid · Rounded" / "Rounded". Glyph, guides, and `primaryKind` unchanged. New test in `snapEngine.test.ts`. `bunx vitest run src/tools/sketch` → 1262 passed / 0 failed; `bunx tsc --noEmit` clean.
- **T3 (WP-U9) done, delegated (impl-standard), diff reviewed.** New `completionHint.ts` (`completionVerb`) used by `finishExtrude`/`finishRevolve` — the agent also found and fixed a latent bug: `booleanMode` was read AFTER the FSM `settle` reset it to `NewBody`, so every Cut/Add commit would have been mislabelled; removed-body ids are now aggregated so a Cut that changes nothing reports "Cut did not intersect any body". Pattern reason text updated; `activateTool.ts` surfaces `verdict.reason` as a warn hint when a disabled tool is re-activated by shortcut/palette. `StatusSeverity` gained `"warn"` (renders like info; no new token). Agent-reported: `bunx vitest run src/tools src/features/toolbar src/features/palette` 2388 passed; e2e `sketch-hole-extrude` + `linear-pattern` chromium 4 passed; tsc clean. Deviation accepted: the pattern reason is 65 chars (design text kept verbatim). Orchestrator re-run pending in the wave L2.
- **T14 re-scoped after the protocol-auditor pre-read.** Verdict: `radius` cannot be added additively to `QueryElement` without amending two normative sentences; `ClassifyElement` already carries it. WP-U11 rewritten to FE-only (see Design). No SCHEMA edit, no fixture, no §14 entry.
- **T2 (WP-U3) done, delegated (impl-standard), diff reviewed.** `guardEdgeOpValue` returns the full clamp result; `publishEdgeOpClampHint` on the typed path (`onFilletChip`) hints "Radius/Distance limited to <v> — largest/smallest that fits the selected edges" or "moved to <v> — nearest size that builds" at `warn`, cleared on the next in-range edit; drag path unchanged. Two tests in `ModelToolController.chamferAngle.test.ts`. Agent-reported: `bunx vitest run src/tools/modelTools src/tools/preview` 978 passed. e2e skipped: the mock `analyzeEdgeOpRange` always answers `confidence: "none"`, so the clamp cannot fire in the mock lane (owed real-stack check: type 100 on the 12 mm plate → chip 11.99 + hint).
- **T12 (WP-U4 steps 1–2) done, delegated (impl-critical), diff reviewed.** Rust mismatch message now lists `requested=[…] returned=[…]`; worker `acquire_ids` logs an unresolved pick at `WLOG_WARN` (stderr). New `src-tauri/tests/selection_promote.rs` (911 lines) is RED by design: `revolved_flange_mesh_ids_promote_after_every_hole` FAILED (4 refusals, all the same minted id), `extruded_box_mesh_ids_promote_after_chamfer` ok. Agent-measured: clippy clean; `topology_rebind` 16 passed; `element_identity` 4, `m2_gate` 2, `hole_ops` 14, `chamfer_reference` 3, `chamfer_angle` 5, `face_color_reopen` 3, `revolve_ops` 10 passed; ctest 193/193; stdout hygiene clean. **Root cause found:** the MESH1 id table substitutes a minted ElementId (`el_…`) for any bound element (`worker/src/tess/Tessellate.cpp`, `IDS_HAVE_ELEMENTIDS`); `Picker.ts` copies that label into `topoKey`; `promoteOne` sends it as a TopoKey; `resolve_pick` only parses `f:/e:/v:` → refusal → "Selection is out of date". Chamfer scenario green because the chamfer consumes the promoted edge; that finding is the FE geometric rebind. Fix routed to T13 (FE short-circuit + Rust shape guard + narrowed test).
- **T14 (WP-U11, FE-only) done, delegated (impl-standard), diff reviewed.** `MeasurePick.radius` from a companion `classifyElement` read (caught to null, generation-guarded); overlay leads with "Ø <2r> mm · Length …" for a circle edge and "R <r> mm · Area …" for a cylindrical face. Mock `classifyElement` now fits circles for non-straight edges (`edgeCircleFit` in `mockMeshMetrics.ts`). Agent-reported: vitest measure/mock 202 passed; `e2e/measure.spec.ts` chromium 6 passed (new "circular edge shows its diameter"); tsc clean. Deviation: `ClassifyResult` has no `present` field (brief was wrong); the panel has no per-pick row so the Ø lives in the chip only.
- **T9 (WP-U1 minus the menu) done, delegated (impl-careful), diff reviewed.** New `src/features/shell/undoActions.ts` router (`runUndo/runRedo`: editable focus → `document.execCommand`, terminal; else sketch/model branch), `isEditableTarget` extracted to `src/shortcuts/editableTarget.ts`, guard moved ahead of the ⌘Z block; projection carries `undoDepth/redoDepth/undoLabel/redoLabel` (labels exist: `Txn.label`); palette commands `onecad.modeling.command.undo/.redo` titled "Undo Extrude" etc., mode-aware; `tauriClient.revert()` reads the label off the pre-call projection and reports it only when the depth moved (the old `applyEdit(CMD.undo)` path never set `opLabel`, so a working undo would have read "Nothing to undo"); controller hints "Undid X" / "Finish the drag first" / "Nothing to undo". Agent-reported: vitest 1289 passed across the touched dirs; `e2e/model-undo.spec.ts` 3 passed; fmt/clippy clean; `cargo test -p onecad --lib api::` 24 passed; QA verifiers exit 0. Orchestrator fixed the one red it flagged: `src/test/snap-decision/arbitration.test.ts` pins "a numeric-only decision is not a snap (label null)", so the T8 "Rounded" label now appends only beside a visible snap (`snapEngine.test.ts` + `src/test/snap-decision` → 220 passed).
- **T11 (WP-U7) done, delegated (impl-careful), diff reviewed; adversarial review dispatched.** Core: `SketchSquashOutcome {Empty|Refused|NetZero|NetStep}` returned by `squash_sketch_session`, `clear_redo`, `redo_depth/undo_label/redo_label`. Runtime `cancel_sketch(id, discard)` → `CancelSketchOutcome { discarded, kept_reason, revert }`; discard undoes only on `NetStep` (a depth delta cannot tell a net step from a refused granular one — agent's correction of the brief, accepted); api mirrors `api::undo` (projection emit, revert regen scheduled). FE: `sketchStore.exitIntent` carries the Cancel intent to `exit()`, `discardSession` drains the mutation queue, cancels with discard, deletes a sketch minted this visit, hints "Sketch changes discarded" / "Cannot discard — <reason>; changes kept". Agent-reported: fmt/clippy clean; `cargo test -p onecad cancel_sketch` 5 passed; `sketch_squash` 6 + `sketch_multi_object` 2 passed; vitest `src/tools/sketch src/ipc` 2187 passed, `src/stores src/features/sketch` 330 passed; e2e `sketch-undo` + `sketch-drag` chromium 4 passed; tsc clean.
- **T7 (WP-U10) done, delegated (impl-standard), diff reviewed.** `DimensionInput.label` prop; every model chip names its field with unit ("Depth (mm)", "Counterbore diameter (mm)", "Angle (°)", …); chip host is `role="group"` with "<tool> options"; the blocking `aria-hidden` was on `ViewportEngine.chipLayer()` (removed there; the decorative canvas overlay stays hidden); Std picker cells print "Ø3.4" with full aria-labels. Locators updated in 12 e2e specs and one probe (`modelingInteraction.numeric.probe.test.tsx`); no contract edited. Agent-reported: `bun run test` 5672 passed / 0 failed; `hole` + `filletChamfer` chromium 24 passed. **Orchestrator-run:** `bunx playwright test` on the nine other edited specs (extrude-draft, chamfer-angle, transform-body, units, revolve-region, revolve-commit, offset-face, construction, datum-create) chromium → **50 passed / 0 failed** (the agent's single `construction.spec.ts:281` camera-settle timeout did not reproduce; it ran on a contended port). Follow-up: a chip's aria-label unit does not re-render on a unit switch while armed.
- **T11 adversarial review (fresh context): 1 MAJOR, 3 MINOR — MAJOR and one MINOR fixed inline.** MAJOR: a refused discard returned without `finishSketch`, leaving the timeline record stale (regen would build from the previous geometry) — fixed in `SketchController.discardSession` (finish on refusal) and the refusal test now asserts the call sequence (`SketchController.exit.test.ts` 6 passed). MINOR fixed: `api::cancel_sketch` called `note_mutation()` on a no-net-step discard; now only when a revert happened (`cargo fmt` OK; clippy in the wave gate). MINOR accepted as follow-ups: ⌘Z right after cancelling a freshly created sketch first resurrects the (empty) sketch because `DeleteSketch` is its own undo step; the refusal path has no mock-lane e2e (the mock cannot refuse). Attacks run and clean: sketch-scoped ⌘Z before cancel, regen interleave, gesture guard, regen scheduling, `createdSketchId` deleting an old sketch, queue drain race, `exitIntent` leak, worker solver staleness, wire fidelity.
- **T10 (WP-U1 native menu) done, delegated (impl-careful), diff reviewed.** New `src-tauri/src/menu.rs` (`#[cfg(desktop)]`): App (mac-only predefined), File New/Open/Save/Save As with accelerators, Edit Undo ⌘Z / Redo ⇧⌘Z + predefined cut/copy/paste/select-all, Window; `install` registers `on_menu_event` → emits `menu-action {action}` (`events::MENU_ACTION`, `MenuActionDto`); `lib.rs` installs it in `setup` and degrades to the chords on failure. FE: `EVT.menuAction` listener in `tauriClient` → `src/features/shell/menuActions.ts` (dynamic import to avoid an ipc→features cycle) → `runUndo/runRedo` and the existing file bridges; `revert()` gained `noRegenWhen` so a no-op undo hints immediately. Double-fire: impossible by construction — `wry` `performKeyEquivalent:` makes the webview and the menu mutually exclusive consumers (source cited: `wry-0.55.1/src/wkwebview/class/wry_web_view.rs:44-56`); no throttle added. Deviations accepted: no predefined Close Window (⌘W must keep meaning "close project"); the Rust test covers `action_for` only because `muda::MenuChild` panics off the main thread under `mock_app`. Agent-reported: fmt/clippy clean; `cargo test -p onecad --lib menu` 4 passed; vitest `tauriClient.test.ts` + `src/features/shell` 228 passed; `e2e/model-undo` 3 passed; `bun run test` 5679 passed / 2 failed (both in `rebindPick.test.ts`, mid-rewrite by T13). **Owed user-run checklist** (bundled app): menubar shows OneCAD/File/Edit/Window; Edit → Undo removes a committed Extrude and Redo restores it; ⌘Z undoes once (hold: steady repeat, not doubles); ⌘Z inside a number field undoes typing only; ⌘Z on empty history hints "Nothing to undo" immediately; ⌘N on a dirty doc prompts; Open/Save/Save As match the chords; ⌘W still closes the project; ⌘X/⌘C/⌘V/⌘A in a text field work.
- **T13 (WP-U4 steps 3–6) done, delegated (impl-critical), diff reviewed; adversarial review + Astra `break` (call 1 of ~3, packet 04fb9a7c75a3, grounded, xhigh) dispatched.** `promoteOne` short-circuits an `el_` label (no wire call); Rust `promote_selection` refuses a non-TopoKey pick with a self-describing message; `rebindPick.ts` geometric rebind deleted (~250 lines) and replaced by `reconcileSelectionForBody` (keep when the new mesh names the label; else confirm via `elementInfo` by elementId, dropping on no answer; hover always dropped; later swap / deselect guards); `clearConsumedSelection` after fillet/chamfer/shell/hole success; `selection_promote.rs` narrowed to the per-namespace contract (2/2 green, red-first probes documented). Agent-reported: `bun run test` 5681 passed; e2e `filletChamfer` + `hole` chromium 24 passed; fmt/clippy clean; `selection_promote` 2, `topology_rebind` 16, `element_identity` 4 passed. Deviations accepted: `elementInfo` instead of `resolveRefs` (the ladder scores = guessing); `kind: ""` on the short-circuit (no consumer reads it). Follow-ups: the projection lane (`projectToSketch` sources carry raw labels, no `elementId`) still has the same defect; `ordinalForRef` trusts a snapshot-scoped ordinal across snapshots.
- **Wave L2/L3 (orchestrator-run, main thread, sequential):** `bunx tsc --noEmit` clean · `bun run test` **322 files, 5681 passed / 78 skipped / 0 failed** · hex gate empty · `verify-modeling-coverage.mjs` 32 rows OK · `verify-modeling-contracts.mjs` 39 rows OK · `verify-modeling-coverage.test.sh` negative controls OK · `cargo fmt --all --check` OK · `cargo clippy --workspace --all-targets -- -D warnings` clean (forced rebuild, 35 s) · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1592 passed / 0 failed / 0 ignored over 99 targets** (worker path set; no skips) · `ctest --test-dir worker/build` **193 / 193** (56 s) · `check-worker-stdout-hygiene.sh` clean. `bun run e2e` (both projects) still owed at the commit gate.
- **T13 reviews: Astra `break` (call 1 of ~3, xhigh, grounded, packet 04fb9a7c75a3, session 01a090eb-…) and the local adversarial-reviewer both returned DEFECTIVE, converging on one blocker.** Blocker: `namedBy` keeps a ref whose snapshot-scoped ordinal still resolves in the new mesh — element counts only grow across the measured regens (chamfer 12→15 edges; each hole +2 faces/+3 edges), so a stale ordinal always resolves, to a different element; `ordinalForRef` prefers topoKey over elementId, the highlight draws the wrong element and the fillet arm authors it (`ModelToolController.ts:3290` nulls the elementId when a topoKey is present; `gate_stale_pick` passes at head; `resolve_pick` never reaches the anchor rung). Also: head-snapshot `elementInfo` answers installed into an older displayed mesh (Astra F2); reply guards keyed by reusable `id` (F3); missing index kept an invisible selection (F4); three other lanes (`projectTool`, edge picks, library placement) still send raw `el_` labels (local MAJOR); Rust guard hardcodes `f:/e:/v:` instead of `TopoKey::parse` (`b:` exists); `kind: ""`; hole seat clear. Astra transcript read one file outside the allowlist (`worker/tests/test_wp6_ladder.cpp`) — recorded in `docs/design/astra/wp-u4-selection-survival.md`; F5 (§10 ladder guarantees) rejected as outside this package's path. Fix package T13b dispatched (impl-critical, red-first).
- **T13b (WP-U4 fixes from both reviews) done, delegated (impl-critical, red-first), diff reviewed.** `rebindPick.ts`: `ordinalForRef` tries the ElementId first; `verdictFor` (keep only when the new id table names the ElementId / confirm via backend when the ref has one / DROP an unpromoted ref) replaces ordinal trust; `confirmRef` fences the answer on body, kind, and drawability in the displayed mesh and matches the ref by object identity; reconcile only on a regen publish (not first load, colour, visibility, self-heal); `loadBody` re-checks `loadSeq` after the colour await; `dropBody` clears the removed body's refs; `promotePick` writes back only `elementId`, by object identity, with the pick's kind. Authoring lanes prefer the persistent handle: edge picks send `elementId` (`prepareEdgeOp`), projection sources carry `elementId` and `api::project_to_sketch` skips promotion for them (`ProjectionAddress`), library placement already did. Rust guard uses `TopoKey::is_valid()`. Hole no longer clears its seat face (four-holes flow). Every finding shown RED first (agent output quoted in its report) then green: `rebindPick.test.ts` 27, `meshSync` +3, `ViewportRoot` +2, `promote` +1, `projectTool` +1, `edgePrepare` +1, `hole` flipped, Rust guard test +1, e2e `filletChamfer` +1 (bystander drop on the commit's regen; mock lane cannot publish on undo/redo, so "redo empties the selection" is an owed real-stack check), `hole` +1 (seat survives, second Hole arms with no stale hint). Agent-reported: vitest 2828 (touched dirs) / `bun run test` 5700; e2e chromium 31 + 34 passed on port 4191; fmt/clippy clean; `promote_selection` 6, `selection_promote` 2, `topology_rebind` 16, `element_identity` 4, `chamfer_reference` 3, `chamfer_angle` 5, `hole_ops` 14 passed.
- **L3 after T13b (orchestrator-run):** `bunx tsc --noEmit` clean · `bun run test` first run **1 failed** (`useShortcuts.test.tsx` "finishSketch confirmation": passes 4×/4 alone, reds only under full-suite load — the confirmation lands after an async queue drain and the tests asserted after one `setTimeout(0)`); fixed by polling with `waitFor` (test-only), then **5700 passed / 0 failed × 2 runs** · hex empty · three QA verifiers OK · fmt OK · clippy clean · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` **1593 passed / 0 failed / 0 ignored, 99 targets** · ctest 193/193 (no C++ change since that run) · stdout hygiene clean · full `bun run e2e` re-launched alone (the earlier lane, 534/4 with the 4 reds all webkit in specs T13b edited mid-run, is discarded as contaminated).
- **Commit-boundary L3 complete:** `bun run e2e` (both projects, retries 0, run alone after every change) → **542 passed / 0 failed (31.0 min)**. With the rungs above, the tree is a full L3-green gated tree. No commit made (not authorised). Ledger written to `TODO.md`.

## Follow-ups

- Feature-level patterns (pattern a Hole, not a body): kernel + SCHEMA work; Astra-first (`derive`) when scheduled.
- Freeze root cause: no surviving evidence; add a `?vpdebug`-independent heartbeat line to `logs/dev.jsonl` on the
  FE side so a future hang leaves a trace (cheap, do with the next debugging pass).
- ViewCube drag orbit vs roll; measurement-chip de-overlap; docked full-parameter inspector; pure hover QA.
- Sketch vertex drag: reproduce with a physical mouse before any change (`e2e/sketch-drag.spec.ts` is green).

## Current hardening plan override — 2026-09-11 (append-only)

The earlier “done” and clamp-D6 notes in this plan belong to a historical smaller plan. The current approved hardening plan overrides them: invalid typed values remain visible and block confirmation; `Use maximum` is explicit and verified non-committing. Full acceptance remains OPEN pending the required independent evidence and native validation.

## Final hardening checkpoint — 2026-09-11 (append-only)

The earlier “done” and clamp-D6 entries above belong to the historical smaller plan. Current approved hardening overrides them: invalid typed values remain visible and block confirmation; `Use maximum` is explicit and verified non-committing; full acceptance remains OPEN.

Main independently recorded promoter + StartScreen **240/240**, native receipt **6/6** including cap, currentness **55/55**, chooser + ViewportRoot + Popover **32/32**, bounded Revolve core **9/9**, real-worker **4/4**, six frontend files **230 tests**, three UI suites **129 tests**, and fresh chooser Chromium + WebKit **2/2** retries 0 after stale-server restart. These are bounded evidence only. Preserve **61 pass / 19 fail**, preliminary A26/shared Add5 labels and mock missing-provenance observation; native app was not relaunched. Full suite/clippy, stress, persistence GUI, remaining adapters/CTest targets and D-camera remain OPEN. No percentage increase claimed.
