# Worker unit + integration tests.

# --- Frame codec unit test (encode/decode + partial reads via a pipe) ---
add_executable(test_frame_roundtrip test_frame_roundtrip.cpp)
target_link_libraries(test_frame_roundtrip PRIVATE worker_core)
add_test(NAME frame_roundtrip COMMAND test_frame_roundtrip)

# --- W-WP3b: NORMATIVE region-id cross-check vs the Rust reference impl ---
add_executable(test_region_id test_region_id.cpp)
target_link_libraries(test_region_id PRIVATE worker_core)
add_test(NAME region_id_crosscheck COMMAND test_region_id)

# --- Canonical selectable planar cells + strict region/profile identity. ---
add_executable(test_region_table test_region_table.cpp)
target_link_libraries(test_region_table PRIVATE worker_core)
add_test(NAME region_table COMMAND test_region_table)

# --- Construction geometry is solved but never bounds a region (SCHEMA §7.3). ---
add_executable(test_sketch_construction test_sketch_construction.cpp)
target_link_libraries(test_sketch_construction PRIVATE worker_core)
add_test(NAME sketch_construction COMMAND test_sketch_construction)

# --- Ellipse on the solver lane: the WireSketch branch, the naive-DOF fallback,
#     the normalization echo, and the corpus (i) `regions_ellipse` area oracle. ---
add_executable(test_sketch_ellipse test_sketch_ellipse.cpp)
target_link_libraries(test_sketch_ellipse PRIVATE worker_core)
add_test(NAME sketch_ellipse COMMAND test_sketch_ellipse)

# --- W0b: `referenceLocked` host-face geometry — the wire flag + child
#     inheritance, region participation (the contrast with `construction`), the
#     tag-0 solver pins, and the now-reachable edit guards. ---
add_executable(test_sketch_reference_locked test_sketch_reference_locked.cpp)
target_link_libraries(test_sketch_reference_locked PRIVATE worker_core)
add_test(NAME sketch_reference_locked COMMAND test_sketch_reference_locked)

# --- W0b: arc endpoints as REAL solver points — the `<id>.start`/`<id>.end`
#     handles, the internal (tag 0) ArcRules coupling, and the welded-slot DOF. ---
add_executable(test_sketch_arc_endpoints test_sketch_arc_endpoints.cpp)
target_link_libraries(test_sketch_arc_endpoints PRIVATE worker_core)
add_test(NAME sketch_arc_endpoints COMMAND test_sketch_arc_endpoints)

# --- SCHEMA §7.4 `entityStates`: the PER-ENTITY constrained state. The ownership
#     union (a Line owns no parameter, an Arc is pinned down by nine), the
#     conflict projection, the reference-lock pins, the ellipse OMISSION (the
#     NDJSON matcher is subset-only and cannot assert an absent key), and the
#     gesture-fixed rule that keeps the map off SolveDrag. ---
add_executable(test_sketch_entity_states test_sketch_entity_states.cpp)
target_link_libraries(test_sketch_entity_states PRIVATE worker_core)
add_test(NAME sketch_entity_states COMMAND test_sketch_entity_states)

# --- SNAP P3: the POINT-PAIR axis alignments (HorizontalPoints/VerticalPoints):
#     error/DOF, the solver actually moving a point onto the alignment, the loud
#     unresolved-handle failure, and applicability for two points. ---
add_executable(test_sketch_point_axis test_sketch_point_axis.cpp)
target_link_libraries(test_sketch_point_axis PRIVATE worker_core)
add_test(NAME sketch_point_axis COMMAND test_sketch_point_axis)

# --- SP-2 W1: the generalized drag entry `Sketch::solveWithTargets` (point AND
#     radius targets, explicit pin set, advisory tag(−1) drives) + the
#     `entityPointIds` handle-order map SCHEMA §7.4's gesture kinds build on. ---
add_executable(test_solve_with_targets test_solve_with_targets.cpp)
target_link_libraries(test_solve_with_targets PRIVATE worker_core)
add_test(NAME solve_with_targets COMMAND test_solve_with_targets)

# --- SP-2 W2: the SCHEMA §7.4 gesture KINDS on the solver lane — the per-kind
#     pin sets, the grab-derived offsets, the degenerate guards the lane owns
#     (MIN_GEOMETRY_SIZE radius floor, MIN_ARC_SWEEP refusal) and the additive
#     `curves` channel. Drives the real verbs through Dispatcher::dispatch_once. ---
add_executable(test_sketch_drag_kinds test_sketch_drag_kinds.cpp)
target_link_libraries(test_sketch_drag_kinds PRIVATE worker_core)
add_test(NAME sketch_drag_kinds COMMAND test_sketch_drag_kinds)

# --- SKETCH-ON-FACE W1a: face-boundary projector + `ProjectFaceBoundary` verb
#     (SCHEMA §7.6). Unit-level projector calls over self-built OCCT shapes PLUS
#     verb-level calls through the handler (in-process, real OCCT). ---
add_executable(test_face_projection test_face_projection.cpp)
target_link_libraries(test_face_projection PRIVATE worker_core)
add_test(NAME face_projection COMMAND test_face_projection)

# --- WP-C1: exact mass properties verb (SCHEMA §7.5). Analytic solids with
#     self-derived expectations + the read-only/determinism invariants. ---
add_executable(test_mass_properties test_mass_properties.cpp)
target_link_libraries(test_mass_properties PRIVATE worker_core)
add_test(NAME mass_properties COMMAND test_mass_properties)

# --- Region preview fill: holes subtracted + the bridge-interiority invariant
#     the frontend ring derivation depends on (pure 2D, no OCCT). ---
add_executable(test_polygon_fill test_polygon_fill.cpp)
target_link_libraries(test_polygon_fill PRIVATE worker_core)
add_test(NAME polygon_fill COMMAND test_polygon_fill)

# --- GEAR-GENERATOR G0: involute tooth math (pure C++, no OCCT), checked
#     against an oracle generated by running the actual upstream
#     freecad.gears pygears.involute_tooth module. ---
add_executable(test_gear_math_involute test_gear_math_involute.cpp)
target_link_libraries(test_gear_math_involute PRIVATE worker_core)
add_test(NAME gear_math_involute COMMAND test_gear_math_involute)

