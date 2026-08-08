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

## 1. Visual — theming and shading (4)

- [ ] **Wireframe in dark mode.** All three render modes in dark; wireframe is
      the known failure case — the edge token must invert, not stay light-on-light.
- [ ] **Studio shading reads machined** across a full orbit: no flat faces, no
      blown highlights top-down.
- [ ] **Sketch dim/restore.** Bodies dim to roughly 35% on entering a sketch and
      return to full on exit.
- [ ] **Repair-candidate crosshair.** Hovering a candidate in the repair panel
      draws a crosshair at the right place in the viewport.

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

---

## Not gated here

- **Mac packaging / notarization** (`docs/PACKAGING.md` §5) — release-time, needs
  Developer ID credentials and a physical Mac.
- **The hex gate** — `grep -rn '#[0-9a-fA-F]\{6\}' src --include='*.ts' --include='*.tsx'`
  must be empty. Mechanical, run it at every gate; it is not wired into `ci.yml`.
