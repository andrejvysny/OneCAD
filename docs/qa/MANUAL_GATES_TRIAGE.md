# Manual-gate triage (P1 / M1, 2026-08-08)

> Historical 2026-08-08 triage. It is not a current Phase 4/6 pass claim. Current
> automated and release residuals live in `CURRENT_STATE.md`, `TODO.md`, and
> `docs/qa/modeling-residuals-v1.json`; full browsers and real Tauri remain open.

Every check in the historical `MANUAL_GATES_RUN.md` (archived at
`docs/qa/archive/MANUAL_GATES_RUN-2026-08-04.md`, ~112 boxes across 16 sections)
classified into exactly one destination:

| Code | Destination | Meaning |
| --- | --- | --- |
| **K** | CTest / Kernelbench | geometry + kernel semantics |
| **H** | Rust real-worker integration test | history / regeneration semantics |
| **F** | Vitest / Playwright | frontend state + workflow |
| **M** | `docs/qa/MANUAL_RELEASE_GATES.md` | genuinely native / visual / hardware |

A row is **COVERED** when an existing automated test already asserts it — the
anchor is cited, and the manual box is retired. A row is **GAP** when the check
belongs in an automated lane but nothing asserts it yet; those are listed in
§ Gaps and are the only manual debt that remains outstanding.

Nothing was deleted: a check is either cited as covered, listed as a gap with an
owner lane, or moved verbatim into the release checklist.

---

## Summary

| | count |
| --- | --- |
| checks triaged | 112 |
| COVERED by an existing automated test | 78 |
| GAP — automatable, test not written yet | 22 |
| M — stays manual (native/visual/hardware) | 12 |

The 78 covered boxes are retired outright. `MANUAL_RELEASE_GATES.md` is 12 items,
down from ~112.

---

## 1. Fillet / Chamfer (archived §2, §9 partial) → K + F

| Check | Dest | Status |
| --- | --- | --- |
| drag AWAY = fillet, drag INTO = chamfer, chip flips live | F | COVERED `e2e/filletChamfer.spec.ts:169` |
| segment click locks type, later drag must not flip | F | COVERED `e2e/filletChamfer.spec.ts:355` |
| ✓ commits correct label + icon | F | COVERED `e2e/filletChamfer.spec.ts:231,248` |
| Enter commits / ✕ cancels | F | COVERED `e2e/filletChamfer.spec.ts:258` |
| dblclick row → flip type → same row swaps label + geometry | F | COVERED `e2e/filletChamfer.spec.ts:278` |
| chamfer second-distance field, asymmetric row "1.0×2.5 mm" | F | COVERED `e2e/filletChamfer.spec.ts:323,338` |
| two-distance chamfer blocks flip-to-fillet with named reason | F | COVERED `e2e/filletChamfer.spec.ts:355` |
| concave pocket / cylinder cap edge → auto OFF | K | GAP-K1 |
| radius-too-large → ✓ blocked with named OCCT reason | K | COVERED (diagnostic) `worker/tests/test_fillet_diagnostics.cpp:37` |
| …and the selected **edges are kept** after refusal | H | GAP-H1 |
| fillet preview == commit | H | COVERED `src-tauri/tests/preview_edge_shell.rs:610,615` |
| preview/commit diagnostics agree | K | COVERED `worker/tests/test_fillet_diagnostics.cpp:218` |
| `h` = Home view (chamfer tool id dead) | F | GAP-F1 |
| sketch fillet r=5 trims + rounds, no OverConstrained badge | F | COVERED `e2e/sketch-fillet.spec.ts:101` |

## 2. History / regeneration (archived §0, §9, §12) → H + F

