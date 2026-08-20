cmake_minimum_required(VERSION 3.25)
project(onecad-worker VERSION 0.1.0 LANGUAGES CXX)

set(ONECAD_OCCT_ROOT "" CACHE PATH "Root of caller-selected OCCT installation")
set(ONECAD_OCCT_VERSION "8.0.1" CACHE STRING "Exact OCCT version required by this worker build")
set(ONECAD_OCCT_BUILD_ID "local" CACHE STRING "Stable identity of the OCCT build artifact")
set(ONECAD_WORKER_BUILD_DIR "${CMAKE_BINARY_DIR}" CACHE PATH "Expected worker binary directory")

file(REAL_PATH "${CMAKE_BINARY_DIR}" _onecad_actual_build_dir)
file(REAL_PATH "${ONECAD_WORKER_BUILD_DIR}" _onecad_requested_build_dir BASE_DIRECTORY "${CMAKE_SOURCE_DIR}")
if(NOT _onecad_actual_build_dir STREQUAL _onecad_requested_build_dir)
    message(FATAL_ERROR
        "ONECAD_WORKER_BUILD_DIR (${_onecad_requested_build_dir}) must match the active CMake binary directory (${_onecad_actual_build_dir})")
endif()

# --- Language standard ---
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# Export compile_commands.json for tooling (clangd, etc.)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# --- Dependency discovery (OCCT / Eigen3 / Boost) ---
include(${CMAKE_CURRENT_SOURCE_DIR}/cmake/Dependencies.cmake)

# --- RPATH follows the resolved OCCT artifact; no machine-local prefix. ---
set(CMAKE_BUILD_RPATH "${OpenCASCADE_LIBRARY_DIR};@executable_path/../Frameworks")
set(CMAKE_INSTALL_RPATH "${OpenCASCADE_LIBRARY_DIR};@executable_path/../Frameworks")
set(CMAKE_MACOSX_RPATH ON)
if(NOT APPLE)
    # GNU ld defaults to --enable-new-dtags, which emits DT_RUNPATH instead of
    # DT_RPATH. DT_RUNPATH is NOT transitive: it resolves only the binary's own
    # DT_NEEDED entries, so OCCT's inter-library edges (libTKOffset -> libTKG2d,
    # 26 of them) fall through to the system search path and the worker dies at
    # startup with "cannot open shared object file". OCCT's own libraries carry
    # no RPATH/RUNPATH, so nothing downstream can recover it. DT_RPATH IS
    # transitive, which is what makes the single OCCT lib dir above sufficient —
    # the behaviour macOS already gets from LC_RPATH.
    add_link_options(-Wl,--disable-new-dtags)
endif()

set(ONECAD_OCCT_SOURCE_COMMIT "unknown")
set(ONECAD_OCCT_NORMALIZED_BUILD_OPTIONS
    "type=${OpenCASCADE_BUILD_TYPE},shared=${OpenCASCADE_BUILD_SHARED_LIBS},tbb=${OpenCASCADE_WITH_TBB},freetype=${OpenCASCADE_WITH_FREETYPE},freeimage=${OpenCASCADE_WITH_FREEIMAGE},vtk=${OpenCASCADE_WITH_VTK},ffmpeg=${OpenCASCADE_WITH_FFMPEG},gles2=${OpenCASCADE_WITH_GLES2}")
if(ONECAD_OCCT_ROOT AND EXISTS "${ONECAD_OCCT_ROOT}/share/onecad/occt-build.cmake")
    include("${ONECAD_OCCT_ROOT}/share/onecad/occt-build.cmake")
    if(ONECAD_OCCT_SOURCE_COMMIT STREQUAL "unknown" OR
       NOT DEFINED ONECAD_OCCT_BUILT_ID OR ONECAD_OCCT_BUILT_ID STREQUAL "" OR
       ONECAD_OCCT_NORMALIZED_BUILD_OPTIONS STREQUAL "")
        message(FATAL_ERROR
            "OCCT artifact metadata is incomplete; rebuild it with scripts/build-pinned-occt.sh")
    endif()
    if(DEFINED ONECAD_OCCT_BUILT_ID)
        if(ONECAD_OCCT_BUILD_ID STREQUAL "local")
            set(ONECAD_OCCT_BUILD_ID "${ONECAD_OCCT_BUILT_ID}")
        elseif(NOT ONECAD_OCCT_BUILD_ID STREQUAL ONECAD_OCCT_BUILT_ID)
            message(FATAL_ERROR
                "ONECAD_OCCT_BUILD_ID (${ONECAD_OCCT_BUILD_ID}) does not match artifact metadata (${ONECAD_OCCT_BUILT_ID})")
        endif()
    endif()
