# 0015 — The desktop UI is native Rust on wgpu; the toolkit is egui

- Status: Accepted direction. Current delivery and qualification status belongs in Plane; this record does not establish gate completion.
- Date: 2026-10-17
- Sequencing update (2026-10-18): the user authorized implementation and testing of extraction and native integration ahead of the outstanding prerequisites. Acceptance, shared-data use, cutover and publication remain gated; this does not close ONECAD-256/257/259/261.
- Supersedes: ADR-0009 (native target)
- Reshapes: ADR-0013 (native target)
- Plane: ONECAD-261 (P0.6), program ONECAD-255
- Scope: the macOS desktop app. The iPad/iOS target was dropped from scope by the user on 2026-10-17.

## Current implementation and historical context

The repository now has one native Rust host and renderer; the former web editor
is retired. See [current architecture](../ARCHITECTURE.md) and the
[canonical migration audit](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/pages/a0db33f3-7c7f-4bd6-bc1d-0c2c9ca549d1).
The following spike measurements and sequencing constraints describe the recorded
2026-10-17/18 decision, not current gate results. Cleanup authorization permits
retiring unused setup without establishing parity or production-data continuity.

The former frontend used React 19, Three.js and zustand in a Tauri v2 webview. The
migration plan lists the webview's recorded defects: the `eval_script` /
webview-registry deadlock, the `performKeyEquivalent` versus menu clash that
lost undos, no tooltips, trackpad versus mouse guessed from wheel events, the
swallowed first click, WebGL context loss and WebKit-only e2e reds. No IPC
latency problem is recorded, and the ~489 ms 40-feature replay is a backend
cost that no UI changes.

Phase 0 built three standalone spikes against the same MESH1 fixture
(`spikes/fixtures/nema17`: 67 bodies, 381,524 triangles, 5,077 edges, generated
through `DocumentRuntime::get_mesh` from the real worker):

- `docs/design/native-ui/toolkit-spike-egui.md`: eframe/egui 0.37.0, wgpu 30.0.1.
- `docs/design/native-ui/toolkit-spike-slint.md`: Slint 1.18.1, femtovg-wgpu
  renderer, wgpu 30.0.1.
- `docs/design/native-ui/render-look-spike.md`: a headless wgpu 30.0.1 renderer
  with no UI toolkit.

Both toolkit spikes drew the fixture with instanced wide lines at MSAA 4x and
reversed-Z, styled chrome from `src/styles/tokens.css`, loaded system SF at
runtime, floated an expression-capable dimension field over the viewport, and
installed a native menu through muda. The recovered reports record fresh-context
reviews with findings; those reports are historical evidence, not current gate
passes. The comparison below retains the reported measurements and corrections.

All numbers come from one Apple M4 Pro with two 60 Hz displays, shared with
other agents. The plan sets no numeric thresholds, so neither spike can pass or
fail on a number alone.

## Historical spike evidence, side by side (2026-10-17)

Where the two spikes measured different things, the row says so. Those rows
cannot be compared directly.

