# Consolidated USER Manual Gate Run (updated 2026-08-04)

## 0. HISTORY-HARDEN (2026-08-04 session) — parametric history: change past decisions
- [ ] **Inline dimension edit (main priority)**: build sketch → extrude → fillet;
      select body → feature chip → history rows now show their value ALWAYS;
      click the extrude's value, type a new depth, Enter → body + fillet rebuild;
      row editor refuses while a tool is armed; sketch row shows a Dimensions
      section — edit a dimension value there without entering sketch mode →
      downstream extrude follows.
- [ ] **Rollback**: right-click a mid row → "Roll to here" → GEOMETRY TRUNCATES
      on screen (this was silently broken — any roll was a visual no-op);
      grayed italic rows + marker line + "N operations rolled back [Roll to end]"
      banner appear; roll to end restores everything.
- [ ] **Insert at cursor**: while rolled back, draw + extrude a new feature →
      it lands AT the cursor (not the tail), hint says "Inserted at step k of n";
      roll to end → later features rebuild on top of it. (Previously: the new
      feature became a permanently inert draft with zero signal.)
- [ ] **Failure visibility + recovery**: edit an upstream dimension so a fillet
      loses its edge → fillet row warn-tinted, downstream rows grayed,
      "Timeline stopped at …" banner; banner's Suppress-feature un-wedges the
      rebuild; repair panel candidates show a viewport crosshair on hover;
      rebind works on a 2-body doc WITHOUT pre-selecting the body.
- [ ] **Reattach**: right-click a world-plane sketch → Reattach → pick a datum →
      sketch + its extrude MOVE to the datum (was: silently inert).
- [ ] **Dependencies**: delete-confirm on a feature with dependents names the
      count; inspector shows "Depends on / Used by".
- [ ] **Undo speed sanity**: on a ~20-feature doc, save, edit the LAST feature,
      undo → revert is near-instant (checkpoint restore, not full replay).

## 0b. SKETCH-PRO SP-2/4/5 (2026-08-05) — pro tools + direct manipulation
- [ ] **3-point arc (⇧A)**: click start, click end, then move — the arc follows the
      cursor through the third point; clicking BELOW the chord gives the arc that
      bulges the other way (not its mirror). Type a radius at the third phase →
      that exact radius commits with a Radius badge.
- [ ] **Tangent arc**: draw a line segment, then either DRAG from the chain end or
      press `A` → the preview is an arc leaving tangent to that segment; click
      commits and the NEXT segment continues tangentially. No over-constrained
      badge appears (a weld + entity Tangent would be redundant — it is suppressed).
- [ ] **Extend (⇧T)**: draw a short segment that stops short of two others; hover
      near one end → a ghost shows the piece it would ADD; click → it grows to
      exactly that crossing and keeps its Horizontal/Vertical.
- [ ] **Direct manipulation**: grab a LINE's middle → the whole line translates
      (cursor shows a move form); grab a circle's RING → it resizes about its
      centre (ew-resize form); drag an ARC's endpoint → the sweep reshapes and the
      radius follows. Esc mid-drag puts each back exactly where it started.
      Re-open the document: the resized/reshaped geometry is still there (this is
      the silent-revert bug — a radius that only reached the worker used to snap
      back on the next edit).
- [ ] **Polar tracking**: while drawing, approach 45° — the cursor locks onto a
      dashed ray and the committed angle is exactly 45°, not a rounded 45.0-ish.
      Also snaps parallel/perpendicular to the previous segment. Toggle it off in
      the snap popover and the lock disappears.
- [ ] **Snap radius (S/M/L)**: set L → picks grab from noticeably further away;
      set S → you must be close. Survives a reload.
- [ ] **Undo during a drag**: start dragging a point, and while still holding,
      press ⌘Z → the undo is REFUSED rather than silently thrown away by the
      pointer-up commit.

## 0a. SKETCH-PRO SP-0/SP-1 (2026-08-04 session) — live dimensions + conflict UX
- [ ] **Live dimension chips (Shapr3D-style)**: arm Line, click an anchor, move —
      length + angle chips track the cursor; committed length lands on a round
      number (zoom-adaptive: zoom in → finer step, verify at two zoom levels);
      type `50` Tab `30` Enter mid-gesture → exact 50mm @ 30° segment commits, a
      Distance badge appears and its value chip re-edits; Esc while a chip is
      focused drops only the typed field (gesture survives), second Esc ends the
      chain; polygon: digits at idle set sides, digits after the center click
      type the radius. Alt suppresses rounding; endpoint snap beats rounding.
      "Dimension rounding" + "Live dimensions" toggles live in the snap popover.