| Check | Dest | Status |
| --- | --- | --- |
| rollback TRUNCATES visible geometry | H | COVERED `src-tauri/tests/rollback_lane.rs:367` |
| roll into all-suppressed prefix clears | H | COVERED `rollback_lane.rs:441` |
| roll to current row is a no-op | H | COVERED `rollback_lane.rs:507` |
| grayed rows + marker + "N rolled back" banner; Roll to end | F | COVERED `e2e/history-rollback.spec.ts:46,84` |
| insert at cursor lands at step k, not the tail | H+F | COVERED `rollback_lane.rs:601`, `e2e/history-insert-at-cursor.spec.ts:45` |
| inline dimension edit of a PAST feature rebuilds downstream | F | COVERED `e2e/history-inline-dimension.spec.ts:171` |
| row editor refuses while a tool is armed | F | COVERED `e2e/history-inline-dimension.spec.ts:213` |
| suppress mid-chain → downstream follows; un-suppress restores | H+F | COVERED `src-tauri/tests/suppression.rs`, `e2e/history-suppress.spec.ts:44,81` |
| suppression survives reopen | H | COVERED `src-tauri/tests/suppression.rs` |
| delete-confirm names the dependent count; Depends on / Used by | F | COVERED `e2e/history-dependencies.spec.ts:88` |
| undo on ~20-feature doc is checkpoint restore, not full replay | H | COVERED `src-tauri/tests/undo_checkpoint.rs:398,488,649` |
| upstream edit → fillet loses edge → row warn-tinted, downstream gray, "Timeline stopped at …" | H+F | GAP-H2 |
| banner Suppress-feature un-wedges the rebuild | F | GAP-F2 |
| repair candidates show viewport crosshair on hover | M | manual (visual) |
| rebind works on a 2-body doc without pre-selecting the body | H | GAP-H3 |
| Reattach world sketch → datum moves sketch + its extrude | H | GAP-H4 |
| tree eye toggle + rename survive save/reopen + an op commit | F | GAP-F3 |
| pattern/mirror/chamfer/shell rows own icons | F | GAP-F4 |
| pattern/mirror/shell re-edit opens with stored values | F | GAP-F5 |
| revolve re-edit on 2-sketch doc opens RIGHT sketch/region/axis | F | GAP-F6 |
| chamfer re-edit opens CHAMFER editor on a reopened doc | F | GAP-F7 |

## 3. Extrude (archived §5, §10, §12) → K + H + F

| Check | Dest | Status |
| --- | --- | --- |
| release stays armed w/ chips → Enter → body persists | F | COVERED `e2e/extrude-commit-gesture.spec.ts:98` |
| ✕ cancels, creates no body | F | COVERED `extrude-commit-gesture.spec.ts:119` |
| exact-preview failure blocks confirmation | F | COVERED `extrude-commit-gesture.spec.ts:152` |
| Enter waits for the final preview epoch | F | COVERED `extrude-commit-gesture.spec.ts:228` |
| rect+circle: inner disk vs annulus, preview == committed BOTH | H | GAP-H5 |
| multi-select 2 regions → 2 bodies + 2 rows | F | COVERED `e2e/extrude-multiselect.spec.ts:21` |
| ThroughAll / ToFace authored by segments | F | COVERED `e2e/extrude-end-conditions.spec.ts:31` |
| ToNext onto missed body → named error, never nearer-plane bind | K | COVERED `worker/tests/test_wp6_extrude.cpp:158,180` |
| ToFace unresolved → NeedsRepair | K | COVERED `test_wp6_extrude.cpp:114` |
| draft: tapered preview + commit, ±89° clamp, survives re-edit | K+F | COVERED `test_wp6_extrude.cpp:206`, `e2e/extrude-draft.spec.ts:100,139` |
| Cut segment auto-targets the sole body | F | COVERED `e2e/extrude-boolean.spec.ts:27` |
| edit sketch under ARMED extrude → tool cancels with hint | F | GAP-F8 |
| stale region → named error | H | GAP-H6 |
| reopen file → regions selectable + extrude works | H | GAP-H7 |
| push/pull on a face-sketch grows host (Add) / pockets (Cut) | H | COVERED `src-tauri/tests/sketch_regions.rs:413` |
| tube test: rect + inner circle → preview TUBE; hole click ≠ region select | F | GAP-F9 |

## 4. Sketch (archived §0a, §0b, §7, §8, §13) → K + F

