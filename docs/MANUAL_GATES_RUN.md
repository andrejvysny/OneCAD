# Consolidated USER Manual Gate Run (2026-08-02)

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
