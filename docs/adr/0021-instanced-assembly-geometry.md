# 0021 — Assembly geometry is shared in the scene, local in MESH1

- Status: Bounded implementation decision (2026-11-09). Delivery and qualification remain in Plane (ONECAD-342); this is not a gate-completion claim.
- Date: 2026-11-04
- Related: ADR-0020 (occurrences), ADR-0022 (joint solver hosting).

## Decision and current implementation

MESH1 remains definition-local, Z-up and right-handed; instance placement is
host state, not a MESH1 transform section. `crates/onecad-ui/src/capture/assembly.rs`
captures visible definition/body geometry once and attaches per-occurrence path,
rigid model transform and display identity. `crates/onecad-scene/src/scene.rs`
shares checked geometry and its pick indices among instances. Selection uses
scene/capture fences and returns occurrence identity with the definition-local
body and topology identity; `crates/onecad-scene/src/picking.rs` transforms rays
into local coordinates for face picking. Visibility is per occurrence.

The current renderer shares prepared geometry and GPU mesh buffers, but does
**not** use a per-instance model-matrix vertex buffer for body draws. Each
rendered body occurrence has a model uniform/bind group and is drawn separately;
see `crates/onecad-render/src/{mesh.rs,prepared.rs,passes.rs}` and
`crates/onecad-render/src/shaders/body.wgsl`. This distinguishes shared geometry
storage from an instanced single draw call. Transform updates still incur host
capture/preparation and renderer work; no no-worker or latency guarantee follows
from storage sharing.

## Admission and limitations

`SceneSnapshot` accounts shared geometry once for unique resident geometry and
also counts **visible** triangles and edge segments per occurrence. Both unique
and visible counts have 500,000 limits under default scene limits; renderer
preparation separately checks visible draw work. Component replay applies its
own aggregate per-definition geometry and residency caps. UI capture also limits
instance bodies and source/preparation allocations. See
`crates/onecad-scene/src/scene.rs`, `crates/onecad-render/src/prepared.rs`,
`crates/onecad-app/src/document_runtime/component_execution.rs`, and
`crates/onecad-ui/src/{capture/assembly.rs,budget.rs}`.

The shared CPU host budget is 768 MiB, with 192 MiB active preview headroom for
Mobile. The separate renderer and parked-cache accounting is partial, with its
own profile allowances; neither value is a measured process-memory ceiling. The
limits of 2000 occurrence entries and path depth 8 are structural bounds, while
current embedded assembly authoring is flat. They are not a promise that 2000
visible instances fit the draw, body, memory or solver budgets.

Tests cover shared geometry, per-occurrence transforms and selection, as well as
bounded host capture: see `crates/onecad-render/src/tests/occurrences.rs`,
`crates/onecad-scene/src/instances.rs`, and
`crates/onecad-ui/src/capture_pressure_tests.rs`. A 10-definition,
20-occurrence initial workload is an evidence case, not a general performance
pass or a guarantee of physical-device behavior.

Pre-transforming each mesh in the worker, embedding instance transforms in
MESH1, or counting only unique geometry while ignoring visible work would lose
these ownership or admission properties.

## Open questions

- What draw and memory workloads are qualified on each target GPU and device?
- When would culling or batched instance draws be justified by measurements?
- Which additional section, hidden-line and nested-assembly workflows need evidence?
