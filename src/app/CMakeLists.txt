add_library(onecad_app STATIC
    Application.cpp
    Logging.cpp
    commands/AddBodyCommand.cpp
    commands/AddOperationCommand.cpp
    commands/DeleteBodyCommand.cpp
    commands/DeleteSketchCommand.cpp
    commands/ModifyBodyCommand.cpp
    commands/RemoveOperationCommand.cpp
    commands/RenameBodyCommand.cpp
    commands/RenameSketchCommand.cpp
    commands/SetOperationSuppressionCommand.cpp
    commands/ToggleVisibilityCommand.cpp
    commands/UpdateOperationParamsCommand.cpp
    commands/CommandProcessor.cpp
    commands/RollbackCommand.cpp
    document/Document.cpp
    history/DependencyGraph.cpp
    history/RegenerationEngine.cpp
    selection/SelectionManager.cpp
)

target_include_directories(onecad_app
    PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${OpenCASCADE_INCLUDE_DIR}
    ${CMAKE_SOURCE_DIR}/src
)

target_link_libraries(onecad_app
    PUBLIC
    Qt6::Core
    onecad_core
    onecad_render
)
