# Sketch Mode UX Audit → Ranked Improvement Plan (2026-08-19)

Branch `OneCAD-sketch-mode-hardening-UI-UX`. Three inputs, one ranked plan:

- **Part A** — hands-on audit of the mock-lane app (Vite + Chromium via playwright-cli,
  `?vpdebug&trace`, seeded "Bracket v2" demo doc), stepping through entry, every draw tool,
  snapping, constraints, dimensions, trim/fillet, lifecycle. Findings carry repro + code anchor.
- **Part B** — industry research, Shapr3D as gold reference, corroborated by SolidWorks /
  Onshape / Fusion 360. Full cited version: `SKETCH_UX_RESEARCH.md` (same directory).
- **Part C** — the ranked plan. Each item is specified to be implementable without
  further clarification. Wire-touching items are ranked but marked **deferred-wire** —
  this program stays frontend-side.

What is already GOOD (verified hands-on, keep):
line auto-armed on sketch entry (Shapr3D-only convention we already match) · live-dim chips
with Tab/Enter/Esc on 9 tools, commit path identical to click · scored snap arbitration with
hysteresis, per-kind glyphs + purple halo + text hint chips · polar rays + composable H/V
guides · trim red/keep preview · one-click corner fillet with typed refusal hints ·
tangent-arc `A` toggle · camera settle on enter, restore on finish · sketch-scoped undo/redo ·
snap popover ("Keep relationships" 3-state, Alt hint, S/M/L radius) · Esc ladder in-gesture ·
tree double-click re-enter · finished closed regions fill in model mode.

---

## Part A — hands-on findings

Severity: **S1** breaks trust/work, **S2** violates the CAD standard or misleads, **S3** polish.

- **A1 · S1 — Sketch geometry is invisible when the sketch plane coincides with a body
  face.** Sketching on world XY with the seeded body sitting on z=0: committed lines and the
  in-progress rubber band render nowhere (z-fight / draw-order vs the body face); snap
  markers and chips float over blank grey. Hiding the body reveals everything. Repro:
  enter sketch on XY under `body1`, draw a line across the body footprint. Anchor:
  `src/viewport/engine/SketchObject.ts` (line materials; no polygon offset / render-order
  guarantee over body faces). The user's very first sketch-on-face draw is invisible.
- **A2 · S1 — Mock-lane constraints are decorative.** `localSolver` is an identity solve:
  (a) auto-inferred Horizontal left line e7 tilted 0.4° forever; (b) Parallel applied to a
  Horizontal + a Vertical line was ACCEPTED (nc 12→13, no conflict, no hint) and nudged e7's
  geometry; (c) editing the committed Diameter chip 25→30 updated the label, circle stayed
  r=12.5. In the browser lane every constraint promise is a lie; conflict UX (reject +
  named loser) is unreachable. Real (worker) lane unaffected. Anchor: `src/ipc/localSolver.ts`.
- **A3 · S1 — Snap indicator + hint chip persist stale after a committing click.** After
  circle commit via Enter, "Grid" chip + marker sat at the center for the whole session
  (survived tool switches, trim, fillet) until a new hover decision replaced them. Same for
  "Quadrant" after a weld click. Anchor: `SketchController` commit paths never
  `setSketchSnap(null)`; only pointermove refreshes.
- **A4 · S2 — Add-constraint menu is ~18 icon-only, unlabeled, all-disabled buttons.**
  No text, no shortcut hints, no "why disabled" (needs selection). Constraint vocabulary is
  unlearnable from the UI. Context chips over the canvas are icon-only too. Anchors:
  `src/features/sketch/ConstraintMenu.tsx`, `ConstraintContextChips.tsx`.
- **A5 · S2 — No live closed-region fill while editing.** Closed loops shade only AFTER
  finish (model mode). During editing there is no "this is extrudable" signal; open-profile
  failure surfaces minutes later as "No closed region to extrude". Standard: live fill is
  the primary closure signal (SolidWorks/Onshape/Fusion documented; see B). Region math
  already exists (`closedLoop.ts`, `sketchTopology.ts`, model-mode fill).
- **A6 · S2 — Constraint badges are near-invisible and inert.** Committed H/V glyphs are
  ~8px grey dashes; not clickable, not selectable, not deletable on canvas
  (`ConstraintBadgeLayer.tsx:96` `pointer-events-none`). Standard (all four): click glyph →
  select, Delete removes; hover cross-highlights. Delete today only via Inspector list row.
