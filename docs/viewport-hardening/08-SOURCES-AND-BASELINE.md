# Sources, inspected baseline, and evidence limits

**Checked:** 13 September 2026.  
**Repository baseline and reconfirmed remote master:** `65b4c60eb226a201f2fa3eb565d7283b1c63b5ac`, committed 12 September 2026 at 13:27:35 UTC.  
**Commit subject:** Record UX hardening handoff status.

## 1. How to read this package

The source review establishes current mechanisms and known defects at the pinned baseline. The specification selects a future design, constants, supported workloads, and acceptance behavior. These two categories are not interchangeable.

No native OneCAD app, OCCT build, browser-GPU test suite, or repository test suite was executed while authoring these documents. There are no measured current OneCAD FPS, VRAM, device-latency, or cross-platform certification claims in this package. Required tests are implementation deliverables and start as not-run.

The standalone original review contains numerical counterexamples and identifies them as isolated algorithm evidence. A counterexample can disprove a heuristic; it does not itself demonstrate that the entire native application reproduced a screenshot defect.

Current upstream documentation can differ from installed source. Verify exact symbols, defaults, constructor options, and build APIs against the repository's locked dependencies before coding. Keep its Three.js 0.185.1 and OCCT 8.0.1 baseline unless a separately recorded dependency change is necessary.

## 2. Pinned repository sources

All repository links below use the reviewed commit, not a moving branch.