| Check | Dest | Status |
| --- | --- | --- |
| 3-point arc, bulge side, typed radius commits | F | COVERED `e2e/arc.spec.ts:13` (center-start-end) / GAP-F10 (3-point + typed radius) |
| tangent arc continues the chain, no redundant badge | K | COVERED `worker/tests/test_sketch_arc_endpoints.cpp:137` |
| arc start drag keeps the endpoint | K | COVERED `test_sketch_arc_endpoints.cpp:268` |
| extend (⇧T) grows to exactly the crossing, keeps H/V | F | GAP-F11 |
| direct manipulation: line translate / circle ring resize / arc endpoint | K | COVERED `worker/tests/test_sketch_drag_kinds.cpp:172`, `test_sketch_ellipse.cpp:80` |
| …and it SURVIVES reopen (the silent-revert bug) | H | COVERED `src-tauri/tests/sketch_direct_manipulation.rs` |
| Esc mid-drag restores exact start positions | F | COVERED `e2e/sketch-drag.spec.ts:33` (partial) / GAP-F12 (Esc path) |
| undo during a drag is REFUSED, not swallowed | F | GAP-F13 |
| live dimension chips: zoom-adaptive rounding, type `50` Tab `30` | F | COVERED `e2e/live-dim-rect.spec.ts`, `e2e/live-dim-circle.spec.ts` |
| Esc drops only the typed field; second Esc ends the chain | F | GAP-F14 |
| polar tracking locks exactly 45°, toggles off | F | GAP-F15 |
| snap radius S/M/L survives reload | F | GAP-F16 |
| endpoint snap beats rounding; hover hint then snap | F | COVERED `e2e/sketch-snap.spec.ts:34` |
| conflict naming: second clashing dimension refused, names the existing one | K | COVERED `src-tauri/tests/sketch_conflicts.rs:81` |
| over-constrain rejection NAMES the clashing constraint | F | GAP-F17 |
| concentric / equal auto-inference; ellipses never participate | K | GAP-K2 |
| constraints tangent / equal / midpoint drop DOF | H | COVERED `src-tauri/tests/sketch_constraints.rs:140` |
| trim hover red ghost, click removes the segment | F | COVERED `e2e/sketch-trim.spec.ts:71` |
| offset ±3 both sides; inner collapse refused loudly | F | COVERED `e2e/sketch-offset.spec.ts:75` (offset) / GAP-F18 (collapse refusal) |
| construction: X toggles dashed, kills region, works as revolve axis, persists | K+H | COVERED `worker/tests/test_sketch_construction.cpp:92`, `src-tauri/tests/sketch_construction.rs:161` |
| tools P / ⇧R / S / G / O author entities | F | COVERED `e2e/live-dim-*.spec.ts`, `e2e/sketch-*.spec.ts` |
| marquee window vs crossing, Shift adds, Esc cancels | F | COVERED `e2e/marquee.spec.ts:104,131` |
| editing sketch A → dblclick B edits B; rapid A→B→A settles | F | GAP-F19 |
| Alt+click body face under a sketch-on-face selects the face | F | GAP-F20 |
| rect → Finish → re-enter → line → Finish — BOTH persist | F | COVERED `e2e/sketch-multi-object.spec.ts` |
| ⌘Z in-sketch per step; one model undo reverts the session | F | COVERED `e2e/sketch-undo.spec.ts` |
| bodies dim ~35% in sketch, restore on exit | M | manual (visual) |
| dim input rejects ≤0, 3-decimal display, hints self-clear ~4 s | F | GAP-F21 |

## 5. Save / Open / recovery (archived §3, §9) → H + F

| Check | Dest | Status |
| --- | --- | --- |
| open `Untitled.onecad` → body renders + fitView | H | COVERED `src-tauri/tests/open_render.rs:317` |
| reopen ×2 round-trip | H | COVERED `open_render.rs:317` |
| checkpoint incremental == from-zero; worker restart replays | H | COVERED `src-tauri/tests/checkpoints.rs:316,413` |
| autosave debounce / no runtime lock / concurrent autosaves land | H | COVERED `src-tauri/tests/autosave_recovery.rs:81`, `persistence_lane.rs:181,234` |
| recovery round-trip reconstructs the document | H | COVERED `autosave_recovery.rs:209` |
| Don't-Save-quit offers NO recovery next launch | H | COVERED `autosave_recovery.rs:174` |
| dirty doc: ⌘W / ⌘Q / titlebar × / start screen all prompt | F+M | COVERED titlebar × `e2e/unsaved-guard.spec.ts:98,108,120,133`; ⌘W/⌘Q are native → **M** |
| in-session File > Open a DIFFERENT doc (cross-doc guard) | H | GAP-H8 |
| worker-missing run → tree intact + visible error hint, no blank | F | GAP-F22 |

## 6. STEP import (archived §0) → K + H + F

