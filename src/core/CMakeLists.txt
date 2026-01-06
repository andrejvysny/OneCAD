# Core CAD module - sketch engine, modeling, patterns, loop detection
add_library(onecad_core STATIC
    sketch/Sketch.cpp
    sketch/SketchEntity.cpp
    sketch/SketchPoint.cpp
    sketch/SketchLine.cpp
    sketch/SketchArc.cpp
    sketch/SketchCircle.cpp
    sketch/SketchEllipse.cpp
    sketch/SketchConstraint.cpp
    sketch/SketchRenderer.cpp
    sketch/SnapManager.cpp
    sketch/IntersectionManager.cpp
    sketch/AutoConstrainer.cpp
    sketch/constraints/Constraints.cpp
    sketch/solver/ConstraintSolver.cpp
    sketch/solver/SolverAdapter.cpp
    sketch/tools/SketchToolManager.cpp
    sketch/tools/LineTool.cpp
    sketch/tools/CircleTool.cpp
    sketch/tools/RectangleTool.cpp
    sketch/tools/ArcTool.cpp
    sketch/tools/EllipseTool.cpp
    sketch/tools/TrimTool.cpp
    sketch/tools/MirrorTool.cpp
    loop/AdjacencyGraph.cpp
    loop/LoopDetector.cpp
    loop/FaceBuilder.cpp
    loop/RegionUtils.cpp
    modeling/BooleanOperation.cpp
)

target_include_directories(onecad_core
    PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${OpenCASCADE_INCLUDE_DIR}
)

target_link_libraries(onecad_core
    PUBLIC
    Qt6::Core
    Qt6::Gui  # For QMatrix4x4 in SketchRenderer.h public API
    ${OpenCASCADE_LIBRARIES}
    PRIVATE
    Qt6::OpenGL
    Qt6::OpenGLWidgets
    planegcs
)

# Define header files for IDE visibility
set(SKETCH_HEADERS
    sketch/SketchTypes.h
    sketch/SketchEntity.h
    sketch/SketchPoint.h
    sketch/SketchLine.h
    sketch/SketchArc.h
    sketch/SketchCircle.h
    sketch/SketchEllipse.h
    sketch/SketchConstraint.h
    sketch/Sketch.h
    sketch/SketchRenderer.h
    sketch/SnapManager.h
    sketch/IntersectionManager.h
    sketch/AutoConstrainer.h
    sketch/constraints/Constraints.h
    sketch/solver/ConstraintSolver.h
    sketch/solver/SolverAdapter.h
    sketch/tools/SketchTool.h
    sketch/tools/SketchToolManager.h
    sketch/tools/LineTool.h
    sketch/tools/CircleTool.h
    sketch/tools/RectangleTool.h
    sketch/tools/ArcTool.h
    sketch/tools/EllipseTool.h
    sketch/tools/TrimTool.h
    sketch/tools/MirrorTool.h
)

set(LOOP_HEADERS
    loop/AdjacencyGraph.h
    loop/LoopDetector.h
    loop/FaceBuilder.h
    loop/RegionUtils.h
)

# Create a custom target to show headers in IDE
add_custom_target(onecad_core_headers SOURCES
    ${SKETCH_HEADERS}
    ${LOOP_HEADERS}
)

# When PlaneGCS types appear in onecad_core public headers,
# move planegcs to PUBLIC linkage in the block above.
