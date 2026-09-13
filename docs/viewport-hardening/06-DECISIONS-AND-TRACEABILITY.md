# Fixed decisions, review traceability, and change control

**Program:** OneCAD VP-HARDENING 1.0  
**Authority:** These are the chosen target decisions. They are not a menu of alternatives for the implementer.  
**Application execution status:** No implementation or native acceptance is asserted by this document.

## 1. Architecture decisions

Create corresponding short repository ADRs during WP00/WP implementation. Preserve existing ADRs and link them; do not renumber unrelated records. The package IDs below are stable even if repository ADR filenames use another numbering convention.

| Decision | Selected approach | Reason and rejected alternative | Owner |
|---|---|---|---|
| D01 — existing stack | Keep imperative Three.js + React/Tauri/Rust/OCCT architecture. | Defects are contracts and algorithms, not evidence that another framework is required. Reject a renderer rewrite or R3F migration. | WP00 |
| D02 — supported backend | WebGL2 production; full feature matrix before another backend can be enabled. | Initialization is not feature parity. Reject experimental WebGPU as the default quality upgrade. | WP02 |
| D03 — screen units | CSS linewidth and logical viewport resolution for screen-space fat lines; device pixels only where explicitly required. | Match the installed draw callback and picker. Reject DPR multipliers at module initialization. | WP01 |
| D04 — scheduling | Dirty reasons consumed before callbacks; single event-driven rAF; submit/present distinction. | No lost redraw and no idle loop. Reject permanent rendering to conceal invalidation bugs. | WP02 |
| D05 — resource ownership | Registry leases; exact shared geometry object for whole-body overlays; compact owned geometry for face/edge overlays; bounded LRU. | Bounded lifecycle without unsafe shared-attribute disposal. Reject never-disposed wrappers and undocumented detach/dispose hacks. | WP03 |
| D06 — validation | Structural parsing followed by branded semantic validation and memory admission before installation. | Well-sized bytes are not necessarily safe geometry. Reject trusting counts/ranges after header validation. | WP04 |
| D07 — stale display | Failed replacement keeps historical geometry inspection-only, explicitly stale and non-actionable. | Preserve user orientation without presenting old geometry as a new publication. Reject blanking valid prior geometry or silently promoting stale IDs. | WP04/WP07 |
| D08 — appearance | Authored data plus pure derived view-state precedence; no nested save/restore state. | Avoid material-instance omissions and collisions. Assembly mode deliberately overrides display colors but never stored appearance. | WP05 |
| D09 — sketch focus | Opaque desaturation/value dimming; active sketch ink deliberately x-ray; no general transparent-body mode in this program. | Predictable depth and section caps. Reject opacity-based focus as an accidental transparency system. | WP05/WP13 |
| D10 — normals | Underlying surface UV normals with orientation/transform policy and explicit singular fallbacks. | Shading should reflect the surface, not incidental triangle density. Reject +Z fallback and welding across all faces. | WP09 |
| D11 — model camera | New views use 35° FOV with 20°–76° control; Z-up turntable; old view migration preserves prior appearance. | Reduce excessive perspective without invalidating existing saved views. This is a product default, not a claim about another CAD product. | WP06 |
| D12 — sketch camera | Full quaternion/basis state; exact orthographic plane X-right/Y-up; restore complete prior view. | No-roll model orbit cannot represent every sketch basis. Reject special-case axis swapping. | WP06 |
| D13 — input arbitration | Explicit UI/tool/nav owner; freeze navigation during active manipulation; armed tools permit inspection. | No moving-camera parameter jumps. Reject inconsistent orbit-only suppression. | WP06/WP15 |
| D14 — edge acquisition | CSS-radius candidate search plus candidate-matched visibility ray; numerical depth tolerance separate. | Easy selection without rear-edge false preference. Reject screen-radius-sized depth bias. | WP07 |
| D15 — exact curves | Positive rational Bézier spans, homogeneous subdivision, finite-chord hull bound, derivative-cone angular criterion. | Catch arbitrary spline excursions; retain an auditable certificate. Reject midpoint/endpoint heuristics. | WP08 |
| D16 — unsupported curves | Fixed OCCT approximate conversion with allocated error budget and estimated status. | Preserve useful support without inventing a mathematical proof. Reject silently treating approximate conversion as exact. | WP08 |
| D17 — sketch quality | One per-entity projected-curve policy across active/static/draft/fill channels with refine/coarsen hysteresis and cap status. | No fixed-count disagreement or global-maximum saturation. Reject changing solver/snap tolerance to match drawing. | WP12 |
| D18 — sketch performance | Stable semantic handles, 4,096-segment batches, dirty-range updates, reused previews. | Hover must not rebuild unchanged geometry. Reject one new GPU geometry per entity on every hover. | WP11 |
| D19 — edge meaning | Hard/open/unknown visible; only verified tangent/seam boundaries suppressed; all-topology mode explicit. | Reduce visual clutter without hiding potential creases or destroying edge identities. | WP13 |
| D20 — section composition | Effective displayed solid set includes exact previews; per-certified-solid stencil/cap/clear; sheets are uncapped. | Avoid missing preview caps and cancellation between independent solids. Caps are visualization, not new editable topology. | WP13 |
| D21 — render policy | Surfaces/caps before depth-tested lines; annotations never write depth; active sketch x-ray explicit. | Predictable layering across modes and depth conventions. Reject magic render-order numbers as a substitute for policy. | WP13 |
| D22 — coordinate precision | Subtract body-local float64 origin before float32 storage; one frame render-origin mapping. | Camera changes cannot recover quantized world coordinates. Reject reinterpreting v1 payloads or changing document axes. | WP10 |
| D23 — wire migration | MESH1 family version 2, dual decoder, negotiated production enablement, independently versioned quality/cache keys. | Persisted and cached v1 retains exact old meaning. No mutation of user BRep/history for a display cache migration. | WP10 |
| D24 — view detail | Quantized quality requests, hysteresis, cancellable/latest-wins jobs, explicit limited/estimated state. | Improve useful display fidelity without one native tessellation per orbit frame. | WP12 |
| D25 — acceleration | Local adapter for indirect triangle BVH plus conservative body/segment bounds; preserve original ordinals. | Scale ordinary picking without corrupting IDs. No global Three.js monkeypatch or unchecked index reorder. | WP14 |
| D26 — colors/memory | Correct face ordinal lookup, indexed face-constant colors, selective split only on genuine ownership conflict. | Avoid both wrong color assignment and unnecessary full de-indexing. | WP05/WP14 |
| D27 — grid | Projected metric, stable 1/2/5 spacing, bounded geometry, separate grid visibility and snap preference. | Consistent view/snap semantics without infinite unstable grids. | WP15 |
| D28 — qualification | Mathematical, browser-GPU, native, physical-device, and performance evidence tracked separately. | No screenshot flag or mock suite proves native geometric correctness. | WP00/WP16 |

