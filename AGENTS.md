# Repository Guidelines

## Scope and project context

OneCAD is a native Rust desktop CAD application backed by a C++20 OCCT worker.
Read applicable nested guides before editing. Keep changes inside this repository;
the sibling SolidCraft checkout is independent. Preserve unrelated work. Never
commit, push or pull without explicit authorization.

Use the existing [ONECAD Plane project](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/issues/)
for project context and task tracking. Consult relevant work items and canonical
Pages before substantial work. Plane stays read-only unless the user approves a
specific save. Propose important findings, validation and follow-up work there;
keep code-coupled protocols, build instructions and tests in Git. Do not create
local task ledgers or duplicate canonical Pages.

## Architecture

The repository-root `Cargo.toml` and `Cargo.lock` own the workspace.

| Path | Responsibility |
|---|---|
| `crates/onecad-ui` | Native eframe host, platform integration and presentation |
| `crates/onecad-interaction` | Pure interaction transitions and tagged service effects |
| `crates/onecad-render` | Sole CAD renderer, shared native wgpu device and queue |
| `crates/onecad-scene` | Bounded immutable geometry, captured views, picking |
| `crates/onecad-app` | Shared document services, regeneration, worker lifecycle, persistence, import/export and library operations |
| `crates/onecad-core` | Pure domain model and regeneration/recovery contracts |
| `crates/onecad-protocol` | OCW1 transport and checked MESH1 decoding |
| `crates/onecad-mesh-fixture` | Headless fixture generator using shared application services |
| `worker` | C++20 OCCT sidecar and cross-track CTest suites |
| `protocol` | Normative wire schemas and executable NDJSON fixtures |
| `assets/native` | Native theme tokens and icon registries |
| `fixtures/render` | Checked renderer fixtures and provenance |
| `packaging/binaries` | Ignored staged sidecar and embedded integrity manifest |
| `corpus` | Frozen legacy behavior oracle |
| `xtask` | Rust repository automation and strict evidence validators |
| `scripts` | CMake, worker and packaging shell entry points |

The React/Tauri application, Three.js paths, component preview renderer,
comparison renderer/vendor and egui/Slint/render-look prototypes are retired.
Purposeful fixtures, approved goldens and required attribution remain. Consolidation does not
establish production readiness or workflow parity with the retired editor.

Only `onecad-app` owns document mutations. The per-document single-writer
`DocumentRuntime` keeps locked preparation, unlocked worker execution and fenced
commit separate. Preserve latest-only preparation, stale-result fencing and
rollback tokens. Worker compatibility, binary SHA-256 and OCCT fingerprint checks
must never be bypassed. `workerEpoch` and `expectedBaseHash` are fencing axes;
`documentRevision` is advisory.

## Commands

Run Cargo from the repository root. Stage the worker before release builds:

```sh
ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1" scripts/build-worker.sh Release
ctest --test-dir worker/build --output-on-failure
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" cargo test --workspace
```

The worker build directory can be overridden with `ONECAD_WORKER_BUILD_DIR`.
Pinned OCCT installation: `scripts/build-pinned-occt.sh`; packaging:
`scripts/package-macos.sh`. The package script rewrites and signs worker dylibs
before embedding its final hash. Never re-sign nested binaries with `--deep`.

`make build` builds/stages the worker and builds the native host. `make run`
launches existing binaries only, using isolated paths, and opens the start page.
Use `make dev` to build first, then launch. `make qa` also generates the
constrained-plate QA document when needed and opens it.
See `Makefile` and `crates/onecad-ui/README.md` for host arguments and environment
requirements.

Generate renderer fixtures through shared services:

```sh
cargo run -p onecad-mesh-fixture -- --worker "$PWD/worker/build/onecad-worker" --out "$PWD/fixtures/render"
```

Repository automation runs through the Rust `xtask` package:

```sh
cargo test -p xtask
cargo xtask check-rust-layers
cargo xtask verify-modeling-coverage
cargo xtask verify-modeling-contracts
```

## Required validation

`ONECAD_REQUIRE_WORKER=1` makes missing real-worker prerequisites fail. A green
suite with quiet skips is not worker qualification. Backend regression suites
and fixtures live in `crates/onecad-app/tests`, including the suites migrated
from the retired Tauri package. Call shared services with `&AppState`; do not
reintroduce a UI toolkit solely to construct test state.

Renderer CPU tests do not prove appearance. The native renderer workflow runs
ignored Metal tests with `ONECAD_REQUIRE_GPU=1`; missing hardware must fail that
qualification. Native host and headless captures use the same renderer. Preserve
approved goldens and declared tolerances; do not regenerate them to hide defects.
Reproduce native geometry failures and inspect actual logs before changing the
owning operation layer. Report exact gate status and remaining acceptance gaps.

## Code conventions

Match surrounding code and formatters. Prefer explicit, simple functions and
concrete ownership. Keep new functions below 50 lines and files below 500 lines
where practical; avoid unrelated splits of existing larger files. Comments
explain why. Rust must pass Clippy with `-D warnings`; pure crates must not depend
on UI/platform toolkits. Document safety invariants for every unsafe block.

C++ uses C++20 and the worker's `.clang-format`. OCCT is checksum-pinned external
source, not vendored. Worker stdout contains OCW1 frames only; use stderr logging.
Run `scripts/check-worker-stdout-hygiene.sh` after worker edits.

`protocol/SCHEMA.md` and `protocol/mesh_format.md` are normative. Changes require
compatible Rust/C++ implementation and cross-track fixtures. MESH1 stays Z-up,
right-handed, outward wound, with preserved topology identity and row ordinals.
Use the checked decoder before geometry traversal or upload. Keep the
500,000-triangle admission limit and existing resource caps.

Release worker resolution is bundled-only, beside the native executable. Debug
uses `ONECAD_WORKER_PATH`, then `worker/build/onecad-worker`, then bundled worker.
Do not silently pair a release host with an arbitrary development worker.
