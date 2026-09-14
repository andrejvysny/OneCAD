# OneCAD viewport gestures (source: `src/viewport/engine/CadOrbitControls.ts`, `navInput.ts`)

World is Z-up, right-handed. The viewport container is `{testId:"viewport-canvas"}`; use its rect
from `ui_inspect` and `space:"webview"` points for every gesture.

| Gesture | Harness call |
|---|---|
| Orbit (turntable, yaw about world Z) | `pointer_drag_path {button:"right", mods:["Shift"], points:[...], durationMs:600}` |
| Pan | `pointer_drag_path {button:"middle", ...}` or `button:"right"` without Shift |
| Zoom to cursor | `pointer_scroll {target:{testId:"viewport-canvas"}, dy:-3}` (negative = wheel down = zoom out; check the app's convention in the screenshot) |
| Home view | `keyboard_press {key:"h"}` |
| Fit | `keyboard_press {key:"f", mods:["Shift"]}` |
| Select / tool drags | left button only — never orbits |

Right-click inside the viewport is safe: the engine prevents the context menu. Outside the
viewport a right-click opens the WebKit context menu; dismiss with `Escape`.

Wheel input is auto-classified as mouse or trackpad by delta size and timing (`navInput.ts`,
`BIG_TICK_PX = 100`). The harness sends line-unit wheel notches ≥ 100 px, spaced ≥ 120 ms, and
`session_status.wheelProbe.device` says what the app decided. If it is `trackpad`, set Settings →
Navigation → Input device = Mouse (`aria-label "Input device"`, option `Mouse`) and re-probe.

Plane picker (sketch entry): hovering a base plane quad near the viewport center shows the chip
`[data-plane-pick-label]`; clicking creates the sketch on that plane. Datum planes render as quads
too. There is no DOM per entity: entity assertions go through chrome (`sketch-dof`, inspector,
history) or a labelled diagnostic read (`get_projection`, `get_sketch`).
