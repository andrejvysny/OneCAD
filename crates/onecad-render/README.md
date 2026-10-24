# onecad-render

OneCAD's sole CAD renderer draws checked MESH1 geometry on a host-owned wgpu 30.0.1
`Device` and `Queue`. Native UI and headless captures use this same implementation.
The crate owns no window, application event loop, adapter, picking authority, or production
readback. Recreate it when the shared GPU device is lost.

## Display contracts

`RenderOptions::display_mode` is authoritative; its default is `ShadedEdges`.

| Mode | Surfaces | Model lines |
| --- | --- | --- |
| `Wireframe` | No committed, preview, or highlighted face fills | All topology rows and geometric silhouettes, with equal front/rear visibility |
| `Shaded` | Opaque studio IBL, GTAO and contact shadows | No base topology edges or silhouettes; explicit interaction edge highlights remain visible |
| `ShadedEdges` | Same opaque studio shading | Boundary, sharp, unknown and non-manifold edges plus geometric silhouettes; smooth, seam and degenerate rows suppressed |
| `Xray` | Weighted blended transparency of front, back and internal surfaces; no GTAO/contact shadows | Clean feature edges and silhouettes; nearest-surface depth separates strong visible lines from subdued hidden lines |

Missing optional edge classification remains conservative. Fresh worker tessellation is
required for clean filtering. The all-topology buffer retains original edge ordinals for
wireframe and explicit highlights; the compact feature buffer is a separate charged resource.
Geometric silhouettes are the only silhouette implementation. There is no competing renderer
or fragment-silhouette comparison path.

Colors come from `assets/native/tokens.css`. Wireframe uses the existing contrasting
`--color-body-edge-wire` token. Selection and hover retain existing accents and
3-logical-pixel emphasis. Wireframe face highlights use incident topology polylines when
adjacency exists. Old meshes use geometric triangle perimeters, cancelling shared face
triangle edges. Both paths produce boundaries without fills. Selected IDs suppress matching
hover. Preview surfaces retain their separate accent overlay and one preview-union section cap in filled modes; preview caps never write committed depth. Previews
never replace committed geometry or affect committed studio occlusion.

## Transparency and depth

X-ray uses two single-sample attachments: RGBA32Float weighted color/alpha accumulation and
R32Float multiplicative revealage. Accumulation has no nearest-surface rejection. The final
resolve computes weighted mean color and coverage, composites in linear light into the MSAA
scene target, and leaves output opaque. Section winding contributes cap coverage once into
these same transparency attachments. A separate nearest-surface depth pass classifies edges;
it does not remove internal transparent contributions.