elseif(ONECAD_OCCT_BUILD_ID STREQUAL "local" OR ONECAD_OCCT_BUILD_ID STREQUAL "")
    message(FATAL_ERROR
        "OCCT artifact metadata is absent; set a unique ONECAD_OCCT_BUILD_ID or use scripts/build-pinned-occt.sh")
else()
    # External exact roots remain usable for development, but never share the
    # default provenance token with another binary-compatible build.
    set(ONECAD_OCCT_SOURCE_COMMIT "external:${ONECAD_OCCT_BUILD_ID}")
endif()
set(ONECAD_KERNEL_POLICY_VERSION "1")
set(ONECAD_OCCT_FINGERPRINT_SEED
    "occtVersion=${OpenCASCADE_VERSION}|sourceCommit=${ONECAD_OCCT_SOURCE_COMMIT}|buildOptions=${ONECAD_OCCT_NORMALIZED_BUILD_OPTIONS}|buildId=${ONECAD_OCCT_BUILD_ID}|kernelPolicyVersion=${ONECAD_KERNEL_POLICY_VERSION}")

# ---------------------------------------------------------------------------
# Vendored PlaneGCS (FreeCAD Sketcher solver) — added BEFORE the first-party
# -Wall -Wextra so PlaneGCS builds quietly, mirroring OneCAD-CPP's root
# CMakeLists (add_subdirectory(third_party/planegcs) precedes its warning
# flags). Directory-scoped add_compile_options only affects targets declared
# after it, so PlaneGCS's own sources never see -Wall -Wextra.
# ---------------------------------------------------------------------------
add_subdirectory(third_party/planegcs)

# Mark PlaneGCS's public headers as SYSTEM for every consumer so warnings from
# GCS.h/Geo.h/Constraints.h do not leak into first-party TUs compiled with
# -Wall -Wextra. Done here (not in the vendored CMakeLists, which is copied
# wholesale) to keep the vendor tree byte-identical to OneCAD-CPP.
get_target_property(_planegcs_inc planegcs INTERFACE_INCLUDE_DIRECTORIES)
if(_planegcs_inc)
    set_target_properties(planegcs PROPERTIES
        INTERFACE_SYSTEM_INCLUDE_DIRECTORIES "${_planegcs_inc}")
endif()

# --- Warnings: first-party code only. No -Werror (per project convention). ---
add_compile_options(-Wall -Wextra)