- [ ] **Conflict naming (now also on mock lane)**: dimension a line 50, then
      dimension the same points 120 → second dimension refused, entity stays,
      hint names the EXISTING Distance it clashes with, DOF unchanged.
- [ ] **Concentric/Equal auto-inference (legacy parity)**: draw a circle whose
      center lands within ~2mm of an existing circle's center → Concentric badge
      auto-appears; near-equal radius (<0.5mm delta) → Equal badge. Ellipses
      never participate.

## 0. NEW FEATURES batch (2026-08-02/03 autonomous run) — run FIRST, freshest code
- [ ] **STEP import**: Start Screen "Import STEP…" picks a real .step (vendor
      file if you have one) → bodies render, names from products, colored faces
      show authored colors; File > Import STEP into an open doc appends without
      doc swap; reopen the saved doc → identical geometry + colors; fillet an
      imported edge → survives reopen; delete the Import row → downstream
      NeedsRepair loud, undo restores.
      NOTE (2026-08-03 WP-FIX W1): colored faces were INVISIBLE on the real lane
      until the VF-B1 mask fix — if bodies with authored colors render at all,
      the fix holds; a colored file rendering blank/missing bodies = regression.
- [ ] **Transform (t)**: select body → t → gizmo (arrows/quads/rings) drags with
      snap (Shift fine); chip typing; second move FOLDS into the same row;
      dblclick row re-edits; Alt-drag or [Copy] duplicates; multi-select moves
      together; [Align] two-pick sets a face flush on another, gizmo nudges
      after; editing a transform under a downstream fillet ⇒ fillet NeedsRepair
      (repair flow rebinds), undo exact.
- [ ] **Units**: display popover Units row → in → chips/labels/status bar
      convert, typing bare 1 = 1 in, geometry unchanged on switch-back, pref
      survives reload.
- [ ] **Measure+**: ? → click a body → volume/area/centroid card; two planar
      faces → angle 60/120 form; parallel faces → separation.
- [ ] **Draft**: extrude armed → [Draft] 10° → tapered preview + commit;
      re-edit.
- [ ] **Chamfer d2**: chamfer armed → set d2 → asymmetric preview, row
      "1.0×2.5 mm"; flip-to-fillet blocked with named reason until d2 cleared.
- [ ] **Sketch fillet (F in sketch) / offset (⇧O)**: L-corner fillet r=5 trims
      + rounds, no OverConstrained badge; rectangle offset ±3 both sides, inner
      collapse refused loudly.
- [ ] **Hole (⇧H)**: face click → standards picker M6 close → Ø6.4 preview at
      point; counterbore SHCS preset; countersink 90°; re-pick point while
      armed; re-edit type switch; through-all on stepped part.


One top-to-bottom `bun run tauri dev` session covering every open manual gate in
TODO.md. Ordered newest → oldest; later WPs supersede a few older steps (annotated).
Tick the matching TODO.md line as each block passes. Capture failures as:
`GATE <TODO line> / step — expected vs observed` (+ `logs/dev.jsonl` excerpt).

Setup: `scripts/build-worker.sh Release` → `bun run tauri dev`. For the DARK-MODE
unresolved repro use a COLD start (no Vite HMR reuse).

## 1. DARK-MODE — unresolved repro FIRST (TODO :29) then gate (:30)
- [ ] Cold start, switch to Dark. If viewport stays light: capture from webview
      devtools — `document.documentElement.dataset.theme`,
      `getComputedStyle(document.documentElement).getPropertyValue('--color-canvas')`,
      `__vpEngine.debugSnapshot()` (needs `?vpdebug`). This closes or confirms the
      HMR-artifact theory.
- [ ] Title-bar appearance button cycles Light → Dark → System; viewport follows
      each step.
- [ ] Toggle themes with model loaded: camera pose, selection, OPEN sketch session
      all survive (no remount).
- [ ] Toggle while sketching: dimmed body + sketch overlay + snap indicator recolor.
- [ ] Toggle with live extrude/revolve preview: Cut tint preserved.
- [ ] All three render modes in dark — wireframe is the failure case (edge token
      must invert).