# --- GEAR-GENERATOR G0: GT/HTD timing-belt pulley arc math (pure C++, no
#     OCCT). Pinned against upstream's arc algebra for all 7 belt sections,
#     plus continuity + tooth-wrap invariants over a parameter sweep. ---
add_executable(test_gear_math_timing test_gear_math_timing.cpp)
target_link_libraries(test_gear_math_timing PRIVATE worker_core)
add_test(NAME gear_math_timing COMMAND test_gear_math_timing)

# --- GEAR-GENERATOR G0: T-profile timing pulley math. Flank endpoints are
#     solved in closed form (line-circle) rather than by upstream's scipy
#     minimiser, so the oracle is exact and upstream agreement is asserted
#     only to that minimiser's own convergence error. ---
add_executable(test_gear_math_timing_t test_gear_math_timing_t.cpp)
target_link_libraries(test_gear_math_timing_t PRIVATE worker_core)
add_test(NAME gear_math_timing_t COMMAND test_gear_math_timing_t)

# --- GEAR-GENERATOR G0: lantern (pin/roller) gear math. Pins the offset-
#     involute flank AND the property that the tooth-root solve finds the
#     meaningful root, never the spurious phi=0 one upstream's unguarded
#     scipy call could return. ---
add_executable(test_gear_math_lantern test_gear_math_lantern.cpp)
target_link_libraries(test_gear_math_lantern PRIVATE worker_core)
add_test(NAME gear_math_lantern COMMAND test_gear_math_lantern)

# --- GEAR-GENERATOR G0: ZA worm thread cross-section. Pins the trapezoid
#     stations, the cylinder wrap, and the exact identity that a thread spans
#     one angular pitch regardless of head/clearance/pressure angle. ---
add_executable(test_gear_math_worm test_gear_math_worm.cpp)
target_link_libraries(test_gear_math_worm PRIVATE worker_core)
add_test(NAME gear_math_worm COMMAND test_gear_math_worm)

# --- MODEL-OPS W3: drag-time preview verb — exactness + the "leaves no trace"
#     invariant (in-process, real OCCT). ---
add_executable(test_preview_op test_preview_op.cpp)
target_link_libraries(test_preview_op PRIVATE worker_core)
add_test(NAME preview_op COMMAND test_preview_op)

# --- W-WP3b: solver-lane latest-wins mailbox (drives the real worker) ---
add_executable(test_solver_latest_wins test_solver_latest_wins.cpp)
target_link_libraries(test_solver_latest_wins PRIVATE worker_core)
add_test(NAME solver_latest_wins
         COMMAND test_solver_latest_wins $<TARGET_FILE:onecad-worker>)

# ---------------------------------------------------------------------------
# W-WP4: session + transactional ExecutePlan (SCHEMA §7.1/§7.2), STUB ops.
# ---------------------------------------------------------------------------

# Hashing primitives unit test — SHA-256 anchor (== Rust HistoryPrefixHash::empty)
# + FNV-1a signatures. Keep asserts/CHECKs live regardless of build type.
add_executable(test_hashing test_hashing.cpp)
target_link_libraries(test_hashing PRIVATE worker_core)
add_test(NAME hashing COMMAND test_hashing)

# ExecutePlan machinery driven against the real worker binary: cancellation,
# the crash chaos drill, two-lane liveness, and cross-run determinism.
foreach(_t executeplan_cancel executeplan_crash concurrent_lanes executeplan_determinism)
    add_executable(test_${_t} test_${_t}.cpp)
    target_link_libraries(test_${_t} PRIVATE worker_core)
    add_test(NAME ${_t} COMMAND test_${_t} $<TARGET_FILE:onecad-worker>)
endforeach()

# --- W-WP5: real OCCT op numerics + ElementMap V2 history + MESH1 (in-process) ---
foreach(_t wp5_plan wp5_partition_history wp5_mesh1)
    add_executable(test_${_t} test_${_t}.cpp)
    target_link_libraries(test_${_t} PRIVATE worker_core)
    add_test(NAME ${_t} COMMAND test_${_t})
endforeach()

# Curved display mesh policy: Fine must stay visibly smoother than transient
# coarse, including selected-edge polylines.
add_executable(test_tessellation_quality test_tessellation_quality.cpp)
target_link_libraries(test_tessellation_quality PRIVATE worker_core)
add_test(NAME tessellation_quality COMMAND test_tessellation_quality)

# --- W-WP6: resolution-ladder calibration + new ops + fast-mode ordering (finding 3)
#     + STEP export (in-process, real OCCT). ---
foreach(_t wp6_ladder wp6_ops wp6_extrude wp6_faststode wp6_exportstep wp6_meshexport wp6_split wp6_checkpoint)
    add_executable(test_${_t} test_${_t}.cpp)
    target_link_libraries(test_${_t} PRIVATE worker_core)
    add_test(NAME ${_t} COMMAND test_${_t})
endforeach()

# Component Library WP-C: the §7.8 `ExtractPrismProfile` verb and the geometry it
# measures — the exact section area / length / ratio, the named refusals, and the
# POSE INDEPENDENCE proof (the same stick rotated 37° about its own axis
# canonicalises to the same FACE — to rounding, ~2e-15 mm; a rotation and its
# canonical undo are not bit-exact, so byte equality is asserted for the same
# input twice instead). The fixtures pin the protocol shape; the millimetres and
# the frame-selection proof live here.
add_executable(test_extract_prism_profile test_extract_prism_profile.cpp)
target_link_libraries(test_extract_prism_profile PRIVATE worker_core)
add_test(NAME extract_prism_profile COMMAND test_extract_prism_profile)

# WP0.6: a requested draft must be applied or NAMED-refused, never silently
# dropped. Carries the circular-profile red probe the work package mandates.
add_executable(test_extrude_draft test_extrude_draft.cpp)
target_link_libraries(test_extrude_draft PRIVATE worker_core)
add_test(NAME extrude_draft COMMAND test_extrude_draft)

# WP5: exact `ToFace` termination on a TILTED planar target — analytic volumes
# across the tilt range and both ends of the model scale, the named refusals for
# curved / uncovered / degenerate targets, and preview==commit. The parallel
# ToFace path is pinned separately by test_wp6_extrude and is untouched here.
add_executable(test_extrude_toface_tilted test_extrude_toface_tilted.cpp)
target_link_libraries(test_extrude_toface_tilted PRIVATE worker_core)
add_test(NAME extrude_toface_tilted COMMAND test_extrude_toface_tilted)

