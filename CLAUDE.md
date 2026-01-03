# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

```bash
make init              # Install deps (macOS/Homebrew) + configure CMake
make run               # Build + run
make test              # Build + run prototype tests

# Manual build
mkdir build && cd build
cmake .. -DCMAKE_PREFIX_PATH=/opt/homebrew/opt/qt
cmake --build .
./OneCAD  # or ./OneCAD.app/Contents/MacOS/OneCAD
```

Qt path override:
- Use env var: `CMAKE_PREFIX_PATH=/path/to/qt cmake ..`
- Or use flag: `cmake .. -DCMAKE_PREFIX_PATH=/path/to/qt`

Examples:
- macOS (Intel Homebrew): `-DCMAKE_PREFIX_PATH=/usr/local/opt/qt`
- Linux (distro packages): `-DCMAKE_PREFIX_PATH=/usr/lib/qt6`
- Windows (Qt installer): `-DCMAKE_PREFIX_PATH=C:\\Qt\\6.6.0\\msvc2019_64`
- Windows (vcpkg): use `-DCMAKE_TOOLCHAIN_FILE=.../vcpkg.cmake` and vcpkg Qt6 triplet

## Project Overview

C++ CAD application. C++20. Dependencies: Qt6, OpenCASCADE (OCCT), Eigen3.

**Platform**: Tested on macOS 14+ (Apple Silicon). Intel macOS, Linux, and Windows are currently out of scope / untested; support may be added in a future phase. Qt path default: `/opt/homebrew/opt/qt`

## Architecture

```
src/
├── app/           # Application lifecycle, singleton controller
├── core/          # Sketch system, constraints, loop detection
├── kernel/        # OCCT wrappers, ElementMap (topological naming)
├── render/        # Camera3D, Grid3D, OpenGL 4.1 Core
├── ui/            # Qt6 widgets (MainWindow, Viewport, ViewCube, Navigator)
└── io/            # STEP import/export, native format

tests/             # Prototype executables (not unit tests)
```

### Key Layers
1. **UI** → Qt6 widgets, decoupled via Commands pattern
2. **Render** → Camera3D (orbit/pan/zoom), Grid3D (10mm fixed), OpenGL 4.1
3. **Core** → Sketch entities (Point, Line, Arc, Circle), constraints, solver (PlaneGCS planned)
4. **Kernel** → OCCT geometry, ElementMap for persistent topology IDs

### Important Patterns
- **EntityID = std::string (UUID)** for all sketch entities
- **ElementMap** = topological naming system. Descriptor hashing is regression-sensitive (example: a hash ordering change can remap IDs; validate with a unit test + golden descriptor comparison + migration notes).
- **Direct parameter binding** in constraint solver (pointers to coordinates)
- **Non-copyable, movable** entities
- **Scale-preserving camera transitions** (ortho ↔ perspective)

## Critical Implementation Notes

- **ElementMap**: Topological naming for persistent geometry. Descriptor logic changes need thorough validation
- **PlaneGCS integration**: Phase 2 blocker. Solver skeleton exists in `src/core/sketch/solver/`
- **LoopDetector**: Graph-based (DFS cycles, point-in-polygon holes). Implementation pending
- **OCCT**: WARNING: Always null-check `Handle<>` objects before dereferencing shapes
- **Qt signals**: Queued for cross-thread, Direct for same-thread. Ensure parent ownership

## Status / Roadmap

- PlaneGCS: Phase 2 blocker, integration pending.
- LoopDetector: Pending (Phase 3), interface defined in `src/core/loop/LoopDetector.h`.

## Specifications

- `SPECIFICATION.md` - Full software spec (3500+ lines)
- `SKETCH_IMPLEMENTATION_PLAN.md` - 7-phase roadmap
- `PHASES.md` - Development phases overview

## Code Standards

- C++20 features are allowed; use `enum class`, `std::optional`, and `std::span` where it improves clarity.
- Ownership: prefer `std::unique_ptr` for single-owner, `std::shared_ptr` only when required.
- Const correctness: pass by `const&` for large types, keep member functions `const` when no mutation occurs.
- Error handling: return `bool`/`std::optional` for recoverable errors; avoid exceptions in hot paths unless required.
- Qt: use parent ownership for QObject lifetimes and avoid raw new without parent.

## Developer Guidance

- Setup: use `CMAKE_PREFIX_PATH` for Qt and `OpenCASCADE_DIR` if OCCT is not auto-detected.
- Testing: run prototype targets via `cmake --build build --target proto_*` and execute from `build/tests/`.
- Troubleshooting:
  - Qt not found: verify `CMAKE_PREFIX_PATH` or Qt installation location.
  - OCCT not found: set `OpenCASCADE_DIR` to a valid CMake config path.
  - Build cache stale: delete `build/` and reconfigure.
- Git/PRs: use Conventional Commit prefixes (`feat:`, `fix:`, `chore:`); include build notes and screenshots for UI changes.

Review profile is assertive (coderabbit). Excluded: `third_party/`, `build/`, `resources/`, MOC/UI generated files.
