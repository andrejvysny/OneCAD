// main.cpp — OneCAD C++ sidecar worker entry point.
//
// Responsibilities at this stage:
//   * assert little-endian host (compile-time + runtime)
//   * route OCCT diagnostics to stderr (proves TKernel linkage; guards stdout)
//   * emit the UNSOLICITED hello frame (SCHEMA §6) as the first output frame
//   * register the lifecycle + solver-lane verbs
//   * run the reader/kernel/solver dispatch loop over stdin/stdout, OR
//   * with --selftest, exercise hello + a solver op in-process and exit 0.
//
// The verb table, handlers and dispatch loop live in capi/WorkerRuntime.cpp,
// shared with the in-process library entry (capi/onecad_worker.h); this file
// keeps only the PROCESS concerns: env log level, argv, stdin/stdout.
//
// stdout carries protocol frames ONLY. All diagnostics go to stderr via WLOG_*.
#include <cstring>
#include <unistd.h>

#include "capi/WorkerRuntime.h"
#include "util/LittleEndian.h"
#include "util/Log.h"

int main(int argc, char** argv) {
    // 0. Read ONECAD_WORKER_LOG before anything else logs, so --selftest and
    //    every startup diagnostic below already run at the requested level.
    onecad::log::init_level_from_env();

    // 1. Little-endian guarantee (compile-time static_assert lives in the
    //    header; this is the runtime belt-and-suspenders check).
    if (!onecad::endian::host_is_little_endian()) {
        WLOG_ERROR("fatal: host is not little-endian; worker requires LE");
        return 2;
    }

    // 2. OCCT diagnostics -> stderr (and proves TKernel is linked).
    onecad::worker::route_occt_diagnostics();

    // 3. Argument handling.
    bool selftest = false;
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--selftest") == 0) {
            selftest = true;
        } else {
            WLOG_WARN("ignoring unknown argument: %s", argv[i]);
        }
    }

    if (selftest) {
        return onecad::worker::run_selftest();
    }

    // 4. Normal operation: emit the unsolicited hello, then dispatch stdin/stdout.
    return onecad::worker::run_dispatch_loop(STDIN_FILENO, STDOUT_FILENO);
}