# WP1 G1: Extrude's Blind-distance and ToFace-coincidence guards, asserted AT the
# authoring-resolution boundary. Every other extrude refusal case sits at 0, which
# any threshold refuses, so none of them can prove the guards still read the
# precision context.
add_executable(test_extrude_precision_boundary test_extrude_precision_boundary.cpp)
target_link_libraries(test_extrude_precision_boundary PRIVATE worker_core)
add_test(NAME extrude_precision_boundary COMMAND test_extrude_precision_boundary)

# F0: oversized Fillet/Chamfer must refuse without fallback geometry or state
# mutation. Also emits deterministic NDJSON for the 7.9.3 comparison artifact.
add_executable(test_edge_op_failure test_edge_op_failure.cpp)
target_link_libraries(test_edge_op_failure PRIVATE worker_core)
add_test(NAME edge_op_failure COMMAND test_edge_op_failure)

# F2: the frozen tangent closure is valid only while our deterministic chainer
# agrees exactly with both OCCT local-operation contour implementations.
add_executable(test_edge_contours test_edge_contours.cpp)
target_link_libraries(test_edge_contours PRIVATE worker_core)
add_test(NAME edge_contours COMMAND test_edge_contours)

add_executable(test_prepare_edge_op test_prepare_edge_op.cpp)
target_link_libraries(test_prepare_edge_op PRIVATE worker_core)
add_test(NAME prepare_edge_op COMMAND test_prepare_edge_op)

# WP4 verb half: the §7.6 feasible-range query. `test_fillet_range` proves the
# kernel MEASURES the frontier; this proves the verb reports the measurement in
# the promised shape, and drives `derive_edge_op_range` directly for the island
# and overflow cases no fixture geometry produces.
add_executable(test_analyze_edge_op_range test_analyze_edge_op_range.cpp)
target_link_libraries(test_analyze_edge_op_range PRIVATE worker_core)
add_test(NAME analyze_edge_op_range COMMAND test_analyze_edge_op_range)

# The identity harness for GeometryPrecisionContext. Asserts every accessor
# against the INLINED literal it is going to replace, so a later call-site move
# that reds a suite is provably the move and not the number. Must stay green
# with zero digest movement at every gate of the precision migration.
add_executable(test_geometry_precision test_geometry_precision.cpp)
target_link_libraries(test_geometry_precision PRIVATE worker_core)
add_test(NAME geometry_precision COMMAND test_geometry_precision)

# WP1-G3: the micro-edge / sliver-face census. Walks a fixed catalog x 4 scales
# x 2 placements, PRINTS the table (the table is the gate deliverable) and
# asserts the detector stays silent on all of it, plus two constructed defects it
# must catch. `deep_audit` is deliberately not the measurement lane — its output
# is inside kernelbench's normalized digest. A dirty census blocks WP1-G4 from
# enabling any `PublicationPolicy` micro-topology bound. Generates a STEP fixture
# into the system temp dir (tests/step_fixture_util.h), so no WORKING_DIRECTORY.
add_executable(test_micro_topology_census test_micro_topology_census.cpp)
target_link_libraries(test_micro_topology_census PRIVATE worker_core)
add_test(NAME micro_topology_census COMMAND test_micro_topology_census)

# F1: constant-radius fillet construction, structured failure evidence, OCCT
# history, and non-gating simulation/vertex characterization.
foreach(_t fillet_builder fillet_diagnostics fillet_history fillet_characterization blend_recognizer)
    add_executable(test_${_t} test_${_t}.cpp)
    target_link_libraries(test_${_t} PRIVATE worker_core)
    add_test(NAME ${_t} COMMAND test_${_t})
endforeach()

# WP4 kernel half: `FilletRangeAnalyzer`. Brackets the feasible radius/distance
# range by running REAL builds as the oracle, ports kernelbench's staged search,
# and pins the measured bounds per fixture. Also carries the probe-cost
# measurement `kDefaultProbeBudget` is derived from — the `COST` line.
add_executable(test_fillet_range test_fillet_range.cpp)
target_link_libraries(test_fillet_range PRIVATE worker_core)
add_test(NAME fillet_range COMMAND test_fillet_range)

# WP3 C1: the ANALYTIC blend proof. A real fillet is Proved, a cylinder at the
# wrong axis distance is Failed, and a B-spline that the production sampler
# measures as a perfect constant-radius blend is Unknown — the refusal the
# direct-modelling path needs before it may destroy a face.
add_executable(test_blend_analytic test_blend_analytic.cpp)
target_link_libraries(test_blend_analytic PRIVATE worker_core)
add_test(NAME blend_analytic COMMAND test_blend_analytic)

# WP3 C2: `BlendSamplingBudget` zero-drift proof. The default budget reproduces
# the exact sample/boundary counts Fillet and kernelbench measure today.
add_executable(test_blend_budget test_blend_budget.cpp)
target_link_libraries(test_blend_budget PRIVATE worker_core)
add_test(NAME blend_budget COMMAND test_blend_budget)

# WP3 C3: `ComposedHistory`. Drives the real defeature -> offset -> re-fillet
# chain, CHARACTERIZES the three `BRepTools_History` facts the composition
# depends on (removal propagation, intermediate-key retention, Modified-vs-
# Generated survival), and asserts the composed contract: transients are
# unanswerable, successor lists are deterministic and final-ordinal sorted,
# and `is_deleted` fires on positive evidence only.
add_executable(test_composed_history test_composed_history.cpp)
target_link_libraries(test_composed_history PRIVATE worker_core)
add_test(NAME composed_history COMMAND test_composed_history)

# WP3 C4: `BlendReconstruction` — the suppress -> reblend engine, not wired to any
# op. The IDENTITY ROUND TRIP (suppress then rebuild at the same radius with no
# offset between) must reproduce the body to 1e-9 relative volume with an identical
# face count, in both signs of convexity; if that is red nothing downstream is
# trustworthy. Also pins the characterized two-stage tolerance cap and asserts each
# refusal path BY REASON CODE.
add_executable(test_blend_reconstruction test_blend_reconstruction.cpp)
target_link_libraries(test_blend_reconstruction PRIVATE worker_core)
add_test(NAME blend_reconstruction COMMAND test_blend_reconstruction)