## 2. Review-to-implementation mapping

The original review is preserved in [reference/original-rendering-review.md](reference/original-rendering-review.md). R18 was identified while preparing this specification and is documented below. Existing review priority labels are historical; the work-package dependencies govern execution.

| Finding | Required change | Requirements | Work packages | Main acceptance IDs |
|---|---|---|---|---|
| R01 | Line units, current metrics, DPI-change events | VP03 | WP01/WP15 | LINE-01–04; LIFE-04 |
| R02 | Highlight leases, owned slices, bounded caches and disposal | VP04 | WP03 | RES-01–05 |
| R03 | Robust exact-span edge sampling and typed cap failure | VP11 | WP08 | CURVE-01–07 |
| R04 | Surface normals and singular/winding policy | VP10 | WP09 | NORMAL-01–06 |
| R05 | Derived material state and assembly precedence | VP06 | WP05 | APP-01–05 |
| R06 | Reentrant invalidation and actual-submit correlation | VP02 | WP02 | LIFE-01–05 |
| R07 | Canonical camera, resize, anchored zoom, tween cancellation, drag ownership | VP07/VP08 | WP06 | CAM-01–08; INPUT-01–04 |
| R08 | Stable sketch entities/batches and reused transients | VP13 | WP11 | SK-01–04; RES-04 |
| R09 | Shared adaptive curve/fill policy and per-entity quality | VP12 | WP12 | SK-05–08; LOD-01–05 |
| R10 | Independent acquisition and visibility; typed pick-through | VP09 | WP07 | PICK-01–07 |
| R11 | Semantic validation, admission, diagnostic completeness | VP05 | WP04/WP09 | MESH-01–06; PUB-01–04 |
| R12 | Qualified envelope, adaptive clipping, local origins, controlled detail | VP07/VP17/VP18 | WP06/WP10/WP12 | PREC-01–05; PROTO-01–06; LOD-01–05 |
| R13 | Verified edge categories; explicit all-topology inspection | VP14 | WP13 | EDGE-01–04 |
| R14 | Opaque focus, effective displayed solids, cap/preview/depth policy | VP06/VP15/VP16 | WP05/WP13 | SECTION-01–07; PREVIEW-01–04 |
| R15 | Indexed colors, worker preparation, topology-safe acceleration | VP06/VP19 | WP05/WP14 | COLOR-01–03; BVH-01–05; PERF-01–05 |
| R16 | Full plane basis, input coordinator, touch centroid, grid metric | VP07/VP08/VP20 | WP06/WP15 | CAM-05; INPUT-01–08; GRID-01–04 |
| R17 | WebGL capability baseline, guarded backend settings, capture evidence | VP01/VP02 | WP02/WP15 | BACKEND-01–03; CAPTURE-01–03 |
| R18 | Face-color override lookup uses face ordinal, not triangle index | VP06 | WP05 | COLOR-01–03 |

