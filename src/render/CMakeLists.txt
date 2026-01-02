# Render module - Camera, Grid, and rendering utilities
add_library(onecad_render STATIC
    Camera3D.cpp
    Grid3D.cpp
)

target_include_directories(onecad_render PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})

target_link_libraries(onecad_render
    PUBLIC
    Qt6::Core
    Qt6::Gui
    Qt6::OpenGLWidgets
)
