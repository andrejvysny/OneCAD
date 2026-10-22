# Rust dependency layers

`cargo xtask check-rust-layers` enforces ADR-0015 using
`cargo metadata --format-version 1 --no-deps` at the repository root. It reads
every dependency declaration, including build, dev, optional, renamed and
target-specific dependencies, without filtering for the current target or
enabled features. It does not compile crates or require a staged worker.

The explicit package graph lives in the Rust xtask checker. Unregistered
workspace packages and local dependencies outside that workspace fail closed.
A local dependency's package name and path must both match the workspace
member. There are no exceptions for external local comparison renderers; the
SolidCraft comparison vendor is retired.
Dependency aliases are checked by actual package name.

Core, protocol and library remain independent foundations. Application
orchestration consumes those foundations. Scene consumes core and protocol;
interaction consumes public app contracts, scene and core; render consumes
scene, protocol and core. Render may use the wgpu family but cannot import a UI
toolkit. Its development dependencies may also use the exact `naga` package to
validate WGSL without creating a GPU device; normal and build dependencies may
not use this exception. Scene, interaction and app cannot import GPU or UI toolkit families.
The native UI composes these layers and is the only UI host. Headless replay
and ingest consume app; the mesh-fixture utility consumes app, core and protocol.
None imports the native UI.
Kernelbench remains independent; the worker stub consumes protocol.

Scene, interaction, render and UI packages are workspace members. Except for
the renderer's development-only shader validator, all dependency kinds share
the same rules. Toolkit/platform and GPU family checks
apply to every non-host package; library also rejects network client families.

Run `cargo test -p xtask` for fixture and negative-control coverage.
The check enforces direct declarations, not the full third-party transitive
dependency closure. New toolkit families require a corresponding checker rule.

Tauri, Wry and WebKit package families are forbidden throughout the workspace,
including the native host and all dependency kinds. Renaming a dependency does
not change its package identity or bypass this boundary.

Extension contracts are a toolkit-independent foundation (`onecad-extension-api`).
The Rust guest SDK consumes only those contracts; the Bambu Studio addon consumes
that SDK. The extension host owns package admission and sandbox execution without
normal dependencies on document services, UI or rendering. Its app/core development
dependencies exercise real export integration; the checker rejects those edges
for normal and build dependencies. Native UI composes the extension host and API
with application services, so extension side effects retain the existing document
service authority. Adding future contribution adapters must preserve this boundary.
