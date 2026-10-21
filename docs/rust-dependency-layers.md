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
The native UI composes these layers. The iPadOS bridge `onecad-ios` is the
second host: it drives the native UI's egui editor without eframe, owns the
UIKit/Metal surface boundary and may use toolkit and GPU families. Nothing
depends on it. `onecad-embedded-worker` only declares and links the in-process
worker C ABI; the application may use it behind its `embedded-kernel` feature. Headless replay
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
