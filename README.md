# OneCAD

> **⚠️ VibeCoding Alert**: Full codebase generated with AI. Project in active development—manual review & validation required.

![OneCAD screenshot](docs/screenshot.png)


## Overview

**OneCAD** is a free, open-source 3D CAD application for makers and hobbyists. Sketch 2D geometry, add constraints, create faces, then extrude to 3D. Inspired by Shapr3D, built with C++20 + Qt6 + OpenCASCADE.

### Current Status: Phase 2 Complete ✅

- ✅ Full sketching engine (2600+ LOC)
- ✅ 15 constraint types with automatic inference
- ✅ PlaneGCS solver integration
- 🚀 Phase 3 (3D modeling) in progress

### Technology Stack

- **C++20** — Modern C++ (CMake enforced)
- **Qt6** — GUI framework
- **OpenCASCADE (OCCT)** — Geometric modeling kernel
- **Eigen3** — Linear algebra
- **PlaneGCS** — Constraint solver
- **OpenGL 4.1 Core** — Rendering

### Platform Support

- **macOS 14+** (Apple Silicon, x86-64)
- **Linux** (experimental)
- **Windows** (planned)

---

## Quick Links

- 📖 **[DEVELOPMENT.md](DEVELOPMENT.md)** — Setup, architecture, best practices, debugging
- 📋 **[docs/SPECIFICATION.md](docs/SPECIFICATION.md)** — Full product requirements (3700+ lines)
- 🗺️ **[docs/PHASES.md](docs/PHASES.md)** — Implementation roadmap
- 🔧 **[docs/ELEMENT_MAP.md](docs/ELEMENT_MAP.md)** — Topological naming system
- 🤖 **[.github/copilot-instructions.md](.github/copilot-instructions.md)** — AI development guidelines

---

## Key Features

### Sketching Engine
- **8 geometry types**: Point, Line, Arc, Circle, Ellipse, Construction geometry
- **Constraint solver**: 15 constraint types (Horizontal, Vertical, Distance, Angle, Tangent, etc.)
- **Automatic inference**: Detects and suggests constraints (±5° tolerance)
- **Smart snapping**: Zoom-independent 2mm snap radius, 9 priority levels
- **Loop detection**: Automatic region highlighting for face creation

### 3D Modeling (Phase 3)
- **Extrude**: 1D → 2D → 3D
- **Revolve**: Rotational extrusion
- **Boolean operations**: Union, difference, intersection
- **Direct modeling**: Push/pull faces interactively
- **Patterns**: Linear and circular arrays

### User Experience
- **Zero-friction startup**: Open to blank document
- **Visual feedback**: Blue/green constraint states
- **Contextual toolbars**: Predictive tool suggestions
- **Real-time preview**: Immediate visual feedback
- **Undo/redo**: Persistent history

---

## Repository Structure

```
OneCAD/
├── src/              # Main source code (C++20)
│   ├── app/          # Application singleton
│   ├── core/sketch/  # Sketch engine (2600+ LOC)
│   ├── kernel/       # OCCT integration & topological naming
│   ├── render/       # Camera & rendering (OpenGL 4.1)
│   └── ui/           # Qt6 Widgets UI
├── tests/            # Prototype executables (not ctest)
├── docs/             # Specification, research, design docs
├── third_party/      # PlaneGCS constraint solver
├── resources/        # Icons, shaders, themes
├── CMakeLists.txt    # Build configuration
└── DEVELOPMENT.md    # Developer guide
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

**Phase 2**: ✅ Complete (Sketching engine)  
**Phase 3**: 🚀 In progress (3D modeling)  
**Phase 4**: 📋 Planned (Advanced features)

See [docs/PHASES.md](docs/PHASES.md) for detailed roadmap.

---

**Want to contribute?** Check out [DEVELOPMENT.md](DEVELOPMENT.md) to get started!