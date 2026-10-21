# Embedded-kernel identity inputs (docs/ios/KERNEL.md, manifest v2).
#
# The in-process worker has no binary SHA-256 a host can check, so its release
# identity (onecad-embedded-kernel.json, written by scripts/build-ios-worker.sh)
# names the version axes the hello reports. They are READ from the worker
# sources here — never restated as literals a later edit could leave stale —
# and configure fails when a pattern no longer matches exactly once. The host
# still checks the manifest against the live hello at start (verify_hello), so
# a drift that slipped past this parse cannot ship silently either.

# `regex` must not match a ';' — file(STRINGS) escapes it in the returned line.
function(onecad_source_constant out_var file regex)
    file(STRINGS "${file}" _matches REGEX "${regex}")
    list(LENGTH _matches _count)
    if(NOT _count EQUAL 1)
        message(FATAL_ERROR
            "embedded identity: expected exactly one line matching '${regex}' in ${file}, found ${_count}")
    endif()
    if(NOT _matches MATCHES "${regex}")
        message(FATAL_ERROR "embedded identity: cannot parse '${_matches}' in ${file}")
    endif()
    set(${out_var} "${CMAKE_MATCH_1}" PARENT_SCOPE)
endfunction()

set(_onecad_src "${CMAKE_CURRENT_SOURCE_DIR}/src")
onecad_source_constant(_onecad_abi "${_onecad_src}/capi/onecad_worker.h"
    "^#define ONECAD_WORKER_ABI_VERSION ([0-9]+)$")
onecad_source_constant(_onecad_protocol "${_onecad_src}/capi/WorkerRuntime.cpp"
    "^constexpr int kProtocolVersion = ([0-9]+)")
onecad_source_constant(_onecad_worker_version "${_onecad_src}/capi/WorkerRuntime.cpp"
    "^constexpr const char\\* kWorkerVersion = \"([0-9A-Za-z.+-]+)\"")
onecad_source_constant(_onecad_quantization "${_onecad_src}/capi/WorkerRuntime.cpp"
    "^constexpr int kQuantizationVersion = ([0-9]+)")
onecad_source_constant(_onecad_solver_policy "${_onecad_src}/util/PolicyVersions.h"
    "^inline constexpr unsigned kSolverPolicyVersion = ([0-9]+)")

if(ONECAD_WORKER_LIB_TEST_HOOKS)
    set(_onecad_test_hooks 1)
else()
    set(_onecad_test_hooks 0)
endif()

# key=value, one per line; the value runs to the end of the line (the seed
# contains '=' and '|'). The fingerprint is NOT computed here: it is
# hex16(fnv1a(seed)) over exactly this seed, done by the script.
file(WRITE "${CMAKE_BINARY_DIR}/onecad_worker_identity.txt"
    "abiVersion=${_onecad_abi}\n"
    "protocolVersion=${_onecad_protocol}\n"
    "workerVersion=${_onecad_worker_version}\n"
    "quantizationVersion=${_onecad_quantization}\n"
    "solverPolicyVersion=${_onecad_solver_policy}\n"
    "occtVersion=${ONECAD_OCCT_VERSION}\n"
    "occtBuildId=${ONECAD_OCCT_BUILD_ID}\n"
    "fingerprintSeed=${ONECAD_OCCT_FINGERPRINT_SEED}\n"
    "testHooks=${_onecad_test_hooks}\n")