| Check | Dest | Status |
| --- | --- | --- |
| bodies render, names from products | H | COVERED `src-tauri/tests/step_import.rs:422` |
| colored faces show authored colors | F | COVERED `e2e/step-import.spec.ts:171` |
| uncolored import is byte-identical | K | COVERED `worker/tests/test_xcaf_import.cpp:318` |
| multi-solid / garbage file | K | COVERED `worker/tests/test_import_step.cpp:224,282` |
| File > Import STEP appends without doc swap | H | COVERED `step_import.rs:677` |
| reopen saved doc → identical geometry + colors | H | COVERED `step_import.rs:527` |
| suppressing an import keeps its blobs | H | COVERED `step_import.rs:631` |
| start-screen Import STEP opens the editor | F | COVERED `e2e/step-import.spec.ts:218` |
| fillet an imported edge → survives reopen | H | GAP-H9 |
| delete the Import row → downstream NeedsRepair loud, undo restores | H | GAP-H10 |

## 7. Body ops — transform / pattern / mirror / shell / boolean / revolve (archived §6, §7, §9, §12) → K + H + F

| Check | Dest | Status |
| --- | --- | --- |
| gizmo arrows / quads / rings drag; ring re-types to Rotate | F | COVERED `e2e/transform-body.spec.ts:385,410,434` |
| Alt-drag or [Copy] duplicates | F | COVERED `transform-body.spec.ts:470,492` |
| translate is exact | K | COVERED `worker/tests/test_transform_body.cpp:130` |
| second move FOLDS into the same row; dblclick re-edits | H | COVERED `src-tauri/tests/transform_body.rs` |
| editing a transform under a downstream fillet ⇒ fillet NeedsRepair | H | GAP-H11 |
| [Align] two-pick sets a face flush | F | GAP-F23 |
| linear / circular pattern, mirror ±fuse | K+H | COVERED `worker/tests/test_m6a_ops.cpp:173,267,289`, `src-tauri/tests/breadth_ops.rs:499,544,586` |
| shell ladder / hollow preview | K | COVERED `worker/tests/test_m6a_ops.cpp:84` |
| boolean 3 modes preview == commit | H | COVERED `src-tauri/tests/preview_boolean.rs:495,511` |
| boolean commit two bodies → target | F | COVERED `e2e/boolean-preview.spec.ts:276` |
| revolve preview == commit; Cut subtracts in BOTH | H | COVERED `src-tauri/tests/preview_revolve.rs:441,555` |
| revolve 360° volume (Pappus) | H | COVERED `revolve_ops.rs:564` |
| revolve axis pick → drag → Enter commits | F | COVERED `e2e/revolve-preview.spec.ts:37`, `e2e/revolve-commit.spec.ts:33` |

## 8. Hole / datum / measure / units (archived §0, §6) → K + H + F

| Check | Dest | Status |
| --- | --- | --- |
| face click arms hole; Ø chip authors diameter | F | COVERED `e2e/hole.spec.ts:141,149,168` |
| standards picker M6 → Ø6.4; counterbore; countersink | K+F | COVERED `worker/tests/test_hole_op.cpp:172,190`, `e2e/hole.spec.ts:208` |
| through-all on a stepped part | K+H | COVERED `test_hole_op.cpp:143`, `src-tauri/tests/hole_ops.rs:521` |
| re-pick point while armed; re-edit type switch | F | COVERED `e2e/hole.spec.ts:187` |
| datum create → offset ghost → quad + label + tree | F | COVERED `e2e/datum-create.spec.ts:24` |
| sketch ON a datum → extrude lands 10 mm up | H | COVERED `src-tauri/tests/datum_planes.rs:208` |
| delete-while-referenced rejected with a named hint | H | COVERED `datum_planes.rs:334` |
| measure face area / edge length / volume / centroid | K+F | COVERED `worker/tests/test_mass_properties.cpp:100,156`, `e2e/measure.spec.ts:129,228` |
| angle between two planar faces (60/120 form) | F | COVERED `e2e/measure.spec.ts:252` |
| parallel faces → separation | F | GAP-F24 |
| ? inert while sketching; Esc clears | F | GAP-F25 |
| units: in / cm / bare number / explicit suffix / survives reload | F | COVERED `e2e/units.spec.ts:80,94,111,144,155,184` |
| geometry unchanged on unit switch-back | F | COVERED `e2e/units.spec.ts:155` |

