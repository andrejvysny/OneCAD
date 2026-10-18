# Toolkit spike: egui (ONECAD-257, Phase 0.2, egui half)

## Numeric-field regression contract

egui is the selected toolkit; the comparison and measurements below remain
historical investigation evidence. The spike's numeric value has local test
history, not an application document or a live geometry worker. These checks
qualify field routing only, not ONECAD-273 modeling acceptance.

`src/input.rs` owns numeric-field focus and composition. Native menu actions
enter egui through `raw_input_hook` in the same frame, with duplicate keyboard
events removed. A focused widget owns Undo/Redo even when its text history is
empty. Application history is eligible only when no widget has focus.

| Existing behavior to preserve | Spike implementation and regression |
|---|---|
| One confirmation per Enter; expression errors keep editing active | `expression_enter_commits_and_blurs`, `invalid_enter_keeps_focus_escape_cancels` |
| IME Enter/Escape belongs to composition, including commit/dismiss frames | `composition_enter_does_not_commit_model`, `composition_escape_does_not_cancel_field`, commit/dismiss ordering tests |
| Focused text never falls through to model undo | Empty-history tests for the dimension and a real second `TextEdit`; `native_edit_keys_target_second_focused_text_field` |
| Native menu and keyboard delivery do not duplicate an edit | `native_and_keyboard_undo_perform_one_text_undo` |
| Cancel and focus transitions retain the committed value | `window_blur_cancels_composition_and_allows_fresh_edit`, `model_history_runs_after_field_escape`, `refocus_after_model_undo_starts_new_text_history` |