- [ ] "System" + OS appearance change: native traffic lights follow; reload shows
      no light-chrome flash.

## 2. FILLET-CHAMFER-UNIFY (TODO :12)
- [ ] Box → pick edge → F → drag AWAY = rounded preview, chip "Fillet"; drag INTO =
      bevel, chip flips "Chamfer" live.
- [ ] Segment click locks type; later opposite drag must NOT flip.
- [ ] ✓ commits correct label + icon.
- [ ] Concave pocket edge → auto OFF (chips control type). Cylinder cap edge → same.
- [ ] Dblclick committed row → segments show committed type → flip → Enter → SAME
      row swaps label + geometry changes; ⌘Z restores type AND geometry.
- [ ] Radius-too-large → ✓ blocked with named OCCT reason, edges kept.
- [ ] `h` = Home view now (chamfer tool id dead).

## 3. SAVE/OPEN-RENDER (TODO :24)
- [ ] Fresh boot → open `Untitled.onecad` → body renders + fitView.
- [ ] In-session File > Open a DIFFERENT doc (cross-doc guard).
- [ ] Reopen ×2 round-trip.
- [ ] Optional: worker-missing run → tree intact + visible error hint, no silent
      blank.

## 4. RENDER-MODE / studio shading (TODO :35, :370)
- [ ] Shaded / Shaded+edges / Wireframe all draw; popover checkmark tracks.
- [ ] Studio shading reads machined across an orbit; no flat/blown faces top-down.
- [ ] Display mode survives reload; sketch-entry dim restores on exit.
- [ ] Context restore (sleep/GPU switch) keeps shading.

## 5. SKETCH-ON-FACE + HOST-BOOLEAN (TODO :48)
- [ ] Tilted-face sketch + extrude lands flush.
- [ ] Snap + dimension to projected corner; L-body coplanar projection appears.
- [ ] Dblclick face re-enters its sketch; non-planar face refuses with hint.
- [ ] Push/pull: extrude on face-sketch grows host outward (Add) / pockets inward
      (Cut) with live chip flip — NO stray body.

## 6. MODELING-REACH (TODO :349-:354)
- [ ] Datum: D → XY quad → offset 10 ghost → ✓ → quad+label+tree; dblclick → sketch
      ON it → extrude lands 10 mm up; save/reopen keeps; delete-while-referenced
      rejected (named hint); undo removes fresh datum.
- [ ] Measure: ? → face = exact area; edge = length + center↔center + deltas; Esc
      clears; ? inert while sketching.
- [ ] Units: "2.5 cm" in a dimension chip → 25 mm; "1 in" → 25.4.
- [ ] Slot drag: caps stay welded to wall ends (tangency as-drawn = expected V1).
- [ ] ⇧F frames selected body; ⇧I isolates, Esc restores; isolate + boolean preview
      don't fight. (Display-button CYCLING at :353 superseded — it's a popover now.)
- [ ] Hints: "No closed region to extrude" visible; success hints survive tool reset.

## 7. SKETCH-POWER (TODO :306-:312)
- [ ] Editing sketch A → tree dblclick sketch B → controller EDITS B (draw lands in
      B); rapid A→B→A settles; exit restores camera projection.
- [ ] Alt+click body face under a sketch-on-face → face selected; without Alt the
      fill wins.
- [ ] Construction: X toggles chrome + dashed; construction edge kills its region in
      extrude pick; dashed centerline works as revolve axis; save/reopen keeps flags.
- [ ] Tools: P point (snap→coincident), ⇧R center-rect, S slot, G polygon (digits
      3-9, vertex drag stays regular), O ellipse (3 clicks rotated → extrude;
      trim-click deletes whole).
- [ ] Constraints: line+circle → Tangent (DOF drops); two lines → Equal;
      center+line → Midpoint; re-apply Tangent on auto-constrained pair → named
      clash rejection.
- [ ] Marquee: rightward solid = fully-inside; leftward dashed = touch; Shift adds;
      Esc cancels; LMB orbit intact after.
- [ ] History rows: pattern/mirror/chamfer/shell own icons.

## 8. SKETCH-MULTI-OBJECT (TODO :276)
- [ ] Rect → Finish → re-enter → line → Finish → re-enter — BOTH persist.