This is approximate weighted blended order-independent transparency, following McGuire and
Bavoil, [JCGT 2(2), 2013](https://jcgt.org/published/0002/02/09/). This implementation uses a
constant weight of 1/16 and clamps each color component to [0,1]. With the fixed
500,000-triangle admission limit plus one cap contribution, each accumulated component is
bounded by 31,250.0625. Revealage underflow represents full coverage. Floating-point rounding
is tested against a declared maximum difference of 2/255 RGB for reversed surface submission
across the complete RGB image, including selected lines. Selected body lines draw after ordinary
body lines so interaction overlays do not inherit scene submission order. This does not promise
exact sorted transparency.

Hosts and tests must request `required_device_features(adapter.features())`. This negotiates
`FLOAT32_BLENDABLE` and optional `DEPTH32FLOAT_STENCIL8`. X-ray rejects devices without enabled
`FLOAT32_BLENDABLE` before options or targets change; there is no low-precision transparency
fallback. Other modes remain usable. Both `Depth32FloatStencil8` and
`Depth24PlusStencil8` preserve reversed-Z (near 1, far 0). CPU picking matrices remain unchanged.
Output is stored as sRGB; `output_view()` exposes a non-sRGB sampling view for egui's existing
gamma-space composition. Internal transparency and downsampling remain linear.

## Host integration and sections

1. Construct `Renderer` with host handles and physical dimensions. Poll startup IBL completion
   through `poll_idle()`; wake the on-demand loop when completion arrives.
2. Copy `preparation_budget()` to a service thread and prepare immutable `PreparedScene`,
   `PreparedHighlights`, and optional `PreparedSectionContours`. Keep only the latest pending
   result; stale scene, preview generation, and plane keys cannot publish.
3. Upload only while idle. Section changes on loaded geometry require
   `set_options_with_section_contours`; active-section scene/preview publications use their
   corresponding `*_with_section_contours` methods. These atomically validate geometry,
   contour identity, budgets and replacement overlap before publication. The convenience
   `replace_scene`/`replace_preview` methods prepare synchronously for headless callers.
4. Call `encode` with the exact immutable scene/view identity and rounded physical viewport.
   Submit its encoder before the host samples `output_view()`.
5. Call `after_submit(wake)` after **all** submissions using the output, including host sampling.
   Encoded or submitted work fences options, targets, geometry and uniforms. If an encoder is
   discarded, drop it before calling `abandon_frame(wake)`.

Frame uniforms are shared by style: normal, selected, preview, selected topology highlight
and hover topology highlight. Body count does not multiply uniform uploads. The normal
binding also feeds postprocessing; the preview binding retains its distinct hidden-edge
opacity. `Renderer::frame_stats()` reports actual uniform write count and bytes of the last
successful encode. Refused and suspended captures leave those statistics unchanged.

Z-up positions, outward winding, topology IDs and ordinals are preserved. Normals are required;
non-finite f64-to-f32 camera conversion is rejected. Captured finite near/far planes and one
section plane clip surfaces, lines, previews and highlights. Bounded CPU triangle-plane
intersections generate nonselectable synthetic contours off-thread. Wireframe draws contours
without caps. Filled modes use winding stencil caps; X-ray adds cap coverage once to OIT.
An active section refuses more than 255 bodies to avoid eight-bit winding wrap.

Zero-sized targets suspend rendering. Rejected resource or option changes preserve previous
publication and options. Geometry buffers survive mode changes. Preview generations increase;
clearing retains cancellation tombstones. No production render call waits for GPU completion.
External retained encoders, UI texture registrations and GPU driver/compiler memory are outside
this crate's ownership and portable accounting.

## Bounds

`RenderLimits` retains existing body, highlight, texture, CPU, upload, GPU and total caps.
Preparation additionally enforces the 500,000-triangle ceiling. Accounting includes source
blobs, sorted silhouette scratch, all/clean edge buffers, expanded face-boundary scratch and
metadata, synthetic contours, three retained body-style uniforms, two shared highlight
uniforms, frame uploads, OIT/MSAA/depth/AO/
contact targets, startup IBL scratch, and old/new replacement overlap. Empty GPU buffers count
at their actual 64-byte minimum. Charges precede expansion and allocation. These are payload
estimates, not measured RSS or driver residency.

## Validation and appearance

```sh
cargo test -p onecad-render
cargo clippy -p onecad-render --all-targets -- -D warnings
cargo test -p onecad-render -- --ignored --nocapture --test-threads=1
```

Hardware qualification requires real Metal and fails if the adapter is unavailable. Run both
depth configurations with float32 blending. Tests cover deterministic preparation and IDs,
old meshes, malformed bounds, cancellation and stale publications, wireframe interiors,
internal X-ray contribution, reversed submission, high overdraw, opacity endpoints, opaque
output, completion fences, rejected changes and geometry-buffer reuse. The NEMA test admits
67 bodies / 381,524 triangles, captures all four modes with both themes/projections, and prints
CPU preparation duration, warm frame completion without readback, capture durations, and payload
accounting. Warm frame completion includes CPU encode/submission and GPU completion; it is not
an isolated GPU timestamp benchmark.

Captures use `ONECAD_RENDER_CAPTURE_DIR`, default `$TMPDIR/onecad-render-quality-smoke`.
New mode tests require substantial changed model regions and at most 2/255 order variation.
Some older feature-presence tests retain their weaker eight-pixel threshold; those checks
alone cannot establish appearance acceptance. Section midpoint checks declare 0.015 linear
RGB tolerance; plane depth checks declare 0.001 tolerance.

No captures are accepted visual goldens. Do not regenerate reference images to turn failures
green. Fixed-camera human review and accepted model/edge-region goldens remain an appearance
gate. Native application workflow parity, production cutover, device-loss recovery and
1M/5M-triangle qualification are not established by these renderer tests.

Attribution for retained SolidCraft, XeGTAO and Khronos-derived techniques remains in
`THIRD_PARTY_NOTICES.md` and repository `ATTRIBUTION.md`; retired comparison vendor code is
not required to retain those permission notices.

### Recoverable target admission

The host may call `resize_reclaiming` or `set_options_reclaiming` only after both
CAD and final host sampling fences drain. These methods preflight the replacement
with the prior output texture still charged, then retire intermediate targets.
Allocation failure retains that output image; `has_targets()` is false and no new
capture may be used for picking or authoring until a later replacement succeeds.
`output_view()` may therefore expose a retained image without drawable targets.
Budget/allocation refusals are recoverable; device/validation errors are distinct.
The native host retries changed dimensions or an explicit Retry action and keeps
Save reachable. It never raises resource caps or silently reduces quality.