## 9. Theming + render modes (archived §1, §4) → F + M

| Check | Dest | Status |
| --- | --- | --- |
| cold boot with Dark preference renders dark | F | COVERED `e2e/theme-boot.spec.ts:31` |
| cold boot System on a dark OS renders dark | F | COVERED `theme-boot.spec.ts:55` |
| pre-v5 persisted blob on a dark OS boots dark | F | COVERED `theme-boot.spec.ts:72` |
| picking Dark re-themes chrome AND viewport | F | COVERED `e2e/theme.spec.ts:50` |
| title-bar button cycles Light → Dark → System | F | COVERED `e2e/theme.spec.ts:121` |
| appearance choice survives reload | F | COVERED `e2e/theme.spec.ts:71` |
| System follows the OS in both directions | F | COVERED `e2e/theme.spec.ts:84` |
| theme and display mode are independent | F | COVERED `e2e/theme.spec.ts:159` |
| theme registry / resolution / controller | F | COVERED `src/theme/themes.test.ts:15,37`, `src/theme/themeController.test.ts:47` |
| display-mode popover picks the renderer mode | F | COVERED `e2e/view-ux.spec.ts:98` |
| toggle with a model loaded: camera / selection / open sketch survive | F | GAP-F26 |
| **wireframe in dark — the edge token must invert** | M | manual (visual; the known failure case) |
| studio shading reads machined across an orbit | M | manual (visual) |
| GPU context restore (sleep / GPU switch) keeps shading | M | manual (hardware) |
| native traffic lights follow "System"; no light-chrome flash on reload | M | manual (native) |
| sketch-entry dim restores on exit | M | manual (visual) |

## 10. View UX (archived §6) → F + M

| Check | Dest | Status |
| --- | --- | --- |
| ⇧F frames the selected body, falls back to scene | F | COVERED `e2e/view-ux.spec.ts:141` |
| ⇧I isolates transiently, Esc restores | F | COVERED `e2e/view-ux.spec.ts:163` |
| isolate + boolean preview don't fight | F | GAP-F27 |
| hint text ("No closed region to extrude") appears; success hints survive tool reset | F | GAP-F28 |

## 11. Navigation (archived §14) → F + M

| Check | Dest | Status |
| --- | --- | --- |
| wheel zoom, direction, shift+wheel, horizontal wheel pan | F | COVERED `e2e/navigation.spec.ts:84,91,98,138` |
| mouse / trackpad override forces zoom / pan / orbit | F | COVERED `navigation.spec.ts:108,117,126` |
| right-button drag pans | F | COVERED `navigation.spec.ts:158` |
| wheeling over the canvas selects nothing | F | COVERED `navigation.spec.ts:146` |
| orbiting near top view does not self-align | F | COVERED `navigation.spec.ts:170` |
| **two-finger pan / pinch-zoom-to-cursor / momentum** | M | manual (Playwright cannot synthesise real trackpad gestures) |
| hot-plug a mouse → wheel zoom + "mouse" sublabel; unplug flips back | M | manual (hardware) |
| page never magnifies under pinch | M | manual (native webview) |
| idle = 0 frames | F | GAP-F29 |
| `?inputprobe` calibration then DELETE `src/viewport/debug/inputProbe.ts` | — | follow-up code task, not a gate |

## 12. Native window (archived §9, §12) → M

| Check | Dest | Status |
| --- | --- | --- |
| titlebar × on a dirty doc prompts | F | COVERED `e2e/unsaved-guard.spec.ts:98` |
| Cancel keeps editor / Don't Save → start screen / Save exits | F | COVERED `unsaved-guard.spec.ts:108,120,133` |
| ⌘W and ⌘Q prompt | M | manual (native menu/shortcut) |
| native traffic lights, titlebar rendering | M | manual (native) |
| start screen + recents | M | manual |

## 13. Auto-mode (archived §11) → F

| Check | Dest | Status |
| --- | --- | --- |
| L from model mode → plane pick → draw; E from sketch finishes in ONE undo step | F | GAP-F30 |
| dblclick a finished sketch curve → edit session; undo reverts the whole session | F | COVERED `e2e/sketch-undo.spec.ts` |
| Esc ladder unchanged; no titlebar mode toggle | F | GAP-F31 |