## 9. TRUST + PREVIEW (TODO :262, :272)
- [ ] Suppress mid-chain feature → downstream follows, geometry changes; un-suppress
      restores; separately-suppressed downstream stays suppressed; reopen keeps it.
- [ ] Tree eye toggle + rename survive save/reopen + an op commit.
- [ ] Revolve re-edit on 2-sketch doc opens RIGHT sketch/region/axis.
- [ ] Dirty doc: ⌘W/⌘Q/titlebar ×/start screen all prompt; Don't-Save-quit does NOT
      offer recovery next launch.
- [ ] Fillet-to-refusal blocks ✓ with named reason; shell preview hollow; boolean 3
      modes preview == commit; revolve Cut subtracts in preview AND commit.
- [ ] Pattern/mirror/shell re-edit opens with stored values on real lane.

## 10. EXTRUDE-REGION-PARITY + COMMIT-FIX (TODO :236, :130, :137)
- [ ] Rect+circle: inner disk vs annulus — preview == committed body BOTH.
- [ ] Edit sketch under ARMED extrude → tool cancels with hint; stale region →
      named error.
- [ ] Reopen file → regions selectable + extrude works.
- [ ] ToNext onto missed body → named error (never nearer-plane bind).
- [ ] Chamfer re-edit opens CHAMFER editor on reopened doc.
- [ ] Fresh doc → sketch → extrude ✓ creates body; Esc-exit then extrude works;
      reopen old broken doc → ✓ works, then delete legacy stacked errored rows.

## 11. AUTO-MODE (TODO :244)
- [ ] No titlebar mode toggle; L from model mode → plane pick → draw; E from sketch
      finishes (ONE undo step) → region → drag; Esc ladder unchanged; dblclick
      finished sketch curve → edit session; undo reverts whole session in one step.

## 12. MODEL-HARDEN / MODEL-OPS / AC-USABILITY (TODO :215, :224, :183)
- [ ] Extrude release stays armed w/ chips → Enter → body persists after tool close
      AND save/reopen; ⌘Z once = extrude, twice = sketch.
- [ ] Multi-select 2 regions → 2 bodies + 2 rows; Cut subtracts; revolve 360°.
- [ ] Tube test: rect + inner circle → preview TUBE; hole click ≠ region select.
- [ ] Through all / To next / To face / two-direction / draft each survive dblclick
      re-edit. (Step "Chamfer via H key" at :224 superseded — F unified tool, `h` =
      Home.)
- [ ] Face sketch → draw → extrude → save/reopen identical (overlaps §5).
- [ ] AC sweep (:183): constraints via toolbar+chips, dimensions incl. angle,
      delete entity+constraint, trim ghost, mirror, re-enter + drag/delete, undo.

## 13. SKETCH-HARDEN (TODO :203)
- [ ] Trim hover shows red doomed ghost; ⌘Z in-sketch per step; after Finish ONE
      model undo step reverts session.
- [ ] Over-constrain → reject NAMES clashing constraint; conflicting sketch rows/
      badges tint red on re-enter.
- [ ] Bodies dim ~35% in sketch, restore on exit; crosshair cursors; line chain
      Enter-end + click-first-close; dim input rejects ≤0; 3-decimal display;
      errors red, hints self-clear ~4 s.

## 14. TRACKPAD-NAV (TODO :184) — needs trackpad + mouse hardware
- [ ] Two-finger pans (incl. diagonal), no rubber-band; shift+two-finger orbits,
      shift release mid-momentum ≠ pan flip; pinch zooms to cursor, page never
      magnifies; hot-plug mouse → wheel zoom + "mouse" sublabel, unplug flips back;
      overrides pin; orbit-while-armed OK but never during a value drag; idle = 0
      frames.
- [ ] THEN `?inputprobe` → `__inputProbeDump()` → calibrate navInput constants →
      DELETE `src/viewport/debug/inputProbe.ts` + ViewportRoot mount (clears the
      last hex-gate hit). (Calibration+delete = a follow-up code task; file findings.)

## Not in this run
- M3 Mac packaging (TODO :101) — release-time, needs Developer ID credentials
  (docs/PACKAGING.md §5).
- DEV-OBSERVABILITY gate (:19) — owned by its concurrent session.
- Stale rows :89 / :113 — closed by later gates, TODO hygiene only.
