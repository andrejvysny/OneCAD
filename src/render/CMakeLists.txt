# Render module - Camera, Grid, and rendering utilities
add_library(onecad_render STATIC
    BodyRenderer.cpp
    Camera3D.cpp
    Grid3D.cpp
    scene/SceneMeshStore.cpp
    tessellation/TessellationCache.cpp
)

target_include_directories(onecad_render
    PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${OpenCASCADE_INCLUDE_DIR}
    PRIVATE
    ${CMAKE_SOURCE_DIR}/src
)

target_link_libraries(onecad_render
    PUBLIC
    Qt6::Core
    Qt6::Gui
    Qt6::OpenGLWidgets
    ${OpenCASCADE_LIBRARIES}
)
