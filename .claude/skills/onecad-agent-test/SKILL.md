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
session_start {mode:"launch", launch:"dev"}                            # foreground: takes the pointer
session_start {mode:"launch", launch:"dev", interaction:"background"}  # leaves the user's desktop alone
```

**Pick the policy first.** Background is right for most OneCAD work: toolbar buttons, the inspector,
property fields, the history tree, keyboard commands, zoom, and every native Save/Open panel all
work there, and the user keeps their cursor. Foreground is required for exactly the gestures
background refuses — **sketch drags, orbit, pan, and the extrude depth-handle drag** — plus anything
where hover styling, wheel device classification or a real keyboard chord is the thing under test.
A background launch still activates the app once; `mode:"attach"` against an app the user started
avoids even that.
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

**Save, including the native dialog.** `keyboard_shortcut {combo:"Primary+S"}`. The first save of a
new document opens a native `NSSavePanel`, which the WebView does not own — `ui_snapshot` sees
nothing of it, and an empty snapshot here means you are looking at the wrong surface, not that the
app is idle. Drive it natively:

1. `native_modal` confirms a panel is up (and the input gate refuses webview targets while it is).
2. `native_find {role:"AXTextField"}` for the filename field, `pointer_click {axRef}`, then
   `keyboard_type_text` the name.
3. `native_find {role:"AXButton", title:"Save"}` and `pointer_click {axRef}`.
4. `wait_for {kind:"element_hidden", target:{testId:"regen-busy"}}` is the wrong probe here — the
   panel is not a regen. Confirm with `observe_logs {grep:"save_document"}` and the title bar losing
   its modified marker.

**Save in a background session.** Same panel, no cursor. Note the order matters: set the filename
BEFORE pressing Save, because an `NSSavePanel`'s Save button is disabled while the field is empty
and `native_press` refuses a disabled control rather than reporting a press that did nothing. Note
too that these three tools observe nothing — no settle, no screenshot, no effects — so confirm the
outcome with `native_modal`, `observe_logs {grep:"save_document"}` or a `ui_snapshot` afterwards.
`native_menu_invoke {path:["File","Save"]}`
opens it (or `keyboard_shortcut {combo:"Primary+S"}`, which reaches the app's own keydown handler
rather than the menu item). Then `native_find {role:"AXTextField"}` →
`native_set_value {ref, value:"part.onecad"}` → `native_find {role:"AXButton", title:"Save"}` →
`native_press {ref}`. Those report `mode:"accessibility"`: they prove the save command runs, not that
a user could click Save, so pair a background pass with one foreground run before calling the flow
accepted.

**Undo and redo: prefer the menu.** `native_menu_invoke {path:["Edit","Undo"]}` and
`{path:["Edit","Redo"]}` press the menu item directly. This side-steps a real defect on this machine:
the helper key map is positional US/ANSI and the active input source is Slovak, where the Z and Y
keys are swapped — so `Primary+Z` sends the wrong character. Use the menu unless the binding itself
is what you are testing.

`Escape` dismisses the panel if you only meant to prove the shortcut arrived.

**Viewport.** See `reference/viewport-gestures.md`: orbit = right button + Shift drag, pan = middle
or plain right drag, zoom = wheel. Never left-drag to orbit; left is selection.

Orbit, pan and every sketch drag need `interaction:"foreground"`; a background session refuses them
with `BACKGROUND_CAPABILITY_UNAVAILABLE` rather than faking a gesture that would move nothing
(`CadOrbitControls` captures the pointer on `pointerdown`, and a synthetic pointer id cannot be
captured). Wheel zoom DOES work in background, because it needs no pointer capture — but the app
never scores a real notch there, so the wheel-versus-trackpad behaviour is untested and the envelope
says so.

## Settle probes

`{testId:"regen-busy"}` visible ⇒ regen in flight. `{testId:"status-hint"}` is the last hint the user
saw. `{testId:"sketch-dof"}` carries `data-dof`. History rows are `history-row-<id>` with
`aria-label "Select feature N: <label>"`.

The harness now settles on more than the DOM, which matters twice in OneCAD. An **orbit** changes
camera matrices and repaints WebGL with zero DOM mutations, so settling waits on the rendered-frame
counter; an **Extrude** runs frontend → Rust → OCCT worker → mesh → Three.js upload, so settling
waits on `regenBusy` reaching 0 and geometry no longer pending. Use `wait_for {kind:"worker_idle"}`,
`{kind:"render_idle"}`, `{kind:"render_frame_after"}`, `{kind:"snapshot_at_least"}` and
`{kind:"camera_stable"}` for those rather than a `delay`.

If a settle warning says a signal was **unavailable**, the session is missing `?vpdebug` or the
`__stores` DEV surface and the check proved less than it looks — say so rather than treating the
action as verified.

The agent lane launches with `?vpdebug&trace` so those signals exist; the `?vpdebug` origin pill is
removed at instrumentation time, so it should never appear in a screenshot. If you see it, report it.

## Evidence for a OneCAD report

- Native input in the journal for every user step (`backend:"cgevent"`). A background run instead
  shows `backend:"webdriver"` or `"ax"` — real interactions with the real app and the real Rust
  backend, but not physical input. Name the policy in the report and do not present a background pass
  as an acceptance result.
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
  dialog. Verify the shortcut reached the app via `observe_logs {grep:"save_document"}`, and report
  the missing Save As as an app gap, not a harness one. (The harness CAN drive native dialogs — see
  the Save recipe above — so an absent panel here is the app's behaviour, not a missing capability.)
- Rarely, the first Extrude commit can roll back with a "did not complete in time" message even though
  the kernel published the body (a frontend apply-edit correlation timeout). If it happens, re-commit;
  it succeeds. Capture the `dev.jsonl` lines (`correlation TIMED OUT`, `rollback`) as evidence.
- A left-click on a body face has been seen to select nothing while an edge click selects; if face
  selection misbehaves, note it and fall back to a labelled diagnostic read rather than assuming a
  harness fault.
