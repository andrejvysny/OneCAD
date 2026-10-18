# Toolkit spike: Slint (ONECAD-257, Phase 0.2, Slint half)

Status: built and measured on 2026-10-17, **macOS only**. The iPad target was dropped the same day. A previous interrupted run had added an iOS simulator bundle (`ios/`), iPad code paths and iPad screenshots. All of that was removed, and nothing was built or run for iOS in this session. The egui half is a separate document.

**Outcome.** Slint 1.18.1 with its wgpu integration meets every item the task asked for on macOS. It needed two workarounds in our own code, both described below:

- **One frame of drag latency.** Slint defers pointer moves, which adds one display refresh (about 17 ms) to every drag. We fix it by reading the raw winit event in the window-event hook.
- **Italic monospace.** The system SF Mono resolves to its italic face. We fix it by registering the upright system file at runtime.

Other gaps remain:

- Tabular numerals are not reachable. Slint exposes no OpenType features.
- SF text sets tighter than in WebKit.
- IME marked text is drawn as a selection highlight, not an underline.
- Tree rows reach accessibility as static text, not rows.

| Criterion (plan Phase 0.2) | Result |
|---|---|
| Look fidelity vs `OneCAD.png`, system SF at runtime | **Partial pass.** Layout, tokens, spacing and SF from the OS match. Text tracking is tighter, a few controls are styled differently, and there is no view cube. |
| AX tree readable by the tauri-agent real-input lane | **Pass.** The lane's own Swift helper reads 197 window nodes and the native menu with key equivalents. Caveats below. |
| IME | **Pass.** Japanese romaji composition, the candidate window and commit all work, and so do dead-key accents. There are minor deviations. |
| Input-to-photon latency | **Pass with the raw-event workaround.** Post to just before present is p50 10.6 ms. Slint's built-in drag path measures p50 27.8 ms. |
| Frame time | **Pass at 60 Hz.** The final 600-frame NEMA orbit stays locked to vsync: p50 16.67 ms, max 18.6 ms. Two of eight femtovg runs had one late frame each (23.0 ms and 38.6 ms). 120 Hz was not testable on this display. |
| Binary size | 16.9 MB stripped (femtovg), 14.1 MB with fat LTO, 19.9 MB with Skia. The plan sets no threshold. |

## Setup

- **Machine and toolchain:** Apple M4 Pro, macOS (Darwin 25.6.0), two 2560×1440 displays at 60 Hz and scale factor 1, rustc 1.97.0.
- **Concurrent load:** other agents ran builds and GPU spikes at the same time. The one-minute load average is recorded with each timing.
- **Slint 1.18.1:** the newest 1.x on crates.io on 2026-10-17. Features: `backend-winit`, `renderer-femtovg-wgpu` (default) or `renderer-skia`, `unstable-wgpu-30`, `unstable-winit-030`, `accessibility` and `compat-1-2`.
- **Resolved dependencies:** wgpu 30.0.1, winit 0.30.13, accesskit_macos 0.26.3 and muda 0.19.3. muda comes in through Slint's `MenuBar`.
- **Project layout:** `spikes/toolkit-slint/` is a standalone cargo project with an empty `[workspace]` table and its own `target/`.

```bash
cd spikes/toolkit-slint
cargo build --release                                   # femtovg-wgpu renderer
cargo build --release --target-dir target/skia --no-default-features --features skia
cargo test --release                                    # 5 unit tests
target/release/toolkit-slint                            # loads ../fixtures/nema17
```