Run from `spikes/toolkit-egui`:

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all --check
cargo build --locked --release
```

The unit harness injects egui events. It does not prove Japanese candidate-window
placement, OS input-source switching, physical trackpad/mouse classification,
native menu arbitration, or accessibility operation. Verify those with actual
macOS input. Navigation still treats point-unit wheel deltas as trackpad input;
smooth-scroll mouse classification remains unresolved. No product performance
or full workflow acceptance follows from these tests.

## Native input probe (2026-10-18)

The rebuilt release spike ran on macOS 26.6.2 / Apple M4 Pro through an isolated
`com.andrejvysny.onecad.input-qa` wrapper. Computer Use supplied native pointer,
keyboard and menu actions; accessibility values and the spike input log supplied
observations. This was synthetic native input, not a physical-device review.

- Clicking the numeric field, Select All, typing `10 mm + 2*3`, then Enter produced
  exactly one logged commit of 16.00 mm from 45.86 mm.
- Native Edit > Undo restored 45.86 mm; Edit > Redo restored 16.00 mm.
- Enter on `invalid` logged `reject: unexpected 'i'` and retained field focus.
  Escape restored 16.00 mm without another commit.
- With the field focused again, Edit > Undo routed to its text undoer and left
  the committed value at 16.00 mm rather than undoing local model history.

An accessibility click did not focus the field; a screenshot-grounded coordinate
click did. Synthetic `super+z` arrived as `Y` under the active keyboard layout,
so this probe does not qualify the Undo key equivalent. The body tree was absent
from the accessible controls. Japanese IME, physical gestures, full CAD
accessibility and application document history remain open gates.

The temporary wrapper and raw logs are local evidence at
`/private/tmp/onecad-native-input.mEWGAD/`; they are not durable test fixtures.
The spike has no project save/reopen workflow and does not prove preview-profile
isolation for the eventual application.

## Historical investigation (2026-10-17)

Status: built and measured on 2026-10-17 on macOS only. The iPad/iOS target was dropped from the program the same day, so nothing here was built or run for iOS; the iOS packaging script, the iPad screenshots and the iOS-only code paths (simulator device-limit workaround, touch navigation, `target_os = "ios"` branches) that an earlier, interrupted run had added were removed. The Slint half is in `toolkit-spike-slint.md`.

The prototype is a standalone eframe/egui 0.37.0 app on wgpu 30.0.1 (Metal). It draws all 67 bodies of the NEMA MESH1 fixture with feature edges, wraps them in OneCAD's chrome styled from `src/styles/tokens.css` (dark), floats an expression-capable dimension field over the viewport, and installs a native macOS menu bar through muda. Every number below was measured in this session on the binaries named in "Method notes"; raw output is in `spikes/toolkit-egui/measurements/`.

## Verdict per criterion

The plan names the criteria but gives no numeric thresholds, so "pass" below means "good enough that it should not decide against egui"; the toolkit choice itself is yours.

| Criterion (plan, Phase 0 step 2) | Result | Verdict |
|---|---|---|
| Look fidelity vs `OneCAD.png`, system SF at runtime | All 10 sampled chrome fills and the 4 panel borders are pixel-identical to `OneCAD.png`; the accent pill is within 1/255. Text lines sit within 1 px of the reference. SF is loaded from the system at runtime. Remaining deltas: egui's own glyph rasterizer draws text and icon strokes slightly heavier than WebKit/CoreText, the view cube is flat, and the model data differs (see "Look fidelity"). | Pass for chrome, your visual sign-off pending |
| AX tree readable by the tauri-agent real-input lane | The tauri-agent macOS helper reads 189 nodes on its first query (3/3 fresh launches), finds the dimension field by role and title, and `AXPress` on a tree row selects the body. System Events sees only the window's 3 buttons and a group on its very first query after launch, then all 189. | Pass |
| IME | Dead-key composition works: marked text is shown underlined, then `Commit("é")`. Pasted CJK text renders through eframe 0.37's system-font fallback. A Japanese input method was **not tested** (none is enabled on this Mac). | Partial: dead keys pass, Japanese IME not tested |
| Input-to-photon latency | Event to compositor display, p50 24.35 ms / p95 34.11 ms, against 22.74 / 31.00 ms for a plain AppKit window measured the same way in the same batch. Over three sessions egui was 1.6–5.9 ms behind AppKit at p50. The first step of a drag is held back about 140 ms by egui's click/drag threshold. | Pass, with the drag-threshold note |
| Frame time (orbiting NEMA) | Display-locked at 60 Hz: mean interval 16.80 ms, p99 18.18 ms. Whole-frame CPU mean 2.31 ms (p95 3.36 ms); GPU 1.45 ms (p95 1.73 ms) offscreen at the viewport size. 120 Hz could not be measured on this 60 Hz display. | Pass at 60 Hz; 120 Hz not measured |
| Binary size | 11,626,224 bytes release (4.9 MB gzip -9); 6,420,336 bytes with a size profile (3.1 MB gzip -9). Only system frameworks are linked. | Pass |
| Native menu with key equivalents (step 2.5) | muda menu bar. With real key events, Cmd+Z, Shift+Cmd+Z, Cmd+A, Cmd+C and Cmd+V reach the app as menu events; Undo goes to the focused field's own undo, otherwise to document history. Cmd+X, Cmd+S, Cmd+0 and Shift+Cmd+H are installed but were not pressed. | Pass |

## What the prototype does

1. **Viewport.** A wgpu paint callback inside egui's render pass draws the 67 NEMA bodies (381,524 triangles) with studio Lambert shading (key light offset from the eye, eye fill, hemisphere ambient, soft highlight), 69,267 instanced screen-space edge segments from the MESH1 edge polylines, a fading grid and X/Y axes, at MSAA 4x with reversed-Z depth. MESH1 bytes go to GPU buffers verbatim (Z-up, no axis swap). The camera is a Z-up turntable with OneCAD's constants (`CadOrbitControls.ts`: FOV 76°, iso yaw, pitch clamp) and fits the scene with OneCAD's `computeCameraFit` rule (margin 1.15) on the first frame, using the real viewport aspect. Navigation: Shift+right-drag orbits, middle or right drag pans, a line-stepped wheel zooms to the cursor, a pixel-precise (trackpad) two-finger scroll pans, Shift+scroll orbits, pinch zooms. Rendering is on demand.
2. **Chrome.** Title bar, body tree (left, 220 px), inspector (right, 259 px), floating toolbar, view cube, nav pill and status bar, sized from the Tailwind classes in `src/features/` and coloured only from `tokens.css` (parsed at build time with `include_str!`; no colour literal in the crate). Icons are OneCAD's own SVG path data from `src/icons/*.ts`, flattened and stroked by egui. Selecting a tree row shows the body's properties in the inspector and tints it with the accent.
3. **Dimension field.** A text field anchored to a real MESH1 edge (the 45.86 mm front edge of the motor's top plate), re-projected every frame. It uses SF Mono 14 px, evaluates expressions such as `10 mm + 2*3` (= 16 mm; units mm, cm, m, in, ft) with a live `= 16.00 mm` preview, commits on Enter, cancels on Escape or focus loss, and keeps focus with an error on a bad expression.
4. **Native menu.** muda 0.21.2 installs OneCAD / File / Edit / View / Window menus with key equivalents and routes the actions as described under "Native menu".

## Setup

Machine: Apple M4 Pro, 24 GiB, macOS 26.6.2 (25G83), one external display at 2560×1440, 60 Hz, scale 1x (so 1 point = 1 pixel, the same as `OneCAD.png`, which is 1521×921). Toolchain: rustc 1.97.0, Xcode 26.6 (17F113), Swift 6.3.3 (`swiftc` for the probe tools).

Crate versions (from `Cargo.lock`): eframe, egui and egui-wgpu 0.37.0 (released 2026-10-15), egui-winit 0.37.0, winit 0.30.13, wgpu 30.0.1, accesskit 0.25.1, accesskit_macos 0.27.1, accesskit_winit 0.34.1, egui_system_font_provider 0.37.0, muda 0.21.2, arboard 3.7.0, objc2-core-text 0.3.2. The earlier run had used egui 0.36.2 (SolidCraft's version); moving to 0.37.0 cost two compile errors and nine deprecations (`WidgetType` became accesskit `Role`, `Ui::id` became `scope_id`, `TextEdit::horizontal_align` became `align`) and brought the system-font fallback and the harfrust shaper.

```bash
cd spikes/toolkit-egui
cargo test                      # 10 unit tests: expression evaluator, camera fit/projection, MESH1 parse, tokens, icon parsing
cargo clippy --all-targets      # 0 warnings
cargo run --release             # the app; reads spikes/fixtures/nema17
```

The crate is standalone (empty `[workspace]` table, own `target/`). Release profile: opt-level 3, thin LTO, 4 codegen units, symbols stripped. eframe features: `accesskit`, `clipboard`, `default_fonts`, `system_fonts`, `wgpu`.

Environment switches: `SPIKE_FIXTURE=<dir>` (another MESH1 directory), `SPIKE_WINDOW_POS=x,y`, `SPIKE_BENCH=orbit:<frames>` with `SPIKE_EXIT=1` (scripted orbit benchmark, then quit), `SPIKE_NOVSYNC=1` (`AutoNoVsync` present mode), `SPIKE_GPU_BENCH=<frames>x<scale>` (window-less GPU benchmark), `SPIKE_LATENCY=1` (logs each frame's consumed pointer position and `prepare` time), `SPIKE_FRAME_LOG=1` (one line per frame), `SPIKE_INPUT_LOG=1` (keys, IME, wheel, pointer and focus events), `SPIKE_NO_MENU=1`, `SPIKE_AUTOFOCUS_DIM=1`. All measurements print `[metric]` lines on stderr.

Probe tools in `spikes/toolkit-egui/tools/` (build each with `swiftc -O <file>.swift -o <name>`; `photon.swift` also needs `-parse-as-library`):

- `photon.swift`: the input-to-photon probe (ScreenCaptureKit).
- `native_baseline.swift`: a plain AppKit window that repaints on right-drag, the latency floor.
- `photon_join.py`: splits each latency sample at the app's frame `prepare`.
- `startup.py`: launch-to-first-frame timing.
- `keys_test.sh` with `keys.swift`, `keytap.swift` and `foreign.py`: the real-input Edit-menu, IME and CJK test. It drives the tauri-agent helper built from `tools/tauri-agent/src/platform/macos/helper/main.swift` (v1.3.0).
- `inject_drag.swift`: posts a Shift+right-button drag.
- `scroll.swift`: posts line or pixel scroll events.
- `look_compare.py`: region colour comparison and composites (`uv run --no-project --with pillow python -I tools/look_compare.py ../../OneCAD.png shots/mac-launch.png shots`).
- `join_latency.py`: the older inject-to-prepare join from the interrupted run, superseded by `photon_join.py`.

## Code map (`spikes/toolkit-egui/src/`)

| File | Lines | What it holds |
|---|---:|---|
| `lib.rs` | 475 | `App` state; menu routing (`route_edit`, `route_clipboard`); orbit benchmark; `raw_input_hook` (injects routed menu actions, input log); `native_options` (window 1521×921, MSAA 4, depth 32, full-size content view); `dimension_anchor` |
| `viewport.rs` | 481 | Central panel: paint callback, navigation, toolbar, view cube, nav pill, the dimension field and its expression preview |
| `render.rs` | 454 | `Renderer` (pipelines, MESH1 buffers, edge instances, grid) and the `egui_wgpu::CallbackTrait` viewport callback; reversed depth and face bias |
| `viewport.wgsl` | 142 | Body shading, instanced wide lines with analytic AA, depth-reset pass |
| `ui.rs` | 399 | Title bar, status bar (with the Persp/Ortho segmented control), body tree, inspector |
| `icons.rs` | 388 | Parses OneCAD's TS icon registries and paints them as egui strokes |
| `camera.rs` | 185 | Z-up turntable camera, `computeCameraFit`-style box fit, projection helpers |
| `expr.rs` | 187 | Length-expression evaluator |
| `theme.rs` | 181 | `tokens.css` parser (light `@theme`, overridden by the dark block) and the egui visuals mapping |
| `metrics.rs` | 168 | `CLOCK_UPTIME_RAW` timestamps, frame-interval and CPU summaries, RSS / physical footprint |
| `menu.rs` | 141 | muda menu bar and action channel |
| `mesh1.rs` | 130 | Minimal MESH1 reader (positions, normals, indices, edge polylines) |
| `fonts.rs` | 122 | System SF / SF Mono lookup through CoreText; weight and optical-size pinning |
| `gpubench.rs` | 115 | Window-less GPU benchmark with the same pipelines |
| `scene.rs` | 85 | Fixture loading |

Adapted SolidCraft code (MIT OR Apache-2.0, @ 6bde126a) is listed in `ATTRIBUTION.md`: the paint-callback shape (`crates/ui-egui/src/gpu.rs`), reversed depth, `FACE_BIAS` and the depth reset (`crates/gpu/src/view.rs`), and the line expansion and studio shading (`assets/shaders/viewport.wgsl`).

## Measurements

### Binary size

Method: `ls -l` and `gzip -9 -c | wc -c` on the release binary; a second build with `CARGO_PROFILE_RELEASE_{LTO=fat,OPT_LEVEL=s,CODEGEN_UNITS=1,PANIC=abort}` into a scratch target; `otool -L` for linkage.

- Release: **11,626,224 bytes** (4,910,636 gzip -9).
- Size profile: **6,420,336 bytes** (3,130,051 gzip -9).
- Linked: only system frameworks and libraries (AppKit, ApplicationServices, Carbon, ColorSync, CoreFoundation, CoreGraphics, CoreServices, CoreText, CoreVideo, Foundation, Metal, QuartzCore, libSystem, libobjc, libiconv). No font is bundled; SF is read from `/System/Library/Fonts` at runtime.
- The first 0.37 build, without eframe's `clipboard` feature and before the later fixes, was 11,022,976 bytes. The cost of the feature alone (needed for Cmd+C, see "Native menu") was not isolated.

There is no built Tauri `.app` in this checkout to compare against.

### Startup

Method: `tools/startup.py` spawns the release binary 10 times. It takes `CLOCK_UPTIME_RAW` just before the spawn and reads the app's marks on the same clock: `main`, `App::new` entry (eframe has created the window, the wgpu device and the surface), `App::new` exit, and the first viewport `prepare`. These are warm-cache launches. A true cold start (after `purge` or a reboot) was **not measured**: `purge` needs sudo.

Final binary (10 runs): spawn to first frame **min 111.9 ms, median 141.2 ms, max 155.3 ms**. The phases:

- Spawn to `main`: 4–15 ms.
- `main` to `App::new`: 88–118 ms (median 107 ms). This is the 4 ms fixture load plus eframe's window, wgpu adapter, device and surface creation.
- `App::new`: 13–19 ms (SF font load, pipeline creation, 13.8 MB GPU upload, menu).
- First frame: 7–8 ms.

Two earlier quiet batches gave medians of 119.4 ms (112.5–141.0, the build before the status-bar changes) and 141.4 ms (118.9–147.4, binary `0d6b6850…`), so batch-to-batch spread is about ±20 ms. "First frame" is the paint-callback `prepare`, before present.

### Frame time while orbiting NEMA

Method: `SPIKE_BENCH=orbit:600` turns the camera 0.012 rad per frame and requests a repaint every frame. The app records the interval between successive viewport `prepare`s, the `App::ui` time, and eframe's whole-frame CPU time (`IntegrationInfo::cpu_usage`: ui, tessellation, paint and submit, vsync wait excluded). Separately, `SPIKE_GPU_BENCH` draws the same pipelines into an offscreen MSAA 4x target and times submit-to-idle per frame. Every run waited for a quiet system (no cargo, rustc, clang, ninja, cmake, swift-frontend or toolkit-slint process). Runs that overlapped a cargo build were discarded.

| Run | Frame interval | UI build | Whole-frame CPU |
|---|---|---|---|
| vsync, 600 frames | mean 16.80, p50 16.70, p95 17.59, p99 18.18, max 96.36 ms (one hitch), 59.5 fps | mean 1.12, p95 1.80 ms | mean 2.31, p95 3.36, max 3.87 ms |
| vsync, 1500 frames | mean 16.68, p99 17.84, max 33.57 ms, 60.0 fps | mean 0.61, p95 1.64 ms | mean 1.26, p95 3.23 ms |
| `AutoNoVsync`, 600 frames | mean 15.46, p50 16.65, p95 17.73 ms, 64.7 fps | mean 0.99, p95 1.91 ms | mean 1.99, p95 3.37 ms |

GPU, offscreen (300 frames each): 1042×842 (the viewport) mean **1.45 ms**, p95 1.73 ms; 2084×1684 mean 1.58 ms, p95 2.54 ms. An earlier quiet batch measured 1.34 and 1.13 ms.

Even with `AutoNoVsync`, winit on macOS paces redraws to the display, so on-screen fps stays at the 60 Hz refresh. CPU times vary about 3x between runs with the same work (UI build p50 0.28–1.21 ms), which looks like core and clock scheduling of a light process. CPU p95 plus GPU p95 is about 5.1 ms, inside an 8.3 ms 120 Hz budget. That is an inference: 120 Hz was not measured, because this display runs at 60 Hz.

### Input-to-photon proxy

Method: `tools/photon.swift` captures the app window with ScreenCaptureKit (BGRA, minimum frame interval 1/240 s, cursor hidden), posts a Shift+right-button-down, then 40 Shift+right-drag steps of 6 px, 120 ms apart, as CGEvents. For each step it reports the delay from the post time to the `displayTime` of the first captured frame whose viewport pixels (a 96×64 sample grid) differ from the last frame before the post. Both stamps are mach host time, so no cross-process clock join is needed. `displayTime` is when the compositor put the frame on screen; panel scanout is not included. With `SPIKE_LATENCY=1` the app logs each frame's `prepare` time, which `photon_join.py` uses to split every sample in two. The same tool measured `native_baseline.swift`, a plain AppKit window at the same size and position that repaints on every right-drag, in the same batch. Each run checked that the target window was frontmost and topmost before posting.

Final batch, 3 runs × 39 steady-state steps (step 0 reported separately):

| | n | mean | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|---:|
| egui, event to display | 117 | 24.77 | **24.35** | **34.11** | 38.67 | 39.72 ms |
| AppKit floor, event to display | 117 | 22.20 | 22.74 | 31.00 | 32.37 | 32.40 ms |
| egui, event to `prepare` | 117 | 3.05 | 3.39 | 4.11 | 13.96 | 16.79 ms |
| egui, `prepare` to display | 117 | 21.72 | 21.88 | 30.68 | 35.48 | 35.83 ms |

Earlier sessions with the same method: egui p50 22.95 ms vs AppKit 17.04 ms, then 26.37 ms vs 22.13 ms. So egui trails the AppKit floor by 1.6–5.9 ms at p50, a tenth to a third of a 60 Hz frame. Nearly all of egui's time lies after `prepare`: egui-wgpu runs paint callbacks before `get_current_texture()` (where macOS waits for a drawable) and before `present`, so `prepare` is not a present timestamp.

**Step 0 of each drag took 138–150 ms** (AppKit: 28–34 ms) and shares its display frame with step 1. egui does not report a drag until the pointer has moved more than `max_click_dist` (6 px by default) or has been held long enough, so the first 6 px of a right-drag produce no orbit. With a real mouse this costs the first few pixels, not 140 ms, but a CAD viewport will want its own threshold: `ctx.options_mut(|o| o.input_options.max_click_dist = …)` or raw pointer handling.

### Memory

Method: `proc_pidinfo` resident size and `proc_pid_rusage` physical footprint, logged by the app; `footprint -p` during an orbit and at idle.

- After the first frame: **resident 134 MB, physical footprint 65 MB**.
- While orbiting: resident 131–138 MB, physical footprint **393.5–403.5 MB**. `footprint` puts 262 MB of that in "owned physical footprint (graphics)", which is driver-owned GPU memory.
- Idle after rendering: footprint 190 MB. That is 61 MB of dirty graphics memory, plus 160 MB of graphics memory marked reclaimable, which macOS does not count.
- The window-less GPU benchmark (same wgpu pipelines and scene buffers, no egui, no window) used 42 MB resident and 327.5 MB footprint at 1x (429.8 MB with 2x targets).

So the large footprint comes from wgpu/Metal and the scene, not from egui. egui, eframe, the window and the fonts add about 90 MB resident, of which the system SF file is 7.9 MB (read once and shared by the three weights).

### Accessibility tree

Method: the tauri-agent macOS helper's `ax_snapshot`, `ax_find` and `ax_press` verbs. Separately, an AppleScript dump of `entire contents of window 1` through System Events. Both ran on fresh launches.

- The helper's first query returns **189 nodes**, `stopReason: complete`, about 100 ms after the request (3/3 fresh launches in a polling test on the build before the final status-bar change, and again in the final batch). The roles are 67 `AXRow` (subrole `AXOutlineRow`, title "Body N", value = selected), 67 `AXCheckBox` ("Show Body N"), 27 `AXButton` (toolbar, nav pill, view-cube column and title-bar buttons, titled with their tooltips), 15 `AXStaticText`, 2 `AXRadioButton` (Persp, Ortho), 1 `AXTextField` (title "Flange width (mm)", value "45.86"), 1 `AXImage` (the 3D viewport), 1 `AXScrollBar`, 5 `AXUnknown` (egui panels and areas) and 3 `AXGroup`.
- `ax_find role=AXTextField` returns the field's bounds; the keyboard test clicks there. `ax_press` on the "Body 19" row selects the body (`actuated: true`; `shots/mac-selected.png`).
- System Events' **first** query after launch returns only 4 elements (the close, minimize and full-screen buttons and one group); the second returns all 189. accesskit builds its tree lazily on the first assistive request, and egui fills it on a later frame. A lane that uses System Events must retry once. The helper did not need to.
- Painter-drawn widgets carry no AX information unless the code calls `Response::widget_info`. The canvas, title bar, Persp/Ortho segments and the field label all needed explicit calls (`egui::Role::Canvas`, `TitleBar`, `RadioButton`, `WidgetInfo::label`). Tree labels painted with `Painter::text` are not separate AX text; the row title carries them.

### IME and text input

Method: `tools/keys_test.sh` posts real key events (CGEvents, 40 ms hold) to the frontmost app on the Slovak QWERTZ layout. It logs egui's `Ime`, `Text` and `Key` events and screenshots the field mid-composition. An event tap confirmed that every key event in the window came from the test (`foreign_events=0`).

- **Dead key**: Slovak acute (the `=` key) then `e`. egui shows `Preedit "'"` underlined in the field (`shots/mac-ime-deadkey-composing.png`), then gets `Commit("é")` (`mac-ime-deadkey-composed.png`).
- Option+e on Slovak types "ę" directly; it is not a dead key on this layout.
- In one run another app took the foreground between the dead key and the `e`. winit then cleared the preedit and no "é" was committed. The clean re-run committed. Focus loss mid-composition drops the composition.
- **CJK rendering**: pasting "東京 10 mm" renders the kanji through eframe 0.37's `system_fonts` fallback (`mac-cjk-paste.png`). According to the `egui_system_font_provider` docs, egui's bundled fonts lack CJK; that was not tested with the feature off.
- **Japanese input method: not tested.** No Japanese input source is enabled on this Mac (only Slovak plus the character and kana palettes), and enabling one changes the user's system settings. Candidate-window placement (winit's IME cursor area) and press-and-hold accents are untested for the same reason.
- **Tabular numerals**: epaint 0.37 shapes with harfrust but exposes no OpenType feature API, so `tnum` on SF Pro is not reachable. The field, the inspector values and the status readout use SF Mono, which is tabular by construction. OneCAD's `DimensionInput` does the same (`font-mono tabular-nums`, `--font-mono: ui-monospace`).

### Native menu and Cmd+Z routing

The menu is muda 0.21.2: App (About, Services, Hide, Quit), File (Save, Cmd+S), Edit (Undo Cmd+Z, Redo Shift+Cmd+Z, Cut, Copy, Paste, Select All), View (Fit All Cmd+0, Home View Shift+Cmd+H), Window. AppKit's key-equivalent pass hands these keys to the menu before winit sees them, so egui never receives them. The app routes each menu event itself (`App::route_edit` and `route_clipboard`):

- **Undo and Redo**: if the dimension field has focus, the app injects Cmd+Z or Shift+Cmd+Z into egui's next frame, and the field's own undoer handles it. Otherwise the action goes to document history (here, the field's committed values).
- **Clipboard items**: these are custom items. The app injects egui `Cut` or `Copy` events, a `Paste` event read through arboard, or Cmd+A. The code comment says muda's predefined items would send `copy:` and the other actions to a first responder that winit's view does not implement. That was **not re-verified** in this run.

Real-key results (`measurements/C_keys_edit.txt`):

| Step | Result |
|---|---|
| Cmd + the key at the ANSI-Z position (prints Y on QWERTZ) | egui saw `Y cmd=true`, no menu event. Key equivalents match the logical character, not the key position. |
| Cmd+Z, field not focused | Menu Undo goes to document history (nothing to undo). |
| Click the field, Cmd+A, type `10 mm + 2*3`, Enter | Commit 16.0000 mm. |
| Cmd+Z, then Shift+Cmd+Z, field not focused | The document value goes back to 45.86 mm, then forward to 16.00 mm. |
| Click the field, End, type `9`, Cmd+Z | The menu Undo goes to the field's undoer: "16.009" becomes "16.00". |
| Cmd+A, Cmd+V (pasteboard `2*21 mm`), Enter | Commit 42.0000 mm. |
| Cmd+A, Cmd+C | Pasteboard holds "42.00". |
| Escape, then Cmd+Z | The field cancels, then document Undo gives 16.00 mm. |
| `10 parsecs`, Enter | "reject: unknown unit 'parsecs'", focus kept; Escape cancels. |

The first version of the spike had two defects, both mine, both fixed and re-tested. First, the field's Cmd+Z restored a stale value ("45.86") instead of removing the typed "9", because egui's `TextEditState` undoer kept history across the app's programmatic text changes. It is now cleared when the field gains focus. Second, Cmd+C never reached the pasteboard, because eframe's `clipboard` feature was off.

### Navigation input and on-demand redraw

These were checked with posted events (`SPIKE_INPUT_LOG=1`, `SPIKE_FRAME_LOG=1`):

- **Idle**: 0 frames in 5 s with no input, after 13 frames at startup.
- **Wheel**: 5 line-unit events arrive as `MouseWheelUnit::Line` and zoom. 5 continuous pixel events arrive as `MouseWheelUnit::Point` and pan, so mouse and trackpad are told apart by the event, not guessed. The 5 events produced 28 and 23 frames respectively, because egui keeps repainting for a short while after scroll input.
- **Orbit**: a Shift+right-drag across the viewport orbits the camera (`shots/mac-orbit.png`: the view cube turned to BACK/RIGHT).
- **Not tested**: real trackpad pinch (the public CGEvent API cannot synthesize magnify gestures, so only the code path exists: winit `PinchGesture` → egui `Zoom` → `zoom_at`), middle-button pan, and Shift+scroll orbit.

### Look fidelity

Method: `screencapture -o -l <window id>` of the 1521×921 window at 1x, compared with `OneCAD.png` (1521×921) by `tools/look_compare.py`. The script compares the mean colour of flat chrome regions, finds the active Persp segment in each image, and writes a side-by-side image, a 2x text-zoom sheet and a difference image. Text positions were measured as rows and columns of bright pixels.

| Region | `OneCAD.png` | spike | max delta |
|---|---|---|---:|
| Title bar | (38, 41, 46) | (38, 41, 46) | 0 |
| Tree panel / inspector panel | (32, 35, 39) | (32, 35, 39) | 0 |
| Status bar | (29, 32, 36) | (29, 32, 36) | 0 |
| Canvas | (35, 38, 43) | (35, 38, 43) | 0 |
| Toolbar fill | (42, 46, 52) | (42, 46, 52) | 0 |
| Borders: tree/viewport, viewport/inspector, title bottom, status top | (52, 56, 63) at x 219, x 1262, y 43, y 886 | same colour at the same pixels | 0 |
| Active Persp segment | (29, 50, 86) | (29, 49, 87) = `--color-sel-bg` | 1 |

Text placement after the fixes listed below: the tree's BODIES label and rows, the inspector's empty state, the title and the status bar's left group all start within 1 px of the reference. The inspector text wraps at the same word ("…face or edge / to see its parameters and history.").

Remaining deltas:

- **Glyph and icon rendering.** egui reads the SF file but rasterizes with epaint, not CoreText. Text and icon strokes come out slightly heavier and brighter than WebKit's at the same size and position (`shots/compare-onecad-text-zoom.png`). Matching CoreText exactly is not something egui offers.
- **The model data is different.** `OneCAD.png` shows a different NEMA import, with STEP colours (white plates, black stator) and part names (PCB, Baseplate, …). The fixture has no colours, and every body carries the same STEP product name, so the spike shows "Body N" in `--color-body-fill`. The shading is the spike's simple studio Lambert, not OneCAD's IBL plus PBR; shading is the render-look spike's subject.
- **The view cube** is flat-shaded with upright labels. OneCAD's has lit faces and labels that follow the face perspective.
- **The status bar's right group** sits 12 px further left, because the coordinate readout string is wider. The title-bar workspace icon is today's Design-workspace glyph; the monitor icon in `OneCAD.png` no longer exists in the icon registry.
- **The dimension label is not occluded** when its edge turns behind the body (`mac-orbit.png`).

Fonts: the system SF is located at runtime with CoreText. `CTFontCreateUIFontForLanguage` gives the font, and `kCTFontURLAttribute` gives `/System/Library/Fonts/SFNS.ttf` (and `SFNSMono.ttf` for mono). The bytes go to egui. The variable axes are pinned per `FontData` with `FontTweak::coords`: `wght` 400, 510 and 590 for regular, medium and semibold, and `opsz` 17. **The optical size matters**: SFNS.ttf defaults to `opsz` 28 (the Display cut), while CoreText applies the Text cut at UI sizes. Without the pin, egui's text set about 10% narrower than WebKit's ("Nothing selected" was 97 px wide against 107 px). Not reproduced: per-size optical sizing (one value serves every size), the `trak` tracking table (−12/2048 em at 13 px, under 0.1 px per glyph) and OpenType features.

Other fixes this comparison forced, which apply to any egui port:

- An egui `Panel` draws its own 1 px separator. Adding a `Frame` stroke doubles every border to 2 px.
- `ui.label(LayoutJob)` replaces the job's wrap width with the available width. A narrower wrap needs a constrained child `Ui`.
- Framing must use the real viewport aspect, so the fit runs on the first frame rather than in `App::new`.
- Tailwind's `gap-3` becomes a per-`Ui` `item_spacing`, and composite controls such as `SegmentedToggle` (well, padding, touching segments) have to be painted by hand.

## Findings to carry into Phase 4

- egui 0.37 gives a working AX tree (accesskit) and an IME path (winit `NSTextInputClient`) out of the box. Both need care: every custom-painted widget needs `widget_info` with a role and label, and System Events clients must expect an empty tree on their first query.
- Cmd+Z and the other key equivalents never reach egui while a native menu holds them. Routing has to live in the app (text field first, then document history, as the plan says). The injected-event approach works, but the app must reset the `TextEdit` undoer whenever it rewrites the field's text.
- egui's click/drag disambiguation adds a dead zone at the start of every drag. Viewport gestures should use raw pointer state or a tuned `max_click_dist`.
- egui-wgpu's paint callbacks run before the drawable wait and `present`. A frame-latency metric taken inside the callback undercounts by most of a frame.
- egui renders text itself. The system SF can be used, but parity with native text needs manual axis pinning, and some typography (`tnum`, tracking) is out of reach without patching epaint.
- On-demand rendering is natural in eframe. Measured idle is 0 frames, but egui adds trailing frames after scroll input.

## Not done

- iOS / iPad: dropped by the scope change, not built or tested.
- A true cold start (needs `purge` or a reboot).
- 120 Hz frame pacing (the display here is 60 Hz).
- A Japanese input method, IME candidate-window placement, press-and-hold accents.
- Real trackpad pinch and rotate gestures, middle-button pan, Shift+scroll orbit.
- Picking, hover highlight and silhouettes in the viewport: out of scope for this spike. Selection works through the tree only.
- The muda predefined-item behaviour claimed in `menu.rs` was not re-verified.
- A size comparison with the Tauri app (no built `.app` here).

## Method notes

### Native tree follow-up, 2026-10-18

The rebuilt input probe (SHA-256
`87117a90304de28c6bcee09cebb967e781b968b9e0a4b4184d91dd4cef004e65`)
exposed a Bodies outline with named selectable rows and visibility checkboxes.
Computer Use clicked Body 1, then sent Down: Body 2 became selected and its
inspector changed. End selected Body 67, changed the inspector, and scrolled its
row into view. The probe logged ArrowDown and End. These are injected native
input checks, not physical keyboard or assistive-technology qualification.

Tab/Space and a subsequent accessibility click did not establish visibility
checkbox activation; selection remained on Body 67. A later coordinate attempt
returned `noWindowsAvailable`. Visibility keyboard/accessibility acceptance
remains open. The local probe log was
`/private/tmp/onecad-native-input.mEWGAD/stderr.txt`; it is temporary evidence,
not a durable acceptance artifact. This probe does not establish CAD workflow,
Japanese IME, physical trackpad, monitor-change, or complete accessibility gates.

- The desktop was shared with other agents. Every timing run waited for a quiet system, and every injected input was preceded by a frontmost-and-topmost check. One earlier screenshot run lacked that check, and its 30-step Shift+right-drag landed in the Terminal window, which had come to the front. Its results were discarded and the guard added.
- The final measurement batches ran on release binary sha256 `0d6b6850a66b8577e016ea21fbb66c3b84e4bfd0ee45b4fa4092a3e8977fef09`. The very last change before the final screenshots, startup run and sizes (sha256 `4a4eabe4541c3651d29822e2748e714cb4973c61697b494467d9669b559d8287`) only made the status bar's "DOF: —" placeholder grey and regular weight, as in `OneCAD.png`. The two binaries are the same size, 11,626,224 bytes.

## Screenshots (`spikes/toolkit-egui/shots/`)

| File | Shows |
|---|---|
| `mac-launch.png` | First frame of the final build, nothing selected (the comparison image) |
| `mac-dimension-editing.png` | Field focused with `10 mm + 2*3` and the `= 16.00 mm` preview |
| `mac-selected.png` | Body 19 selected through `AXPress` from the tauri-agent helper |
| `mac-orbit.png` | After a real Shift+right-drag orbit |
| `mac-ime-deadkey-composing.png`, `mac-ime-deadkey-composed.png` | Marked text during the dead key, then "é" (2x crops) |
| `mac-cjk-paste.png` | Pasted "東京 10 mm" through the system-font fallback (2x crop) |
| `compare-onecad-side-by-side.png` | `OneCAD.png` (left) and the spike (right), half size |
| `compare-onecad-text-zoom.png` | 2x zooms of the tree, inspector, status bar and toolbar, reference above the spike |
| `compare-onecad-diff.png` | Amplified absolute difference with the sampled regions outlined |
| `mac-before-look-fixes.png` | The first 0.37 build, before the fit, optical-size, border and spacing fixes |

## Evidence (`spikes/toolkit-egui/measurements/`)

- `A_build.txt`: fmt, clippy, tests, sizes, sha256, rustc.
- `B_*`: startup, the three orbit benchmarks, `footprint` at orbit and idle, GPU benchmark, idle frames and wheel.
- `C_*`: per-run latency for egui and the AppKit floor, the pooled split, AX dumps (`C_ax_helper.json`, `C_ax_sysevents1.txt`, `C_ax_sysevents2.txt`), and the keyboard runs (app log, step log and event-tap log for edit, ime and cjk).
- `D_*`: the final build's startup, size profile and look comparison.
- `session*`: the earlier quiet sessions quoted above.