| Criterion | egui 0.37.0 | Slint 1.18.1 (femtovg-wgpu) |
|---|---|---|
| **License** | egui, eframe, egui-wgpu and wgpu are `MIT OR Apache-2.0`; winit is `Apache-2.0`. No runtime conditions. | Usable only under Royalty-free 2.0, which is not OSI open source. It requires the `AboutSlint` widget or a badge, forbids distributing Slint alone, forbids exposing Slint's APIs from the app, and excludes embedded use. Forks inherit these terms. GPLv3 is ruled out (`licensing.md` §4, D3). |
| **Historical look vs [`OneCAD.png`](https://github.com/andrejvysny/OneCAD/blob/ed9ed33f6f394897612e0a40edee015edd775ef8/OneCAD.png)** | 10 chrome fills and 4 panel borders are pixel-identical; the accent is within 1/255; text starts within 1 px. Glyphs and icon strokes are slightly heavier than CoreText. SF needed `opsz` pinned to 17. | Layout, tokens and spacing match, judged from crops; no per-region pixel diff was made. Text is set tighter than CoreText, and Skia's text is heavier still. Some controls are styled differently, and there is no view cube. |
| **Fonts** | System SF read via CoreText and handed to egui; no `tnum`, so values use SF Mono. | System SF found by fontique. `.SF NS Mono` resolves to the italic file. The `SLINT_FONT_PATH` fix rebinds the generic sans families to SF Mono (found by the reviewer). No `tnum`. |
| **AX tree (tauri-agent helper)** | 189 nodes on the first query. All 67 tree rows are `AXRow` with selected state, including rows scrolled out of view. Every painter-drawn widget needs an explicit `widget_info`. | 197 nodes, but only the 25 on-screen rows of 67 appear (reviewer). Rows arrive as `AXStaticText`, with no selected state. 72 of 73 icon images are unlabelled. |
| **AX via System Events** | The first query sees only 4 elements; the second sees all of them. | The first query sees only the title-bar buttons; a retry is needed. |
| **IME** | Dead keys work, with underlined marked text. CJK paste renders. **Japanese IME not tested.** | Japanese romaji works, with a correctly placed candidate window, and so do dead keys. Marked text is drawn as a highlight; commit takes two Returns; the selection is not replaced during composition. Switching the input source while the app runs did not engage the IME (cause unknown), so the reviewer rates this a conditional pass. |
| **Input latency** | Measured from CGEvent post to the compositor's `displayTime` (ScreenCaptureKit): p50 24.35 / p95 34.11 ms. A plain AppKit window measured the same way gave 22.74 / 31.00 ms; that window is CPU-drawn, not Metal. Drags start only after a 6 px `max_click_dist` threshold, which is configurable. | Measured from post to *just before present*; the compositor's ~1 refresh is not included. Slint's own drag path: p50 27.83 ms. With a raw winit `CursorMoved` hook: 10.56 ms. Slint defers pointer moves to `about_to_wait`, which costs one frame on every drag unless the toolkit's input path is bypassed. |
| **Frame time, NEMA orbit** | Locked to 60 Hz: p99 18.18 ms. Whole-frame CPU mean 2.31, p95 3.36 ms. GPU 1.45 ms mean. 120 Hz not measurable. | Locked to 60 Hz: p50 16.67, max 18.6 ms; 2 of 8 runs had one late frame. Frame CPU p50 4.38 ms (means 3.09–4.60 across runs), of which chrome is about 1.4 ms. GPU scene 0.90 ms. 120 Hz not measurable. |
| **Startup, warm** | Median 141.2 ms, spawn to first `prepare` (before present). | Median 247.4 ms, spawn to the first frame's `AfterRendering` (window revealed). |
| **Binary** | 11.6 MB release; 6.4 MB with a size profile. | 16.9 MB stripped; 14.1 MB with fat LTO; 19.9 MB with Skia. |
| **Memory** | 134 MB resident after the first frame. Footprint 394–404 MB while orbiting, of which 262 MB is driver graphics. | RSS 173 MB idle, 184 MB after orbits. Footprint 198–249 MB. Measured differently, so not comparable. |
| **Native menu** | muda 0.21.2, used directly. Undo is routed to the focused field, otherwise to document history. Custom Cut/Copy/Paste items take ⌘X/C/V app-wide; the reviewer flags a risk for native dialogs' text fields, untested. | The `MenuBar` element builds an NSMenu through muda 0.19.3 and adds the app menu itself. Undo routing is proven, and the field never sees the Z key-down. Cut/Copy/Paste are not in the menu yet. |
| **Viewport integration** | A wgpu paint callback inside egui's render pass. The callback runs before the drawable wait, so in-callback latency stamps undercount. | The scene renders into a texture, which is shown as a `slint::Image` (the `wgpu_texture` pattern). |
| **Idle redraw** | 0 frames in 5 s, plus trailing frames after scroll input. | 1 frame in 3 s (the menu handler's redraw). |
| **API stability** | 0.x. Moving from 0.36.2 to 0.37.0 cost 2 compile errors and 9 deprecations. | 1.x. The wgpu and winit integrations sit behind `unstable-wgpu-30` and `unstable-winit-030` features. |
| **Reference code** | The SolidCraft files the plan adapts (paint-callback viewport, scenario runner, pick index) live in `crates/ui-egui`. The spike already adapted the paint callback. | No SolidCraft equivalent. Adapted from Slint's MIT `wgpu_texture` example. |

The render-look spike used no toolkit, so its results apply to either choice.
On NEMA at 2560×1600 the baseline frame costs 0.47 ms and the candidate quality
look 1.77 ms. Blended line fragments make up most of the difference: about
0.80 ms against 0.12–0.17 ms for the MSAA-only or alpha-to-coverage variants.
On 2026-10-18 the user approved quality lighting and grid in both themes and
delegated the line choice. Blended analytic lines were selected for visual
continuity. This appearance decision does not validate native qualification,
determinism or golden images.

## Decision (accepted direction; gates open)

1. **Replace the webview with native Rust, egui and wgpu on macOS only.** egui
   is selected subject to ONECAD-257's native-input gates. Slint remains
   comparative investigation evidence; no automatic fallback or toolkit
   reselection is authorized. Preserve the current visual language, layout,
   shortcuts and workflow behavior. Keep bundle id `com.andrejvysny.onecad`.
   The original decision deferred removal pending separate authorization. The
   subsequent cleanup scope retires unused Tauri and JS setup while preserving
   behavior; it does not implement or establish production-data continuity.
2. **Use core/protocol/app/scene/interaction/render/ui boundaries.**
   `onecad-core` remains pure domain; `onecad-protocol` owns transport and checked
   MESH1 decoding below the scene boundary. `onecad-app` owns application
   orchestration and public command/projection contracts, with no concrete
   interaction, egui, wgpu or Tauri dependencies. `onecad-interaction` consumes
   those public app contracts; app does not import the interaction implementation.
   `onecad-scene` exposes renderer-neutral scene/query/overlay data.
   `onecad-render` depends on wgpu without a UI-toolkit dependency;
   `onecad-ui` owns egui integration. Scene and interaction remain wgpu-free.
3. **The host owns the shared GPU device, queue and surface.** The renderer owns
   its private render targets and resources. Toolkit integration receives the
   host's shared GPU context; it does not create a second device. The scene seam
   (`SceneQuery` / `RenderSink` / `Anchor`) carries data rather than Three.js
   objects, toolkit widgets or GPU handles. Raw window input is adapted to
   interaction events for CAD tools rather than relying on egui's default
   6 px drag threshold.
4. **Decode the complete MESH1 contract before scene construction.** Checked
   decoding belongs in `onecad-protocol`, including section bounds, typed data
   and cross-section references required by `protocol/mesh_format.md`; the
   spike's minimal reader is not the production contract. Preserve OCW1/MESH1
   wire bytes, IDs, snapshot scope and Z-up coordinates. The native workspace
   now uses the checked decoder; its tests, not the spike reader, provide
   implementation evidence.
5. **Use synchronous CPU picking.** Preserve the eight-logical-pixel edge-pick
   aperture and two-logical-pixel depth slack evaluated at the hit depth.
   Rendering, picking and overlays consume the same captured camera, projection,
   viewport size, scale, clipping and scene/view/session identity. Optional GPU
   results may only narrow candidates conservatively and must reject stale
   captures; they do not replace synchronous CPU authority.
6. **Complete native-input and appearance verification.** ONECAD-257 still owes
   Japanese IME composition/candidate placement/input-source switching, clipboard
   and native-dialog routing, and real mouse/trackpad behavior, including
   `TouchPhase` classification. The historical measurements above do not close
   these gates. Quality lighting/grid in both themes were approved on 2026-10-18; the
   delegated line choice is blended analytic for continuity. This does not approve
   egui text weight or establish native visual parity. The historical decision
   did not freeze goldens; preserve the current checked renderer goldens and
   their declared tolerances. The user approved
   licensing decisions D1/D4/D5 on 2026-10-18; packaging implementation, release
   compliance and publication remain unverified.

## Consequences

- **ADR-0009 is superseded for the native target.** `ViewportContext` (`root: Object3D`,
  `createLabel`, `onFrame`, `onThemeChange`, `raycastFromClient`) has no
  successor in Three.js terms. Its intent carries over: the host owns the frame
  loop, the projection maths and label placement. Theme colours become a GPU
  palette uniform, so the "subscribe to `onThemeChange` or go stale" trap goes
  away by construction.
- **ADR-0013 is reshaped, not dropped.** `@onecad/sdk`, `ExtensionContext` and
  the rule that "`react`, `react-dom` and `three` are host-provided" exist only
  in TypeScript. They retire with `src/`. The principle survives: whatever an
  extension can reach is a narrower, curated surface than the host's internals,
  narrowed by construction rather than by cast. Where that principle lands is
  ADR-0017.
- **All user-visible contract semantics remain parity requirements.** Preserve
  keymap, modelingInteraction, toolbar order, shell layout/mount semantics,
  inspector sections and tree/menu behavior. Rust contract fixtures and probes
  may replace DOM-specific representations; the behaviors do not retire.
- **The original appearance decision did not freeze goldens.** Historical blended quality
  images were not bit-reproducible (atomic silhouette append); the reviewer
  reported up to 6 levels on one pixel of `quality_nema_dark`. Choose
  deterministic compaction or justified tolerances; no tolerance or
  golden is accepted by this record. Current checked goldens and tolerances
  are maintained with the renderer fixtures and must not be changed to hide defects.
- **Native packaging must stay non-sandboxed.** A sandbox would redirect
  `~/Library/Application Support`. The historical review also identified an
  effect on a proposed WKWebView settings import; cleanup does not add that import.
- **egui accessibility and field ownership require explicit handling.** Every
  custom-painted widget needs `widget_info`; viewport drags use CAD interaction
  state. Reset `TextEdit` undo state when the app rewrites a field.

## Cost

A full UI rewrite: about 115k LOC of TypeScript production code, about 108k LOC
of tests, and the 38.7k-LOC `src/tools/` interaction layer, which is ported per
tool family against written specs. That is the plan's Phases 1.5 to 6. The former web UI has since been retired; only the native host is supported.

Neither toolkit reproduces CoreText typography. `tnum` and SF's tracking table
are out of reach without patching the text stack, so values use SF Mono, as
today's `DimensionInput` already does.

Not measured by either spike, and still owed: 120 Hz pacing, a true cold start,
real trackpad pinch, and a Metal clear-colour latency baseline to separate the
toolkit's cost from drawable presentation.

## Verification ownership

Current qualification, licensing, continuity and publication outcomes belong in
canonical Plane records. Historical approvals and measurements in this ADR do
not close those gates. Required worker/GPU prerequisites, current fixture
manifests and exact CI inventories define executable checks; native input and
physical display acceptance remain separate.
