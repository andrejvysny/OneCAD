# OneCAD-Tauri — Current State (2026-08-03, STEP-IMPORT shipped; BODY-TRANSFORM in flight)

## ROADMAP (approved 2026-08-02, plan `~/.claude/plans/act-as-senior-software-reflective-swing.md`)
User priorities: 3D-print + machined parts + daily driver; light multi-part (no
mates); STEP import first. Queue: Step-0 USER manual gates
(`docs/MANUAL_GATES_RUN.md`) → WP-0 ✓ → WP-A ✓ → WP-B (in flight) → WP-C
pro-ops sweep (mass props, units, hole tool, draft UI, 2-dist chamfer, sketch
offset/fillet, revolve region-parity). NOT next: assemblies/mates, Loft/Sweep,
drawings. Internal adversarial review pre-approval: REVISE → 3 BLOCKER + 7
MAJOR + 8 MINOR all folded (incl. B1 transform wrong-bind, B2 256-solid cap,
B3 checkpoint-replay fallacy → brep/xbf-primary codec).

## WP-B BODY-TRANSFORM W0 (2026-08-03, commit 0dc8c91) — parametric move/rotate/copy
Core `TransformBody{targets[], translate 3×Scalar, rotate{frozen center, axis,
angleDeg}, copy}` + `can_fold_transform` lineage query (fold rule: one
cumulative record per placement intent). Worker TransformOp: normative T∘R,
copy:false = modify-in-place id-preserved via apply_history level-1 rebind +
NEW `apply_placement` (anchors move WITH the body — fixed the pre-existing
stale-anchor latent); copy:true mints §2 N-body ordinals. EDIT-SAFETY GATE
(refined from the plan's blanket ban): healthy transform-then-model flows
resolve clean; editing/suppressing a TransformBody seeds NeedsRepair on
downstream lineage refs (descriptor scoring against moved geometry admits
congruent-decoy WRONG binds — H5-B class; gate holds until pickFrame W3).
ctest 85/85 · cargo 680/0 · transform_body 8/8. W1 FE (FSM/chips/`t`/fold/
exact mock) in flight; then W2 gizmo, W2.5 align, W3 pickFrame deferred-safe.

## WP-A STEP-IMPORT (2026-08-02, commits 0598369→885d92e) — COMPLETE
Real import: Start Screen + in-editor File menu → XCAF product names + per-face
colors → bodies are FIRST-CLASS (fillet/boolean/sketch-on-face/patterns) →
identity survives process death. Architecture: §7.8 import VERB was
structurally wrong (session mutation outside ExecutePlan = deleted by first
regen) → §7.3 `ImportStep` RECORD, content-addressed source in NEW
authoritative `imports/<sha256>` container section (256 MiB cap, refcount at
save pins suppressed records + provenance, doc with missing blob still OPENS).
Replay = **xbf-primary** (BinXCAF storage v12; plain brep DROPS XCAF attrs;
STEP co-stored as provenance via `provenanceSha256`; W0 proved cross-process
determinism + BinTools ~100× faster than STEP re-parse — and `from:0` regen
NEVER consults checkpoints, so replay speed is open-time UX). `InspectStep`
probe (includeGeometry conversion lane) runs BEFORE record authoring — bad
file leaves the start screen intact. MESH1 gains FACE_COLORS (type 12, flags
bit 4, unset=alpha-0 → body token, theme-rebake in place); FE de-indexes
colored bodies (triangle ordinals preserved — picking pinned by real-raycast
test), shadedVertex MaterialKind white-base.
THREE stdout-corruption defects found+fixed en route (all would land bytes
mid-OCW1-frame): BinTools_ShapeSet::Read version banner, TDocStd_Application's
PRIVATE messenger (58B ANSI), plus ExportStep's own knob leak (observed).
XCAF does NOT inherit colors downward — solid-label fill pass (whole-part-blue
was 0/6 faces). W5 acceptance gate: ordinal-agnostic, survived the brep→xbf
flip unchanged; delete-import ⇒ NeedsRepair "no-candidates", undo exact.
Suites at close: ctest 84/84 · cargo 654/0 · FE 2223/162 · e2e 114/114 · tsc/
clippy/fmt/hex clean. Deferred: W6 preflight dialog + progress frames.
SEAM FLAGGED (pre-existing): promoted-but-unconsumed ElementId doesn't survive
process death → face-sketch dblclick re-entry match can fail after reopen
(TODO.md, route to face-sketch owner).

## WP-0 (2026-08-02, commit d875ef9) — split-child identity prerequisite
`split_origin` 256-probe DELETED → `split_child_uuid` memoizes derived→(op,k)
at its sole mint path; exact + unbounded (>256-solid import used to silently
lose `split_of` ⇒ cross-process REF_UNRESOLVED). SCHEMA §2/§7.2 `:<k>` widened
to "ordered children of any N-body op" + single-vs-multi minting rule.
Locked at k=300; M5a cold-interner proof green.

# Previous state (2026-08-02, FILLET-CHAMFER-UNIFY shipped)

## FILLET-CHAMFER-UNIFY (2026-08-02, commit (pending)) — ONE direction-driven edge tool
Plan `~/.claude/plans/act-as-senior-software-peppy-quilt.md` (internal adversarial
review REVISE → 1 BLOCKER + 6 MAJOR folded pre-approval). Shapr3D parity: F arms
one "Fillet / Chamfer" tool; drag AWAY from the body rounds, INTO it bevels, the
chip flips live; `[Fillet|Chamfer]` segments are the explicit override that ends
auto for the session (host-boolean precedent). `chamfer` tool id + `H` are dead
(`h` = Home again); the chamfer ICON lives on in history rows.
THE HONESTY RULE: the drag direction may pick the op ONLY where the sign is
provable — the bisector tier (sum of adjacent flat face normals reconstructed
from MESH1; FE has no edge→face topology). The bbox fallback points INTO
material on a concave edge, so off-bisector arms are `auto:false` and the chips
own the type. A cylinder cap edge degrades tiers HONESTLY (smooth normals fail
the tangent-plane test) and is pinned by test.
FLIP MECHANICS: `beginPreview` freezes opType, so a type flip is a session
close+reopen fenced by `++armGen` (two hysteresis crossings inside one in-flight
begin = stale-kind session + leak — race-tested); hysteresis 0.25 > value clamp
0.1 so type never strobes at the floor; mid-drag flip restores the 80ms trailing.
RE-EDIT TYPE FLIP: core `op_type_edit_allowed` sanctions exactly the
Fillet⇄Chamfer pair (enum-variant matched; params field-identical; undo exact via
whole-record inverse; legacy `mode` string normalized in core). Re-edit chip
became the full armed cluster — necessary: `DimensionInput.commit` fires only on
a CHANGED number, so a pure type flip could never commit from the bare chip.
PRE-EXISTING LEAK FIXED (W2): re-edit while the same tool was armed never swept
the old arm's in-flight preview session (`setTool` no-op → no `onToolChange`);
`editEdgeOpFeature` now fences AFTER `setTool`.
PROOF vs real worker: swap fillet(r=2)→chamfer(d=2) on a 20×... box — removed
volume 23.463 → 50.000, ratio 2.131 (analytic 2.33; asserted on the RATIO, never
the record's own opType (tautology) or label (fixture rename)); undo restores;
zero needsRepair.
Suites at final gate (orchestrator-verified): ctest 79/79 · cargo 600/0 vs real
worker · tsc 0 · FE 2135/156 · e2e 111/111 · hex 1 pre-existing (inputProbe).
NOTE: workspace `fmt`/`clippy` at gate time were red from the CONCURRENT
DEV-OBSERVABILITY session's in-flight edits (its files only); this WP's crates
tested clean before those landed.
REMAINING: USER manual Tauri gate (TODO.md FILLET-CHAMFER-UNIFY checklist).
Backlog: 2-distance chamfer / chordal / G2, edge-midpoint drag gizmo.

## DARK-MODE (2026-08-02, commit (pending)) — Light/Dark/System, chrome + live viewport
Plan `~/.claude/plans/act-as-senior-software-transient-puddle.md`.
The app had no theming layer at all: one flat `@theme` block of light hexes, and
a viewport that memoizes `THREE.Color` and builds ~30 materials once. So a token
swap would have repainted the chrome instantly and left the entire 3D scene
light. The only pre-existing seam, `resetPaletteCache()`, was exported with zero
callers; this wave wires it.

`src/theme/` owns resolution — a registry (`themes.ts`, mirroring the
render-mode table) plus a controller that is the sole writer of `data-theme` on
`<html>`. Only the PREFERENCE is persisted; "system" resolves against the OS at
runtime, following the precedent `navigation.inputDevice` set. Chrome re-themes
through CSS alone: Tailwind v4 compiles color utilities to `var(--color-*)`, so
a `:root[data-theme="dark"]` block re-themes everything with no `dark:` variants
anywhere.

The viewport cannot do that, so it is told: `ViewportEngine.applyTheme()` drops
through clear color → light levels → environment → eleven layer refreshes. The
order is load-bearing — PMREM fills the directions the studio room does not
cover with the renderer's clear color, so the environment must rebuild AFTER the
new background lands.

Two findings worth carrying forward. Tailwind INLINES `--shadow-*` at build
time, so overriding those tokens per theme is a silent no-op — dark shadows go
through `--tw-shadow-color` instead. And `--color-body-edge` has to INVERT:
wireframe mode draws edges with no faces behind them, so near-black edges on a
dark canvas would be invisible.
Appearance is reachable two ways: the full three-way radio in the display
popover, and a one-click cycling shortcut in the title bar (naming the current
value, so its changing icon is never ambiguous).
Suites at gate: tsc 0 · FE 2085/153 · e2e 11/11 new (94 prior untouched) · hex
clean. cargo/ctest not re-run — only `capabilities/default.json` changed on the
Rust side.
OPEN: a user reported dark chrome with a still-LIGHT viewport under `tauri dev`.
Not reproducible in the mock lane on either path (toggle-after-boot and
cold-boot-already-dark are both green vs real WebGL) and the dark canvas token
is confirmed in the built CSS. The palette cache was re-keyed BY THEME in
response, which removes the ordering-bug class that would explain it: a read
taken while the document says dark can no longer return a light color, so
correctness no longer depends on `resetPaletteCache()` having fired first. The
leading remaining theory is a Vite HMR artifact, not a product bug — a CSS-only
hot reload re-themes chrome instantly while `data-theme` never CHANGES, so no
event fires and the engine's already-built materials stay stale. Needs a cold
restart plus devtools output from the real webview to close.
BACKLOGGED THIS WAVE: the first per-layer test draft was VACUOUS (most layers
keep shared materials off the scene graph until something draws with them, so an
idle traversal proved nothing) — caught only by neutering an implementation and
watching the test stay green; the helper now refuses an empty traversal and every
case attaches real content. `--tw-shadow-color` is a Tailwind internal and no
test can catch a regression in it, because jsdom never runs the Tailwind
pipeline. Dark collapses per-shadow alphas to a single tint. `--color-on-accent`
goes near-black on the accent for AA contrast, which reads unfamiliar.
`palette.ts` now mirrors ~40 token values by hand across two themes with nothing
enforcing the match.
REMAINING: USER manual Tauri gate (TODO.md DARK-MODE checklist).



## RENDER-MODE (2026-08-02, commit 55454d0) — display-mode registry + studio shading
No plan doc; direct WP continuing MODELING-REACH W3, which shipped the
display-mode button as a 3-state cycler over session-only `viewportStore` state.
Now the mode is DATA: `renderModes.ts` holds the descriptor table
(`faceVisible` / `edgeVisible` / `materialKind`), so adding a mode is a table
entry, not a branch in `BodyObject`, the ingest controller, or the store.
`bodyMaterials.ts` becomes the sole owner of body materials — shared per-kind
sets, dim save/restore that replays the observed prior state, tints that copy
rather than retain the shared palette color. `displayMode` moved to persisted
`settingsStore` (v4) and is coerced on EVERY hydration, not just on a version
bump, because a same-version blob can still carry an unknown id. The cycling
button became a radio-row popover sharing one open-slot with the snap popover.
Shading is the other half: Neutral tone mapping, a `RoomEnvironment` PMREM IBL
built once at init (never per frame — the idle-zero-rAF contract holds), and a
camera-relative key/fill rig. WebGL-only by CONSTRUCTION — `createEnvironment`
is simply absent on the WebGPU and mocked handles, so no `isWebGPU` branch
exists in the engine. Every overlay layer now sets `toneMapped: false`: tone
mapping is for lit body faces, everything else renders its design token exactly.
Suites at gate: tsc 0 · FE 2012/149 · e2e 94/94 · ctest 79/79 + cargo 594/0
untouched (no Rust/C++ files in the diff) · hex clean.
BACKLOGGED THIS WAVE: the IBL path has NO automated coverage — the unit-test
renderer mock deliberately omits `createEnvironment` and e2e asserts scene-graph
flags, not pixels, so the studio look rests on the manual gate alone; the WebGPU
lane (lights-only, higher intensities) is untested; PMREM fills uncovered
directions with the renderer clear color, so `--color-canvas` now silently
drives body shading; `palette.ts` still caches colors forever and
`resetPaletteCache()` is still unwired, leaving every construction-time material
frozen for the life of the engine; two `BodyMaterialLibrary` instances exist
with no single owner able to reach both.
REMAINING: USER manual Tauri gate (TODO.md RENDER-MODE checklist).
NOTE: the frozen-material and unwired-`resetPaletteCache()` flags above were
closed by the DARK-MODE wave that followed.

## SKETCH-ON-FACE (2026-08-01, commits 2ac7aba→(final)) — sketch on model geometry, with the host outline as locked reference
Plan `~/.claude/plans/act-as-senior-software-twinkly-crown.md` (internal
adversarial review REVISE → 2 BLOCKER + 6 MAJOR + 6 MINOR folded pre-approval).
Four gates + HOST-BOOLEAN follow-up, all 4-suite green vs the real worker.
**HOST-BOOLEAN (user-reported same day)**: every op off a face sketch spawned a
NEW body — Shapr3D push/pull expects the HOST modified. Fresh arm on a hostFace
sketch now seeds Add + host target (`BooleanSeed` through the pure arm event;
re-edit never clobbered; hidden host falls back); extrude drag is
DIRECTION-AWARE while auto (away=Add, into=Cut, live chip+tint; symmetric never
flips; explicit chip click ends auto for the session); revolve seeds Add. Bound
target survives Add↔Cut (was: re-prompted pick on multi-body docs). Proof vs
real worker: Add → bodyEvents MODIFIED / 1 body / 153600 exact; Cut inward →
86400 exact. Typed-negative-depth chip does not flip (drag only — open).
THE POINT: a part is built by sketching on what you already made. Before this
wave a face-hosted sketch opened EMPTY — no outline, nothing to snap or
dimension against — so "sketch on a face" was decorative. Now the host face's
own boundary is projected in as `referenceLocked` geometry that bounds regions
but cannot be moved or deleted.
**W0 (2ac7aba)**: W0a `face_sketch_plane` topoKey rung — the SAME latent
MODELING-REACH W2 fixed for `element_info` and flagged unfixed here: element-map
entries mint only when an OP consumes an id, so a just-promoted face is
genuinely absent and the feature's single most common path failed loudly.
Red-first vs the real worker. W0b `referenceLocked` END-TO-END with zero
producers (flag on all 5 entity variants, L1 guards, worker + solver, FE
hydration + marshal latch). **DEFECT CLASS FOUND**: the C++ oracle's
`addConstraint` veto SILENTLY DROPPED every non-Fixed constraint naming locked
geometry — removed; immobility moved to solver tag-0 pins (endpoint-position
pinning is singular on 180° arcs, so arcs pin by ANGLE).
**W1 (d145ae6)**: worker `FaceBoundaryProjector` rewritten to oracle parity
(exact Line/Circle/Arc, CCW-via-D1-tangent, normative ordering, byte-identical
determinism) + `ProjectFaceBoundary` verb (SCHEMA §7.6, no fixture bump) +
Rust `add_sketch_on_face` (frameOnly → Rust-owned basis → real projection in
that basis, 1e-9 exact tripwire, failure leaves the document untouched).
**PRE-EXISTING BUG FOUND+FIXED**: `CoplanarFacePatch`'s predicate took
`gp_Dir::Dot` through an implicit `gp_Vec→gp_Dir` conversion, which made the
1e-3mm DISTANCE test angular (a 0.05mm-off face at 100mm lateral passed) and
threw `Standard_ConstructionError` on coincident origins. Coplanar collection
also became an all-faces scan, not BFS — disconnected coplanar prongs were
being missed. Proof: a tilted 10°-draft face projects onto its own plane with
origin-on-plane ≤1e-4 and extrudes to kernel-area×height to 8 sig figs.
**W2 (b01b71b)**: the real lane (`tauriClient` face branch → the new command;
the FE can no longer author a host-face `AddSketch` at all), mock parity from
the shared mock-mesh corners, L3 guards (drag/Trim/Delete all refuse, loudly),
machine pins hidden from the inspector without touching the session. The
**bbox-centre anchor** was the blocker here — it is not a point ON the face, so
the anchor became the kernel-exact plane origin. Hint-clobber fixed early:
`tryEnterOnSelectedFace` returns `{entered, refusal}` so a non-planar reason
survives the picker's own prompt (a hint published before `beginPlanePick`
writes is silently overwritten — the last-word-wins trap MODELING-REACH W0a
already catalogued).
**W3**: the two remaining entry triggers. The plane picker now accepts a body
FACE after the datum and the three world quads (order pinned with all three
stacked under one pointer), with the standard face tint driven through
`selectionStore.setHover` — one writer — and a prompt that names the body.
Deliberately NO hover-time planarity check: that is one backend round-trip per
pointer move. Double-clicking a face in model mode re-enters the sketch already
hosted there, else creates one, and VALIDATES BEFORE flipping the mode so a
non-planar face hints without moving the user anywhere. Re-entry needed the
frontend to know which sketch lives on which face, which nothing projected —
hence the additive internal `SketchDto.hostFace {bodyId, elementId}` (from the
attachment's `primary`; `protocol/SCHEMA.md` untouched, zero worker frames
moved, world/datum rows byte-identical).
Suites at final gate: tsc 0 · FE 1962/143 · cargo 592/0 vs real worker · ctest
79/79 · e2e 94/94 · clippy/fmt clean.
BACKLOGGED THIS WAVE: "newest sketch on this face" is last-in-projection-order,
which is uuid-lexicographic in the real lane (harmless — both candidates are
valid hosts — but not literally newest); `FaceExtrudeProfileBuilder` keeps its
own coplanar scan (correct math, repointing changes shipped-op behavior);
locked bare Point / Ellipse carry no solver pins; `beginGroupDrag` lacks a
locked pre-filter (fails loudly via pins); synthesized mock bodies keep the +Z
fallback frame; a body face is not in the orbit gate during plane picking (same
deliberate choice datum planes made).
REMAINING: USER manual Tauri gates (TODO.md SKETCH-ON-FACE checklist + the older
MODELING-REACH/SKETCH-POWER/TRUST/PREVIEW ones); Codex post-hoc reviews after
2026-08-05. NOTE: `src/viewport/debug/inputProbe.ts:87` holds one raw hex in the
file already marked TEMPORARY — the hex gate is otherwise clean.


## MODELING-REACH (2026-08-01, commits c960183→be4beb4) — datum planes, measure, view UX, arc-endpoint welds
Plan `~/.claude/plans/do-thorough-exploration-and-rosy-lollipop.md` (adversarial
review REVISE → 2 MAJOR + 3 MINOR folded pre-approval). Five gates, all 4-suite
green vs real worker.
**W0a (c960183)**: hint-clobber 8-site class → `resetToSelect` (20 sites, hints
finally visible); stale static layer after sketch-switch → local geometryToken
bump; Equal/Midpoint real-worker DOF proofs (all 7 kinds now proven).
**W0b (7bc73a5)**: arc endpoints are REAL solver points — worker mints
`.start`/`.end` handles coupled by PlaneGCS ArcRules (tag-0,
redundancy-invisible), Rust Coincident gains optional positions (frozen serde
byte-stable), slot caps welded to walls. SCHEMA §7.3 positions + §7.4 e3.start
examples became TRUE. Findings: entity-Tangent degenerate once welded (dropped
from slot, DOF 9 honest; endpoint-tangency kind backlogged); latent
constraint-free drag TELEPORT fixed; dup wire center points confirmed (+4 DOF,
pre-existing, backlogged).
**W1 (3a210cf)**: datum offset planes — D tool (PlanePicker + offset ghost),
core resolves frames AT CREATION (frozen, Rust = basis authority, stamps
datum-attached sketch planes), DatumLayer + tree section + sketch-on-datum both
clients, DeleteDatum referenced-guard. Worker + protocol byte-identical. Proof:
extrude off an XY+10-hosted sketch → bbox z=[10,15] exact; legacy-swapped bases
pinned (XZ offset moves world +X).
**W2 (d1b151b)**: measure tool (?) — exact kernel face area / edge length +
center↔center distance (bbox centers, labeled honestly); LATENT FOUND+ROUTED:
element-map entries mint on demand so promoted-but-unused ids are absent → new
`query_element_by_topo_key` ladder rung; **`face_sketch_plane` has the SAME
latent (flagged follow-up)**. Units: `src/units/format.ts` single seam,
mm/cm/m/in input parsing, valueText round-trips guarded both directions; `?`
inert while sketching (NO_CROSS_MODE keymap set).
**W3 (be4beb4)**: display-mode button REAL (was dead — renderer never read it;
default shadedEdges), ⇧F fits selection (explicit visible-filter —
Box3.expandByObject recurses invisible children), ⇧I transient isolate (never
persisted; preview-stomp guarded both directions).
Suites at final gate: tsc 0 · FE 1893/138 · cargo 570/0 vs real worker · ctest
77/77 · e2e 90/90 · clippy/fmt/hex clean. 8 implementation agents, every diff
reviewed + every gate re-verified by orchestrator.
BACKLOGGED THIS WAVE: endpoint-tangency constraint kind; wire center-point
dedupe (worker honoring centerRef); face_sketch_plane topoKey rung;
solved-arc-endpoint/radius FE write-back; datum rename/visibility +
OffsetFromFace/AngledFromEdge; chained-datum delete guard if resolution ever
re-derives.
REMAINING: USER manual Tauri gates (TODO.md MODELING-REACH checklist + older
SKETCH-POWER/TRUST/PREVIEW ones); Codex post-hoc reviews after 2026-08-05.


## SKETCH-POWER (2026-07-31→08-01, commits 130854b→b191ed2) — sketch expressiveness wave
Plan `~/.claude/plans/do-thorough-exploration-and-rosy-lollipop.md` (internal
adversarial review REVISE → 2 MAJOR + 2 MINOR folded pre-approval; Codex gate
dead until Aug 5). Four waves, 5 gate commits, every gate 4-suite green vs real
worker.
**W0 latents (130854b, all red-first)**: tree sketch-switch while IN sketch mode
retargets the controller (was silent wrong-sketch writes; self-switch guard vs
open session + openSession-echo bracket — naive subscription self-switched every
entry); `prepare_sketch_regions` refuses during same-sketch drag +
`finish_sketch` clears the dangling gesture (Enter-mid-drag race); Alt
pick-through (face under coplanar sketch fill selectable, default path
byte-identical); `getSketch` backend-id resolve; history icons opType-first.
**W1 construction geometry (c2566e8 + 27db309)**: worker `WireSketch` now READS
the flag Rust always emitted (hardcoded false before — LoopDetector's filters
were dead code; an all-construction rect published a region+extruded, ctest bug
pin); SCHEMA §7.3/§7.4 documented, no fixture bump (remaining region ids
byte-stable, runtime-baseline-pinned); core `SetEntityConstruction` (memento
inverse free, squash-safe); FE: X = selection flip (mixed rule
`!every(construction)`) / sticky draw mode + chrome button; `marshalUpsert` flip
branch over a last-SENT cache (flip emitted ZERO ops before); construction
centerline as revolve axis pinned both lanes.
**W2 tools batch (2626e46, FE-only verified)**: Tangent/Equal/Midpoint
user-apply (solver-bounded matrix; Midpoint gated on `marshalsAsPoint` — arc
endpoints would marshal null + silently drop; dups → existing
reject-on-conflict); point (P) / centerRect (⇧R) / slot (S) / polygon (G,
digits 3-9, construction circumcircle, DOF 4 any n) via new `ToolConstraintSpec`
(tool-authored constraints, intra-batch inference suppressed); slot ships
Tangent×4+Equal ONLY (arc-endpoint Coincidents unmappable — real-lane DOF ≈13
documented, mock e2e labeled); marquee box select (rightward=window true
containment / leftward=crossing touch — the semantics legacy UI promised but its
findInRect never had; 8 teardown paths restore LMB orbit; plane-AABB =
conservative superset off-normal).
**W3 ellipse (b191ed2, protocol change)**: SCHEMA un-UNSUPPORTs Ellipse w/
normative normalization-echo; ONE WireSketch branch unlocks both lanes (5
red-first fixtures failed "unsupported entity type" incl. the plan-profile lane);
solver-free legacy parity (naive DOF, PlaneGCS registration deliberately
skipped — vendored GCS has full ellipse support, backlogged); true Geom_Ellipse
extrude (vol 0.19% off π·a·b·h); FE 3-click tool (key O — e is the extrude
handoff) w/ live swap-normalization, applicability BAILS on ellipse targets
(oracle-parity), trim=whole-delete, mirror=copy-only.
Suites at final gate: tsc 0 · FE 1660/126 · ctest 76/76 · cargo 547/0 vs real
worker · e2e 76/76 · clippy/fmt/hex clean. Delegation: 9 implementation agents
(3 parallel max), every diff orchestrator-reviewed, every gate
orchestrator-re-verified.
LATENTS FLAGGED (accepted): `sketchStaticSync` A→B switch relies on real-lane
geometryToken bump (mock may show stale until exit); `enterRegionPick`
hint-clobber (pre-existing — "No closed region" never visible, setTool clears
it); FE full-suite exit-1-with-all-passing observed once (teardown flake class);
`bbox_dims` exactness tests inflate ~0.35mm (BRepBndLib gap).
REMAINING: USER manual Tauri gate (TODO.md SKETCH-POWER checklist) + the older
TRUST/PREVIEW/AUTO-MODE manual gates still open.


## SKETCH-MULTI-OBJECT (2026-07-31, commit 267af13) — re-entry deleted prior objects
USER-REPORTED: rect + disconnected line in one sketch → only latest survived
Finish. Root cause: `seedIdMapFromWire` MERGED the enter_sketch wire into the
per-sketch id-map instead of REBASING — stale frontend ids from the previous
session made `marshalUpsert`'s removals-first diff emit `removeEntity` for every
previously drawn object's backend uuid on the FIRST edit after re-entry (12 ops
for a rect); viewport rendered from the untouched local session so loss showed
only after Finish. Fix: clear entity/point/constraint/constraintValue before
seeding (backendSketchId/planeKind survive). Red-first proven; pins in 4 lanes
(sketchWireMap unit, sketchMultiObject full-stack vitest, e2e, real-worker
sketch_multi_object.rs asserting document sketch + timeline record + re-entry
union). Suites: FE 1407/119 · e2e 61/61 · cargo workspace green · clean.
ADJACENT LATENT (flagged, unfixed): tauriClient.getSketch passes raw frontend id
(siblings resolve backendSketchId); tree sketch-switch while already in sketch
mode is a controller no-op (chrome points at new sketch, controller edits old).

