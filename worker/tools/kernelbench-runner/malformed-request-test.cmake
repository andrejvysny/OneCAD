execute_process(
    COMMAND "${RUNNER}"
    INPUT_FILE "${INPUT}"
    OUTPUT_VARIABLE runner_stdout
    ERROR_VARIABLE runner_stderr
    RESULT_VARIABLE runner_status)

if(NOT runner_status EQUAL 2)
    message(FATAL_ERROR "malformed request returned ${runner_status}, expected 2")
endif()
if(NOT runner_stdout STREQUAL "")
    message(FATAL_ERROR "malformed request wrote stdout: ${runner_stdout}")
endif()
if(runner_stderr STREQUAL "")
    message(FATAL_ERROR "malformed request did not explain failure on stderr")
endif()
