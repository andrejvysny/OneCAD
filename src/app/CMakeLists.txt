add_library(onecad_app STATIC
    Application.cpp
)

target_include_directories(onecad_app PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})

target_link_libraries(onecad_app
    PUBLIC
    Qt6::Core
)