Table shorthand omits the `TEST-` prefix only for readability. Execution artifacts use full IDs such as `TEST-COLOR-01`.

## 3. R18 — additional confirmed source defect

At the pinned baseline, `bakeFaceColors()` in `src/viewport/mesh/faceColors.ts` iterates face ordinal `f` and calls `topo.idAt(f)`.

`TopoIndex.idAt()` takes an element index, specifically a triangle or segment index, and first converts it to an owning ordinal. `TopoIndex.idOf()` accepts the ordinal directly. Therefore the loop must use `idOf(f)` after the face count/ranges are validated. [REP-06 and REP-07.]

Minimal example:

```text
Face ranges: face A = firstTri 0, triCount 2
             face B = firstTri 2, triCount 7
             face C = firstTri 9, triCount 1

Incorrect idAt(1): triangle 1 belongs to face A
Correct   idOf(1): face ordinal 1 is face B
```

The previous display-color policy did not detect this because one-triangle-per-face fixtures make the two indices accidentally equal. Do not fix it only by rewriting test fixtures. Add the unequal-count and zero-triangle-face cases before the production patch, then preserve that distinction in any color-buffer optimization.

This is a source-confirmed indexing defect. No native screenshot of its effect was captured while writing this package.

## 4. Requirement-to-module closure

| Requirement | Accountable integration owner | Completion evidence |
|---|---|---|
| VP01 backend | WP02 | Production preference/capability tests, no missing layers |
| VP02 lifecycle | WP02 | Idle, reentrant invalidation, context recovery, submit truth |
| VP03 metrics | WP01 | Actual CSS widths and current resolution across display changes |
| VP04 ownership | WP03 | Lease and renderer resource soak |
| VP05 mesh validity | WP04 | Semantic fuzz/admission plus stale-display restrictions |
| VP06 appearance | WP05 | Material precedence and R18 color identity |
| VP07 camera | WP06 | Projection, safe fit, limits, exact plane state |
| VP08 input | WP06/WP15 | Reducer tests and declared physical evidence |
| VP09 picking | WP07 | Visible acquired topology and freshness proofs |
| VP10 normals | WP09 | Analytic/reference vectors and native shading gallery |
| VP11 curves | WP08 | Certificate-aware sampler and adversarial native curves |
| VP12 sketch quality | WP12 | Per-entity projected bounds and fill agreement |
| VP13 incremental sketches | WP11 | Stable geometry and bounded upload on hover |
| VP14 feature edges | WP13 | Correct classes, conservative unknown handling |
| VP15 effective display | WP13 | Exact replacement/cancel/visibility behavior |
| VP16 render order | WP13 | Pixel/picking parity under section and depth modes |
| VP17 origins/wire | WP10 | Three-language fixtures and far-origin operations |
| VP18 refinement | WP12 | Fenced coalesced jobs, qualified/limited status |
| VP19 scalability | WP14 | Topology-preserving BVH and main/stress measurements |
| VP20 finishing | WP15 | Grid/annotation/capture tests |
| VP21 acceptance | WP16 | All gate states and qualified platform report |

## 5. Controlled engineering deviations

The implementer must not blindly implement a contradiction. Correctness outranks literal execution of a disproven assumption, but a departure requires evidence.

A deviation record contains: decision ID, exact contradiction, minimal source or numeric counterexample, affected requirements/tests, proposed narrow correction, impact on stored/wire data, and verification method. Store it in the execution ledger and mark affected acceptance blocked until resolved. Do not erase the original text or pretend a failed approach was never attempted.

Allowed local adaptations without a product-level deviation: symbol/file naming that matches the current codebase; using an already equivalent service; exact dependency/build target selection after checking installed APIs; safer checked arithmetic; test isolation; improving an error message without changing its semantics.

Require a deviation for: relaxing error or memory budgets; changing visible/selectable topology; changing transparency/default camera behavior; changing wire bytes/semantics; replacing the chosen ownership or curve algorithm; certifying an estimated bound; removing a mandatory gate; broad dependency upgrades.

A missing dependency, native device, or time-consuming build is an evidence or environment constraint, not permission to reduce mathematical correctness. Complete independently testable work and leave a precise blocked state rather than inventing success.