# ---------------------------------------------------------------------------
# worker_core : STATIC library — everything except main.cpp
# ---------------------------------------------------------------------------
add_library(worker_core STATIC
    src/protocol/Frame.cpp
    src/protocol/Envelope.cpp
    src/protocol/Dispatcher.cpp
    # --- W-WP3b: sketch solver lane (SCHEMA §7.4 verbs) ---
    src/protocol/SolverLane.cpp
    # --- W-WP4: session + transactional ExecutePlan (SCHEMA §7.1/§7.2) ---
    src/util/Hashing.cpp
    src/session/Session.cpp
    src/session/ScratchJob.cpp
    src/session/ShapeMetrics.cpp
    src/session/Signatures.cpp
    src/session/PlanExecutor.cpp
    src/session/PreviewOp.cpp
    # --- W-WP5: REAL OCCT ops + ElementMap V2 + tessellation ---
    src/elementmap/ElementMapPartition.cpp
    # --- WP3 C3: one history over a multi-stage chain (OffsetFace DirectModeler) ---
    src/elementmap/ComposedHistory.cpp
    # --- W-WP6: resolution ladder (scoring / min-cost assignment / orchestration) ---
    src/elementmap/Scoring.cpp
    src/elementmap/Assignment.cpp
    src/elementmap/Ladder.cpp
    src/kernel/diagnostics/OperationDiagnostic.cpp
    src/kernel/validation/GeometryPrecision.cpp
    src/kernel/validation/ShapeAudit.cpp
    src/kernel/fillet/FilletAnalyzer.cpp
    src/kernel/fillet/FilletBuilder.cpp
    src/kernel/fillet/EdgeContour.cpp
    src/kernel/fillet/FilletSemanticChecks.cpp
    src/kernel/fillet/BlendEvidence.cpp
    src/kernel/fillet/BlendRecognizer.cpp
    src/kernel/fillet/AnalyticBlend.cpp
    src/ops/OpCommon.cpp
    src/ops/ExtrudeOp.cpp
    src/ops/BooleanOp.cpp
    # --- W-WP6: new OCCT ops (Revolve / Fillet / Chamfer) + STEP IO ---
    src/ops/RevolveOp.cpp
    src/ops/FilletChamferOp.cpp
    # --- M6a: breadth ops (Shell / Linear+Circular pattern / MirrorBody) ---
    src/ops/ShellOp.cpp
    # --- OFFSET-FACE W1: direct-modeling per-face offset (`op.offsetFace`, §7.3)
    #     + its read-only authoring handshake (`PrepareOffsetFace`, §7.6). ---
    src/ops/OffsetFaceOp.cpp
    src/session/PrepareOffsetFace.cpp
    src/session/PrepareEdgeOp.cpp
    src/ops/PatternOp.cpp
    src/ops/MirrorOp.cpp
    # --- WP-B W0: rigid multi-target placement (`TransformBody`, SCHEMA §7.3) ---
    src/ops/TransformOp.cpp
    # --- WP-C T3: machined hole (simple / counterbore / countersink, SCHEMA §7.3) ---
    src/ops/HoleTool.cpp
    src/ops/HoleOp.cpp
    # --- COMPONENT-LIBRARY WP-0.2: placed-component op family (`PlaceComponent`,
    #     SCHEMA §7.3) ---
    src/ops/ComponentOp.cpp
    # --- COMPONENT-LIBRARY WP-A1: per-family generator dispatch + dimension tables
    #     (spec §6) ---
    src/ops/ComponentGenerators.cpp
    src/ops/FastenerTables.cpp
    # --- COMPONENT-LIBRARY P3 WP-3.1: mate re-seat candidate-transform math ---
    src/ops/ComponentMateSolver.cpp
    src/io/ExportStep.cpp
    # --- COMPONENT-LIBRARY WP-3.2: body → §7.3 replay-codec bytes (`ExportGeometry`,
    #     SCHEMA §7.8) — what an `embedded` / `document` component source is baked
    #     from. ---
    src/io/ExportGeometry.cpp
    src/io/MeshExport.cpp
    src/io/Checkpoint.cpp
    # --- STEP-IMPORT WP-A W0: pure STEP read core + OCCT global-state guards. ---
    src/io/OcctStaticGuard.cpp
    src/io/StepRead.cpp
    # --- STEP-IMPORT WP-A W1: the `ImportStep` op (§7.3) + the read-only
    #     `InspectStep` verb (§7.8) + the BinTools brep byte form they share. ---
    src/io/BrepCodec.cpp
    # --- WP-A W4.5: XCAF fidelity — the BinXCAF (`.xbf`) replay codec that keeps
    #     product names + face colors across save/reopen, and the attribute-only
    #     STEP pass that harvests them. OCCT's XCAF/OCAF libs (TKXCAF, TKLCAF,
    #     TKBinXCAF, TKCDF, TKVCAF) already ride in ${OpenCASCADE_LIBRARIES}. ---
    src/io/OcafApp.cpp
    src/io/XcafCodec.cpp
    src/io/XcafRead.cpp
    src/io/InspectStep.cpp
    src/ops/ImportOp.cpp
    src/session/ElementIdentity.cpp
    # --- WP-C1: read-only exact mass properties (SCHEMA §7.5) ---
    src/session/MassProperties.cpp
    src/session/BodyTopology.cpp
    src/tess/Mesh1.cpp
    src/tess/MeshHandle.cpp
    src/tess/Tessellate.cpp
    src/sketch/WireSketch.cpp
    src/sketch/RegionId.cpp
    # --- SKETCH-ON-FACE W1: face-boundary projection (SCHEMA §7.6) ---
    src/sketch/FaceBoundaryProjector.cpp
    src/session/FaceProjection.cpp
    # --- COMPONENT-LIBRARY P0.1: interactive surface classification (SCHEMA §7.5) ---
    src/session/ClassifyElement.cpp
    # --- GEAR-GENERATOR G0: pure-math gear samplers (no OCCT), SCHEMA §7.3
    #     `Gear` op family (Gear Generator implementation plan, phase G0). ---
    src/kernel/gear/InvoluteMath.cpp
    src/kernel/gear/TimingMath.cpp
    src/kernel/gear/TimingTMath.cpp
    src/kernel/gear/LanternMath.cpp
    src/kernel/gear/WormMath.cpp
    # --- GEAR-GENERATOR G1: new kernel geometry capability + the gear solid
    #     builder (SCHEMA §7.3 `Gear`). ---
    src/kernel/geometry/BSpline.cpp
    src/ops/gear/GearTool.cpp
    src/ops/GearOp.cpp
    # --- W-WP2 kernel port (mirrors OneCAD-CPP src/kernel onecad_kernel lib) ---
    src/kernel/geometry/EdgeChainer.cpp
    src/kernel/modeling/FaceExtrudeProfileBuilder.cpp
    src/kernel/topology/CoplanarFacePatch.cpp
    src/kernel/topology/FacePatchResolver.cpp
    src/kernel/topology/SelectionTopologyResolver.cpp
    # --- W-WP3a: Qt-free sketch entity stack (ported from OneCAD-CPP
    #     src/core/sketch; QString/QUuid/QJson/qDebug stripped). ---
    src/sketch/SketchEntity.cpp
    src/sketch/SketchPoint.cpp
    src/sketch/SketchLine.cpp
    src/sketch/SketchArc.cpp
    src/sketch/SketchCircle.cpp
    src/sketch/SketchEllipse.cpp
    src/sketch/SketchConstraint.cpp
    src/sketch/constraints/Constraints.cpp
    src/sketch/solver/ConstraintSolver.cpp
    src/sketch/solver/SolverAdapter.cpp
    src/sketch/Sketch.cpp
    src/sketch/ConstraintApplicability.cpp    # onecad::ui selection→constraint rules
    # --- W-WP3a: loop stack (W-WP2 copied, now compiled — needs sketch types) ---
    src/loop/AdjacencyGraph.cpp
    src/loop/FaceBuilder.cpp
    src/loop/LoopDetector.cpp
    src/loop/PolygonFill.cpp
    src/loop/RegionTable.cpp
    src/loop/RegionUtils.cpp
    # --- W-WP3a: boolean op (W-WP2 copied, now compiled; include repointed to
    #     worker-local modeling/BooleanMode.h for app::BooleanMode) ---
    src/modeling/BooleanOperation.cpp
)