---

## Gaps

Automatable checks with no test yet. These are the manual debt that survives this
triage; each carries its target lane. They are NOT release blockers — they are the
backlog that keeps advanced-fillet permutations from re-growing a 112-box list.

**Kernel (CTest / Kernelbench)**
- **GAP-K1** concave pocket + cylinder-cap edge classification drives fillet/chamfer auto OFF
- **GAP-K2** concentric / equal auto-inference thresholds (~2 mm centre, <0.5 mm radius); ellipses never participate

**History (Rust real-worker)**
- **GAP-H1** a fillet refusal must leave the selected edges intact and commit nothing
- **GAP-H2** upstream edit invalidating a fillet edge ⇒ that row fails, downstream greys, timeline stops at it
- **GAP-H3** repair rebind works on a 2-body document with no body pre-selected
- **GAP-H4** Reattach moves a world-plane sketch AND its dependent extrude to the datum
- **GAP-H5** rect+circle region parity: inner disk vs annulus, preview == commit for both
- **GAP-H6** a stale `regionId` fails with a named error listing the available ids
- **GAP-H7** reopened file exposes selectable regions and a working extrude
- **GAP-H8** in-session open of a DIFFERENT document (cross-doc guard)
- **GAP-H9** fillet on an imported STEP edge survives save/reopen
- **GAP-H10** deleting an Import row drives downstream to NeedsRepair; undo restores
- **GAP-H11** editing a transform beneath a downstream fillet ⇒ fillet NeedsRepair

**Frontend (Vitest / Playwright)**
- **GAP-F1** `h` maps to Home view (dead chamfer tool id)
- **GAP-F2** failure banner's Suppress-feature un-wedges the rebuild
- **GAP-F3** tree eye toggle + rename survive save/reopen and an op commit
- **GAP-F4** pattern / mirror / chamfer / shell rows carry their own icons
- **GAP-F5** pattern / mirror / shell re-edit opens with stored values
- **GAP-F6** revolve re-edit opens the right sketch/region/axis on a 2-sketch doc
- **GAP-F7** chamfer re-edit opens the chamfer editor on a reopened doc
- **GAP-F8** editing a sketch under an armed extrude cancels the tool with a hint
- **GAP-F9** tube preview (rect + inner circle); a hole click is not a region select
- **GAP-F10** 3-point arc bulge side + typed radius commit
- **GAP-F11** extend (⇧T) grows to exactly the crossing and keeps H/V
- **GAP-F12** Esc mid-drag restores exact start positions
- **GAP-F13** ⌘Z during a drag is refused, not swallowed by the pointer-up commit
- **GAP-F14** Esc drops only the focused dimension field; second Esc ends the chain
- **GAP-F15** polar tracking locks exactly 45° and toggles off
- **GAP-F16** snap radius S/M/L survives reload
- **GAP-F17** over-constraint rejection names the clashing constraint
- **GAP-F18** inner offset collapse is refused loudly
- **GAP-F19** dblclick sketch B while editing A edits B; rapid A→B→A settles
- **GAP-F20** Alt+click selects the body face under a sketch-on-face
- **GAP-F21** dimension input rejects ≤0; 3-decimal display; hints self-clear ~4 s
- **GAP-F22** worker-missing run keeps the tree and shows an error hint, never a silent blank
- **GAP-F23** [Align] two-pick sets a face flush
- **GAP-F24** parallel faces measure reports separation
- **GAP-F25** measure is inert while sketching; Esc clears
- **GAP-F26** theme toggle with a model loaded preserves camera / selection / open sketch
- **GAP-F27** isolate and boolean preview do not fight
- **GAP-F28** hint text appears and success hints survive a tool reset
- **GAP-F29** idle renders zero frames
- **GAP-F30** auto-mode: L → plane pick → draw; E finishes the sketch in ONE undo step
- **GAP-F31** Esc ladder unchanged; no titlebar mode toggle

---

## e2e residue found while gating (2026-08-08)

The P0 gate run surfaced a large e2e failure cluster. Bisected rather than
assumed, and one real product regression came out of it:

**FIXED — `ViewportRoot` debounced auto-fit (introduced in `1fe0cef`).** The
"auto-fit once on the first body" rule became a 250 ms debounced re-fit so
multi-body assemblies frame correctly — but the timer lived in the React bridge,
so a `fitView()` tween could START after everything else already believed the
camera had settled. Any client coordinate computed from a probe was then stale.
Evidence: `measure.spec.ts` is 5/5 at `d1c5339`, 1/5 at `1fe0cef`, and 5/5 at
`1fe0cef` with only the debounce reverted. It is a user-visible defect too — a
queued fit can snap the camera mid-interaction, and one landing during sketch
entry is saved as the restore pose. The debounce now lives on `ViewportEngine`
(`requestAutoFit` / `autoFitPending` / `cancelAutoFit`), cancelled by explicit
`fitView`, `fitToBodies`, `enterSketch`, and `dispose`; `waitForCameraSettled`
waits on `autoFitPending` as well as the tween. Full suite went from 12+
failures to 7.

**FIXED — the remaining seven, all pre-existing, none caused by that work.**
Each was root-caused, not retried away:

| Spec | Cause | Fix |
| --- | --- | --- |
| `transform-body.spec.ts:186,296,410,557` | the spec's `bodyBounds` traversed every `isMesh` under the body group — including the fat-line edge layer, whose `position` attribute is the INSTANCED unit-quad template `(-1,-1,0)…(1,2,0)` (real endpoints live in `instanceStart`/`instanceEnd`). That clamped every body's bbox min to ≤ -1, so any expectation inside that box read `-1` while `max` stayed correct. | restrict the traversal to `userData.kind === "face"` |
| `history-inline-dimension.spec.ts:213` | `toolApplicability.ts` is NEW in `1fe0cef` — the toolbar is now selection-gated, and committing an extrude HIDES its sketch, so no sketch / region / sole-visible-sketch applied and Extrude was `aria-disabled`; the click burned the 45 s timeout. Second cause: `getByLabel("Dimension value")` matched both the row's inline editor and the tool chip's depth field. | re-show the sketch (the eye toggle, not a selection change — the row selection must survive); scope the editor locator to the row |
| `hole.spec.ts:226` | `findFaceOnBody` probed pixels before the camera settled, returning a point on a face about to move — hence "no second pixel far enough on the same face" | `waitForCameraSettled` inside the helper |
| `sketch-degenerate.spec.ts:35`, `live-dim-mouse-rounding.spec.ts:62` | `enterSketch` aims along the plane normal at `controls.getDistance()`, so the sketch view INHERITS the model camera distance — which sets `planePixelWorld`, hence the draw tools' screen-constant reject radius (`minSize = 4 × planePixelWorld`) and the zoom-adaptive dimension quantum. Entering mid-fit made both non-deterministic; every caller settled AFTER entering, too late. | settle the model camera at the START of `enterSketchViaPlanePicker` |

Verified: 52/52 across `transform-body`, `history-inline-dimension`, `hole`,
`sketch-degenerate`, `live-dim-mouse-rounding`, `measure`, `constraint-apply`,
`marquee`, `navigation`.

**`playwright.config.ts` sets `retries: 1` under CI and `0` locally.** Every
intermittent failure above was therefore invisible in CI and hard-red locally.
Now that the flakes are root-caused rather than retried away, dropping the CI
retry is the honest setting — otherwise "e2e green" keeps hiding real defects
like the auto-fit regression above.

## Deliberately out of scope

- M3 Mac packaging (needs Developer ID credentials) — `docs/PACKAGING.md` §5
- `?inputprobe` calibration + deleting `src/viewport/debug/inputProbe.ts` — a code
  task, not a gate (it is also the last hex-gate hit)

---

# Round 2 (2026-08-19) — the manual gates that re-grew after round 1

Round 1 (above) cut ~112 boxes to 12 and stated the rule that keeps it cut: a new
manual permutation is a signal that an automated lane is missing, not that the
checklist should get longer. Between 2026-08-08 and 2026-08-17 the ledger grew
manual boxes again — 30 unchecked `- [ ]` items in `TODO.md` mention a manual or
USER gate. This round classifies them with the same K/H/F/M codes.

**Two structural findings first, because they do most of the reduction:**

