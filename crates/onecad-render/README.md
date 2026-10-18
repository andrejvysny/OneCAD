# onecad-render

This crate renders checked CAD geometry to private sampled wgpu targets. It has no egui,
Tauri, window, device-creation, or production GPU-readback dependency. The host supplies its
existing wgpu 30.0.1 `Device` and `Queue`; cloned handles refer to that same device. The host
owns the surface, event loop, submission order, and UI composition. Recreate the renderer
when the shared device is lost.

## Full SolidCraft viewport

The native host uses `SolidCraftViewport`, backed by the complete vendored
SolidCraft `geom`, `render` and `gpu` source at
`a228a5830eedf88856fd392b7db0ff5977f352d6` (0.16.0). See
[vendor provenance](../../vendor/solidcraft-viewport/README.md) for source hashes,
licenses and excluded engine-dependent tests. The original quality `Renderer`
described below remains available; it is not the primary native viewport.

Prepare checked OneCAD meshes with `PreparedSolidCraftScene` on a service
thread, using `SolidCraftPreparationBudget`. The adapter preserves captured
camera matrices, scene/body/revision identity, preview generations and CPU
picking authority. It renders into private 4x-MSAA color/depth targets and a
single-sample output on the host's shared device. Poll completion before changing
resources; submit before egui samples the output, then fence the final sampling
submission. Retired geometry and highlight resources remain charged until their
submission completes. Supersampling and accessibility-map cache high-water
allocations remain charged while the donor retains them.

The copied stack includes section/stencil, transparency, hidden edges, analysis,
PBR, image and supersampling APIs. The native Display popup currently exposes
only silhouettes, hidden edges, opacity, supersampling and section controls.
Copying these APIs does not expose every donor tool or replace OCCT/PlaneGCS.

The explicit hardware smoke test covers both projections, checked geometry,
highlight/preview updates, section clipping, opacity, supersampling, resize and
completion fences on a real shared device:

```sh
cargo test -p onecad-render headless_full_solidcraft_viewport_shared_device -- --ignored --nocapture
```

Captures are diagnostic, not accepted visual goldens. Native input and full
frontend migration remain separate gates.

## Host integration

1. Construct `Renderer::new(&device, &queue, physical_size, limits)`. Startup IBL work has an
   asynchronous completion fence. Use `poll_idle()` without blocking; request another
   on-demand frame while startup completes.
2. Copy `renderer.preparation_budget()` to a background worker. Build
   `PreparedScene::new(&scene_snapshot, budget)` there. This owns bounded arrays and computes
   deterministic silhouette candidates without wgpu. Keep at most one latest pending CPU
   preparation; a stale preparation must be dropped before preparing its replacement.
3. When `poll_idle()` is true, consume the preparation with `replace_prepared_scene`, or use
   `replace_prepared_preview(PreviewIdentity { base, generation }, prepared)`. GPU upload
   rechecks current budgets before allocation. Check readiness before taking the host's
   pending preparation: the by-value upload API consumes its argument on error.
4. `encode(&mut encoder, &view_snapshot, style)` checks committed scene/session identity and
   the exact rounded physical viewport, then records CAD passes. Submit this encoder on the
   shared queue before the UI pass samples `output_view()`. Register that view as the UI's
   native texture; remove its old registration when replacing the target.
5. Call `after_submit(wake)` only after **all** submissions using the frame, including the
   UI pass sampling the output. With eframe, the next frame's raw input hook occurs after the
   prior egui submission and can register this fence. Calling immediately after the CAD
   submission alone is insufficient. The completion callback should wake the on-demand loop.
6. A second encode, resize, geometry replacement, or preview clearing returns `Busy` until
   completion. This bounds the renderer to one outstanding frame, prevents uniform writes
   from overwriting an earlier capture, and fences replacement resources. If an encoded frame
   is discarded, drop its encoder and call `abandon_frame(wake)` to flush/fence queued writes.

No production method waits for GPU completion. The convenience `replace_scene` and
`replace_preview` methods also perform CPU preparation; native UI code should use the
prepared APIs to keep geometry packing off the UI thread. The host must not retain old
encoders, texture registrations, or sampled views after the corresponding completed frame;
those externally retained resources cannot be bounded by this crate.

Zero-width or zero-height resize suspends CAD targets and `encode` records no commands.
An over-budget resize/preparation/upload leaves committed GPU geometry and target size
unchanged. Transient preview geometry has separate resources and identity; it never replaces
committed geometry or contributes to committed AO/contact shadows. Preview generations must
increase; clearing retains the generation tombstone so late results cannot resurrect a
cancelled preview.

## Camera and look