## TRUST + PREVIEW (2026-07-31) — silent-wrong-behavior class killed + all-op kernel preview
Two waves in one day (plan `mossy-foraging-muffin.md`, internal adversarial
review substituting the Codex gate; dual review REVISE → all 5 blockers +
16 majors across both cycles fixed red-first).
**TRUST (commit 49089bc)**: Suppression was geometrically INERT (record flag
never reached StepState; deeper: `OperationRecord::outputs` never populated in
production → dependency graph had ZERO body edges → cascade AND anti-time-travel
validation silently dead) — now real: one predicate (records), hash filter
(repairs latent checkpoint mismatch), cascade-on-suppress-only, all-suppressed
publishes a Clear, executed-scoped outputs sync (checkpoint regen must not wipe
the prefix). Body name/visible durable (adopt+overlay+save-merge; DeleteBody on
timeline bodies rejected; suppressed body leaves the tree). Revolve got extrude's
parity: stored-profile re-edit (exact regionId, volume-proven), pure-read arm,
commit-boundary record guarantee (NEGATIVE tripwire vs real worker), stored-axis
restore, geometryToken cancel. Pattern/Mirror/Shell re-edit was DEAD on the real
lane (kind-guards vs folded FeatureKind; mock emitted nonexistent kinds — the
postmortem false-green class) — opType-gated now, sourced from stored params,
loud refusals. Boolean op-swap re-edit. Tree: context menu, F2 inline rename,
backend-backed visibility over a no-awaiter metadata transport (sketch delete
excluded — ToEnd). Optimistic suppress overlay DELETED (un-suppress after reopen
was impossible: `!undefined→true` lock). Unsaved-changes guard on every close
path incl. start screen + ⌘Q (ExitGuard self-healing; Don't-Save-quit no longer
resurrects discarded work as crash recovery). statusMessage no longer dropped by
history actions; doc-lifecycle store resets; trace.ts dev/?trace-gated.
**PREVIEW (this tree)**: PreviewOp exercised by ALL op types — one shared
builder table (previewOps.ts), each builder fixture-pinned byte-equal to its
commit call-site; controller session infra generalized behavior-preserving
(extrude parity pins untouched, e2e 50/50 before wiring). Revolve previews the
real kernel result during drag (Cut actually subtracts); Fillet/Chamfer/Shell
moved to armed-commit gesture (Enter/✓/Esc; NO click-away — armed tools claim
every press as a value drag) with kernel preview and OCCT-refusal BLOCKING ✓
with the named reason; Boolean previews the fused/cut candidate with both
sources hidden. preview==commit proven per op vs real worker (revolve 2352.411 /
cut 9502.555 / fillet 19981.229 / chamfer 19960 / shell 2224 / union 30000 /
cut 10000 — all exact), head fingerprint + revision untouched by every preview.
Latent bugs killed en route: trailing-throttle >90ms froze coalesced previews;
progress hint buried error hints; test-fixture record-plane≠document-plane
diverged preview vs commit. SCHEMA §14 doc-only entry (no wire change).
Suites at Wave-2 gate: FE 1404+ · cargo workspace green vs real worker (+~35
tests: suppression 8, body_metadata 7, preview_revolve 2, preview_edge_shell 4,
preview_boolean 2, revolve_ops 8, dto/wire pins) · ctest test_preview_op 14
cases · e2e 60/60 · clippy/fmt/tsc/hex clean.
REMAINING: USER manual Tauri gates (TODO.md — Wave 1 + Wave 2 checklists, plus
the older AUTO-MODE/EXTRUDE-COMMIT-FIX ones); preview p95 latency budget
unpinned; worker-side preview coalescing deferred (cancel groundwork in ctest).