# --- OFFSET-FACE Phase 0: OCCT selective-offset go/no-go spike (pure OCCT,
#     characterizes BRepOffset_MakeOffset per-face behavior before the op is
#     built; see the plan's Phase-0 gate). ---
add_executable(test_offsetface_spike test_offsetface_spike.cpp)
target_link_libraries(test_offsetface_spike PRIVATE worker_core)
add_test(NAME offsetface_spike COMMAND test_offsetface_spike)

# Imported-blend direct-model reconstruction spike: recognize a history-free
# constant-radius fillet, suppress it, and prove the sharp support body is restored.
add_executable(test_offsetface_reblend_spike test_offsetface_reblend_spike.cpp)
target_link_libraries(test_offsetface_reblend_spike PRIVATE worker_core)
add_test(NAME offsetface_reblend_spike COMMAND test_offsetface_reblend_spike)

# --- OFFSET-FACE W1: the `op.offsetFace` executor (SCHEMA §7.3) + the read-only
#     `PrepareOffsetFace` authoring handshake (§7.6). Exact analytic volumes/radii,
#     the σ-convention regression, the tangent-closure guard (incl. the 0.75° draft
#     counterexample), every trap the postconditions exist for, the offset-image
#     history rebind, and the verb's refusals / fence / determinism. In-process,
#     real OCCT. ---
add_executable(test_offsetface test_offsetface.cpp)
target_link_libraries(test_offsetface PRIVATE worker_core)
add_test(NAME offsetface COMMAND test_offsetface)

# --- WP3 C5: `op.offsetFace` at `resultPolicyVersion: 3` — the blend-aware closure
#     and the suppress -> offset -> reblend pipeline. Asserts the canonical side-face
#     push (V3 1191.4159265 with the corner still R2), the SAME record at V2 still
#     producing 1405.6637 (zero legacy drift), V3 == V2 wherever no blend is in the
#     closure, the wire-observable identity outcomes, the named refusals and
#     determinism. In-process, real OCCT. ---
add_executable(test_offsetface_reblend test_offsetface_reblend.cpp)
target_link_libraries(test_offsetface_reblend PRIVATE worker_core)
add_test(NAME offsetface_reblend COMMAND test_offsetface_reblend)

# --- WP3 C6: the ADVERSARIAL CORPUS for `op.offsetFace` at
#     `resultPolicyVersion: 3`. Split from test_offsetface_reblend.cpp (which is the
#     CONTRACT — the exact volumes that must never move) because the combined file
#     would run past 1900 lines. This one is the corpus: the decoys (chamfer,
#     user-authored quarter-round rib, partially-trimmed round, variable-radius and
#     torus blends), the conditioning cases (0.01 mm and 10 m scale, +1e6 mm from
#     origin, imported-STEP-style inflated tolerances), the BrepCodec persistence
#     round trip, and the first MULTI-BLEND engagement. In-process, real OCCT. ---
add_executable(test_offsetface_corpus test_offsetface_corpus.cpp)
target_link_libraries(test_offsetface_corpus PRIVATE worker_core)
add_test(NAME offsetface_corpus COMMAND test_offsetface_corpus)

# --- MODEL-HARDEN W3: Revolve boolean-mode multi-solid split-children parity
#     (`body_<opId>:<k>`, SCHEMA §7.2) — in-process, real OCCT. ---
add_executable(test_revolve_split test_revolve_split.cpp)
target_link_libraries(test_revolve_split PRIVATE worker_core)
add_test(NAME revolve_split COMMAND test_revolve_split)

# --- MODEL-HARDEN review follow-up: Revolve boolean-mode SINGLE-solid branches of
#     publish_boolean_result (Add/Cut-miss/Intersect) — the counterpart to
#     test_revolve_split's multi-solid Cut. In-process, real OCCT. ---
add_executable(test_revolve_boolean_modes test_revolve_boolean_modes.cpp)
target_link_libraries(test_revolve_boolean_modes PRIVATE worker_core)
add_test(NAME revolve_boolean_modes COMMAND test_revolve_boolean_modes)

# --- P0: standalone Boolean must refuse, not publish, a zero-solid result. ---
add_executable(test_boolean_empty test_boolean_empty.cpp)
target_link_libraries(test_boolean_empty PRIVATE worker_core)
add_test(NAME boolean_empty COMMAND test_boolean_empty)

# --- P4: standalone Boolean Intersect fixtures (overlap, containment, identity,
#     touching). ---
add_executable(test_boolean_intersect test_boolean_intersect.cpp)
target_link_libraries(test_boolean_intersect PRIVATE worker_core)
add_test(NAME boolean_intersect COMMAND test_boolean_intersect)

# --- M6a: breadth ops (Shell / Linear+Circular pattern / MirrorBody) numerics. ---
add_executable(test_m6a_ops test_m6a_ops.cpp)
target_link_libraries(test_m6a_ops PRIVATE worker_core)
add_test(NAME m6a_ops COMMAND test_m6a_ops)

# --- P3 Pattern V2: ExecutePlan lifecycle must preserve source and create only
#     transformed ordinal children. ---
add_executable(test_pattern_v2_plan test_pattern_v2_plan.cpp)
target_link_libraries(test_pattern_v2_plan PRIVATE worker_core)
add_test(NAME pattern_v2_plan COMMAND test_pattern_v2_plan)

# --- WP-C T3: `Hole` numerics — EXACT analytic volumes (through-all / blind /
#     counterbore / countersink) against the exact BRep via GProp, the point
#     re-projection fences, the non-planar-seat + cb/cs guards, and bit-equal
#     determinism. In-process, real OCCT. ---
add_executable(test_hole_op test_hole_op.cpp)
target_link_libraries(test_hole_op PRIVATE worker_core)
add_test(NAME hole_op COMMAND test_hole_op)

# --- COMPONENT-LIBRARY WP-0.2: `PlaceComponent` numerics — the hardcoded M6
#     SHCS generator's exact analytic volume, rigid-placement invariance, and
#     the P0-scope refusals (non-generator source, empty generatorId).
#     In-process, real OCCT. ---
add_executable(test_component_ops test_component_ops.cpp)
target_link_libraries(test_component_ops PRIVATE worker_core)
add_test(NAME component_ops COMMAND test_component_ops)

