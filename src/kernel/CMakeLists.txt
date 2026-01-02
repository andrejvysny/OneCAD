# Kernel module - OCCT wrappers, ElementMap, PlaneGCS solver
add_library(onecad_kernel INTERFACE)

target_include_directories(onecad_kernel INTERFACE ${CMAKE_CURRENT_SOURCE_DIR})

# Link against OpenCASCADE when content is added
# target_link_libraries(onecad_kernel INTERFACE ${OpenCASCADE_LIBRARIES})