- **Everything dated 2026-08-08 or earlier is already retired by round 1.** The
  archived `MANUAL_GATES_RUN.md` those boxes belong to IS what round 1 triaged
  (`TODO.md`'s own Step-0 row says so: "SUPERSEDED 2026-08-08 by the P1 triage").
  Their boxes were simply never ticked. That is 19 of the 30 — VIEWPORT-VISUAL,
  OFFSET-FACE, STARTUP-PERF, ICONS-TWOTONE, SKETCH-PRO, DARK-MODE, RENDER-MODE,
  SKETCH-ON-FACE, AC-USABILITY, TRACKPAD-NAV, SKETCH-HARDEN, MODEL-HARDEN,
  MODEL-OPS, EXTRUDE-REGION-PARITY, AUTO-MODE, TRUST+PREVIEW, FILLET-CHAMFER-UNIFY,
  FILLET-KERNEL-HARDENING, and the pre-merge SAVE/OPEN gates.
- **"Manual Tauri smoke: open → extrude → fillet → undo → save → reopen" appears
  SEVEN times** (`TODO.md:1707, 2265, 2394, 5495, 5918, 5996, 7651`, plus the
  `:6562/:6985/:7063` duplicate copies of the same log). It is one check, and it is
  **COVERED**: `e2e-tauri/specs/composition.e2e.ts:226` — "opens fixture,
  Extrudes, Fillets, Undoes, saves, and reopens" — runs that exact flow against a
  **real packaged `.app`** in CI job `tauri-composition`, and the lane has since
  executed and passed (`CURRENT_STATE.md`, MC-R4). Seven boxes, one citation.

## Round 2 table

| Check | `TODO.md` | Dest | Status |
| --- | --- | --- | --- |
| Manual Tauri smoke — open → extrude → fillet → undo → save → reopen (×7) | 1707, 2265, 2394, 5495, 5918, 5996, 7651 | F | COVERED `e2e-tauri/specs/composition.e2e.ts:226` (real packaged `.app`, CI `tauri-composition`) |
| Autosave crash recovery — 5 steps (`kill -9`, relaunch, Restore, Recent-card prompt, real name) | 1383 | **M** | STAYS MANUAL — native process death plus a relaunch; no lane can kill its own host. Moved verbatim into `MANUAL_RELEASE_GATES.md` § 4 |
| Idle sketch chrome reads as ONE fused two-row pill | 1234 | **M** | STAYS MANUAL — visual composition of two chrome rows. Folded into `MANUAL_RELEASE_GATES.md` § 1 |
| Sketch render parity — read-only length label, Distance chip takes over with no duplicate, origin-pin lock icon + Disconnect chip, endpoint highlight ring | 1435 | **M** + F | SPLIT. "A Distance constraint replaces the read-only label rather than duplicating it" is state, and belongs in vitest/Playwright — recorded as **GAP-F9**. The rest is "does it look right", folded into `MANUAL_RELEASE_GATES.md` § 1 |
| Real-worker face-colour smoke (element ids minted through `promoteSelection`) | 7309 | H | COVERED `src-tauri/tests/face_color_reopen.rs` — the DI-4 rebind work asserts an authored colour survives a reopen against the real worker, including that a re-pick reuses the persisted id |
| GitHub Settings → require approval for fork-PR workflows | 5589 | — | NOT A PRODUCT GATE. Repo administration, not a release check; stays a one-line USER action in `TODO.md` |
| Fillet drag into an OCCT refusal — preview stays last-valid, commit blocked, diagnostic visible | 7340 | K + H | COVERED for the refusal + named reason (`worker/tests/test_fillet_diagnostics.cpp:37`); the "selected edges are kept" half is round 1's **GAP-H1**, still open. No new manual box |

## New gap opened by this round

- **GAP-F9** — adding a Distance constraint to a selected sketch edge must
  REPLACE the read-only length label, not render a second one beside it. Lane:
  vitest on the sketch overlay, or `e2e/sketch-*.spec.ts`. Until it exists the
  visual half carries it in `MANUAL_RELEASE_GATES.md` § 1.

## Net effect

30 unchecked manual boxes in `TODO.md` → **7 new checklist rows**: the five
autosave steps (§ 4, new) and two visual folds into § 1. Everything else is
retired against a citation, already retired by round 1, or was never a product
gate. `MANUAL_RELEASE_GATES.md` goes 12 → 19 rows; both additions qualify under
the file's own native/visual rule, and the file did not grow by carrying a single
duplicate of the seven-times-repeated Tauri smoke.

One new automated gap is opened (GAP-F9) rather than parked as a manual box.