# --- COMPONENT-LIBRARY P3 WP-3.1: `solve_mate_placement` numeric parity
#     against `placementSolver.test.ts`'s own cases. Pure math, no OCCT
#     session needed. ---
add_executable(test_component_mate_solver test_component_mate_solver.cpp)
target_link_libraries(test_component_mate_solver PRIVATE worker_core)
add_test(NAME component_mate_solver COMMAND test_component_mate_solver)

# --- COMPONENT-LIBRARY P3 WP-3.1: mate re-seat on regen (spec §5.5) — reseat
#     on move, no-op within epsilon, NeedsRepair on a vanished target, VF-M7
#     cross-body parity. In-process, real OCCT. ---
add_executable(test_component_mate_reseat test_component_mate_reseat.cpp)
target_link_libraries(test_component_mate_reseat PRIVATE worker_core)
add_test(NAME component_mate_reseat COMMAND test_component_mate_reseat)

# --- COMPONENT-LIBRARY WP-3.2: the `embedded` / `document` blob source kinds and
#     the `ExportGeometry` verb that bakes them — export a published body, place
#     the bytes back, and pin every refusal (2 solids, wrong/absent format pin,
#     unmaterialized blob, unknown codec/kind). In-process, real OCCT. ---
add_executable(test_component_blob_source test_component_blob_source.cpp)
target_link_libraries(test_component_blob_source PRIVATE worker_core)
add_test(NAME component_blob_source COMMAND test_component_blob_source)

# --- COMPONENT-LIBRARY WP-A1: the per-family generator dispatch and the seed
#     catalog's dimension tables — one exact analytic volume per family (values
#     hardcoded FROM THE SOURCE, never read back out of the table), the
#     partial/full thread split, the loud unknown-generator and unknown-size
#     refusals, and a builds-at-every-seeded-size sweep. In-process, real OCCT.
#     COMPONENT-LIBRARY WP-F2 extends it with the ISO 15 bearing + NEMA stepper
#     families (exact volume + bounding-box dimensions per family). ---
add_executable(test_component_generators test_component_generators.cpp)
target_link_libraries(test_component_generators PRIVATE worker_core)
add_test(NAME component_generators COMMAND test_component_generators)

# --- VF-M7: an ElementMap entry minted on body A must never bind a face of body B.
#     Hole + Shell against two DIFFERENT bodies, asserted by exact analytic volume
#     (silent wrong drill / wrong seat / wrong shell wall). In-process, real OCCT. ---
add_executable(test_cross_body_element_ref test_cross_body_element_ref.cpp)
target_link_libraries(test_cross_body_element_ref PRIVATE worker_core)
add_test(NAME cross_body_element_ref COMMAND test_cross_body_element_ref)

# --- HISTORY-HARDEN H4 / VF-M3: `AcquireElementIds` refuses what it cannot honestly
#     resolve — a stale `snapshotId` (REF_UNRESOLVED) and an anchor fallback whose
#     world point is nowhere near the candidate it "won". In-process, real OCCT. ---
add_executable(test_element_identity_gate test_element_identity_gate.cpp)
target_link_libraries(test_element_identity_gate PRIVATE worker_core)
add_test(NAME element_identity_gate COMMAND test_element_identity_gate)

# --- WP-B W0: `TransformBody` numerics (exact bbox/volume, the normative T∘R
#     order, §2 copy ordinals) + the element-map placement migration
#     (`apply_placement`: level-1 rebind with ZERO scoring, anchors moved by
#     exactly the trsf). In-process, real OCCT. ---
add_executable(test_transform_body test_transform_body.cpp)
target_link_libraries(test_transform_body PRIVATE worker_core)
add_test(NAME transform_body COMMAND test_transform_body)

# --- STEP-IMPORT WP-A W0: the pure STEP read core. Both binaries GENERATE their
#     own STEP fixtures at run time (see tests/step_fixture_util.h) — nothing
#     binary is committed — so they need a writable working directory; CTest runs
#     them in ${CMAKE_CURRENT_BINARY_DIR}. test_step_read_determinism re-executes
#     ITSELF (argv[0] + "digest <fixture>") for the cross-process probe, so it
#     must stay a single self-contained binary with no extra COMMAND args. ---
foreach(_t step_read step_read_determinism)
    add_executable(test_${_t} test_${_t}.cpp)
    target_link_libraries(test_${_t} PRIVATE worker_core)
    add_test(NAME ${_t} COMMAND test_${_t})
    set_tests_properties(${_t} PROPERTIES WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR})
endforeach()

# --- STEP-IMPORT WP-A W1: the `ImportStep` op through a real ExecutePlan
#     transaction (both codecs) + the read-only `InspectStep` verb and its brep
#     conversion lane. Generates its own STEP fixtures into the system temp dir
#     (see tests/step_fixture_util.h), so no WORKING_DIRECTORY is needed. ---
add_executable(test_import_step test_import_step.cpp)
target_link_libraries(test_import_step PRIVATE worker_core)
add_test(NAME import_step COMMAND test_import_step)

# --- WP-A W4.5: XCAF fidelity end to end — a colored, named multi-solid STEP
#     (authored via STEPCAFControl_Writer, see step_fixture_util.h) through the
#     probe, the xbf conversion, the `sourceCodec:"xbf"` replay op and out into the
#     MESH1 FACE_COLORS section. Generates its fixtures into the system temp dir. ---
add_executable(test_xcaf_import test_xcaf_import.cpp)
target_link_libraries(test_xcaf_import PRIVATE worker_core)
add_test(NAME xcaf_import COMMAND test_xcaf_import)

# --- DI-5 W3: the EXPORT half of the same fidelity claim — body names + per-face
#     colours out through STEPCAFControl_Writer, asserted by reading the written
#     file back through the XCAF read lane and comparing values (test_wp6_exportstep
#     covers the structural claim only). Generates its fixtures into the system
#     temp dir. ---
add_executable(test_exportstep_xcaf test_exportstep_xcaf.cpp)
target_link_libraries(test_exportstep_xcaf PRIVATE worker_core)
add_test(NAME exportstep_xcaf COMMAND test_exportstep_xcaf)

