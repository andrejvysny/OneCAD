# Repository Guidelines

## Repository Snapshot
- Current code focuses on the sketch engine + OpenGL viewport. 3D modeling/feature history/undo/STEP I/O are not implemented yet despite roadmap claims.
- Document model owns multiple sketches (JSON serialization), but no on-disk save/load pipeline or feature tree.
- Kernel and I/O targets are interface placeholders; ElementMap is header-only and only used by prototypes.
- UI is Qt6 Widgets (MainWindow + Viewport + ModelNavigator + overlay toolbar/panels). PropertyInspector exists but is not built or wired into the app.
- Resources are minimal (stack.svg, paper_pencil.svg). Import/Export menu entries are placeholders.

## Project Structure & Modules
- `src/app/`: `Application` singleton and `document/Document` (sketch registry with QUuid IDs, JSON round-trip).
- `src/core/`: Sketch engine (points/lines/arcs/circles/ellipses), constraints (Fixed, Midpoint, Coincident, OnCurve, Parallel, Perpendicular, Tangent, Concentric, Equal, Distance, Angle, Radius, Diameter, Horizontal/Vertical), PlaneGCS solver adapter, snapping, auto-constrainer, SketchRenderer, tools (Line, Arc, Circle, Rectangle, Ellipse, Trim, Mirror), loop detection + FaceBuilder -> OCCT faces.
- `src/render/`: `Camera3D` orbit camera, `Grid3D` (fixed 10 mm spacing, OpenGL shaders).
- `src/ui/`: MainWindow, Viewport (OpenGL, plane selection overlay, ViewCube, DimensionEditor), ModelNavigator, ContextToolbar, ConstraintPanel, SketchModePanel, ThemeManager, SidebarToolButton. PropertyInspector lives in `inspector/` but is excluded from the CMake target.
- `src/kernel/`: `elementmap/ElementMap.h` (topological naming utilities) only; library is INTERFACE and not linked into OneCAD.
- `src/io/`: Empty INTERFACE library placeholder.
- `resources/`: `resources.qrc` registering `icons/stack.svg` and `icons/paper_pencil.svg`.
- `third_party/planegcs`: Vendored PlaneGCS constraint solver.
- `tests/`: Prototypes under `tests/prototypes/` plus `tests/test_compile.cpp` (UI compile sanity).
- `build/`: Out-of-tree build output (local only).

## Build, Run, and Prototype Commands
From repo root:
- `mkdir -p build && cd build`
- `cmake ..` (C++20, Qt6, OCCT, Eigen; Qt hint in CMakeLists.txt points to `/opt/homebrew/opt/qt`)
- `cmake --build .` (or `make`)
- `./OneCAD` to launch from `build/`

Prototype targets (all built from `build/`):
- Sketch engine: `proto_sketch_geometry`, `proto_sketch_constraints`, `proto_sketch_solver`, `proto_loop_detector`, `proto_face_builder`
- PlaneGCS only: `proto_planegcs_integration`
- ElementMap/OCCT: `proto_tnaming`, `proto_custom_map`, `proto_elementmap_rigorous`
- UI compile check: `test_compile`
Run with `./tests/<target_name>` after building.

## Coding Style & Naming Conventions
- C++20, 4-space indentation, braces on the same line.
- Classes in PascalCase; functions/variables in lower camelCase.
- Keep `.h/.cpp` pairs and match class names to filenames.

## Architecture & Runtime Notes
- `src/main.cpp` sets OpenGL 4.1 CoreProfile via `QSurfaceFormat` before creating `QApplication`.
- Viewport: orbit/pan/zoom camera (`Camera3D`), fixed-spacing `Grid3D`, plane selection overlay for XY/XZ/YZ, ViewCube, inline DimensionEditor for dimensional constraints.
- Sketch mode: ContextToolbar + SketchModePanel/ConstraintPanel drive tool and constraint actions; rendering via `core::sketch::SketchRenderer`.
- Document state is in-memory only; `Document::toJson/fromJson` exist but are not wired to file dialogs. STEP import/export menu items are TODOs.
- ThemeManager persists theme choice in `QSettings`; MainWindow persists camera angle.

