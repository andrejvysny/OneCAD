# OneCAD Tests
enable_testing()

# OCAF Prototype
add_executable(proto_tnaming prototypes/proto_tnaming.cpp)
target_link_libraries(proto_tnaming 
    PRIVATE 
    ${OpenCASCADE_LIBRARIES}
)
target_include_directories(proto_tnaming PRIVATE ${OpenCASCADE_INCLUDE_DIR})

# Custom Map Prototype
add_executable(proto_custom_map prototypes/proto_custom_map.cpp)
target_link_libraries(proto_custom_map 
    PRIVATE 
    ${OpenCASCADE_LIBRARIES}
)
target_include_directories(proto_custom_map PRIVATE ${OpenCASCADE_INCLUDE_DIR} ${CMAKE_SOURCE_DIR}/src)

# ElementMap Rigorous Tests
add_executable(proto_elementmap_rigorous prototypes/proto_elementmap_rigorous.cpp)
target_link_libraries(proto_elementmap_rigorous
    PRIVATE
    ${OpenCASCADE_LIBRARIES}
)
target_include_directories(proto_elementmap_rigorous PRIVATE ${OpenCASCADE_INCLUDE_DIR} ${CMAKE_SOURCE_DIR}/src)

# Sketch Geometry Prototype
add_executable(proto_sketch_geometry prototypes/proto_sketch_geometry.cpp)
target_link_libraries(proto_sketch_geometry
    PRIVATE
    onecad_core
)

# Sketch Constraints Prototype
add_executable(proto_sketch_constraints prototypes/proto_sketch_constraints.cpp)
target_link_libraries(proto_sketch_constraints
    PRIVATE
    onecad_core
)

# Sketch Solver Adapter Prototype
add_executable(proto_sketch_solver prototypes/proto_sketch_solver.cpp)
target_link_libraries(proto_sketch_solver
    PRIVATE
    onecad_core
)

# PlaneGCS Integration Prototype
add_executable(proto_planegcs_integration prototypes/proto_planegcs_integration.cpp)
target_link_libraries(proto_planegcs_integration
    PRIVATE
    planegcs
)

# Loop Detector Prototype
add_executable(proto_loop_detector prototypes/proto_loop_detector.cpp)
target_link_libraries(proto_loop_detector
    PRIVATE
    onecad_core
)

# Face Builder Prototype
add_executable(proto_face_builder prototypes/proto_face_builder.cpp)
target_link_libraries(proto_face_builder
    PRIVATE
    onecad_core
)

# Fixed constraint and translatePlaneInSketch prototype
add_executable(proto_sketch_fixed_and_move prototypes/proto_sketch_fixed_and_move.cpp)
target_link_libraries(proto_sketch_fixed_and_move
    PRIVATE
    onecad_core
)

# Viewport Compilation Test
add_executable(test_compile test_compile.cpp)
target_link_libraries(test_compile
    PRIVATE
    onecad_app
    onecad_ui
    onecad_io
    Qt6::Widgets
    Qt6::Gui
    Qt6::Core
    Qt6::OpenGL
    Qt6::OpenGLWidgets
)
target_include_directories(test_compile PRIVATE ${CMAKE_SOURCE_DIR}/src)

# Model Picker Prototype
add_executable(proto_model_picker prototypes/proto_model_picker.cpp)
target_link_libraries(proto_model_picker
    PRIVATE
    onecad_ui
    onecad_io
    onecad_app
    Qt6::Gui
    Qt6::Core
)
target_include_directories(proto_model_picker PRIVATE ${CMAKE_SOURCE_DIR}/src)

# Tessellation Cache Prototype
add_executable(proto_tessellation_cache prototypes/proto_tessellation_cache.cpp)
target_link_libraries(proto_tessellation_cache
    PRIVATE
    onecad_app
    Qt6::Core
)
target_include_directories(proto_tessellation_cache PRIVATE ${CMAKE_SOURCE_DIR}/src)

# Pick Mesh Integration Prototype
add_executable(proto_pickmesh_integration prototypes/proto_pickmesh_integration.cpp)
target_link_libraries(proto_pickmesh_integration
    PRIVATE
    onecad_ui
    onecad_io
    onecad_app
    Qt6::Core
    Qt6::Gui
)
target_include_directories(proto_pickmesh_integration PRIVATE ${CMAKE_SOURCE_DIR}/src)

# Pick Topology Promotion Prototype
add_executable(proto_pick_topology_promotion prototypes/proto_pick_topology_promotion.cpp)
target_link_libraries(proto_pick_topology_promotion
    PRIVATE
    onecad_ui
    onecad_io
    onecad_app
    Qt6::Core
    Qt6::Gui
)
target_include_directories(proto_pick_topology_promotion PRIVATE ${CMAKE_SOURCE_DIR}/src)

# Regeneration Engine Prototype
add_executable(proto_regeneration prototypes/proto_regeneration.cpp)
target_link_libraries(proto_regeneration
    PRIVATE
    onecad_app
    onecad_core
    Qt6::Core
)
target_include_directories(proto_regeneration PRIVATE ${CMAKE_SOURCE_DIR}/src)
