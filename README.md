# OneCAD

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![C++](https://img.shields.io/badge/C++-20-blue.svg)
![Platform](https://img.shields.io/badge/platform-macOS-lightgrey.svg)
![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)



> **⚠️ Development Notice:** This project is under active development and contains multiple known bugs. Not all features work correctly — use at your own risk.

> **⚠️ VibeCoding Alert**: Full codebase generated with AI. Project in active development—manual review & validation required.

![OneCAD screenshot](docs/screenshot.png)


## Overview

**OneCAD** is a free, open-source 3D CAD application for makers and hobbyists. Sketch 2D geometry, add constraints, create faces, then extrude to 3D. Inspired by Shapr3D, built with C++20 + Qt6 + OpenCASCADE.

### Current Status: Phase 3 ~85% Complete

- ✅ Full sketching engine (~10,000 LOC)
- ✅ 15 constraint types with automatic inference
- ✅ PlaneGCS solver integration (1450 LOC)
- ✅ All core modeling operations (Extrude, Revolve, Fillet, Chamfer, Shell, Boolean)
- ✅ Complete I/O layer (Save/Load, STEP import/export)
- ✅ Light/Dark themes, ViewCube, StartOverlay
- 🚀 ~41,000 LOC total across 170 files

### Implementation Matrix

| Feature Area | Status | Notes |
|--------------|--------|-------|
| Sketching engine + constraints | ✅ Complete | 7 tools, 15 constraints, PlaneGCS solver, loop detection |
| Selection & picking | ✅ Complete | Deep select, click cycling, mesh-based 3D picking |
| Rendering (Shaded + Edges) | ✅ Complete | BodyRenderer + SceneMeshStore + preview meshes |
| Adaptive Grid3D | ✅ Complete | Pixel-targeted spacing |
| Extrude + Push/Pull | ✅ Complete | SketchRegion + Face input, draft angle, auto-boolean (500 LOC) |
| Revolve | ✅ Complete | Profile+Axis selection, drag interaction, boolean mode (430 LOC) |
| Boolean (Union/Cut/Intersect) | ✅ Complete | BRepAlgoAPI + ModifyBodyCommand + smart mode detection |
| Fillet/Chamfer | ✅ Complete | Combined tool, drag mode switching, edge chaining (408 LOC) |
| Shell | ⚠️ Partial | Single-face working; multi-face UI wiring pending (312 LOC) |
| Command + Undo/Redo | ✅ Complete | Full CommandProcessor, 9 commands, transactions |
| ViewCube | ✅ Complete | 3D navigation widget (537 LOC) |
| Native save/load (.onecad) | ✅ Complete | ZIP package, BREP cache, history serialization |
| STEP I/O | ✅ Complete | Import/export pipeline (250 LOC) |
| Light/Dark themes | ✅ Complete | ThemeManager (885 LOC) |
| Start overlay | ✅ Complete | Project browser with thumbnails |
| Patterns (Linear/Circular) | ⏳ Planned | Not started |
| Feature history UI | ⏳ Planned | HistoryPanel not implemented |
| Command Search | ⏳ Planned | Cmd+K palette |
| Camera inertia | ⏳ Planned | Physics not implemented |

### Technology Stack

- **C++20** — Modern C++ (CMake enforced)
- **Qt6** — GUI framework
- **OpenCASCADE (OCCT)** — Geometric modeling kernel
- **Eigen3** — Linear algebra
- **PlaneGCS** — Constraint solver
- **OpenGL 4.1 Core** — Rendering

### Platform Support (v1.0)

- **macOS 14+** (Apple Silicon native) — primary target
- **Intel Mac** — v2.0 planned
- **Linux/Windows** — future consideration

---

## Quick Links

- 📖 **[DEVELOPMENT.md](DEVELOPMENT.md)** — Setup, architecture, best practices, debugging
- 📋 **[docs/SPECIFICATION.md](docs/SPECIFICATION.md)** — Full product requirements (3700+ lines)
- 🗺️ **[docs/PHASES.md](docs/PHASES.md)** — Implementation roadmap
- 🔧 **[docs/ELEMENT_MAP.md](docs/ELEMENT_MAP.md)** — Topological naming system
- 🤖 **[.github/copilot-instructions.md](.github/copilot-instructions.md)** — AI development guidelines

---

## Key Features

### Sketching Engine ✅ Complete
- **7 tools**: Line, Arc, Circle, Rectangle, Ellipse, Mirror, Trim (~2,000 LOC)
- **5 entity types**: Point, Line, Arc, Circle, Ellipse (with construction geometry toggle)
- **Constraint solver**: PlaneGCS with 15 constraint types (1450 LOC)
- **Automatic inference**: AutoConstrainer with 7 inference rules (1091 LOC)
- **Smart snapping**: SnapManager with 8 snap types, 2mm radius (1166 LOC)
- **Loop detection**: LoopDetector with DFS cycles + hole detection (1887 LOC)
- **Face builder**: OCCT bridge with wire repair (719 LOC)

### 3D Modeling ✅ Core Complete (~85%)
- ✅ **Extrude + Push/Pull**: SketchRegion + Face input, draft angle, auto-boolean (500 LOC)
- ✅ **Revolve**: Profile+Axis, drag interaction, boolean mode (430 LOC)
- ✅ **Boolean ops**: Union/Cut/Intersect with smart mode detection (130 LOC)
- ✅ **Fillet/Chamfer**: Combined tool, variable radius, edge chaining (408 LOC)
- ✅ **Shell**: Hollow solid creation (312 LOC, single-face working)
- ⏳ **Patterns**: Linear/Circular arrays (not started)

### File I/O ✅ Complete
- ✅ **Native format (.onecad)**: ZIP package with BREP cache + history (~2,400 LOC)
- ✅ **STEP import/export**: Industry standard exchange (250 LOC)
- ✅ **Dual package backend**: QuaZip primary, system zip fallback

### User Experience
- **Zero-friction startup**: Open to blank document or project browser
- **Visual feedback**: Blue/green constraint states (DOF tracking)
- **Light/Dark themes**: Full UI theming (885 LOC)
- **Real-time preview**: All modeling operations with drag-to-commit
- **ViewCube**: 3D navigation widget (537 LOC)
- **Undo/redo**: Full transaction support with 9 commands

---

## Repository Structure

```
OneCAD/
├── src/                      # ~41,000 LOC across 170 files
│   ├── app/                  # Application lifecycle (25 files, ~3,000 LOC)
│   │   ├── commands/         # 9 commands (Add/Delete/Modify/Rename Body/Sketch, Visibility)
│   │   ├── document/         # Document model, OperationRecord
│   │   └── selection/        # SelectionManager with deep select
│   ├── core/                 # Core geometry (~13,000 LOC)
│   │   ├── sketch/           # 48 files - entities, tools, constraints, solver
│   │   ├── loop/             # LoopDetector, FaceBuilder, AdjacencyGraph
│   │   └── modeling/         # BooleanOperation, EdgeChainer
│   ├── kernel/               # ElementMap - topological naming (990 LOC)
│   ├── render/               # Camera3D, Grid3D, BodyRenderer (~2,500 LOC)
│   ├── io/                   # File I/O (~2,400 LOC)
│   │   ├── step/             # STEP import/export
│   │   └── ...               # OneCADFileIO, DocumentIO, ZipPackage
│   └── ui/                   # User interface (~13,000 LOC)
│       ├── viewport/         # Viewport (2921 LOC) + RenderDebugPanel
│       ├── tools/            # Extrude, Revolve, Fillet, Shell tools
│       ├── navigator/        # ModelNavigator
│       ├── viewcube/         # ViewCube navigation
│       ├── theme/            # ThemeManager
│       └── start/            # StartOverlay, ProjectTile
├── tests/                    # Prototype executables
├── docs/                     # SPECIFICATION.md (3700+), PHASES.md
├── third_party/              # PlaneGCS (42MB static lib)
└── resources/                # Icons, shaders
```

---

## Contributing

### For Developers

Start here: **[DEVELOPMENT.md](DEVELOPMENT.md)** contains:
- ✅ Environment setup (macOS, Linux)
- ✅ Build instructions
- ✅ Architecture overview
- ✅ Code best practices (C++20, Qt, naming conventions)
- ✅ Guide to key systems (sketch engine, solver, rendering)
- ✅ Common development tasks (adding tools, constraints, debugging)
- ✅ Testing strategy (prototype executables)

### Quick Build

```bash
# Clone repository
git clone https://github.com/yourusername/OneCAD.git
cd OneCAD

# See DEVELOPMENT.md for full setup instructions
```

---

## License

MIT or Apache 2.0 (to be determined)

---

## Acknowledgments

- **OpenCASCADE (OCCT)**: Geometric modeling kernel
- **PlaneGCS**: Constraint solver
- **Qt6**: GUI framework
- **Eigen3**: Linear algebra library
- Inspired by **Shapr3D**, **FreeCAD**, **SketchUp**

---

## Project Status

| Phase | Status | Progress |
|-------|--------|----------|
| **Phase 1** Foundation | ✅ Complete | ~98% |
| ↳ 1.1 Project & Rendering | ✅ | ~98% |
| ↳ 1.2 OCCT Kernel | Partial | ~50% |
| ↳ 1.3 Topological Naming | ✅ | ~95% |
| ↳ 1.4 Command & Document | ✅ | 100% |
| **Phase 2** Sketching | ✅ Complete | 100% |
| **Phase 3** Solid Modeling | 🚀 In Progress | **~85%** |
| ↳ 3.1 I/O Foundation | ✅ Complete | **100%** |
| ↳ 3.2 Parametric Engine | Partial | ~25% |
| ↳ 3.3 Modeling Operations | ✅ Complete | **100%** |
| ↳ 3.4 Pattern Operations | Not Started | 0% |
| ↳ 3.5 UI Polish | In Progress | ~40% |
| **Phase 4** Advanced Modeling | Partial | ~10% |
| **Phase 5** Optimization | 📋 Planned | 0% |

See [docs/PHASES.md](docs/PHASES.md) for detailed roadmap.

---

**Want to contribute?** Check out [DEVELOPMENT.md](DEVELOPMENT.md) to get started!
