# Manual release gates

The complete list of checks that a human must still perform by hand. Everything
else that used to live in `MANUAL_GATES_RUN.md` is either asserted by an
automated test or tracked as a named gap — see `docs/qa/MANUAL_GATES_TRIAGE.md`
for the item-by-item classification, and
`docs/qa/archive/MANUAL_GATES_RUN-2026-08-04.md` for the historical checklist.

**A check belongs here only if a machine cannot make the judgement.** Three
reasons qualify:

- **visual** — "does it look right", where no oracle exists short of an eye
- **native** — OS chrome, menus, or shortcuts outside the webview
- **hardware** — real trackpad/mouse/GPU behaviour Playwright cannot synthesise

Anything else goes to CTest, Kernelbench, a Rust real-worker integration test, or
Vitest/Playwright. **Do not grow this file** — a new manual permutation is a
signal that the corresponding automated lane is missing, not that the checklist
should get longer. That rule is what keeps advanced-fillet work from re-growing a
112-box list.

Setup: `scripts/build-worker.sh Release` → `bun run tauri dev`. Use a COLD start
(no Vite HMR reuse) for the theming block.

Record a failure as `GATE <section>/<item> — expected vs observed`, plus the
relevant `logs/dev.jsonl` excerpt.

---

## 1. Visual — theming and shading (6)

- [ ] **Wireframe in dark mode.** All three render modes in dark; wireframe is
      the known failure case — the edge token must invert, not stay light-on-light.
- [ ] **Studio shading reads machined** across a full orbit: no flat faces, no
      blown highlights top-down.
- [ ] **Sketch dim/restore.** Bodies dim to roughly 35% on entering a sketch and
      return to full on exit.
- [ ] **Repair-candidate crosshair.** Hovering a candidate in the repair panel
      draws a crosshair at the right place in the viewport.
- [ ] **Idle sketch chrome is ONE fused two-row pill**, not two stacked bars with
      a seam between them.
- [ ] **Sketch render parity.** A selected edge shows a read-only length label;
      adding a Distance constraint REPLACES it rather than drawing a second one
      (the state half of this is GAP-F9, still unautomated); the origin pin shows
      its lock icon with a Disconnect chip; an endpoint selection draws the
      highlight ring.

## 2. Native — window, menus, shortcuts (4)

- [ ] **⌘W and ⌘Q on a dirty document** both raise the unsaved prompt. (The
      titlebar × path is covered by `e2e/unsaved-guard.spec.ts`; these two are
      native shortcuts and are not.)
- [ ] **Traffic lights follow the theme** under the "System" appearance when the
      OS switches, and a reload shows no light-chrome flash.
- [ ] **Titlebar renders correctly** — custom chrome aligns with the native
      traffic lights, no overlap, no double titlebar.
- [ ] **Start screen and recents.** A freshly saved document appears in recents;
      opening it from there lands in the editor with geometry.

## 3. Hardware — input devices and GPU (4)

- [ ] **Trackpad gestures.** Two-finger pan including diagonal, no rubber-band;
      shift+two-finger orbits and releasing shift mid-momentum does not flip to
      pan; pinch zooms toward the cursor.
- [ ] **The page itself never magnifies** under pinch (native webview zoom must
      stay off).
- [ ] **Device hot-plug.** Plugging a mouse switches the wheel to zoom and the
      status sublabel to "mouse"; unplugging flips back; an explicit override
      pins the mode against both.
- [ ] **GPU context restore.** Sleep/wake or a GPU switch keeps shading and the
      camera pose intact.

## 4. Native — autosave and crash recovery (5)

A machine cannot kill its own host and relaunch it, so this block stays manual in
full. It is the one section here with real data-loss risk — run it every release.

- [ ] **Untouched document.** Open a saved project, touch nothing, `kill -9`,
      relaunch ⇒ **NO** recovery offer.
- [ ] **Continuous work.** Model for 3 minutes with no pause longer than 30 s,
      `kill -9` ⇒ an offer exists and holds the last edit.
- [ ] **Restore, then crash again.** Restore from the offer, then `kill -9` again
      while unsaved ⇒ the offer still exists.
- [ ] **Recent-card path.** Crash with an unsaved edit to a saved project;
      relaunch and click the project in Recent ⇒ prompt appears, and
      "Open saved version" is the only way the autosave is discarded.
- [ ] **Identity.** The restored document shows its real name, never a UUID, and
      the card shows a time.

---

## Not gated here

- **Mac notarization** (`docs/PACKAGING.md` §4) — needs Developer ID credentials,
  which this project does not have; the shipped path today is an ad-hoc-signed
  local bundle, and notarization is a named follow-up rather than a gate.
- **The packaged-app composition flow** (open → extrude → fillet → undo → save →
  reopen) — automated against a real `.app` in `e2e-tauri/specs/composition.e2e.ts`
  and CI job `tauri-composition`. It was a manual box seven times over in
  `TODO.md`; see `MANUAL_GATES_TRIAGE.md` § Round 2.
- **The hex gate** — `grep -rn '#[0-9a-fA-F]\{6\}' src --include='*.ts' --include='*.tsx'`
  must be empty. Mechanical, run it at every gate; it is not wired into `ci.yml`.
