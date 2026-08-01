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

# --- Region preview fill: holes subtracted + the bridge-interiority invariant
#     the frontend ring derivation depends on (pure 2D, no OCCT). ---
add_executable(test_polygon_fill test_polygon_fill.cpp)
target_link_libraries(test_polygon_fill PRIVATE worker_core)
add_test(NAME polygon_fill COMMAND test_polygon_fill)

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

# --- W-WP6: resolution-ladder calibration + new ops + fast-mode ordering (finding 3)
#     + STEP export (in-process, real OCCT). ---
foreach(_t wp6_ladder wp6_ops wp6_extrude wp6_faststode wp6_exportstep wp6_meshexport wp6_split wp6_checkpoint)
    add_executable(test_${_t} test_${_t}.cpp)
    target_link_libraries(test_${_t} PRIVATE worker_core)
    add_test(NAME ${_t} COMMAND test_${_t})
endforeach()

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

# --- M6a: breadth ops (Shell / Linear+Circular pattern / MirrorBody) numerics. ---
add_executable(test_m6a_ops test_m6a_ops.cpp)
target_link_libraries(test_m6a_ops PRIVATE worker_core)
add_test(NAME m6a_ops COMMAND test_m6a_ops)

# --- Region-with-hole extrude: volume-checked (GProp) hole subtraction, incl. a
#     no-hole negative control (in-process, real OCCT). ---
add_executable(test_hole_extrude test_hole_extrude.cpp)
target_link_libraries(test_hole_extrude PRIVATE worker_core)
add_test(NAME hole_extrude COMMAND test_hole_extrude)

# Harness-driven ExecutePlan / session fixtures (NDJSON send/expect flows).
foreach(_fx executeplan_happy executeplan_fail_step executeplan_needsrepair
            executeplan_fencing executeplan_discard double_executeplan session_reset
            executeplan_deltas tessellate_acquire resolve_refs
            executeplan_region_nomatch export_mesh checkpoint_roundtrip
            executeplan_linearpattern executeplan_shell
            executeplan_ellipse_extrude)
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
foreach(_fx sketch_upsert sketch_gesture sketch_conflicting sketch_regions
            sketch_unknown_gesture sketch_latest_wins
            sketch_redundant sketch_overconstrained_full sketch_fully_constrained
            sketch_ellipse_upsert sketch_ellipse_regions sketch_ellipse_gesture)
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
