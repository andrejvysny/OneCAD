# 0015 — The viewport is hardened in place under the VP-HARDENING 1.0 contract, not rewritten

- Status: Accepted
- Date: 2026-09-13

## Context

A static source audit of the viewport at commit `65b4c60` (preserved verbatim in
`docs/viewport-hardening/reference/original-rendering-review.md`) found eighteen
defects that are contracts and algorithms rather than missing features: fat-line
widths scaled by device pixel ratio against a logical-pixel resolution, highlight
geometry wrappers that are never disposed, an edge sampler whose midpoint test
accepts a 1.125 mm excursion, triangle-averaged shading normals, save/restore
material state keyed by kind, a scheduler that clears a dirty flag set during
the frame it is clearing, clamped zoom that still moves the target, a pointer
radius reused as a world-depth allowance in picking, structurally-validated but
semantically-unchecked mesh payloads, a fixed 0.1/100000 depth range with
float32 world coordinates, and a face-color loop that indexes faces with a
triangle lookup (`idAt` where `idOf` was meant).

The package under `docs/viewport-hardening/` selects one design for all of
them. It is normative for this program: the specification, the numerical
appendix and the acceptance plan decide the architecture, the constants and the
evidence required. This record exists so a future reader knows those documents
are law for the viewport and not a proposal.

## Decision

1. **Keep the stack.** Imperative Three.js 0.185.1 under React 19, WebGL2 as the
   only production backend, Tauri/Rust/OCCT 8.0.1 unchanged. No
   react-three-fiber, no ECS, no render-graph framework, no production WebGPU,
   no photorealism in this program.
2. **The package is the design authority for the viewport.** Numbered
   decisions D01–D28 in `docs/viewport-hardening/06-DECISIONS-AND-TRACEABILITY.md`
   are binding. Work is delivered as WP00–WP16 in the dependency order of the
   implementation guide, with evidence recorded in
   `docs/viewport-hardening/execution/STATUS.md` and
   `docs/qa/viewport-hardening/`.
3. **Contracts that change wire bytes or persisted meaning get their own ADR**
   when they land: MESH1 binary version 2 with local origins (D22/D23), the
   canonical camera state and its 35° default (D11/D12), the topology-edge
   classification bits (D19). Those records will cite this one.
4. **Evidence states are separate from implementation states.** `implemented`
   and `native-accepted` are different columns everywhere; a green mock or
   jsdom suite never closes a native gate. The acceptance matrix is machine
   checked by `scripts/verify-viewport-acceptance.mjs`.
5. **Baseline reds are preserved, not repaired by this program.** The gate
   runs recorded under `docs/qa/viewport-hardening/baseline/` at the start of
   WP00 were already red in areas outside the viewport (model-tool controller
   fixtures, feature-pattern repair, hole/shell integration). Those stay
   attributed to their owners; this program reports whether it changes them
   and never deletes or weakens their tests.

## Consequences

- Small services composed by `ViewportEngine` (metrics, frame scheduler,
  resource registry with leases, effective display set, appearance resolver,
  camera controller, pick service) replace ad-hoc state spread across layers.
  `ViewportEngine.ts`, `ViewportRoot.tsx`, `meshSync.ts`, `SketchObject.ts`,
  `settingsStore.ts`, the protocol documents, the Rust DTOs and the worker
  Dispatcher are single-owner integration files during the program.
- Screen-space lines are authored in CSS pixels with a logical-pixel
  resolution; device pixels remain explicit where they are genuinely needed
  (drawing buffer, readback, point sprites). The comment in `engine/dpr.ts`
  that says otherwise is superseded by the installed Three.js draw callback.
- Every CPU/GPU/cache/job allocation has a named owner and a limit. Garbage
  collection is not the GPU ownership protocol.
- A discovered contradiction between the package and the installed source is
  a deviation record with a counterexample in the execution ledger, never a
  silent relaxation.

## Rejected

- A renderer rewrite or framework migration: the defects are in contracts, not
  in the choice of renderer.
- "Fixing" line widths by removing all DPR handling: points, textures and the
  drawing buffer have different unit contracts.
- Disposing the shared-attribute highlight wrappers directly: it would free the
  body's own buffers.
- More midpoint samples as a curve-error proof: samples cannot bound an interior
  excursion; the hull criterion can.
