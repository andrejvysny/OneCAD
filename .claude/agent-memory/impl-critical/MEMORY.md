# Memory Index

- [WP-P projection facts](repo_wpp_projection.md) — FE-reachable SketchEditOps, rowless projected points, worker faceOutline counting rules
- [Verification notes](repo_verification_notes.md) — red-first without reverting; env-gated audit assertions (and proving they fire); per-agent worker build dirs; shared-worktree churn
- [Region identity + detection](repo_region_identity.md) — V3 ids hash geometry (not samples), the two build_profile_face configs, bbox-culled curve pairs, chord-tolerance sampling, degenerate-entity warning
- [Worker op diagnostics](repo_worker_diagnostics.md) — Ok-step warnings ride the planStep event, never a failed step; sketch UV lift + build_profile_face arg order
- [Protocol fixtures + harness](repo_protocol_fixtures.md) — what worker_harness really implements vs the README, no standalone tolerance line, auto-run lanes
- [Sketch solver residual](repo_sketch_solver_residual.md) — maxConstraintResidual, the two exact-solve wire sites, why EndGesture measures from the sketch
- [Worker probe lanes](repo_worker_geometry_lane.md) — direct op-executor + Session/mint lanes, AcquireElementIds mints nothing, classify_shape by geometry, XY sketch frame, un-oriented normals
- [Worker liveness probes](repo_worker_liveness_probes.md) — single-threaded stub, in-process Dispatcher/PlanExecutor drivers, MESH1 header offsets, restore keyed on stepIndex
- [Fillet acceptance envelope](repo_fillet_acceptance.md) — two blend classes, curvature-derived residual, remnant floor, which kernelbench rows may move
- [OCW1 driver for fixtures](repo_ocw1_driver.md) — harness --repl desyncs on ExecutePlan; frame the worker directly, plus the param names that bite
- [MESH1 id namespace](repo_mesh_id_namespace.md) — mesh tables mix TopoKeys with minted ElementIds; only TopoKeys promote, and which lanes still trip on it
- [Promotion proof lane](repo_promotion_proof.md) — the two typed promote entry points, where the pick proof is captured, what clears it, the cosmetic-republish trap
- [Viewport selection policy](repo_viewport_selection_policy.md) — D-5 + Astra break: persistent id first, an ordinal is not evidence across a regen, regen-only reconcile
- [Regen isolation + closure](repo_regen_isolation.md) — why graph.downstream() is not proof of independence, what timeline_of(n) really builds, checkpoint-mint seams
- [Curve sampling + MESH1 edges](repo_curve_sampler.md) — EDGE_RANGES is 7 not 8, gp_XYZ::IsEqual is strict, OCCT conversion behaviour, worker/build is Makefiles
- [Mesh validation + admission](repo_mesh_validation.md) — buildBodyObjects' 15-file caller fan-out, what worker MESH1 bytes really guarantee, parser-vs-validator boundary
- [Mesh preparation plan](repo_mesh_preparation_plan.md) — one layout/cost authority, why `plan` is the last param, reserve-after-colour-resolve, pre-existing red modelTools lane
- [Surface normals + tessellation](repo_surface_normals.md) — BRepAdaptor world frame + transformed BSpline poles, IsNegative is not det, untransformed poles for a derivative bound, per-parameter units, why "no enclosure" is not "refuted", brep injection for verb tests