- **A7 · S2 — Quadrant snap authors no relation.** Line started on circle quadrant:
  committed leg got only the far-end Coincident; the quadrant end floats free — looks
  attached, is not. A silent non-attachment (H5-B-adjacent in spirit). Anchor:
  `snapPersistence.ts` / quadrant candidates carry no `SnapRelationIntent`.
- **A8 · S2 — Sketch-mode coordinate readout is world-XYZ, not sketch u,v.** On the XY
  plane the sketch's u axis is world +Y, so the numbers can't be matched to typed
  dimensions; on a face-hosted plane they are meaningless to the sketch. Anchor: StatusBar
  coordinate source. Related, worth an investigation note: TOP view renders world Y
  screen-right / X screen-down (plane basis `xAxis=[0,1,0]`), unconventional.
- **A9 · S2 — Static sketches render saturated + region-filled while editing another
  sketch.** Sketch 2's region fills teal mid-edit of Sketch 6 (reads as selection),
  its entities z-fight for attention with active geometry. Named deferral exists:
  `staticSketchRoot`/`activeSketchRoot` split (TODO.md §10.5 deferral row).
- **A10 · S3 — Refusal/feedback channel is one small line, far corner.** All refusals
  ("Dimension removed — it would over-constrain…", locked-geometry, fillet/offset caps)
  land in the bottom-left status line; error severity only recolors text. Applying a
  constraint gives NO feedback at all when it succeeds (only the near-invisible badge).
  Standard is ambient + near-the-action (inline bubble / cursor label / top bubble).
- **A11 · S3 — Small paper cuts** (each verified):
  a. Empty sketch inspector: "Fully constrained · DOF 0 — Sketch is fully defined."
  b. Rect hint identical in both phases (no "click the opposite corner").
  c. Tangent-arc radius chip overlaps/clips its own text mid-gesture.
  d. `point` tool: no idle hint entry, no live-dim frame (skips the SP-1 lane).
  e. Origin pill ("origin" black chip) permanently occludes the sketch origin area.
  f. ~~Model-toolbar tooltips lack shortcut hints~~ — RETRACTED (Wave 1): tooltips
     already render "(E)"-style via `ToolButton`; the audit read aria labels only.
     A regression test now pins it.
  g. H-/V-Distance exist in code but are absent from `DIMENSIONAL_TYPES`
     (`constraintCatalog.ts:72`) — unreachable from the Add-constraint menu.
- **A12 · S3 — Esc ladder's third press exits sketch mode silently.** In-gesture tiers are
  right; but idle-Esc×3 = finish sketch with zero feedback. Shapr3D's own users file this
  exact complaint; majority convention keeps an explicit Finish affordance primary (we have
  one) and makes full exit deliberate. Minimum: a visible "Finished Sketch N" confirmation.

Known deferrals folded in from TODO.md (not re-discovered): slot endpoint-tangency
constraint kind (worker/wire) · slot wire round-trip mints +4 DOF stray points ·
centerRect has no persistent symmetric linkage · W3 project-edges (separate wave, wire).

---

## Part B — the standard, and where OneCAD diverges

Condensed; full citations in `SKETCH_UX_RESEARCH.md`.

| Convention (strength) | OneCAD today |
|---|---|
| Live fill of closed regions while sketching = the closure signal (3/4 documented, likely 4/4) | Fill only after finish (A5) |
| Per-entity constrained-state color: blue=under, black/green=full, red=conflict (4/4; Shapr3D per-point green) | One whole-sketch tint (blue/green/amber) |
| Constraint glyph: click→select, Delete removes, hover cross-highlights (4/4) | Badges inert (A6) |
| Inference line + preview glyph near cursor BEFORE commit (3/4) | Guides+labels yes; no preview of the constraint to be authored |
| Type-number-while-drawing → driving dimension (4/4) | ✅ SP-1, best-in-class (Tab cycle, expression support is the only gap) |
| Text label at cursor naming the snap (Shapr3D "Snapping Hints", Onshape) | ✅ have it (chip), plus stale-chip bug A3 |
| Purple as unified guide color (Shapr3D) | ✅ matches |
| Line auto-armed on sketch entry (Shapr3D-only) | ✅ matches |
| Esc = in-progress tier first, tool stays; full exit explicit (majority-leaning) | Mostly ✅; silent 3rd-Esc exit (A12) |
| Single canonical Finish button (3/4) | ✅ "Finish sketch" |
| Refuse conflicting constraint at apply-time with inline message (Shapr3D) | Real lane ✅ (reject + named loser); mock lane broken (A2) |
| Constraint keyboard shortcuts Shift+letter (Shapr3D) | None |
| Dedicated constraint manager panel for bulk ops (2/4) | Inspector list is close; no filter, no conflict-only view |
| Snap suppression modifier (Shift 2/4, Alt SW) | ✅ Alt (SolidWorks convention) |
| Projected/reference geometry in distinct color (Fusion purple) | N/A until W3 — spec it there |

