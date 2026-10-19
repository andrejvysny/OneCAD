# Local source provenance

This crate adapts local OneCAD spike code rather than adding the spike as a
runtime dependency. These are source-content fingerprints, not claims of a
clean tag, reproducible binary, native acceptance or third-party license review.

- `fonts.rs` (now `onecad-widgets/src/fonts.rs`): originally adapted from `spikes/toolkit-egui/src/fonts.rs`. Source SHA-256: `88a10beb02432ff420ebf27b2803ecde9f7e58ae5dced263cb48cbdffcf3ccbf`.
- `theme.rs` (now `onecad-widgets/src/theme.rs`): originally adapted from `spikes/toolkit-egui/src/theme.rs`. Source SHA-256: `9f2bf47e2a49e974aac6d7ef2a5ce464d0676ad70c08b497b7336ed118c58087`.
- `menu.rs`: adapted from `spikes/toolkit-egui/src/menu.rs`. Source SHA-256: `569f271f8356c3a8bdce608f3b7fe9910d9d1a1c1e0890a00f8864884723d955`.

Fonts, theme tokens and icons are no longer local to this crate; `onecad-widgets`
provides them. The old `icons.rs` painter was removed, and nothing in Rust reads
`assets/native/cppIcons.generated.ts` or `assets/native/authored.ts` any more (the
files remain). The menu adapter retains typed actions and duplicate-key
routing, removes spike metrics, and routes Quit through application shutdown.

## Native UI design source

The editor chrome, start screen and components follow `docs/design/native-ui/final/`
(`README.md`, `OneCAD Final.dc.html`, `OneCAD Components.dc.html`). Icons are baked
from the design's `cad-icons.js` (colour isometric set, light and dark palettes) by
`scripts/bake-cad-icons.ts` into `crates/onecad-widgets/assets/icons/`, and are
rasterised at runtime with the `resvg` crate (a `onecad-widgets` dependency).
`onecad-widgets` has its own gallery example for visual comparison.

Layout measurements (220 px tree, 259 px inspector, 44 px title, 34 px status)
and input ownership derive from the spike and existing OneCAD shell. The spike's
mock dimension history, minimal mesh reader, fixed fixture renderer, measurements
and fake toolbar actions are not used. Modeling authority remains onecad-app;
meshes pass through onecad-protocol and onecad-scene, and onecad-render owns CAD
GPU resources on the host's device. The standalone SolidCraft repository is not
a runtime dependency.

`navigation.rs` adapts the ordered raw wheel extraction and finite pinch handling
from SolidCraft `crates/ui-egui/src/navigation.rs::{immediate_wheel,pinch_factor}`
at `6bde126a5cabd1f5af0c68e6955bd0176fab805a` under its MIT option. See the
repository attribution and third-party notice. The adaptation uses egui 0.37's
explicit mouse/trackpad/momentum source and OneCAD's existing navigation contract:
16-point lines, wheel/pinch gains, Shift gesture latching, horizontal wheel zoom
fallback, and no temporal scroll smoothing. Unknown unbracketed sources use the
mouse path; device preference persistence and hardware qualification remain open.

`camera_navigation.rs` ports OneCAD's `src/viewport/engine/CadOrbitControls.ts`
turntable, pan sensitivity and target-plane zoom anchor to checked captured
camera math. It preserves Z-up, projection type and logical-pixel coordinates.
Distance clamping uses the actual applied zoom ratio when retaining the cursor
anchor. Both projection types have automated anchor checks; synthetic input does
not establish physical trackpad, focus or native-input acceptance.

## Full viewport and manipulators

The native host composes OneCAD's indexed quality `onecad-render::Renderer`.
The comparison-only SolidCraft geometry/render/GPU adapter and rendering
prototypes are retired. OneCAD-owned renderer modes define face and edge
visibility; existing host controls retain section and supersampling settings.
Section contour preparation uses the renderer's bounded latest-only worker;
source scene and host reservations survive cancellation until execution ends.
Attribution for adapted
renderer techniques remains in `onecad-render/README.md`.

`gizmo*.rs` adapts the complete transform triad from
`crates/ui-egui/src/gizmo.rs`: axes, plane squares, rotation rings, center ball,
distance snapping and five-degree angular snapping. Native raw input starts CAD
gestures without the toolkit drag threshold. Scene, camera, viewport and frozen
triad state fence each drag; changing a frame sequence alone does not cancel it.

`navigation_cube.rs` and `cube_*.rs` adapt
`crates/ui-egui/src/navigation_cube.rs` and
`assets/icons/ui/source/navigation.rs`, including the font-atlas output hook.
Faces/corners, quarter orbit/roll controls, home/fit, projection selection and
the world triad use OneCAD's captured camera. These widgets own their input before
sketch/model interaction; they do not invoke donor commands.

`transform_ui.rs` supplies OneCAD body identities and a frozen pivot, maps triad
deltas into the existing typed preview flow and leaves authoring to explicit
Apply/Create. A drag never directly commits a donor command or a matrix-only
visual transform. Full SolidCraft UI, engine, document, kernel, preferences and
navigation presets are not application dependencies.

The MIT option and original notices cover these adaptations. Exact upstream
paths are recorded in root `ATTRIBUTION.md`; copied source hashes and local
source provenance remains in root `ATTRIBUTION.md` after vendor retirement.
