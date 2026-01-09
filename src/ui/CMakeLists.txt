# UI module - Qt Widgets UI components
add_library(onecad_ui STATIC
    components/SidebarToolButton.cpp
    mainwindow/MainWindow.cpp
    theme/ThemeConfig.cpp
    theme/ThemeManager.cpp
    viewport/Viewport.cpp
    viewport/RenderDebugPanel.cpp
    viewport/ViewportRevolve.cpp
    viewport/ViewportModeling.cpp
    viewcube/ViewCube.cpp
    navigator/ModelNavigator.cpp
    toolbar/ContextToolbar.cpp
    tools/ModelingToolManager.cpp
    tools/ExtrudeTool.cpp
    tools/RevolveTool.cpp
    tools/FilletChamferTool.cpp
    tools/ShellTool.cpp
    sketch/ConstraintPanel.cpp
    sketch/DimensionEditor.cpp
    sketch/SketchModePanel.cpp
    selection/DeepSelectPopup.cpp
    selection/SketchPickerAdapter.cpp
    selection/ModelPickerAdapter.cpp
)

target_include_directories(onecad_ui PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})

target_link_libraries(onecad_ui
    PUBLIC
    onecad_render
    onecad_core
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
    Qt6::OpenGLWidgets
    Qt6::Svg
    PRIVATE
    onecad_app
)