| ID | Source | Why it matters |
|---|---|---|
| REP-01 | [AGENTS.md](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/AGENTS.md) | Four-layer architecture, Bun/build discipline, worker staging, protocol and native-test constraints |
| REP-02 | [package.json](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/package.json) | Three.js pin and frontend/native script entry points |
| REP-03 | [Viewport engine README](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/README.md) | Existing imperative engine, coordinate/depth conventions and known backend limitations; comments remain subject to source verification |
| REP-04 | [Canonical mesh format](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/protocol/mesh_format.md) | Original 64-byte MESH1 header, types 1–12, v1 coordinate and table semantics |
| REP-05 | [Worker CMakeLists](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/worker/CMakeLists.txt) | Exact OCCT 8.0.1 baseline, C++20, artifact/provenance requirements |
| REP-06 | [faceColors.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/mesh/faceColors.ts) | Color conversion/deindexing and additional R18 ordinal error |
| REP-07 | [faceRangeIndex.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/mesh/faceRangeIndex.ts) | Distinct idAt(elementIndex) and idOf(ordinal) contracts |
| REP-08 | [Tessellate.cpp](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/worker/src/tess/Tessellate.cpp) | Existing display deflections, custom edge sampling, triangle-derived normals, world float32 output |
| REP-09 | [bodyMaterials.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/bodyMaterials.ts) | Shared versus assembly-specific materials, dimming, line widths, clipping |
| REP-10 | [renderer.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/renderer.ts) | Renderer construction, context attributes, production and experimental backend behavior |
| REP-11 | [ViewportEngine.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/ViewportEngine.ts) | Scheduling, environment, capture, resolution, section/preview roots and integration seams |
| REP-12 | [meshRegistry.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/mesh/meshRegistry.ts) | Geometry construction, edge expansion, ownership and publication records |
| REP-13 | [Picker.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/Picker.ts) | Acquisition, depth preference, topology mapping, overlap/currentness |
| REP-14 | [SketchObject.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/SketchObject.ts) | Active lines, markers, preview rebuilding and detail policy |
| REP-15 | [SectionLayer.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/SectionLayer.ts) | Stencil/cap lifecycle and committed-body population |
| REP-16 | [CameraRig.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/CameraRig.ts), [CadOrbitControls.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/CadOrbitControls.ts) | Camera basis, projection, navigation, zoom and gesture handling |
| REP-17 | [2026-09-12 handoff](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/CLAUDE-CODE-HANDOFF-2026-09-12.md) | Existing incomplete acceptance and preservation/concurrency constraints |
| REP-18 | [HighlightLayer.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/HighlightLayer.ts) | Shared-attribute wrapper lifetime and overlay depth policy |
| REP-19 | [meshSync.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/mesh/meshSync.ts) | Existing load/publication/selection fences and fine-tier requests |
| REP-20 | [curveTessellation.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/curveTessellation.ts), [SketchStaticLayer.ts](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/src/viewport/engine/SketchStaticLayer.ts) | Prior adaptive/fixed curve paths and static-sketch rendering |
| REP-21 | [Canonical OCW1 schema](https://github.com/andrejvysny/OneCAD/blob/65b4c60eb226a201f2fa3eb565d7283b1c63b5ac/protocol/SCHEMA.md) | Wire changes must be integrated here with executable cross-track fixtures |

The original report's source list provides further per-finding links. It is preserved in [the reference copy](reference/original-rendering-review.md).

## 3. External primary documentation

| ID | Primary source | Verified use in this design |
|---|---|---|
| EXT-01 | [Three.js LineMaterial](https://threejs.org/docs/pages/LineMaterial.html) | Screen-space linewidth is CSS-sized; resolution and actual before-render behavior matter; WebGL material is not a WebGPU parity guarantee |
| EXT-02 | [Three.js WebGLRenderer](https://threejs.org/docs/pages/WebGLRenderer.html) | Logical viewport versus physical drawing buffer, renderer metrics, context/depth capabilities; current docs name the option `reversedDepthBuffer` with EXT_clip_control support |
| EXT-03 | [OCCT BSplineCurveToBezierCurve](https://occt3d.com/dev/doc/refman/html/class_geom_convert___b_spline_curve_to_bezier_curve.html) | Exact span decomposition API for B-spline curves; installed 8.0.1 signatures and trims must be checked |
| EXT-04 | [OCCT ToolTriangulatedShape](https://occt3d.com/dev/doc/refman/html/class_b_rep_lib___tool_triangulated_shape.html) | Surface/UV normal utilities; do not assume an existing normal array is overwritten or singularities automatically satisfy this specification |
| EXT-05 | [OCCT ApproxCurve](https://occt3d.com/dev/doc/refman/html/class_geom_convert___approx_curve.html) | Explicit approximate B-spline fallback; requested/result approximation error is not relabeled exact |
| EXT-06 | [three-mesh-bvh project](https://github.com/gkjohnson/three-mesh-bvh), [package metadata](https://github.com/gkjohnson/three-mesh-bvh/blob/master/package.json) | Selected external triangle acceleration library; use a local compatibility adapter and indirect indexing, verify/pin a published compatible package before installation |
| EXT-07 | [Claude Code custom subagents](https://code.claude.com/docs/en/sub-agents) | Project Markdown agent files and supported frontmatter; templates use model inheritance and scoped tools |
| EXT-08 | [Claude Code model configuration](https://code.claude.com/docs/en/model-config) | Select and verify the configured model instead of inventing a provider-specific Fable identifier |
| EXT-09 | [Claude Code CLI reference](https://code.claude.com/docs/en/cli-reference) | Verify the installed command behavior; no undocumented permission bypass or model flag is required by this package |

The BVH repository metadata observed during preparation says version **0.9.15** and peer dependency Three.js **>=0.159.0**. A repository package version is not proof of npm publication or tested compatibility with OneCAD. The selected dependency is fixed; WP00/WP14 must verify the published artifact, exact version, integrity and local adapter tests before pinning it. This is dependency resolution, not an invitation to choose a different picking architecture.

### Upstream source inspected for renderer contracts

- [Three.js r185 LineSegments2](https://github.com/mrdoob/three.js/blob/r185/examples/jsm/lines/LineSegments2.js): actual resolution update before drawing.
- [Three.js r185 WebGLRenderer](https://github.com/mrdoob/three.js/blob/r185/src/renderers/WebGLRenderer.js): logical viewport and physical buffer behavior.
- [Three.js r185 WebGLGeometries](https://github.com/mrdoob/three.js/blob/r185/src/renderers/webgl/WebGLGeometries.js): per-geometry registration/disposal and shared attribute implications.
- [Three.js r185 WebGLBindingStates](https://github.com/mrdoob/three.js/blob/r185/src/renderers/webgl/WebGLBindingStates.js): geometry-specific binding state and release ownership.

The installed 0.185.1 package remains the coding-time authority for exact behavior. Upstream source links are evidence for the diagnosed mechanism, not permission to depend on private renderer internals.

## 4. Deliberate new choices, not externally sourced claims

The 35° default FOV, CSS line hierarchy, opacity-free focus transform, memory/job limits, supported scale/workload, test thresholds, per-solid section policy, exact Bézier error strategy, and version-2 section layout are design decisions made for this project. They are not claimed as the internal implementation or published requirements of Shapr3D or Fusion 360.

Mathematical arguments in NUM are explicit derivations with assumptions, not benchmark claims. The native regression cases are required precisely because implementation details, numerical conditioning, mesher behavior, and platform rasterization still need evidence.

No source establishes a universal professional-readiness percentage. Qualification follows the test matrix, supported-platform evidence, resource budgets, and correct user-visible behavior.

## 5. Document-authoring checks actually performed

Local Markdown links, code-fence balance, requirement/work-package coverage, and the presence of all **127 defined acceptance case IDs** were checked while preparing the package. These are document checks, not passing application tests.

Selected mathematical identities were exercised independently in Python/NumPy: the rational derivative numerator, perspective-correct segment parameter, clip-W compensated dash distance, pathological S-curve, Jacobian singular-value expression, clamped zoom factor, and float32 spacing. The runnable code and observed output are in [reference/design-math-checks.md](reference/design-math-checks.md). These limited checks do not certify the complete design or substitute for its required native regressions.
