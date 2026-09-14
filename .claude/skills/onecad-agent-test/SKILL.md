---
name: onecad-agent-test
description: Launch, run, drive, screenshot, and verify the REAL OneCAD desktop app (Tauri v2, OCCT worker) as a user would, through the `tauri-agent` MCP server. Use for any request to test, check, QA, reproduce, or "see it working" in the OneCAD app — sketching, extrude/revolve/fillet flows, viewport orbit/pan/zoom, shortcuts, history, save/open — and for any screenshot of the app window. OneCAD facts live here (launch command, selectors, keymap, viewport gestures, log lanes); the generic real-user policy is the global `tauri-agent-test` skill, which this overlays. Not for the Playwright mock lane (`bun run e2e`), vitest, or the wdio composition spec.
---

# OneCAD real-user testing overlay

Load the global `tauri-agent-test` skill for the operating policy and evidence rules. This file adds
what is specific to OneCAD. Reference files: `reference/selectors.md` (testids, tool names),
`reference/shortcuts.md` (keymap and its traps), `reference/viewport-gestures.md` (orbit/pan/zoom),
`reference/logs.md` (`logs/dev.jsonl` lanes and greps).

## Prerequisites (check once per session)

- Worker sidecar staged: `src-tauri/binaries/onecad-worker-aarch64-apple-darwin` exists (otherwise
  `ONECAD_OCCT_ROOT=<prefix> scripts/build-worker.sh Release`).
- The `tauri-e2e` Cargo feature builds (`cd src-tauri && cargo build --features tauri-e2e`) — the
  harness launches `bun run tauri:agent`, which is `tauri dev --features tauri-e2e --config
  src-tauri/tauri.agent.conf.json`. First build after a clean is minutes, not seconds.
- macOS Accessibility + Screen Recording granted to the terminal app (see global troubleshooting).
- Nothing else on port 1420 (Vite) or 4445 (embedded WebDriver). A hand-started
  `TAURI_WEBDRIVER_PORT=4445 bun run tauri:agent` can be attached to instead.

## Session

```
session_start {mode:"launch", launch:"dev"}
```
The app opens on the start screen. Click `{role:"button", name:"New project"}` (exact) to reach
the editor. Never open one of the user's recent documents or the autosave "Restore" entries for a
test: those are real files on this machine, and a test flow must not mutate them; the viewport container is `{testId:"viewport-canvas"}`. Wait for the initial camera fit
to settle (`wait_for {condition:{kind:"revision_stable"}}`) before entering a sketch — the sketch
view inherits the model camera distance.

`launch:"bundled"` uses `src-tauri/target/release/bundle/macos/onecad.app` (built with
`bun run tauri build --features tauri-e2e --config src-tauri/tauri.e2e.conf.json --bundles app`).
That artifact is a production Vite build: `window.__stores`, `__client`, `__logsDump`, `?vpdebug`
do **not** exist there. In `dev` they do, and are diagnostic-only.

## Recipes (real_user)

**Enter a sketch on a base plane.** Click tool `{role:"button", name:"New sketch"}`. The chrome bar
shows "Select a sketch plane". Move the pointer to the viewport center (`pointer_move` with a small
second move so a pointermove delta reaches the picker) until `ui_find {target:{css:"[data-plane-pick-label]"}}`
resolves, then `pointer_click` at that point. Chrome switches to "Editing …", tool `Line` is pressed.

**Draw a rectangle.** Click tool `Rectangle`, then two `pointer_click`s at viewport-relative points
(e.g. center −150,−100 and +150,+100). Watch `{testId:"sketch-dof"}` (`data-dof` attribute) change.
`keyboard_press {key:"Enter"}` finishes the sketch (the Finish button in the chrome bar is the same
action). `Escape` is the cancel ladder — it discards the sketch when pressed in plane-pick phase.

**Extrude.** Click inside the closed region (viewport center if the rectangle surrounds it), then
`{role:"button", name:"Extrude"}`. The tool chip (`operation-hud`) appears with `chip-confirm` /
`chip-cancel`; drag the depth handle in the viewport or type a value in the chip, then `Enter` or
click `{testId:"chip-confirm"}`. `wait_for {kind:"element_hidden", target:{testId:"regen-busy"}}`
then confirm the history row: `ui_find {target:{css:'[data-testid^="history-row-"]'}}` or
`{text:"Extrude"}` in the inspector history section.

**Save.** `keyboard_shortcut {combo:"Primary+S"}`; the first save of a new document opens the native
Save dialog — the harness cannot yet drive native dialogs (Phase 1 gap), so use `Escape` and report
that the shortcut reached the app via `observe_logs {grep:"save_document"}` on the `fe` lane.

**Viewport.** See `reference/viewport-gestures.md`: orbit = right button + Shift drag, pan = middle
or plain right drag, zoom = wheel. Never left-drag to orbit; left is selection.

## Settle probes

`{testId:"regen-busy"}` visible ⇒ regen in flight. `{testId:"status-hint"}` is the last hint the user
saw. `{testId:"sketch-dof"}` carries `data-dof`. History rows are `history-row-<id>` with
`aria-label "Select feature N: <label>"`.

## Evidence for a OneCAD report

- Native input in the journal for every user step (`backend:"cgevent"`).
- A window screenshot after each state change (the harness attaches them under
  `.tauri-agent/artifacts/<session>/`).
- For geometry claims, pair the screenshot with `observe_logs {grep:"regen:"}` (outcome line) or a
  diagnostic `get_projection` read, clearly labelled diagnostic.
- `session_stop` ran and its `teardown` payload reported `survivors:[]` and both ports free — cite
  that, not an assumption, for "no `onecad` / `onecad-worker-` process left running".
- `sketch-dof` state: read the `data-dof` attribute via `ui_inspect` (its `attrs`), not the element's
  accessible name, which can lag the live value.

## Known app behaviour (do not re-report as harness faults)

- Loft and Sweep are unsupported at the worker; re-edits preview at L1 only; Shell has no L1 preview.
- The start screen lists the user's recent projects and any autosaved "Restore"/"Discard" rows. Those
  are real files — only click `New project`; never open, restore, or discard them in a test.
- **Library tool tooltip renders `Library ()`** — it is registered without a shortcut, and OneCAD's
  tooltip always formats `Label (Shortcut)`. A real defect, but a known one; note it, don't chase it.
- The Measure tool's tooltip shows `?`, not `⇧?`; the shifted-slash binding is correct.
- **`Primary+S` on a never-saved document logs `io error: no save path`** and does not open a Save As
  dialog (the harness cannot drive native dialogs yet anyway). Verify the shortcut reached the app via
  `observe_logs {grep:"save_document"}`, and report the missing Save As as an app gap, not a harness one.
- Rarely, the first Extrude commit can roll back with a "did not complete in time" message even though
  the kernel published the body (a frontend apply-edit correlation timeout). If it happens, re-commit;
  it succeeds. Capture the `dev.jsonl` lines (`correlation TIMED OUT`, `rollback`) as evidence.
- A left-click on a body face has been seen to select nothing while an edge click selects; if face
  selection misbehaves, note it and fall back to a labelled diagnostic read rather than assuming a
  harness fault.
