# UI module - Qt Widgets UI components
add_library(onecad_ui STATIC
    mainwindow/MainWindow.cpp
    theme/ThemeManager.cpp
    viewport/Viewport.cpp
    viewcube/ViewCube.cpp
    navigator/ModelNavigator.cpp
    inspector/PropertyInspector.cpp
    toolbar/ContextToolbar.cpp
)

target_include_directories(onecad_ui PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})

target_link_libraries(onecad_ui
    PUBLIC
    onecad_render
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
    Qt6::OpenGLWidgets
)
