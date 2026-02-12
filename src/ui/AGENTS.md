# UI Guidelines

## Scope
- Qt6 Widgets shell: MainWindow, Viewport (OpenGL), ContextToolbar, ModelNavigator, ViewCube, overlays/panels (ConstraintPanel, SketchModePanel, DimensionEditor, SnapSettingsPanel, RenderDebugPanel, HistoryPanel, StartOverlay), selection popups/adapters, modeling tool UI, ThemeManager.
- Components: SidebarToolButton, ToggleSwitch, Start tiles, PropertyInspector placeholder, History dialogs, modeling tools (Extrude/Revolve/FilletChamfer/Shell) orchestrated by ModelingToolManager.

## Core invariants
- QSurfaceFormat (OpenGL 4.1 core) is set in src/main.cpp before QApplication; do not create QOpenGLWidget/Viewport before that.
- Use Qt parent ownership for widgets and QObject lifetimes; Viewport owns GL resources, tool managers own their tools.
- PropertyInspector exists but is not wired into MainWindow; leave dock hidden unless integrated deliberately.
- ThemeManager is a Meyers singleton; settings are persisted via QSettings; call ThemeManager::applyTheme only on the GUI thread.

## MainWindow layout/flows
- Manages Document + CommandProcessor lifetimes; keeps active sketch id to gate sketch mode UI.
- Positions floating overlays (ContextToolbar, ConstraintPanel, SketchModePanel, History, Snap settings, RenderDebug, StartOverlay) relative to the viewport; re-position on resize.
- DOF status pulled from Sketch via updateDofStatus; cache flags avoid stale UI.
- Uses eventFilter for overlay buttons/panels; keep input from leaking to the viewport if a panel is visible.

## Viewport
- QOpenGLWidget with Camera3D, Grid3D, BodyRenderer, SketchRenderer; initializeGL owns GL resources—ensure QSurfaceFormat set early.
- Manages sketch mode: beginPlaneSelection/enterSketchMode/exitSketchMode; preserves/restores camera angle/pose when toggling.
- Tools: SketchToolManager for 2D tools; ModelingToolManager for 3D (extrude/revolve/fillet/shell). Tool activations emit signals consumed by MainWindow/toolbar.
- Selection: integrates SelectionManager; uses SketchPickerAdapter (2D) and ModelPickerAdapter (3D) plus DeepSelectPopup; selectionContextChanged maps to toolbar context (Default/Edge/Face/Body).
- Snap settings: updateSnapSettings propagates toggle state (grid, guides, 3D snap) to SnapManager via SketchRenderer/tool layer; panel swallows mouse events to avoid passing through.
- Debug toggles (normals/depth/wireframe/gamma/matcap) bound to F1–F5; RenderDebugPanel controls default state.

## ViewCube
- Widget rendering cube orientation; bound to Camera3D (setCamera). viewChanged triggers camera snaps. updateTheme adjusts colors per ThemeManager.

## Toolbar & Navigator
- ContextToolbar contexts: Default, Sketch, Body, Edge, Face; shows/hides relevant SidebarToolButtons and emits tool/operation requests (new sketch, extrude, revolve, fillet, shell, exit sketch, sketch tools).
- ModelNavigator tree tracks bodies/sketches; inline rename/edit, visibility/isolation, collapse animation. Keep item maps (sketchItems/bodyItems) in sync with document signals.

## Sketch panels
- ConstraintPanel lists constraints for current sketch; setSketch must be called on mode enter; refresh after solver/constraint changes; uses theme colors for unsatisfied highlight.
- SketchModePanel exposes constraint buttons; updateButtonStates should reflect current selection; emits constraintRequested/toolRequested for callers.
- DimensionEditor inline edits dimensional constraints; supports math expressions; ensure focus/escape handling and theme connection.

## Snap/Start overlays
- SnapSettingsPanel manages snap toggles (grid, sketch guides, 3D points/edges, hints); overrides mouse/touch events to avoid forwarding to viewport.
- StartOverlay shows new/open/recent projects; uses ThemeManager; callbacks for delete/open/recent actions; rebuilt on showEvent.

## Selection adapters
- SketchPickerAdapter uses SketchRenderer picking; options allow/disable constraints/regions; requires pixelScale + tolerance.
- ModelPickerAdapter caches per-body meshes/topology; requires viewProjection + viewport size; supports face/edge/vertex queries and pick tolerance in pixels; keep mesh cache in sync with SceneMeshStore updates.
- DeepSelectPopup offers candidate list; emits hovered/selected indices; caller owns dismissal.

## Modeling tools
- ModelingToolManager owns Extrude/Revolve/FilletChamfer/Shell; requires Document and CommandProcessor; routes selection/mouse events from Viewport.
- ExtrudeTool builds preview via TessellationCache + ElementMap; tracks boolean mode (NewBody/Add/Cut) and uses neutralPlane/direction from selection. Ensure document/commandProcessor pointers are valid before begin().

## Components
- SidebarToolButton renders symbol or SVG; updates icon on changeEvent for theme responsiveness. ToggleSwitch used in panels; keep them parented to avoid leaks.

## Testing
- UI build sanity: test_compile. For interaction regressions, manual check viewport sketch mode (tool activation, constraint panels, snapping overlays) and modeling tool activation flows.

## Logging & Debugging
- UI startup/flow logs use `onecad.main`; optional high-volume interaction tracing uses `onecad.ui.events`.
- `onecad.ui.events` is opt-in and only active when both conditions are true:
- debug logging is enabled (`Debug` build or `ONECAD_LOG_DEBUG=1`)
- `ONECAD_LOG_UI_EVENTS=1` is set
- When adding new UI diagnostics, prefer dedicated categories (`onecad.ui.<component>`) over uncategorized Qt logging.
- For event-handling bugs, capture logs around:
- `Installed UI event debug logger`
- `ui_event`, `ui_key`, `ui_mouse`, `ui_wheel`
- window/show lifecycle markers in `onecad.main`
- In headless/offscreen runs, repeated `QOpenGLWidget` context warnings are expected; treat them as environment limitations unless accompanied by app init failure.
- Suggested triage command:
- `rg -n "onecad\\.(main|ui\\.events|ui\\.)|QOpenGLWidget|QLayout" logs/<latest>.log`
