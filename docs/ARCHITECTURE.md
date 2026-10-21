# OneCAD architecture

OneCAD is a native Rust desktop application using egui/eframe and wgpu, with a
C++20 OCCT/PlaneGCS geometry sidecar. The root `Cargo.toml` and `Cargo.lock` own
the workspace. This document describes current code boundaries and invariants;
project status, research and future decisions live in the existing
[ONECAD Plane project](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/issues/).
The [documentation index](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/pages/0bfe867d-215a-4cfb-a501-9f628976bc9d)
and [migration audit](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/pages/a0db33f3-7c7f-4bd6-bc1d-0c2c9ca549d1)
identify canonical project knowledge. Historical ADRs record design provenance;
they do not prove native qualification or define an active TypeScript SDK.

## Workspace boundaries

| Crate | Responsibility |
|---|---|
| `onecad-core` | Pure domain model, document/history types, expressions and recovery contracts |
| `onecad-protocol` | OCW1 transport and checked MESH1 decoding |
| `onecad-library` | Library domain data and operations |
| `onecad-app` | Shared application services, authoritative document runtime, regeneration, worker supervision, persistence, import/export and library operations |
| `onecad-scene` | Bounded immutable CPU geometry, captured camera/viewport identity, synchronous picking and fit calculations |
| `onecad-interaction` | Pure interaction transitions and tagged application effects |
| `onecad-render` | Sole CAD renderer, using the host's shared wgpu device and queue |
| `onecad-widgets` | Presentation widgets, native theme, fonts and baked icons |
| `onecad-ui` | Native eframe host, platform integration, presentation and bounded execution of application effects |
| `onecad-mesh-fixture` | Headless fixture generation through shared application services |
| `onecad-regen`, `onecad-library-ingest` | Headless consumers of application services |
| `onecad-kernelbench` | Bounded kernel benchmark supervision, separate from the product worker protocol |
| `onecad-worker-stub` | Protocol test worker |
| `xtask` | Build support, repository checks, generated assets and strict evidence validation |

`cargo xtask check-rust-layers` enforces the direct dependency graph across
normal, development, build, optional, renamed and target-specific declarations.
See [dependency layers](rust-dependency-layers.md). `onecad-app` must not depend
on concrete UI, rendering, scene or interaction implementations. Scene and
interaction remain independent of GPU and UI toolkits. Rendering remains
independent of UI toolkits. Tauri, Wry and WebKit dependencies are forbidden
throughout the workspace, including the host.

## Document and history authority

Only `onecad-app` owns document mutations. The per-document single-writer
`DocumentRuntime` separates locked preparation, unlocked worker execution and
fenced commit. Preserve latest-only preparation, stale-result fencing and
application-issued rollback tokens. `workerEpoch` and `expectedBaseHash` fence
results; `documentRevision` is advisory.

The native host and automation call the same application services. Neither
scene construction, rendering nor an interaction reducer provides an alternate
document mutation path. Authoring acceptance, regeneration completion,
per-feature failure and `NeedsRepair` are distinct. A failed accepted edit is
not automatically removed: provisional rollback requires its guarded token.
Deterministic `NeedsRepair` takes precedence over a silent incorrect binding.

Profile selections and previews carry captured document, runtime, revision,
snapshot, gesture and operation identities. Stale replies are rejected. New
feature preview and commit share one operation id. An existing-feature preview
requires an authoritative rollback-prefix preview rather than borrowing the
new-feature path.

Persistence belongs to application services and the core I/O contracts. Archive
entry, size, path and decompression limits apply throughout the container.
Unknown module-owned state is preserved according to the existing serialization
contracts. This architecture does not authorize file-format or production-data
migration. Settings and document state remain separate.

## Geometry worker

Production, normal development and required worker CI use the C++20 sidecar.
OCCT is pinned external source; PlaneGCS remains part of the worker. The
optional in-process library experiment is nondefault and does not replace the
sidecar's crash and wedge isolation. Its source/build contract and limitations
remain in the [in-process investigation](design/native-ui/inprocess-worker-spike.md).

Worker stdout contains OCW1 frames only; diagnostics use stderr. Preserve
compatibility checks, worker binary SHA-256, OCCT fingerprint checks, resource
caps and solver stdout protection. Release worker resolution is bundled-only,
beside the executable. Debug resolution uses `ONECAD_WORKER_PATH`, then
`worker/build/onecad-worker`, then the bundled worker. Packaging rewrites and
signs nested binaries before recording their final hashes.

[OCW1](../protocol/SCHEMA.md) and [MESH1](../protocol/mesh_format.md) are normative.
Changes require compatible Rust/C++ implementations and cross-track fixtures.
Checked decoding precedes traversal or upload. Preserve topology identity,
row ordinals, outward winding, right-handed Z-up coordinates and the
500,000-triangle admission limit.

## Rendering and input

`onecad-render::Renderer` is the sole CAD renderer used by the native host and
headless captures. The host supplies the shared native wgpu device and queue;
the renderer owns its render targets and internal resources. Native integration
samples the output texture without CPU image readback. Resource replacement
waits for CAD completion and final egui sampling fences.

Picking is synchronous against the last drawable camera and scene. Pending
navigation must not change the captured basis for hover, click, profile or gizmo
queries. Scene, camera, viewport, display options, theme and preview changes
invalidate captured picks. Off-thread preparation and asynchronous effects
remain bounded and reject stale identities. View-only clipping must not invent
selectable topology.

Native input routing processes fresh events once and preserves retained
text/IME events for presentation. Native menu, text focus and model history
remain separate routing concerns. Host automation, control and MCP interfaces
remain native application capabilities. Repository cleanup must not change
input semantics, interaction behavior, chrome, display styles or theme values.

Theme data remains in `assets/native/tokens.css`; its CSS syntax is a static
input, not a web application runtime. Baked icon data and native application
icons live under `assets/native`. Checked renderer fixtures and provenance live
under `fixtures/render`.

## Qualification and maintenance

See [native host instructions](../crates/onecad-ui/README.md),
[packaging](PACKAGING.md), [debugging](DEBUGGING.md) and
[manual release procedures](qa/MANUAL_RELEASE_GATES.md).
Required worker tests use `ONECAD_REQUIRE_WORKER=1`; required Metal tests use
`ONECAD_REQUIRE_GPU=1`. Missing prerequisites fail qualification. Exact required
test inventories, zero skipped required tests, artifact identities, relative
paths, hashes and negative controls remain strict gates.

CPU tests do not prove appearance. GPU submission or completion does not prove
OS presentation. Cleanup, compilation and synthetic host tests do not establish
From-New modeling, physical input, IME, display, product or release acceptance.
