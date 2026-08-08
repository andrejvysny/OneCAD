add_executable(filletbench main.cpp)
target_link_libraries(filletbench PRIVATE worker_core)

add_test(
    NAME filletbench_smoke
    COMMAND filletbench --quick
            --out ${CMAKE_CURRENT_BINARY_DIR}/RESULTS_smoke.md
)
