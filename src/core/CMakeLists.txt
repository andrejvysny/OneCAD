# Core CAD module - sketch engine, modeling, patterns, loop detection
add_library(onecad_core STATIC
    sketch/Sketch.cpp
    sketch/SketchEntity.cpp
    sketch/SketchPoint.cpp
    sketch/SketchLine.cpp
    sketch/SketchArc.cpp
    sketch/SketchCircle.cpp
    sketch/SketchConstraint.cpp
    sketch/constraints/Constraints.cpp
    sketch/solver/ConstraintSolver.cpp
    sketch/solver/SolverAdapter.cpp
)

target_include_directories(onecad_core
    PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${OpenCASCADE_INCLUDE_DIR}
)

target_link_libraries(onecad_core
    PUBLIC
    Qt6::Core
    ${OpenCASCADE_LIBRARIES}
)

# Define header files for IDE visibility
set(SKETCH_HEADERS
    sketch/SketchTypes.h
    sketch/SketchEntity.h
    sketch/SketchPoint.h
    sketch/SketchLine.h
    sketch/SketchArc.h
    sketch/SketchCircle.h
    sketch/SketchConstraint.h
    sketch/Sketch.h
    sketch/SketchRenderer.h
    sketch/constraints/Constraints.h
    sketch/solver/ConstraintSolver.h
    sketch/solver/SolverAdapter.h
)

set(LOOP_HEADERS
    loop/LoopDetector.h
)

# Create a custom target to show headers in IDE
add_custom_target(onecad_core_headers SOURCES
    ${SKETCH_HEADERS}
    ${LOOP_HEADERS}
)

# When PlaneGCS is integrated, add linking here:
# - Use PRIVATE if PlaneGCS is internal to implementation
# - Use PUBLIC if PlaneGCS types appear in onecad_core public headers
# target_link_libraries(onecad_core PRIVATE planegcs)