Render and CPU queries consume `onecad_scene::ViewSnapshot`: its exact view/projection,
near/far clipping, viewport, scale, and scene/session identity. Z-up MESH1 positions are
uploaded unchanged. This slice requires a checked normal section and refuses meshes without
normals rather than silently inventing shading. No scene-radius camera, axis rotation, or
renderer-owned camera exists.
The renderer converts captured f64 matrices to f32 and rejects non-finite conversion; it
does not restore precision already lost in mesh coordinates.

Both perspective and orthographic AO reconstruct view positions through the captured inverse
projection. Analytic feature lines clip at the captured finite near/far planes. The quality
pipeline uses depth/normal prepass, GTAO and bilateral blur, contact height/occlusion and blur,
procedural studio IBL, neutral tone mapping, analytic ground grid, MSAA 4x, and blended analytic
feature/silhouette lines. Geometric silhouettes are the default. Sorted CPU edge candidates
and vertex culling replace the spike's nondeterministic atomic append. A fragment-silhouette
option remains available for comparison. This change requires image qualification; it does
not establish parity with the approved spike captures.

Colors come from embedded `src/styles/tokens.css`, including both themes. Whole-body
selection applies the existing accent tint at 0.45 and a 3-logical-pixel selected outline,
following `src/viewport/engine/HighlightLayer.ts`. Separate preview faces use accent at 0.3,
following `PreviewMesh.ts`. Snapshot-scoped face and edge selection/hover use separate
flat alpha overlays, as described below. These subset styles do not cover cut tint,
imported face colors, wire modes, section caps, sketches, annotations, or the full overlay
ladder. Renderer-only visibility was deliberately omitted: hiding must be represented by the
same immutable scene used for CPU picking.


## Snapshot-scoped topology highlights

`HighlightTarget` carries the exact checked topology ID, kind, body ID, mesh revision, and
`SceneIdentity`. `HighlightTarget::from_pick` preserves these fields from the existing CPU
`PickHit`; this crate creates no new picking authority. A service thread calls
`PreparedHighlights::new(&scene, selected, hover, renderer.preparation_budget())`. It resolves
checked face spans into scalar index ranges and edge point spans into the segment ranges
already uploaded for that body. Unknown/ambiguous IDs, wrong bodies, changed scenes/revisions,
empty ranges, overflow, and out-of-bounds ranges are errors. Duplicate selected targets collapse;
selected targets suppress a matching hover. The resulting ranges are private, snapshot-scoped
render caches, never persistent triangle identities.

When idle, `replace_highlights(prepared)` rechecks scene identity, body order/identity, mesh
revision, range units and bounds. `clear_highlights()` also requires the frame completion
fence. Committed scene replacement clears all highlight caches. Preview replacement/clearing
retains committed highlights and includes them in memory accounting. Both preparation and
replacement use the existing `PreparationBudget`/`RenderLimits`; `max_highlights` defaults to
4096 input targets including hover. Metadata, two shared uniform buffers, per-frame writes,
and old/new replacement overlap are charged before allocation. No geometry copies or new
picking indexes are required. The host must refresh hover from its authoritative CPU capture
when the view changes; scene-scoped selected IDs remain valid while that scene is unchanged.

Faces use unlit accent selection at 0.55 or cyan hover at 0.45, alpha blending, depth testing,
no depth writes, and the existing OneCAD forward-depth polygon offset of -1/-1. Selected and
hovered edges use 3 logical pixels and ignore depth, matching `HighlightLayer.ts`. Whole-body
selection stays at 0.45. The donor's -8 constant exists for separate CPU/GPU-plan geometry;
identical shared committed buffers do not need that stronger offset. Face overlays precede
feature lines; highlighted edges follow the separate translucent preview batch. Near/far
clipping and camera matrices remain those of the captured view. Section clipping, cut-ghost
and xray tool-body preview variants are not implemented in this slice.

## Resource bounds

`RenderLimits` explicitly caps body count, texture dimension, CPU preparation, per-call upload,
GPU payload, and combined source/preparation/staging/GPU replacement peaks. Every count and
byte multiplication is checked before preparation or GPU allocation. Estimates include both
committed and transient CPU mesh sources, old/new geometry and targets during replacement,
per-body uniforms and metadata, deterministic silhouette sorting scratch, upload staging,
frame staging, and the temporary IBL radiance cube. Empty GPU buffers are charged at their
actual 64-byte minimum. Target estimates include every MSAA, depth, normal, AO, and contact
attachment. The one-frame completion contract prevents repeated calls from retaining an
unbounded chain of retired renderer resources or uniform staging.

These are conservative payload estimates, not a process-RSS or driver-allocation limit.
Portable wgpu does not expose pipeline compiler memory, driver metadata, allocation alignment,
or memory retained by the host/UI. Those costs require measured host limits. A preparation
budget is a snapshot: the host must keep only one pending preparation and the GPU upload
revalidates it against the then-current renderer state.