Divergence summary: OneCAD's snap/live-dim ENGINE meets or beats the standard; the gap is
almost entirely in **feedback and manipulation surfaces** — region fill, per-entity status,
badge interactivity/legibility, refusal visibility — plus the two S1 trust defects (A1, A2).

---

## Part C — ranked improvement plan

Rank = user impact ÷ scope. Scope: S <½ day, M ≈ 1 day, L > 1 day (implementer-time).
Cut line (user-approved): implement EVERYTHING frontend-scope; deferred-wire items are
specified but not built here.

### P0 — trust defects, visible every session

1. **Active-sketch draw priority over body faces** (A1, M, viewport).
   Active sketch content (lines, points, preview, region fill of #2) must render above
   coplanar body faces: `polygonOffset` on body face materials while a sketch session is
   active, or `renderOrder` + `depthTest` tuning on the active sketch group only. Static
   sketches keep normal depth. Must not disturb picking (raycast unaffected by renderOrder;
   verify `probePick`). Acceptance: line drawn across the seeded body's footprint on XY is
   fully visible at every zoom; e2e: draw-on-face spec asserts pixel presence.
2. **Live closed-region fill while editing** (A5, M, viewport + topology reuse).
   On every `setSession`, compute closed loops (`closedLoop.ts` — already powers commit)
   and render translucent fills (token: reuse the model-mode region fill color at lower
   alpha) under the active sketch's entities, updating per solve. Open loop = no fill;
   closing click makes the fill appear instantly = the closure signal. Perf guard: recompute
   on entity-set change, not per pointer frame.
3. **Clear snap indicator on commit** (A3, S).
   Every commit path (`applySteppedClick` when the step commits, `commitLiveDimGesture`,
   Enter/Esc chain end, tool switch, dimension/trim/extend/fillet/offset clicks) ends with
   `setSketchSnap(null)` unless a fresh decision follows. Regression test: after commit,
   indicator group empty until next pointermove.
4. **Constraint menu + chips legibility** (A4 + A11g, S).
   `ConstraintMenu`: text label + glyph per row, section split (Geometric / Dimensional),
   disabled rows get a tooltip "Select two lines…" from `evaluateApplicability`'s reason
   (extend it to return why-not), add H-/V-Distance to `DIMENSIONAL_TYPES`.
   `ConstraintContextChips`: add `title` tooltips now; keep icons.

### P1 — standard-conformance gaps

5. **Interactive constraint badges** (A6, M, overlay + selection).
   Badges become clickable: click selects the constraint (`sketchSelectionStore`), selected
   badge tints accent, `Delete` removes it (reuse ConstraintList delete path incl.
   referenceLocked filtering), hover badge ⇄ highlight entities (both directions — canvas
   hover already flows one way from the list). Raise badge size to ≥14px CSS and use the
   token ink color at full contrast. Coincident stays hidden-by-default.
6. **Constraint shortcuts** (S/M): sketch-scope `Shift+H/V/P/T/E/M/C` → apply
   Horizontal/Vertical/Parallel/Tangent/Equal/Midpoint/Coincident to current selection via
   the same `applyConstraint` path; no-op with hint when inapplicable. Update keymap
   contract test as a feature change (contract edit is legitimate here: new bindings, not a
   probe fix). Show shortcut in menu rows (item 4).
7. **Quadrant snap authors OnCurve** (A7, S): quadrant candidates carry a
   `SnapRelationIntent` = OnCurve, flowing through the existing accepted-intent channel.
   Test: leg started on quadrant ends with OnCurve persisted; Alt suppresses.
   LANDED (Wave 1), covering circle/arc/ellipse extrema. Note: this deliberately reversed
   the SNAP §9.5 "placement-only" decision — silent detachment is the worse H5-B-class
   failure, and the standard authors attachment. OnCurve is weaker than "at this extremum";
   the axis-alignment intent (quadrant point H/V-aligned to the center) is a named
   FOLLOW-UP, not landed, so a re-solved point can still slide along the curve.
8. **Mock-lane dimension honesty** (A2 subset, M/L, `localSolver.ts` only).
   Bounded, no full solver: (a) editing a committed Radius/Diameter/Distance/Angle re-drives
   geometry through the same measure/rebuild frames SP-1 uses while drawing
   (`liveDimFrames.ts`), falling back to reject-with-hint when the frame can't express it;
   (b) obviously-unsatisfiable geometric adds (H+V+Parallel class: direction constraints on
   axis-locked pairs) are rejected with the standard reject hint. Explicitly NOT a solver;
   e2e gains a dimension-edit spec that the real lane also passes.
9. **Static sketch de-emphasis while editing** (A9, M, viewport §10.5 deferral).
   Implement the recorded `staticSketchRoot`/`activeSketchRoot` split: static sketches drop
   region fills + render at reduced opacity ink while a session is active; Layers filter
   gains the recorded semantics. Integration tests per the deferral note.
10. **Refusal + success feedback** (A10 + A12, S/M).
    Error-severity hints additionally pulse near the action point (reuse hint-chip
    infrastructure, 1.5s, no new toast system); constraint apply success pulses the new
    badge once (single accent flash). Finishing a sketch (Enter/Esc-ladder/button) sets a
    non-sticky "Finished Sketch N — M entities" hint so the silent 3rd-Esc exit becomes
    visible.
11. **Sketch-local coordinates** (A8, S): StatusBar shows `u,v` (display unit) while a
    session is active; world XYZ stays in model mode. Investigate-only note filed for the
    XY-plane basis orientation (do NOT change plane bases — persisted sketches depend on
    them; if anything, a camera-roll change, separately reviewed).

### P2 — paper cuts (bundle into nearest wave)

12. Empty-sketch inspector copy: "Empty sketch — draw to begin" (A11a, S).
13. Rect/centerRect phase-2 hints ("click the opposite corner / a corner") (A11b, S).
14. Fix tangent-arc chip overlap (A11c, S). Root cause found in Wave 1: length-domain
    chips hardcode `w-9` (36px) for mm in `LiveDimField.tsx`, so a 5–6 char radius clips
    its own text — width must follow content (ch-based). Moved to Wave 3 (features scope).
15. `point` tool: idle hint + (if trivial) live-dim x/y frame (A11d, S).
16. Origin pill: hide while a draw gesture is armed (A11e, S).
17. Model-toolbar tooltips gain shortcut hints (A11f, S).

### Deferred-wire (specified, not built here)

- **Per-entity constrained-state coloring** — needs per-entity remaining-DOF (or
  fully-constrained flag) from the solver on both lanes; wire addition to §7.4 solve
  result (additive field). The single highest-value item after P0; blocked on schema.
- **Slot endpoint-tangency constraint kind** (FreeCAD-style) — worker + §7.6.
- **Slot wire round-trip stray points** (+4 DOF) — worker-side wire shape.
- **centerRect symmetric linkage** — needs Symmetric-with-construction-point authoring.
- **W3 project-edges** — already its own wave; adopt Fusion's distinct-color convention
  for projected geometry when it lands.

### Implementation waves (this session)

- **Wave 1 (S items, parallel×2 sonnet):** #3, #4, #7, #11, #12–#17.
- **Wave 2 (viewport, opus + adversarial review):** #1, #2, #9, #16.
- **Wave 3 (interaction, opus + adversarial review):** #5, #6, #10.
- **Wave 4 (sonnet/opus):** #8.
- Gate R2 (tsc + full vitest + targeted sketch e2e + hex grep) per wave, R3 (full e2e) at
  final. Owed user-run at close: Tauri-app smoke of the sketch flows (record in TODO.md).
