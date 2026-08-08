add_library(benchmark_core STATIC
    ../../src/benchmark/CaseParser.cpp
    ../../src/benchmark/CaseParserV2.cpp
    ../../src/benchmark/SelectorParser.cpp
    ../../src/benchmark/Geometry.cpp
    ../../src/benchmark/GeometrySupportPair.cpp
    ../../src/benchmark/DeepAudit.cpp
    ../../src/benchmark/FilletRun.cpp
    ../../src/benchmark/Artifacts.cpp
    ../../src/benchmark/Execution.cpp
    ../../src/benchmark/SemanticValidation.cpp
)
target_include_directories(benchmark_core PUBLIC ${PROJECT_SOURCE_DIR}/src)
target_link_libraries(benchmark_core PUBLIC worker_core)

add_executable(onecad-kernelbench-runner main.cpp)
target_link_libraries(onecad-kernelbench-runner PRIVATE benchmark_core)
set_target_properties(onecad-kernelbench-runner PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}")

add_test(NAME kernelbench_runner_schema
    COMMAND ${CMAKE_COMMAND}
        -DRUNNER=$<TARGET_FILE:onecad-kernelbench-runner>
        -DINPUT=${CMAKE_CURRENT_SOURCE_DIR}/malformed-request.json
        -P ${CMAKE_CURRENT_SOURCE_DIR}/malformed-request-test.cmake)

add_executable(test_kernelbench_audit audit_fixtures.cpp)
target_link_libraries(test_kernelbench_audit PRIVATE benchmark_core)
add_test(NAME kernelbench_audit COMMAND test_kernelbench_audit)

add_executable(test_kernelbench_case_v2 case_v2_fixtures.cpp)
target_link_libraries(test_kernelbench_case_v2 PRIVATE benchmark_core)
add_test(NAME kernelbench_case_v2 COMMAND test_kernelbench_case_v2)