# Cross-version producer/consumer binary. CI writes under 7.9.3 and reads the
# exact artifact under 8.0.1; it is not a same-build CTest. Keep this target
# independent of worker_core so comparison-worker compile drift cannot block
# the shipping kernel's required one-way persistence gate.
add_library(occt_persistence_codec STATIC
    ../src/io/BrepCodec.cpp
    ../src/io/OcafApp.cpp
    ../src/io/XcafCodec.cpp
)
target_include_directories(occt_persistence_codec PUBLIC
    ${PROJECT_SOURCE_DIR}/src
    ${OpenCASCADE_INCLUDE_DIR}
)
target_link_libraries(occt_persistence_codec PUBLIC ${OpenCASCADE_LIBRARIES})
if(OpenCASCADE_LIBRARY_DIR)
    target_link_directories(occt_persistence_codec PUBLIC ${OpenCASCADE_LIBRARY_DIR})
endif()
add_executable(test_occt_persistence test_occt_persistence.cpp)
target_link_libraries(test_occt_persistence PRIVATE occt_persistence_codec)

# --- Region-with-hole extrude: volume-checked (GProp) hole subtraction, incl. a
#     no-hole negative control (in-process, real OCCT). ---
add_executable(test_hole_extrude test_hole_extrude.cpp)
target_link_libraries(test_hole_extrude PRIVATE worker_core)
add_test(NAME hole_extrude COMMAND test_hole_extrude)

# Harness-driven ExecutePlan / session fixtures (NDJSON send/expect flows).
foreach(_fx executeplan_happy executeplan_fail_step executeplan_needsrepair
            executeplan_fencing executeplan_discard double_executeplan session_reset
            executeplan_deltas tessellate_acquire
            executeplan_region_nomatch export_mesh checkpoint_roundtrip
            executeplan_linearpattern executeplan_shell executeplan_offsetface
            executeplan_ellipse_extrude import_step)
    add_test(
        NAME harness_${_fx}
        COMMAND worker_harness
                --worker $<TARGET_FILE:onecad-worker>
                --fixture ${CMAKE_CURRENT_SOURCE_DIR}/fixtures/${_fx}.ndjson
    )
endforeach()

# --- Harness-driven fixtures against the real worker ---
# The harness (worker_harness) and the worker (onecad-worker) are declared in
# sibling directories; reference them by target name so CTest picks up the
# built binaries regardless of build layout.
add_test(
    NAME harness_hello
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${CMAKE_CURRENT_SOURCE_DIR}/fixtures/hello.ndjson
)

add_test(
    NAME harness_bad_verb
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${CMAKE_CURRENT_SOURCE_DIR}/fixtures/bad_verb.ndjson
)

# --- W-WP3b: solver-lane verb fixtures (SCHEMA §7.4) ---
# sketch_redundant / sketch_overconstrained_full / sketch_fully_constrained pin
# the OverConstrained state + "redundant" drag status (benign DOF-preserving
# redundancy) and the clean-dof-0 FullyConstrained regression.
# sketch_gesture_kinds covers the SP-2 §7.4 target kinds (entityBody / arcEnd /
# radius) + the `curves` channel; sketch_gesture stays the byte-identical
# back-compat regression for the pre-SP-2 pointId-only form.
foreach(_fx sketch_upsert sketch_gesture sketch_conflicting sketch_regions
            sketch_unknown_gesture sketch_latest_wins
            sketch_redundant sketch_overconstrained_full sketch_fully_constrained
            sketch_ellipse_upsert sketch_ellipse_regions sketch_ellipse_gesture
            sketch_gesture_kinds)
    add_test(
        NAME harness_${_fx}
        COMMAND worker_harness
                --worker $<TARGET_FILE:onecad-worker>
                --fixture ${CMAKE_CURRENT_SOURCE_DIR}/fixtures/${_fx}.ndjson
    )
endforeach()