target_include_directories(worker_core
    PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/src
    ${CMAKE_CURRENT_SOURCE_DIR}/src/kernel  # kernel-internal includes: "elementmap/...", "geometry/..."
    ${CMAKE_CURRENT_SOURCE_DIR}/third_party
    ${OpenCASCADE_INCLUDE_DIR}
)

target_link_libraries(worker_core
    PUBLIC
    ${OpenCASCADE_LIBRARIES}
    Eigen3::Eigen      # REQUIRED as of W-WP2
    Boost::headers     # REQUIRED as of W-WP2
    planegcs           # vendored solver (linked now; used by W-WP3 sketch solver)
)

target_compile_definitions(worker_core PUBLIC
    ONECAD_OCCT_VERSION="${ONECAD_OCCT_VERSION}"
    ONECAD_OCCT_BUILD_ID="${ONECAD_OCCT_BUILD_ID}"
    ONECAD_OCCT_FINGERPRINT_SEED="${ONECAD_OCCT_FINGERPRINT_SEED}"
    ONECAD_KERNEL_POLICY_VERSION=${ONECAD_KERNEL_POLICY_VERSION}
)

if(OpenCASCADE_LIBRARY_DIR)
    target_link_directories(worker_core PUBLIC ${OpenCASCADE_LIBRARY_DIR})
endif()

# ---------------------------------------------------------------------------
# onecad-worker : the sidecar executable
# ---------------------------------------------------------------------------
add_executable(onecad-worker src/main.cpp)
target_link_libraries(onecad-worker PRIVATE worker_core)

# ---------------------------------------------------------------------------
# Testing
# ---------------------------------------------------------------------------
enable_testing()
add_subdirectory(tests)
add_subdirectory(tools/harness)
add_subdirectory(tools/solverbench)
add_subdirectory(tools/filletbench)
add_subdirectory(tools/kernelbench-runner)