## Reuse and local changes

The original spike remains unchanged. Adaptations originate from:

| Source | New destination | Local changes |
| --- | --- | --- |
| `spikes/render-look/src/gpu.rs` | `src/gpu.rs` | Borrow shared host handles; remove adapter/device factory, polling wait and benchmark helpers. |
| `spikes/render-look/src/render.rs`, `src/render/passes.rs` | `src/{pipelines,targets,passes}.rs` | Quality-only private target ladder; per-body selection/preview; sampled output; forward finite-depth capture; remove benchmark variants/readback/atomic compaction. |
| `spikes/render-look/src/look.rs` | `src/frame.rs` | Consume immutable captured camera; exact physical target dimensions; both projection kinds; token selection/preview styles. |
| `spikes/render-look/src/scene.rs` | `src/{mesh,silhouette,prepared}.rs` | Checked protocol mesh views; preflight caps; deterministic sorted geometric edge candidates; off-thread CPU preparation. |
| `spikes/render-look/src/ibl.rs` | `src/ibl.rs` | Shared queue startup; asynchronous host completion fence instead of blocking wait. |
| `spikes/render-look/src/shaders/*.wgsl` | `src/shaders/*.wgsl` | Preserve quality formulas; inverse-projection AO; captured finite near/far; orthographic lighting/silhouette directions; selected tint and preview shader. |

The spike's upstream SolidCraft, XeGTAO, and Khronos derivations remain applicable. See
`THIRD_PARTY_NOTICES.md` and the root `ATTRIBUTION.md`. Root attribution must cover the new
shader paths, depth-bias pipeline, and geometric silhouette adaptation before distribution.

## Validation and remaining gates

`cargo test -p onecad-render` runs CPU preparation, budget, camera, and deterministic-candidate
checks. The real-GPU test is explicit:

```sh
cargo test -p onecad-render headless_quality_shared_device_capture -- --ignored --nocapture
```

It requires a real adapter, captures both themes/projections under
`/private/tmp/onecad-render-quality-smoke`, checks opaque/nonuniform output and wgpu validation,
and exercises capture rejection, preview isolation/tombstones, zero-size resize, rejection
without mutation, frame completion, and dropped-encoder recovery. Test readback is confined
to `src/tests.rs`. Native eframe integration, complete mounting-plate workflow, performance,
determinism tolerances, and accepted-image comparison remain separate gates. No images here
are accepted goldens.

## Historical topology overlay increment (quality Renderer)

Owner: OneCAD native renderer (`onecad-render`). Optional host/service-thread integration is
owned by `onecad-ui`; this increment does not claim that integration or native acceptance.

Donor checkout was inspected read-only and clean at
`6bde126a5cabd1f5af0c68e6955bd0176fab805a`, origin
`https://github.com/tangency-labs/solidcraft.git`. The donor declares `MIT OR Apache-2.0`;
this adaptation uses its MIT option and retains its copyright/permission notice. Its egui
0.36 and vendor patches were not imported; OneCAD remains on egui 0.37 and wgpu 30.0.1.

| Exact donor path at that revision | Local destination | Reused behavior and local changes |
| --- | --- | --- |
| `crates/gpu/src/view.rs::highlight_depth`, `HIGHLIGHT_BIAS`, `Renderer::draw`, `GpuScene` | `src/pipelines.rs::highlight_depth`, `src/passes.rs::draw_highlights`, `src/highlights.rs` | Separate small highlight/preview batches; tested alpha face overlays without depth writes. Adapted to checked MESH1 spans and existing per-body buffers, immutable scene/body revision guards, OneCAD -1/-1 offset and depth-independent edge highlight semantics; bounded CPU metadata/two shared uniforms with existing submission fences. |
| `assets/shaders/viewport.wgsl::shade` unlit selection branch | `src/shaders/body.wgsl::fs_highlight` | Flat overlay colors without PBR/tone mapping; explicit fragment entry and shared frame color replace donor zero-normal sentinel/per-vertex packed colors. |
| `crates/gpu/src/view.rs::GpuScene::{trans,ghost,xray}`, `Renderer::draw` | `src/passes.rs` existing preview batch plus new highlight ordering | Retained committed/preview separation and alpha/no-depth-write preview ordering. Did not import ghost/xray semantics, section stencil, plan evaluation, or the whole donor renderer. |

Pure regression checks in `src/highlight_tests.rs` cover checked face/edge units, deduplication
and selected-over-hover precedence, unknown IDs/body and stale scene/revision rejection,
invalid/overflowed ranges, preparation/uniform/replacement caps, OneCAD opacity/depth semantics,
and CPU WGSL parsing/validation through Naga. No GPU capture or manual acceptance was run for
this increment; the explicit hardware test remains ignored by default.