# ---------------------------------------------------------------------------
# W-WP3c: CANONICAL cross-track fixtures. protocol/fixtures/*.ndjson are the
# Rust-authored executable form of SCHEMA.md (the same files the Rust
# ProtocolClient runs). The C++ worker MUST pass them verbatim — this is the
# interop contract. Referenced by absolute path (sibling protocol/ dir).
# ---------------------------------------------------------------------------
set(PROTOCOL_FIXTURES ${CMAKE_CURRENT_SOURCE_DIR}/../../protocol/fixtures)
add_test(
    NAME canonical_hello
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/hello.ndjson
)
add_test(
    NAME canonical_echo_error
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/echo_error.ndjson
)
# SCHEMA §7.2 `checkpointFallbackReplay` (VF-M5). The pair differs by that field
# (plus the `editedFrom` it rides with) and expects IDENTICAL frames: the field
# must be read and must not perturb a plan with nothing to gate. The behavioural
# proof lives in the Rust lane-D test — see the fixture headers.
add_test(
    NAME canonical_execute_plan_ordinary
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/execute_plan_ordinary.ndjson
)
add_test(
    NAME canonical_execute_plan_checkpoint_fallback
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/execute_plan_checkpoint_fallback.ndjson
)
add_test(
    NAME canonical_bind_element_ids
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/bind_element_ids.ndjson
)
add_test(
    NAME canonical_boolean_empty_refusal
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/boolean_empty_refusal.ndjson
)
# SCHEMA §7.2 `diagnostics[].reasonCode` (WP1-G2) — a publication refusal names
# WHICH policy branch refused it, machine-readably, while the top-level §8 code
# stays `GEOMETRY_INVALID`. See the fixture header for the recipe.
add_test(
    NAME canonical_publication_refusal
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/publication_refusal.ndjson
)
# SCHEMA §7.5 — every ResolveRefs resolution echoes {snapshotId, revision, refId,
# bodyId}. Rust validates that echo fail-closed; this is the worker-side half.
add_test(
    NAME canonical_resolve_refs_snapshot_echo
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/resolve_refs_snapshot_echo.ndjson
)
# SCHEMA §7.3 Pattern V2 lineage — `count-1` children named `body_<opId>:<k>`, the
# source preserved with no lifecycle event of its own. The step ANGLE is geometry
# and stays pinned in test_m6a_ops.cpp; see the fixture header.
add_test(
    NAME canonical_circular_pattern_lineage
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/circular_pattern_lineage.ndjson
)
# SCHEMA §7.3 OffsetFace `resultPolicyVersion` 2 vs 3 (WP3-C5). The PAIR is the
# contract: the same model and the same push under both readings, differing only
# in the elementMapDelta each produces. Geometry numbers (V3 1191.4159265358979 /
# V2 1405.6637061435916) are NOT pinned here — `json_subset` compares numbers
# exactly and NDJSON is the wrong instrument for a measured volume; they stay in
# test_offsetface_reblend.cpp. See each fixture header.
add_test(
    NAME canonical_offset_face_v3_reblend
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/offset_face_v3_reblend.ndjson
)
add_test(
    NAME canonical_offset_face_v2_unchanged
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/offset_face_v2_unchanged.ndjson
)
# An absolute distance against a blend-bearing closure is refused BY NAME, at
# `classify`, and the head survives it.
add_test(
    NAME canonical_offset_face_v3_refusal
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/offset_face_v3_refusal.ndjson
)
# WP5: a TILTED planar `ToFace` target crosses the wire and terminates instead of
# refusing, and the curved/uncovered/degenerate cases carry their named codes.
# This pins the TRANSITION and the diagnostic VOCABULARY, not exactness — the
# analytic volumes live in test_extrude_toface_tilted, for the same reason the
# OffsetFace pair splits contract from corpus.
add_test(
    NAME canonical_extrude_to_face_tilted
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/extrude_to_face_tilted.ndjson
)
# SCHEMA §7.6 `AnalyzeEdgeOpRange` (WP4). Pins the ECHO, the closure, the
# classification vocabulary and the refusal shape, plus the read-only guarantee
# as an identical GetWorkerHead either side of dozens of real OCCT builds. The
# measured frontier is `$any` here and pinned in test_fillet_range.cpp — the same
# contract/corpus split the OffsetFace pair uses. See the fixture header.
add_test(
    NAME canonical_analyze_edge_op_range
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/analyze_edge_op_range.ndjson
)
# SCHEMA §7.3 Chamfer `angleDeg` (WP6 item 1) — and the first canonical fixture to
# drive a Chamfer at all, so it covers equal-leg and two-distance too. Pins the
# delta/bodyEvents SHAPE (a distance-angle chamfer is indistinguishable from an
# equal-leg one at the identity layer), the three named refusals, and the head
# surviving all of them. The analytic volumes live in test_wp6_ops.cpp.
add_test(
    NAME canonical_chamfer_angle_distance
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/chamfer_angle_distance.ndjson
)
# SCHEMA §7.3 `Hole.thread` (WP-T1) — cosmetic tapped hole. Pins that a threaded
# hole and its unthreaded reference produce the IDENTICAL matcher character for
# character (Option B: `params.diameter` is the drilled hole either way, and the
# worker reads nothing else from `thread`), plus the two `detail` preflight
# refusals (`HOLE_THREAD_DETAIL_UNSUPPORTED`/`_UNKNOWN`) and the head surviving
# both. The volume identity itself is measured in test_hole_op.cpp.
add_test(
    NAME canonical_hole_threaded
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/hole_threaded.ndjson
)
# SCHEMA §7.4 `entityStates` (per-entity constrained state) — and the FIRST canonical
# fixture to drive the sketch solver lane at all. Round 1 is the load-bearing one: one
# under-constrained sketch in which the line is fully constrained and the circle is
# not, which no whole-sketch echo could produce. The two OMISSION rules (ellipse omits
# the whole map, SolveDrag never carries it) are NOT assertable here — the matcher is
# subset-only — and are pinned in test_sketch_entity_states.cpp. See the fixture header.
add_test(
    NAME canonical_sketch_entity_states
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/sketch_entity_states.ndjson
)
# SCHEMA §7.3 `source.kind:"profile"` + §7.8 `ExtractPrismProfile` (Component
# Library WP-C). SELF-GENERATING: the fixture builds the vendor stick, bakes its
# canonical end-cap face to a temp path, and places that face back at a DIFFERENT
# length — so no binary is tracked and the input is reproducible from the file
# alone (the `import_step.ndjson` pattern). Every measured number is `$present`
# here and pinned in test_extract_prism_profile.cpp / test_component_blob_source.cpp,
# the same contract/corpus split the OffsetFace pair uses. See the fixture headers.
add_test(
    NAME canonical_place_component_profile
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/place_component_profile.ndjson
)
add_test(
    NAME canonical_extract_prism_profile_refusal
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/extract_prism_profile_refusal.ndjson
)
# …and the determinism half of that contract, which no in-process test can prove:
# replay the SAME fixture twice, each replay spawning its own worker, and `cmp`
# the two written profiles. §7.8 claims the bytes are identical across FRESH
# PROCESSES, so a frame that quietly depends on process-scoped state — a static
# initialised in load order, an OCCT session tolerance — is only visible here.
# No digest literal is pinned: libm differs across macOS and Linux, so a frozen
# SHA-256 would gate the platform instead of the determinism.
add_test(
    NAME canonical_extract_prism_profile_bytes
    COMMAND ${CMAKE_CURRENT_SOURCE_DIR}/determinism/check_prism_profile_bytes.sh
            $<TARGET_FILE:worker_harness>
            $<TARGET_FILE:onecad-worker>
            ${PROTOCOL_FIXTURES}/place_component_profile.ndjson
            /tmp/onecad_canonical_profile.brep
)

# W-WP3c: interop smoke — replay the Rust-authored contract via the harness (no
# Rust compiled here; live Rust<->C++ interop is W-WP4/R-WP11). See the script
# header for the boundary rationale.
add_test(
    NAME interop_smoke
    COMMAND ${CMAKE_CURRENT_SOURCE_DIR}/interop/check_interop.sh
            $<TARGET_FILE:worker_harness>
            $<TARGET_FILE:onecad-worker>
            ${PROTOCOL_FIXTURES}
)

