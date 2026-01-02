# I/O module - native format and STEP import/export
add_library(onecad_io INTERFACE)

target_include_directories(onecad_io INTERFACE ${CMAKE_CURRENT_SOURCE_DIR})

# Link against OCCT STEP modules when content is added
# target_link_libraries(onecad_io INTERFACE ${OpenCASCADE_LIBRARIES})