| Env var | Effect |
|---|---|
| `SPIKE_WINDOW=1521x921`, `SPIKE_WINDOW_POS=x,y` | Window size and position in logical px (1521×921 is the size of `OneCAD.png`). |
| `SPIKE_BENCH=N` | After the first frame, orbit 360° over N frames back to back, print the metrics, and quit. |
| `SPIKE_GPU_BENCH=N` | Render N scene frames with a GPU wait after each one (the scene pass's real cost). |
| `SPIKE_EXIT_AFTER_FIRST_FRAME=1` | Quit after the first frame (launch timing). |
| `SPIKE_LOG_FRAMES=1`, `SPIKE_LOG_INPUT=1` | Per-frame input-to-present lines, and raw winit input lines with `CLOCK_UPTIME_RAW` stamps. |
| `SPIKE_SLINT_DRAG=1` | Use Slint's `TouchArea` `moved` callback for drags instead of the raw winit event (the comparison path). |
| `SPIKE_NO_CHROME=1` | Hide the panels (profiling only). |
| `ONECAD_UI_FONT`, `ONECAD_MONO_FONT` | Font family overrides. The defaults are `.SF NS` and `.SF NS Mono`. |

In the running app, View > Run Orbit Benchmark (⇧⌘B) and View > Report Metrics (⇧⌘R) print the same metrics.

## Code map

| Path | Lines | Role |
|---|---:|---|
| `build.rs` | 82 | Parses `src/styles/tokens.css`, using the `@theme` block plus the `:root[data-theme="dark"]` overrides. It generates `tokens.slint` (the `Tokens` global) and `tokens.rs` (the viewport palette), so the CSS stays the single colour source. Then it compiles the UI. |
| `ui/app.slint` | 238 | The window: native `MenuBar`, title strip, viewport host (`Image` of the wgpu texture, `ScaleRotateGestureHandler` and `TouchArea`), floating toolbars, and the anchored dimension chip. |
| `ui/panels.slint` | 393 | Body tree, inspector, toolbars, nav pill and status bar, with accessible roles and labels. |
| `ui/dimension.slint` | 86 | The floating dimension chip: `TextInput` with Esc handling and an undo/redo surface. |
| `ui/icons.slint` | 76 | Two-tone icon path data copied from `src/icons/` (OneCAD's own masters). |
| `src/lib.rs` | 423 | Startup, backend selection (wgpu 30 with downlevel limits, macOS window attributes), the rendering notifier (render into a texture, import it as a `slint::Image`), benchmarks, and chip placement. |
| `src/render.rs` | 419 | The wgpu renderer. One indexed draw covers every body; one instanced wide-line draw covers grid, edges and axes. Reversed-Z, MSAA 4×, `device.poll` each frame. |
| `src/scene.wgsl` | 155 | Studio Lambert shading (key light, eye fill, hemisphere ambient, small Blinn highlight), fragment silhouettes, and screen-space wide lines. |
| `src/input.rs` | 166 | The raw winit hook (device kind, timestamps, IME logging, raw-event drag) and the viewport pointer, scroll and pinch callbacks. |
| `src/commands.rs` | 213 | Tree selection and visibility, dimension edit, commit and cancel, menu commands, and undo routing. |
| `src/camera.rs` | 122 | Z-up orbit camera: fit, orbit, pan, zoom-to-cursor, and `project` for overlay anchors. It has a unit test. |
| `src/mesh1.rs` | 152 | Minimal MESH1 reader. Sections are looked up by type. It has a fillet-block count test. |
| `src/expr.rs` | 150 | Length-expression evaluator: `+ - * /`, parentheses, unary sign, and units mm/cm/m/in/". It has tests. |
| `src/state.rs`, `src/metrics.rs` | 187, 117 | App state, plus the timing series (p50/p95/p99) and the `CLOCK_UPTIME_RAW` clock. |
| `tools/` | 387 | Measurement harness: `coldstart.py`, `inject.swift` (CGEvent drag, pixel scroll and line wheel with post stamps), `latency.py` and `latency-run.sh`, `drive.py` (real-input scenarios through the tauri-agent helper), and `ime.swift` (input-source switcher). |

Adapted code is recorded in `ATTRIBUTION.md`. It covers SolidCraft `viewport.wgsl` `shade`, `vs_line` and the silhouette term at 6bde126a (MIT OR Apache-2.0, checked against the source in this session), and Slint `examples/wgpu_texture` (MIT).

## What the prototype does

1. **Viewport.**
   - **Geometry:** all 67 NEMA bodies, 381,524 triangles and 69,391 line instances (grid, 5,077 MESH1 feature edges as polyline segments, and axes). The GPU upload takes about 5–8 ms. MESH1 positions go to the GPU verbatim, Z-up, with no root rotation.
   - **Integration:** the scene renders into an `Rgba8Unorm` texture through an sRGB view, and that texture is set as a `slint::Image` (the Slint `wgpu_texture` pattern). Inside Slint's `BeforeRendering` notifier, the scene re-renders only when the camera, bodies or size changed.
   - **Mouse:** left-drag orbits; Shift+left, right or middle drag pans; the wheel zooms to the cursor.
   - **Trackpad:** two-finger scroll pans, Shift+scroll orbits, and Ctrl+scroll or pinch zooms. Pinch arrives through `ScaleRotateGestureHandler`, which Slint feeds from winit `PinchGesture`. The device is classified from the winit delta type: `LineDelta` is a wheel and `PixelDelta` is a trackpad.
2. **Panels.**
   - **Layout:** a 220 px body tree, a 258 px inspector, a 44 px title strip under a transparent native title bar (the traffic lights stay native), a floating tool strip, a nav pill and a status bar.
   - **Colours:** every colour is a generated token from the dark theme.
   - **Fonts:** UI text uses the family `.SF NS`. Slint's font backend, fontique, enumerates `/System/Library/Fonts` and maps `SFNS.ttf` at runtime (confirmed with `lsof`). Nothing is bundled. Values use `.SF NS Mono`, matching OneCAD's `ui-monospace`.
3. **Dimension input.**
   - **Anchoring:** the chip and a leader dot are anchored to the top of the model bounding box, which is the shaft tip. The point is re-projected on every camera change, so the chip follows orbit, pan and zoom.
   - **Expressions:** as you type, the chip shows a live `= 16.00 mm` preview or an error.
   - **Keys:** Enter commits through the evaluator and Esc cancels. ⌘D focuses the chip and selects all.
   - **Undo:** committed values form a small document undo stack, shown in the inspector history.
4. **Native menu.**
   - **Menus:** Slint's `MenuBar` element becomes an NSApp menu built by muda: an app menu, then File, Edit and View. `@keys(Control + Z)` maps to ⌘Z on Apple platforms.
   - **Undo routing:** Undo and Redo go through one routing function. If the dimension field has focus, the command goes to its `TextInput` undo; otherwise it goes to the document stack.

## Measurements

### Binary size

| Build | Bytes |
|---|---:|
| femtovg-wgpu, `--release`, `strip = true` (the final binary, sha256 prefix 9b83b8c707500460) | 16,944,976 |
| The same with `lto="fat"`, `codegen-units=1` | 14,051,360 |
| Skia renderer, `--release` | 19,908,096 |

- **Method:** `stat -f %z`. All three binaries were built from the final sources.
- **Linking:** `otool -L` lists only system frameworks (AppKit, Metal, QuartzCore, CoreText and others). Fixtures load from disk at runtime, so they add nothing to the binary.

### Launch to first frame

- **Method:** `python3 -I tools/coldstart.py 6`. It spawns the binary with `SPIKE_EXIT_AFTER_FIRST_FRAME=1`. The spawn time and the app's `[spike]` stamps share `CLOCK_UPTIME_RAW`.
- **What "first frame" means:** the `AfterRendering` callback of the first scene frame. Slint keeps the window at alpha 0 until that frame is submitted (`RevealOnFirstFrame`), so this is effectively "window visible with content".
- **Results** (final binary, load average 3.2–3.8):

| Phase | First run | Median of 6 | Range |
|---|---:|---:|---:|
| Spawn to `main` | 5.4 | 13.7 | 5.4–14.6 ms |
| Fixture read and parse (12.5 MB) | 4.0 | 7.4 | 4.0–7.9 ms |
| `main` to window created | 65.3 | 69.8 | 65.3–74.2 ms |
| `main` to first frame | 236.2 | 235.8 | 226.6–239.7 ms |
| **Spawn to first frame** | 241.6 | **247.4** | 240.2–254.3 ms |

- **Not truly cold:** the OS file cache was not purged, because `purge` needs root. The one launch that was closest to cold was the very first run of a fresh build in this session, under concurrent load. It logged `window_created ms_since_main=385.5` and `first_frame ms_since_main=941.0`. That is a single sample and was not repeated.

### Frame time while orbiting NEMA

- **Method:** `SPIKE_WINDOW=1521x921 SPIKE_GPU_BENCH=300 SPIKE_BENCH=600 target/release/toolkit-slint`.
- **Viewport:** 1043×843 physical px, MSAA 4×.
- **Series** (definitions in `metrics.rs`):
  - `frame_intvl`: time between presented scene frames.
  - `frame_cpu`: from `BeforeRendering` to `AfterRendering`. This is Slint's UI render plus the scene encode, measured on the CPU.
  - `scene_cpu`: scene encode and submit only.
  - `gpu_bench`: the scene pass with a GPU wait after each frame.

| Final run, load 3.3 | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|
| Frame interval (600 frames) | 16.670 | 17.037 | 17.356 | 18.556 ms |
| Frame CPU | 4.375 | 4.753 | 4.962 | 5.521 ms |
| Scene encode CPU | 0.055 | 0.075 | 0.089 | 0.108 ms |
| GPU-synced scene pass (300 frames) | 0.901 | 2.072 | 2.589 | 15.523 ms |

- **Vsync:** across eight femtovg orbit runs with chrome, six had a maximum interval of 19.4 ms or less, so no frame was dropped. Two runs had one late frame each:
  - The no-chrome run had one 23.0 ms interval.
  - The menu-triggered orbit during the memory session had one 38.6 ms interval, which is one dropped refresh (its p99 was 19.3 ms).
- **Frame CPU variance:** across seven femtovg runs with chrome, mean frame CPU ranged from 3.09 to 4.60 ms. Two back-to-back runs of the identical final configuration gave 3.46 and 4.60 ms. I could not tie this to fonts: setting `SLINT_FONT_PATH=` or switching to Menlo stayed inside that range. The cause is likely core or clock scheduling, which I did not verify.
- **Chrome cost:** without chrome (`SPIKE_NO_CHROME=1`, 1521×877 viewport), frame CPU mean was 1.73 ms. So Slint drawing the chrome costs about 1.4 ms of CPU per frame with femtovg. That figure comes from an earlier revision of the sources, before the font and raw-drag edits.
- **Skia renderer (final sources, load 1.9):** frame CPU p50 2.73 ms and p95 3.69 ms. Frame interval p95 was 18.5 ms with a maximum of 24.8 ms. The GPU scene pass p50 was 0.94 ms. An earlier Skia build gave frame CPU p50 0.88 ms, p95 3.66 ms and a 28.3 ms maximum interval.

### Input to present (photon proxy)

- **Method:** `tools/latency-run.sh`, using the tauri-agent helper for window lookup and focus, and `tools/inject.swift` for input. It posts 150 real CGEvent left-drags (orbit) and 120 trackpad-style continuous pixel scrolls (pan), one every 8 ms, and refuses to post unless the spike window is frontmost.
- **App-side stamps:** the app stamps raw winit event arrival, and stamps each scene frame at `AfterRendering`. In femtovg-wgpu, that callback runs after drawable acquisition and immediately before `queue.present()`. A frame is charged to the oldest input it shows. `tools/latency.py` joins the app stamps with the post stamps.
- **Not photons:** the measurement stops just before present. At 60 Hz, the compositor adds about one refresh (16.7 ms) before scanout. That was not measured.

| Final runs, load 2.4–5.3 (ms) | Post to winit, p50 | Winit to before present, p50 / p95 | **Post to before present, p50 / p95 / max** |
|---|---:|---:|---:|
| Drag, raw winit event (default) | 5.26 | 5.79 / 6.55 | **10.56 / 15.26 / 16.44** |
| Drag, Slint `TouchArea` path (`SPIKE_SLINT_DRAG=1`) | 5.32 | 22.79 / 23.21 | 27.83 / 32.06 / 33.70 |
| Trackpad pixel scroll (pan) | 4.92 | 5.10 / 5.32 | **10.11 / 13.69 / 16.23** |

**Why Slint's drag path costs one frame:**

- AppKit delivers one coalesced mouse-drag event per refresh.
- Slint's winit backend buffers `CursorMoved` and dispatches it from `about_to_wait` (`SharedBackendData::buffer_mouse_move` in i-slint-backend-winit 1.18.1, added for performance issues #9038 and #10912).
- A redraw requested from that point lands on the next refresh. The event log shows each drag event displayed one frame late, at a constant 22–23 ms.
- Scroll events are dispatched immediately, so they do not pay this frame.
- **Workaround:** handle the drag in `on_winit_window_event` from the raw `CursorMoved`. It needs the viewport origin, which `.slint` exposes as `vp-x`/`vp-y` from `absolute-position`. This removes the frame.
- **Skia:** an earlier Skia build, which used Slint's drag path, gave drag p50 17.9 ms and scroll p50 1.1 ms (event to before present). The final Skia build was not latency-tested.

### Memory

- **Method:** `ps -o rss` and `footprint -p <pid>` at idle after launch, then after each of two 600-frame orbits (⇧⌘B).
- **Build:** the same sources as the final binary.

| State | RSS | `phys_footprint` |
|---|---:|---:|
| Idle after launch | 173,344 KB | 198 MB |
| After the first orbit | 184,128 KB | 249 MB |
| After the second orbit | 184,256 KB | 204 MB |

- **No leak:** RSS is flat across orbits.
- **Footprint movement:** the changes come from reclaimable graphics memory. The second breakdown shows owned unmapped graphics at 54 MB dirty plus 132 MB reclaimable, `MALLOC_SMALL` 47 MB, `MALLOC_LARGE` 28 MB, `IOSurface` 17 MB and `__TEXT` 10 MB (clean).
- **Device polling:** `render.rs` calls `device.poll(Poll)` after each submit, because Slint's renderers never poll the shared device. A comment from the previous run says the footprint grew by about 430 KB per frame without it. I did not re-measure that; the flat RSS above is with the poll in place.

### Accessibility tree

**With the tauri-agent helper.** I built the real-input lane's own helper from source into a scratch directory: `swiftc -O -framework Cocoa -framework Carbon tools/tauri-agent/src/platform/macos/helper/main.swift`. I then drove its JSON-lines verbs against the spike's pid.

- **Window tree:** `ax_snapshot` returned 197 nodes, with `stopReason` complete and nothing truncated.
  - Role counts: `AXImage` 73, `AXStaticText` 64, `AXCheckBox` 25, `AXButton` 23, `AXGroup` 7, `AXRadioButton` 2, `AXOutline` 1, `AXTextField` 1, `AXWindow` 1.
  - The tree is `AXOutline 'Model tree'`.
  - Tool buttons are labelled (Select, Sketch, … Measure) and advertise `AXPress`.
  - Visibility toggles are `AXCheckBox 'Toggle Body N visibility'`.
  - The dimension is `AXGroup 'Height dimension'` containing `AXTextField 'Height'`, value `61.15`.
  - Persp and Ortho are `AXRadioButton`s. The inspector and status bar are labelled groups.
  - Bounds come out in global points, as the lane expects. The private window-id API resolved the `CGWindowID`.
- **Native menu:** `ax_menu` listed the app's menus with key equivalents:
  - Edit > Undo `⌘Z` and Redo `⇧⌘Z`, Edit Dimension `⌘D`.
  - File > Save `⌘S`; Open… `⌘O` is disabled.
  - View > Home View `⇧⌘H`, Fit All `⇧⌘F`, Toggle Orthographic `⇧⌘O`, Run Orbit Benchmark `⇧⌘B`, Report Metrics `⇧⌘R`.
  - macOS adds AutoFill, Start Dictation and Emoji & Symbols to Edit, and Enter Full Screen to View.
  - The app menu is titled `toolkit-slint` (the process name, since there is no bundle). Slint always creates it on macOS with About, Services, Hide and Quit.
- **Real-input lookup:** `drive.py` found the text field with `ax_find role=AXTextField` and clicked it at its AX bounds. It found the tree row `Body 3` the same way. After the click, the inspector rows appeared in AX.

**With `osascript` System Events.**

- The menu bar works: `Apple, toolkit-slint, File, Edit, View`, and Undo's `AXMenuItemCmdChar` is `Z`.
- Window children are reachable by explicit paths (`group 1 of window 1` → `AXStaticText, AXOutline, AXImage, AXGroup, AXGroup`).
- `count (entire contents of window 1)` returns 0, which is why the helper's own walker is the tool to use.

**Caveats.**

- **Lazy activation:** the first System Events query after a launch saw only the three native title-bar buttons, and the next query saw the Slint group. AccessKit builds its tree on the first request. The helper's walk began with `ax_windows` and then saw the full tree, but a lane that queries only once should retry.
- **Tree rows:** a tree row (`accessible-role: list-item`) arrives as `AXStaticText` with `AXPress`, not as `AXRow`. The snapshot carries no selected state for rows.

### IME

All runs used real virtual-key CGEvents (the helper's `press` verb). The user's layout is Slovak QWERTZ.

**Accents.** On the Slovak layout, key code 24 is a dead acute (`=` in US-ANSI naming).

- Acute then `e` produced `Preedit("'")`, then `Commit("é")`.
- Shift+key 24 then `e` produced `Preedit("ˇ")`, then `Commit("ě")`.
- The field value became `éě` (`shots/mac-accent.png`).
- winit reported `Ime::Enabled` when the field took focus and `Disabled` when it lost focus.

**Japanese** (Kotoeri romaji).

- **Setup:** I enabled the input method and its Japanese mode with `tools/ime.swift`, then selected the mode. Afterwards I restored Slovak and disabled both. `defaults read com.apple.HIToolbox AppleEnabledInputSources` was byte-identical before and after each of the six runs.
- **Composition:** typing `nihon` gave `Preedit` steps `n` → `に` → `にh` → `にほ` → `にほn`. Space gave `Preedit("にほん", (0, 9))`, and the system candidate window (にほん, 日本, 🇯🇵, 🗾) appeared directly under the composition in the chip. So the IME cursor area that Slint passes to winit is correct (`shots/mac-ime-candidate.png`, `shots/mac-ime-preedit-crop.png`).
- **Commit:** Return accepted, then a second Return produced `Commit("にほん")`. The commit replaced the selected value, so the field read `にほん`.
- **Deviations:**
  - Marked text is drawn as a selection highlight, not underlined.
  - Committing after the candidate list took two Returns. I did not compare this with a native NSTextView.
  - Of the six runs, the last two completed the sequence. The first four failed for setup or harness reasons:
    - `TISSelectInputSource` returned -50, because the parent input method was not yet enabled.
    - The harness re-activated the app between keys mid-composition, after which Return and Esc never reached the app.
    - The source was switched while the spike was already frontmost, and the app never engaged the IME. Launching the app after the switch worked; the cause was not investigated.
    - The harness crashed on the nameless candidate window.
  - The candidate window belongs to the app's own process, so a capture tool that picks "the app's first window" can pick the candidate panel instead.

### Native menu and ⌘Z routing

- **Method:** `UNDO_KEY=y HELPER=… python3 -I tools/drive.py <pid> <log> shots menu`. On QWERTZ the `z` character sits on the physical ANSI-Y key, so AppKit's character-based key-equivalent matching was exercised on a real non-US layout.
- **Log evidence:**
  - ⌘D, ⇧⌘R and ⇧⌘B reach the menu handler, and the view sees only the key-up. The NSMenu consumed the key-down.
  - Field unfocused, after committing `10 mm + 2*3` (`dim commit '10 mm + 2*3' -> 16 mm`):
    - ⌘Z gives `menu undo -> document (value 61.1485595703125)`.
    - ⇧⌘Z gives `menu redo -> document (value 16)`.
  - Field focused, after ⌘A and typing `5`: ⌘Z gives `menu undo -> text field`. The `TextInput` received only the ⌘ modifier and never the Z key-down, so the undo was handled once. That is the opposite of the webview's `performKeyEquivalent` clash.
  - Esc gives `dim cancel` and restores `16.00`. Return on `abc` does not commit.
- **Undo granularity:** after ⌘A and typing `5`, one text undo left the field empty instead of `16.00`. Slint's `TextInput` records replacing a selection as two steps.
- **Click does not select all:** a click in the field places the caret, so the script uses ⌘A after clicking. ⌘D selects all.

### On-demand redraw

- **Method:** park the pointer on the title strip, reset the counters with ⇧⌘R, wait 3 s, then report with ⇧⌘R.
- **Result:** `slint_frames=1 scene_frames=1`. The one frame is the redraw that the first report's menu handler requested.

### Look fidelity

**Screenshots.** Taken with `screencapture -x -o -l <CGWindowID>` at 1521×921, scale 1, which is the size of `OneCAD.png`:

- `spikes/toolkit-slint/shots/mac-femtovg-1521.png`
- `spikes/toolkit-slint/shots/mac-skia-1521.png`

**Comparisons.** Made with ImageMagick:

- `compare-ref-vs-slint-femtovg.png`: side by side.
- `compare-tree-text.png`, `compare-toolbar.png`, `compare-statusbar.png`, `compare-inspector.png`, `compare-titlebar.png`: reference, femtovg and Skia crops, enlarged.

**Other screenshots.** All are under `spikes/toolkit-slint/shots/` and were taken in this session:

- `mac-dim-typing.png`: expression with live preview.
- `mac-dim-error.png`: invalid expression.
- `mac-selected.png`: Body 3 selected, inspector filled, history of two commits.
- `mac-wheel-zoomed.png`: after mouse-wheel zoom out; the chip is still anchored at the shaft tip.
- `mac-ime-preedit.png`, `mac-ime-preedit-crop.png` and `mac-ime-candidate.png`: Japanese composition.
- `mac-accent.png`: dead-key accents.

**Matches.** Panel widths, row pitch (33 px), the 44 px title strip, the 34 px status bar, every colour, the floating toolbar position and radius, the eye toggles, the empty-inspector state, the grid look and the dark canvas.

**Deltas** (from the crops):

- **Text is set tighter than WebKit** in both renderers ("Nothing selected", "BODIES"). WebKit applies SF's size-specific tracking through CoreText. Slint's parley/fontique path does not appear to. Skia's text is also heavier than femtovg's.
- **Status bar:** the reference's Persp/Ortho control is a segmented group with a background, and the active label is accent-coloured. The spike draws a filled pill. The reference also has FOV 76° against the spike's 40°, which is a camera choice.
- **Title strip:** it lacks the `File ⌄` chevron, and the dirty dot appears only after an edit.
- **Toolbar:** narrower (481 against about 580 px). Some tools and icons differ.
- **Inspector:** the empty-state icon circle is smaller and not accent-tinted.
- **No view cube.**
- **Model:** names and colours differ. The fixture's STEP has no `PRODUCT` names and no colours (see `spikes/fixtures/README.md`), so rows read `Body N` and bodies use the `body-fill` token. The reference document had named, coloured bodies.

## Slint-specific findings for the decision

1. **Pointer-move coalescing costs a frame** on every drag, as described above. Every drag-driven tool needs the raw winit hook, or an upstream change to Slint. This affects orbit, sketch drags and gizmo handles.
2. **Font resolution.** SF comes from the OS at runtime, which is allowed: fontique scans `/System/Library/Fonts` and resolves the hidden `.SF NS` family. Three problems remain:
   - `.SF NS Mono` resolves to `SFNSMonoItalic.ttf` (`lsof` showed that file mapped). The fix is to set `SLINT_FONT_PATH=/System/Library/Fonts/SFNSMono.ttf` in-process before Slint starts (`lib.rs`). After the fix, both renderers draw it upright.
   - There is no `.slint` control for OpenType features, so no `tnum`. Tabular figures come from SF Mono instead.
   - Tracking is tighter than CoreText.
3. **Accessibility** works through AccessKit, but activation is lazy, and the tree and list roles are coarse.
4. **IME** works, including the system candidate window. Marked-text styling and the Return handling after the candidate list deviate from native.
5. **Native menu:** Slint's `MenuBar` gives a real NSMenu with layout-correct key equivalents and an auto-generated app menu. The app still has to route Undo explicitly (done in `commands.rs`).
6. **Renderer choice:** femtovg-wgpu is the default here. Skia's frame CPU varied between runs (p50 0.88 and 2.73 ms), it had longer worst-case frame intervals (24.8 and 28.3 ms), its text renders heavier, and its binary is 3 MB larger.
7. **License:** use Royalty-free 2.0 with the `AboutSlint` widget in an About dialog. See `docs/design/native-ui/licensing.md` §4 (sign-off D3).

## Not done

- **Trackpad pinch zoom:** not verified with real input. There is no public CGEvent API to synthesize magnify gestures. The code path is winit `PinchGesture` → `ScaleRotateGestureHandler` → `vp-gesture`.
- **Right/middle-drag pan:** not injected by me.
- **120 Hz pacing:** this display is 60 Hz.
- **True cold start:** needs root to purge the file cache.
- **Photon latency:** needs a high-speed camera. Only the proxy up to just before present was measured.
- **A native NSTextView IME comparison.**
- **A screenshot of the open Edit menu:** a synthetic click on the menu-bar title did not leave the menu open. The menu contents are evidenced by the AX dump.
- **Memory and accessibility for the Skia build.**
- **Light theme:** only the dark theme was built.
- **The view cube.**
- **Simulator cleanup:** the previous run's iOS app may still be installed on an iPad simulator. No simulator was booted, and I did not touch the simulators.

## Reproduce

```bash
cd spikes/toolkit-slint
S=$(mktemp -d)
swiftc -O -framework Cocoa -framework Carbon \
  ../../tools/tauri-agent/src/platform/macos/helper/main.swift -o $S/helper
swiftc -O tools/inject.swift -o $S/inject && swiftc -O tools/ime.swift -o $S/ime
python3 -I tools/coldstart.py 6
SPIKE_WINDOW=1521x921 SPIKE_GPU_BENCH=300 SPIKE_BENCH=600 target/release/toolkit-slint
HELPER=$S/helper INJECT=$S/inject tools/latency-run.sh target/release/toolkit-slint $S
SPIKE_SLINT_DRAG=1 HELPER=$S/helper INJECT=$S/inject tools/latency-run.sh target/release/toolkit-slint $S
# scenarios: start the app with SPIKE_LOG_INPUT=1 SPIKE_WINDOW=1521x921 SPIKE_WINDOW_POS=1030,60 2>app.log
UNDO_KEY=y HELPER=$S/helper python3 -I tools/drive.py <pid> app.log shots menu   # UNDO_KEY=z on US layouts
HELPER=$S/helper python3 -I tools/drive.py <pid> app.log shots accent             # Slovak dead keys
```

The input scenarios move the real pointer and type into the frontmost window. They refuse to post input unless the spike is frontmost.