# ---------------------------------------------------------------------------
# W-WP2 ported kernel prototypes (assert-style standalone mains).
# All link worker_core (kernel port + OCCT + vendored planegcs) and inherit
# its PUBLIC include dirs (src, src/kernel, third_party, OCCT).
# ---------------------------------------------------------------------------
set(WP2_PROTOTYPES
    proto_elementmap_rigorous     # THE parity gate (ElementMap descriptor/hash)
    proto_custom_map              # ElementMap history update / ID preservation
    proto_tnaming                 # OCAF TNaming reference (OCCT-only)
    proto_planegcs_integration    # vendored PlaneGCS solve smoke
)
foreach(_proto ${WP2_PROTOTYPES})
    add_executable(${_proto} prototypes/${_proto}.cpp)
    target_link_libraries(${_proto} PRIVATE worker_core)
    # Keep assert() live regardless of build type. CI/default is Release
    # (-DNDEBUG), which would otherwise turn the assert-based prototypes into
    # unconditional passes. Target COMPILE_OPTIONS are emitted after
    # CMAKE_CXX_FLAGS_<CONFIG>, so -UNDEBUG wins (mirrors OneCAD-CPP tests).
    target_compile_options(${_proto} PRIVATE -UNDEBUG)
    add_test(NAME ${_proto} COMMAND ${_proto})
endforeach()

# --- ID-parity gate: diff proto_elementmap_rigorous stdout against the frozen
#     OneCAD-CPP corpus recording (proves descriptor/hash/ID parity). ---
add_test(
    NAME parity_elementmap_rigorous
    COMMAND ${CMAKE_CURRENT_SOURCE_DIR}/parity/check_parity.sh
            $<TARGET_FILE:proto_elementmap_rigorous>
            ${CMAKE_CURRENT_SOURCE_DIR}/../../corpus/expected-values/proto_elementmap_rigorous.txt
)

# ---------------------------------------------------------------------------
# W-WP3a sketch-stack prototypes (Qt-stripped ports of OneCAD-CPP sketch tests).
# All link worker_core (now including the Qt-free sketch entity stack + loop +
# vendored planegcs) and keep asserts live via -UNDEBUG.
# ---------------------------------------------------------------------------
set(WP3_PROTOTYPES
    proto_loop_detector                  # region detection from closed sketch loops
    proto_face_builder                   # OCCT wire/face build from detected loops
    proto_sketch_geometry                # entity geometry math
    proto_sketch_constraints             # constraint satisfaction (JSON test stripped)
    proto_sketch_solver                  # PlaneGCS solve + drag semantics
    proto_sketch_fixed_and_move          # Fixed constraint + plane translation
    proto_sketch_constraint_applicability  # selection→applicable-constraint rules
)
foreach(_proto ${WP3_PROTOTYPES})
    add_executable(${_proto} prototypes/${_proto}.cpp)
    target_link_libraries(${_proto} PRIVATE worker_core)
    target_compile_options(${_proto} PRIVATE -UNDEBUG)  # keep assert() live (Release)
    add_test(NAME ${_proto} COMMAND ${_proto})
endforeach()

# --- Parity gates (terminator + exit code vs frozen OneCAD-CPP corpus).
#     Full byte-parity is impossible: the recordings contain Qt category logging
#     with random per-run UUIDs (non-reproducible even by the oracle). See
#     parity/check_terminator.sh for the rationale; the gate proves the ported
#     binary passes the same in-source asserts and reaches the same terminal
#     state as the oracle. ---
add_test(
    NAME parity_loop_detector
    COMMAND ${CMAKE_CURRENT_SOURCE_DIR}/parity/check_terminator.sh
            $<TARGET_FILE:proto_loop_detector>
            ${CMAKE_CURRENT_SOURCE_DIR}/../../corpus/expected-values/proto_loop_detector.txt
)
add_test(
    NAME parity_sketch_solver
    COMMAND ${CMAKE_CURRENT_SOURCE_DIR}/parity/check_terminator.sh
            $<TARGET_FILE:proto_sketch_solver>
            ${CMAKE_CURRENT_SOURCE_DIR}/../../corpus/expected-values/proto_sketch_solver.txt
)
# NOTE: proto_face_builder has NO corpus recording in corpus/expected-values/,
# so it is gated exit-0 only (the add_test above); its asserts are the oracle.

# --- GEAR-GENERATOR G1: the gear SOLID builder (OCCT). Validity, solid count,
#     volume bounds against root/addendum cylinders, determinism, bore
#     arithmetic, placement, and the refusal matrix. ---
add_executable(test_gear_tool test_gear_tool.cpp)
target_link_libraries(test_gear_tool PRIVATE worker_core)
add_test(NAME gear_tool COMMAND test_gear_tool)

# --- GEAR-GENERATOR G1: the `Gear` OPERATION (SCHEMA §7.3) — D1 mint lineage,
#     both placement modes, the 1e-3 mm placement fence, determinism, and the
#     refusal matrix including the UNSUPPORTED helical path. ---
add_executable(test_gear_op test_gear_op.cpp)
target_link_libraries(test_gear_op PRIVATE worker_core)
add_test(NAME gear_op COMMAND test_gear_op)

add_executable(test_extrude_boolean_modes test_extrude_boolean_modes.cpp)
target_link_libraries(test_extrude_boolean_modes PRIVATE worker_core)
add_test(NAME extrude_boolean_modes COMMAND test_extrude_boolean_modes)
# Kernel-hardening WP-C — an Extrude `Add` whose tool never touches the target is a
# NAMED refusal (`EXTRUDE_ADD_DISJOINT`) with the target intact, never a split.
add_test(
    NAME canonical_extrude_add_disjoint_refusal
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/extrude_add_disjoint_refusal.ndjson
)
# Kernel-hardening WP-E — a revolve profile that CROSSES its axis is refused by name
# (`REVOLVE_PROFILE_CROSSES_AXIS`) before any kernel work; cross-track pin.
add_test(
    NAME canonical_revolve_profile_crosses_axis
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/revolve_profile_crosses_axis.ndjson
)
# Kernel-hardening WP-A — ResolveRefs all-three-outcome coverage, moved from the
# harness lane so the §9 `scoringVersion` (4) is pinned in BOTH lanes (§13).
add_test(
    NAME canonical_resolve_refs
    COMMAND worker_harness
            --worker $<TARGET_FILE:onecad-worker>
            --fixture ${PROTOCOL_FIXTURES}/resolve_refs.ndjson
)

# Kernel-hardening WP-E — commit-tier validation: a revolve profile crossing its axis
# refuses by name; touching the axis is legal (degenerate pole edges).
add_executable(test_commit_tier_validation test_commit_tier_validation.cpp)
target_link_libraries(test_commit_tier_validation PRIVATE worker_core)
add_test(NAME commit_tier_validation COMMAND test_commit_tier_validation)