## Testing Guidelines
- No `ctest` wiring; use prototype executables. Register new prototypes in `tests/CMakeLists.txt`.
- ElementMap changes: run `proto_elementmap_rigorous` (and `proto_custom_map`/`proto_tnaming` if relevant).
- Sketch/solver changes: run `proto_sketch_geometry`, `proto_sketch_constraints`, `proto_sketch_solver`, `proto_loop_detector`, `proto_face_builder`.
- UI build breakage: `test_compile`.

## Logging & Debugging Workflow
- Central log setup lives in `src/app/Logging.cpp`; it is initialized in `src/main.cpp` before `QApplication`.
- Use categorized Qt logging only: `Q_LOGGING_CATEGORY` + `qCDebug/qCInfo/qCWarning/qCCritical`. Avoid uncategorized `qDebug/qInfo/qWarning/qCritical` in new code.
- Category naming convention: `onecad.<module>[.<subsystem>]` (for example `onecad.core.snap`, `onecad.main`, `onecad.ui.events`).
- Current log format includes timestamp, level, thread id, category, file:line, function, and message.
- Log file directory fallback order:
- `ONECAD_LOG_DIR` (if set)
- `QStandardPaths::AppLocalDataLocation/logs`
- `./logs` (repo-local fallback)
- `QStandardPaths::TempLocation/onecad/logs`
- If file logging cannot be opened, startup must continue with console logging; do not make logging failures fatal.
- Retention policy is enforced at startup: keep logs up to 30 days, cap to 30 files.
- Runtime toggles:
- `ONECAD_LOG_DEBUG=1` enables debug logs in non-debug builds.
- `ONECAD_LOG_DEBUG_CATEGORIES` (comma-separated) selects release debug categories; default is `onecad.main,onecad.app,onecad.io`.
- `ONECAD_LOG_QT_DEBUG=1` enables Qt internal debug categories when needed.
- `ONECAD_LOG_UI_EVENTS=1` enables verbose UI event tracing (`onecad.ui.events`) when debug logging is active.
- `ONECAD_HEADLESS=1` (or `CI=1` / `CODEX_CI=1`) runs `make run` using offscreen Qt platform.
- `ONECAD_HEADLESS_SMOKE=1` exits event loop immediately after startup for headless validation.
- Standard validation loop after implementation:
- run `make run` and confirm startup/shutdown markers in logs (no `FATAL`, no unexpected `ERROR`)
- run relevant prototype binaries for touched modules
- inspect the newest log file under `logs/` when behavior diverges
- Useful local triage commands:
- `ls -lt logs | head`
- `tail -n 200 logs/<latest>.log`
- `rg -n "\\[(WARN|ERROR|FATAL)\\]|onecad\\.(core|ui|render|kernel)" logs/<latest>.log`

## Key Documents
- `docs/SPECIFICATION.md`: product requirements and architecture targets.
- `docs/PHASES.md`: roadmap/status (aspirational; code is behind 3D claims).
- `docs/researches/SHAPR_UX.md`, `docs/researches/SHAPR_SKETCHING.md`, `GEMINI.md`: UX and interaction research.
- `docs/ELEMENT_MAP.md` / `docs/researches/TOPO.md`: topological naming analysis.
- `docs/researches/STEP.md`: STEP import/export research notes.

## Commit & PR Guidelines
- Prefer Conventional Commit prefixes (`feat:`, `fix:`, `chore:`). Keep subjects short and imperative.
- PRs should include a brief summary, build/test output or commands, and screenshots for UI changes. Link related specs/issues where applicable.

## Configuration Tips
- Qt6 expected at `/opt/homebrew/opt/qt` or via `CMAKE_PREFIX_PATH`.
- OCCT and Eigen are expected from Homebrew on macOS.
