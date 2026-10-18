# Repository guidance

Read [AGENTS.md](AGENTS.md) before work. It is the canonical operational guide
for this repository, including current native workspace structure, commands,
validation requirements, scope and project-management rules. Do not restore the
retired React/Tauri, Three.js, comparison or prototype renderer build paths.

OneCAD now uses the native `onecad-ui` host and shared
`onecad-render::Renderer`. Backend services and migrated regression suites live
in `crates/onecad-app`. Theme/icon assets are under `assets/native`, renderer
fixtures under `fixtures/render`, and worker staging under
`packaging/binaries`. This consolidation does not establish production cutover,
workflow parity with the retired editor or appearance acceptance.

## Sources of authority

- [OCW1 schema](protocol/SCHEMA.md) and [MESH1 format](protocol/mesh_format.md)
  define wire contracts for Rust and C++ implementations.
- [Architecture](docs/ARCHITECTURE.md), [dependency layers](docs/rust-dependency-layers.md)
  and `docs/adr/` define code boundaries and their rationale.
- The existing private [ONECAD Plane project](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/issues/)
  owns current project status and durable knowledge. Consult relevant work items
  and canonical Pages before substantial work. Obtain approval before saving.
- [Packaging](docs/PACKAGING.md), [debugging](docs/DEBUGGING.md),
  [native host instructions](crates/onecad-ui/README.md) and crate documentation
  own version-specific setup and technical procedures.

Historical design assessments, screenshots and old session notes are evidence
for their recorded revision, not current executable instructions. The frozen
`corpus/` oracle is read-only. Generated knowledge graphs and agent memory are
not authoritative and may be stale.

## Evidence and scope

Preserve unrelated changes. Never commit, push or pull without explicit user
instructions. Reproduce failures before fixing their owning layer. Report only
gates actually run, with measured counts and exact failures or missing
qualification. A command's success does not prove displayed geometry or repaint
acknowledgement. Required real-worker tests use `ONECAD_REQUIRE_WORKER=1`;
required Metal qualification uses `ONECAD_REQUIRE_GPU=1` and fails without GPU
hardware. Do not infer appearance acceptance from CPU tests or changed pixels.

Use proportionate validation and bounded concurrency. Avoid attributing timing
regressions from competing heavy jobs without isolation. Keep preview,
manipulator, commit, geometry, repair, undo/redo and save/reopen evidence distinct.
Keep source-coupled requirements in the linked Git documentation and tests;
propose approved outcomes and future tasks for existing Plane records.
