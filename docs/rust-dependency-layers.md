# Rust dependency layers

`bun scripts/check-rust-layers.mjs` enforces ADR-0015 using
`cargo metadata --format-version 1 --no-deps` at the repository root. It reads
every dependency declaration, including build, dev, optional, renamed and
target-specific dependencies, without filtering for the current target or
enabled features. It does not compile crates or require a staged worker.

The explicit package graph lives in `LAYERS` in the checker. Unregistered
workspace packages and local dependencies outside that workspace fail closed.
A local dependency's package name and path must both match the workspace
member. The only external local exceptions are `onecad-render` normal dependencies
on `solidcraft-gpu` and `solidcraft-render` at their exact vendored paths under
`vendor/solidcraft-viewport/crates/`. Other consumers, paths, and dependency kinds
remain forbidden. The vendored workspace is excluded from the application workspace;
its pinned geometry/render/GPU sources contain no donor engine or document model.
Dependency aliases are checked by actual package name.

Core, protocol and library remain independent foundations. Application
orchestration consumes those foundations. Scene consumes core and protocol;
interaction consumes public app contracts, scene and core; render consumes
scene, protocol and core. Render may use the wgpu family but cannot import a UI
toolkit. Its development dependencies may also use the exact `naga` package to
validate WGSL without creating a GPU device; normal and build dependencies may
not use this exception. Scene, interaction and app cannot import GPU or UI toolkit families.
The native UI composes these layers; the legacy Tauri host consumes app and
the foundations. Headless replay and ingest consume app, never either UI host.
Kernelbench remains independent; the worker stub consumes protocol.

Scene, interaction, render and UI packages are workspace members. Except for
the renderer's development-only shader validator, all dependency kinds share
the same rules. Toolkit/platform and GPU family checks
apply to every non-host package; library also rejects network client families.

Run `bun test scripts/tests/check-rust-layers.test.mjs` for fixture coverage.
The check enforces direct declarations, not the full third-party transitive
dependency closure. New toolkit families require a corresponding checker rule.