## EXTRUDE-COMMIT-FIX (2026-07-30) — select+drag worked, apply silently failed
User-reported: region select + drag preview OK, committing the extrusion did
NOTHING. Root cause (two halves): (1) the interactive flow never authored a
sketch's `Sketch` TIMELINE record — only tests did — so the regen planner could
not resolve any modeling op's profile and every extrude commit failed "profile
sketch not found in plan" (autosave forensics: 20 Extrude records, 0 Sketch
records; each failed ✓ stacked a duplicate errored record). (2) documents saved
BEFORE the half-1 fix carry no records either, so reopening stayed broken.
Shipped (all in the working tree's AUTO-MODE batch + this session):
`finish_sketch` mints/refreshes the record (+ outcome → scheduler, feature row),
failed commits roll back their errored record (no more retry stacking), and NEW
this session — `from_document` BACKFILLS missing Sketch records at the timeline
front on open/recover (cursor shifted, in-memory until next save, fixed-point
proven). Real-worker repro: legacy container (sketch, zero records) → open →
pure region read → exact-region `at_cursor:false` commit → publishes 1 body;
red-first proven (kill-switch run fails). Suites: cargo workspace green vs real
worker (scheduler_commit 6/6 incl. both repros, m2_gate, wire_contract,
topology_rebind, sketch_squash) · clippy/fmt clean · tsc/build clean · FE
1232/1232 (stale StartScreen marker updated for the deleted mode toggle) ·
e2e 38/38. REMAINING: user manual Tauri gate — reopen the broken document,
✓-commit an extrude, then delete the legacy stacked errored rows.

# OneCAD-Tauri — Current State (2026-07-29, AUTO-MODE shipped)

## AUTO-MODE (2026-07-29) — tool+context-driven mode switching; titlebar toggle DELETED
Mode is now derived intent: `toolStore.mode` remains the single state (all
consumers untouched), but only tools/context set it — never the user. New
`src/tools/activateTool.ts` dispatcher (toolbar + shortcuts both route through
it): a sketch-only tool from model mode enters sketch mode WITH that tool
preserved (`setMode` gained `opts.tool`); a model-only tool from sketch mode
finishes through the SAME drained-squash path as Enter (one undo step) and arms
the tool — Extrude rides the existing pendingExtrude region-pick handoff
byte-identical. keymap `resolveBinding` gained a cross-mode fallback (tool
actions only — L in model mode starts a sketch with Line; shared letters
R/C/M/H stay context-local; Delete can't cross). Viewport double-click on a
static sketch (model mode + select tool) re-enters its edit session, mirroring
the tree. Design approved via 4 user decisions; an auto-arm-on-single-region
prototype was CUT after e2e exposed it breaking the pinned multi-region
rejection flow (picking is select-tool-gated; an early arm locks selection and
turns region clicks into click-away commits). Suites: tsc 0 · FE 1230/109 ·
build clean · e2e 38/38 (+3 auto-mode) · hex 1 pre-existing (inputProbe).
REMAINING: user manual Tauri gate (TODO.md AUTO-MODE checklist).

# OneCAD-Tauri — Current State (2026-07-29, EXTRUDE-REGION-PARITY shipped)

## EXTRUDE-REGION-PARITY (2026-07-29) — exact selected profile, preview == commit
User-reported: extrude previewed/committed the WRONG sketch region and was
preview-only. Root cause: the live Tauri preview bypassed Rust's typed operation
lowering — the frontend sent nested `params.profile.regionId`, C++ expected flat
`params.regionId`, received none, and extruded the FIRST region; the commit used
the correct mapper, so preview and commit diverged. Implemented across a Codex
CLI session (P1 worker + P2 Rust + most of P3 FE; died on its usage limit
mid-hardening) and a same-day Claude continuation (P3 gaps, adversarial-review
defects, proofs, gate).
**Architecture**: one worker-authoritative `RegionTable` per solved sketch feeds
BOTH `SketchRegions` publication and modeling profile lookup — nested cells
(annulus + inner disk) and intersection fragments are independently selectable,
holes participate in region identity (`cell-v2` canonical signature under the
stable `r_<16hex>` wire shape; legacy outer-only ids resolve only when unique,
else loud failure listing candidates). `PreviewOp` consumes the SAME canonical
worker operation as `ExecutePlan` via one shared Rust lowering; typed
`sketchRegion` selection + full-FSM param snapshots (stable opId = commit
recordId) close the frontend end. `geometryToken` invalidates stale profiles
across undo/reopen — including a NEW proactive cancel of an armed extrude when
its sketch is edited underneath.
**Continuation hardening**: commit BARRIER — confirm flushes the final params as
the newest preview epoch and holds `endPreview(true)` until that exact candidate
answers (failure → re-armed, work kept; 4s timeout proceeds, backend re-validates
authoritatively). Partial ear-clip now fails `SketchRegions` closed (exact
`loop−2` triangle-count law) instead of publishing incomplete material. `ToNext`
casts rays from profile vertices + centroid against BOUNDED faces
(`IntCurvesFace_ShapeIntersector`) — the legacy nearest-ray-PLANE rule could bind
a face plane the profile never crosses. `STALE_PREVIEW` structured error code
replaces message-text sniffing (SCHEMA §8). `opType` reaches FeatureMeta on the
real lane (Chamfer re-edit opened the FILLET editor). Document-scoped UI resets
on new/open/import/recover when replacing an open document.
**Proofs** (real worker): inner disk by exact id — preview == commit volume ±1
(≈π·25·7); annulus binds independently; save → FRESH worker reopen → identical
hash chain + identical region-id set from the read-only query; ToNext
laterally-missed pillar FAILS loudly, nearer-missed-face is skipped (binds z=8
not z=5, vol 800 exact). e2e: commit-barrier epoch equality, multiregion,
hole-extrude, booleans.
Suites at gate (2026-07-29): ctest 70/70 · cargo 486/0 vs real worker ·
clippy/fmt clean · tsc 0 · FE 1205/107 · build · e2e 35/35 · hex 1 pre-existing
(inputProbe). REMAINING: user
manual Tauri gate (TODO.md); backlog — revolve region-parity (+ reopened-sketch
revolve is broken on the real lane), analytic fragment wires (chord V1
limitation), `PreparedSketchRegions` vs live-gesture race, coplanar-fill pick
precedence, `historyActions` statusMessage drop.

## MODEL-OPS (2026-07-26) — sketch-based modeling correctness + breadth, 4 waves
Goal: make the sketch-driven feature set correct and reachable. The backend was
far ahead of the frontend, and one shipped behaviour was silently wrong.
**W0 profile correctness**: `SolverLane` ear-clipped a region's OUTER loop only
while `FaceBuilder` builds the face WITH hole wires — a rect+inner-circle
previewed as a slab and committed as a tube, and a click INSIDE a hole selected
the region. New `loop/PolygonFill.{h,cpp}`: bridge-merged holes with SHARED
vertex indices so bridges stay interior — load-bearing, because the frontend
recovers extrusion rings from single-use-edge topology. FE `PrismProfile.holes` +
inner walls; mock `detectRegions` learned the worker's containment rule (so the
e2e lane can even see a tube). SCHEMA §7.3 profile-binding prose corrected — it
documented an `inputs[]` semantic ref **no layer has ever produced or consumed**.
**W1 extrude end conditions + Chamfer**: the worker has implemented
ThroughAll/ToNext/ToFace + two-direction + draft since W-WP6 and the wire carried
the fields; the tool authored only Blind/Symmetric. Chamfer was absent from BOTH
the `ModelTool` and authorable `WireOperation` unions despite a shipping
`execute_chamfer`. Fixed, plus two latent defects: `default_label` keyed off the
coarse `FeatureKind` bucket (a Chamfer read "Fillet", a pattern read "Boolean"),
and re-edits routed on `kind`, so Chamfer opened the fillet editor and
Shell/patterns/Mirror were unreachable on the real lane.
**W2 sketch-on-face**: `SketchAttachment::HostFace` was typed in Rust AND C++
since M1 with ZERO constructors — every sketch that ever reached the worker was a
world plane. No worker change was needed (`parse_plane` always accepted a custom
basis); it needed a producer. `QueryElement` got its first Rust caller ever
(`ElementQuery` seam → `face_sketch_plane`, which refuses a non-planar face), and
`plane_from_point_normal` is lock-tested because the frame is frozen with the
sketch. Datum resolution (`resolve_datum`) landed pure + tested; datum
CREATION UI did not.
**W3 backend preview verb**: the drag-time "exact" mesh was synthesized in
JavaScript by the same function the mock uses, so Cut never subtracted. New
kernel-lane `PreviewOp` runs the candidate op through the same executor a plan
step uses, over a throwaway head copy — invisible to fencing (no fence, no
prepare, no scratch). **Proven: preview Cut = 7500 while the real body stays
8000, and committing the same op lands on 7500.**
Suites: FE 1162/105 · cargo 461/0 vs real worker · ctest 69/69 · e2e 32/32 ·
clippy/fmt/hex clean. OUT OF SCOPE by user decision: Loft/Sweep.
REMAINING: preview latency gate, worker-side preview coalescing, Fillet/Shell/
Revolve preview sessions, datum creation UI, user manual Mac gate (TODO.md).

# OneCAD-Tauri — MODEL-HARDEN (2026-07-22)

## MODEL-HARDEN (2026-07-22) — Extrude/Revolve commit fix + professional UX, 6 gates
Root-caused USER-REPORTED "extrude preview vanishes on tool close": append-at-end
AddOperation returned RegenHint::None and the planner clamps to the applied cursor
prefix — **no fresh op commit EVER regened in the real app** (HISTORY row real,
geometry never computed, silent teardown; every UI suite runs the mock lane, every
integration test drove regen explicitly — new coverage class closes this forever).
Plan Codex-reviewed (terra/high, "revise" → all 10 findings folded). W0: frontier
append joins applied prefix (core cursor promotion; redo-draft regression pinned),
production-driver test seam (regen_driver_with_emitter) + scheduler_commit.rs vs
real worker. W0.5: squash all-or-nothing guard (stray finish_sketch can't pop a
model op's undo entry), model-mode arms → pure getSketch, regen completion carries
sourceRevision (exact per-commit correlation under rapid commits; superseded
ignored), FeatureDto.statusMessage → HistoryList tint+tooltip, legacy-draft open
hint. W1: armed-commit gesture — release keeps tool ARMED w/ live preview + chip
cluster [value|⇔|✓|✕]; Enter / ✓ / click-away commits, Esc cancels; failed commits
re-arm + name the reason. W2: boolean picker [New Body|Add|Cut] (auto-target or
click-to-pick, Cut tint), multi-select regions → N sequential ops (stop-on-first-
failure, previews kept), sketch auto-hide on success; revolve parity (N-op loop,
all-regions axis validity; quickCommit → click-away 360°). W3: worker RevolveOp
boolean split-children parity (publish_boolean_result; SCHEMA §7.2/§14 signed off,
no fixture bump) + revolve_ops.rs — FIRST real-worker Revolve wire coverage
(Pappus, 180°, Cut split adoption, stale-axis loud, region binding).
Suites: FE 1108/105 · cargo 423/0 vs real worker · ctest 66/66 · e2e 29/29 ·
clippy/fmt/hex clean. Commits d190d6a→(final). REMAINING: user manual Mac gate —
TODO.md MODEL-HARDEN checklist.


## SKETCH-HARDEN (2026-07-21/22) — sketch production hardening + UX, 6 waves, all gates passed
Plan Codex-reviewed (terra/high, "revise" → all findings folded). W0: Rust
squash-at-finish txn (whole sketch session = ONE model undo step), FE mutation
coordinator (generation fencing, transactional id-map, finish flush barrier),
sketch-scoped ⌘Z (snapshot stack, coalesced dim edits), dimension validation
(FE+Rust, H/V-Distance stays signed), zoom-normalized degeneracy guards,
dispose parity. W1 (protocol, signed off): per-constraint conflict ids on every
solve surface incl. session re-enter — rows/badges tint, reject hints name the
clashing constraint; fixed latent solveDrag raw-UUID bug. W2: Finish≠Cancel,
statusHint severity/sticky/4s auto-dismiss (all ~76 sites classified), line
chain Enter-end + click-first-close, formatDimensionValue 3dp, badge stagger,
glyph single-source, bodies dim 0.35 in sketch, crosshair cursors, mirror
hover. W3 (reviewed, APPROVE): REAL parametric trim (param-space crossings,
doomed-piece destructive hover ghost, whole-delete fallback). W4: snap
candidate cache (O(n²) intersections once per edit, equivalence-proven).
W5: draw-path integration tests, +5 e2e specs (23 total), angleUnits tests,
flake root-cause fix, stale-comment/debt cleanup.
Suites: FE 1065 vitest, Rust 406 vs real worker, ctest 65, e2e 23/23, clippy/
fmt/hex clean. Commits f8deef1→d619f53 (5 wave gates).
REMAINING: user manual Mac gate — TODO.md SKETCH-HARDEN checklist.

## AC-USABILITY (2026-07-20) — sketch/extrude usability gap-closure, 3 waves, all gates passed
Frontend interaction layer completed: sketch entity selection+hover, select tool
w/ point-handle drag via the real gesture lane, entity+constraint deletion
(Delete key + per-row ConstraintList), user-applicable constraints (toolbar pill
+ context chips, applicability matrix ported from C++), Trim + Mirror tools,
multi-region extrude/revolve pick. Five cross-layer bugs fixed: Tangent
wire-dropped, angle deg/rad (3 sites), OverConstrained never emitted,
orphan points on delete, re-entry representation split (centerRef ownership).
Plus: extrude auto-arm ordering, real-worker drag-preview reverse map, e2e port
collision. New proofs: sketch_reentry/sketch_edit/sketch_constraints real-worker
gates, hole-extrude volume ctest, Playwright e2e 6 specs (harness net-new).
Suites: FE 832 vitest, Rust workspace green vs real worker, ctest 65, e2e 6/6.
REMAINING: user manual Mac gate (`npm run tauri dev` — TODO.md checklist).

Non-destructive migration of OneCAD-CPP (~69k LOC C++20 Qt6+OCCT) into a 4-layer
Tauri app per NEW_SPEC.md. Tracker: `TODO.md` (per-WP gates + flags). OneCAD-CPP
stays untouched.

## Milestone status

| Milestone | Status |
|---|---|
| M0 foundations (protocol contract, scaffolds, corpus) | DONE |
| M1 cores (Rust document/history/regen/io · C++ worker OCCT+solver · frontend slice) | DONE |
| M2 first micro-slice integration gate | **PASS** (`m2_gate.rs` vs real worker) |
| M2-R implementation review | DONE — systemic BodyId wire-form defect found+fixed, `wire_contract.rs` regression gate, independent review APPROVE |
| M3 packaging gate | Linux portion DONE (externalBin deb-verified, path chain, bundle-dylibs.sh, PACKAGING.md); **Mac-side verification DEFERRED** (checklist §5) |
| FX file/app UX | DONE — save/open/recents/STEP-export UI, worker-status, live constraints |
| **M4 topology slice (backend + repair UI)** | **DONE, review-closed** (H5-B proven vs real worker) |
| **M5 lifecycle** (Revolve tool, STL/OBJ, checkpoints, splits, autosave+recovery, onecad-regen CLI, crash drills, drift gate) | **DONE** |
| M6 hardening + backlog | Shell+Patterns+Mirror + sketch parity (snaps, autoconstrain, Dimension) DONE; remaining: datum, Loft/Sweep, Playwright e2e, perf |

## What works end-to-end (real worker, automated gates)
Sketch (PlaneGCS, dof) → regions → extrude (multi-region by normative FNV id;
stale id fails loudly, never a silent wrong profile) → booleans/pocket/ToFace
(wire_contract volumes exact) → MESH1 → pick → ElementId promotion (stable,
Invariant 1) → fillet via scored ladder → **parametric edit → auto-rebind
(fillet survives) or deterministic NeedsRepair — the H5-B fix the corpus
documents as the legacy app's unfixed defect** (`topology_rebind.rs`) → repair
UI (banner → panel → score-ranked candidates → click-to-rebind via
promote + EditOperationInput) → save v2 container → reopen → deterministic
replay → STEP export → undo. Plus: Revolve tool (axis-pick + angle drag +
lathe preview), file menu (⌘O/⌘S/⇧⌘S/Export STEP), recents, worker-status
surfacing, history suppress/roll/delete affordances, solver-position
hydration on sketch re-entry.

## Suites (all green, orchestrator-verified)
- Worker: 61/61 ctests (OCCT 7.9.3; breadth ops m6a_ops incl.)
- Rust: 379 tests, clippy -D warnings + fmt clean (chaos 14, real-worker 5,
  m2_gate 2, wire_contract 8, topology_rebind 5, breadth_ops 6, checkpoints, regen CLI — vs real binary;
  ONECAD_REQUIRE_WORKER=1 guard in CI prevents vacuous greens)
- Frontend: 559 vitest, build green, hex-token gate 0

## Architecture decisions log (D-series + session additions)
- D1: NewBody BodyIds worker-minted `body_<opId>`, Rust adopts+fences
- D2: STEP export in worker · D3: `primary.topoKey` removed · D4: fencing =
  workerEpoch+expectedBaseHash only · D5: from-0 plans always base-valid
- Hash authority: Rust sole; planner hash decoupled from wire form (golden-pinned)
- Wire body form: ALL body-bearing params render `body_<uuid>` at the wire layer
  (core serde frozen); `intent` subtrees round-trip verbatim (never rewritten)
- Region binding: non-empty regionId MUST match (OP_FAILED naming available ids);
  empty = first-region V1 fallback; legacy ids sanitized on load (migrate diagnostic)
- AutoBind: elementId slot carries the Rust-minted id; topoKey is evidence (§9)
- Ladder policy: auto-bind ≥0.85 AND margin ≥0.10; symmetric tie ⇒ NeedsRepair;
  a fillet consumes its edge — re-resolving it NeedsRepairs (autoBind there = mis-bind)

## Key flags / known gaps
- Mac packaging verification (signing/notarization/bundle-dylibs first run) — needs a Mac
- L2 exact preview still local (no backend preview verb); revolve L1 only
- Checkpoints live (save-on-explicit-save policy, in-session restore V1; restore-fallback D1 edge flagged in TODO)
- Autosave+recovery live (30s debounce, startup-only recovery V1); onecad-regen CLI in CI
- Repair UI seams: resolveRefs sends refId-only; >1-body operated-body derivation;
  candidate viewport highlight = data seam; suppressed flag = optimistic overlay
- STEP import stub; Loft/Sweep UNSUPPORTED at worker (Shell/Patterns/Mirror LIVE end-to-end)
- Env note: Linux dev container uses conda-forge OCCT 7.9.3 at /opt/occt793
  (apt 7.6.3 too old); CI = macos-14 + Homebrew

## Conventions
- Orchestrator (Fable) designs/briefs/reviews; WPs → Opus subagents; RISKY WPs
  get independent adversarial review; commits at gate boundaries only
- protocol/SCHEMA changes need sign-off + §14 changelog (+ fixture bump if shapes move)
- Worker binary for tests: `ONECAD_WORKER_PATH=worker/build/onecad-worker`
