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
