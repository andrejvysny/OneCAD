// WorkerRuntime.h — the worker's verb table + dispatch loop, shared by the
// sidecar executable (main.cpp) and the in-process library entry
// (capi/onecad_worker.cpp). P0.3 in-process worker spike, ONECAD-258.
//
// Factored out of main.cpp verbatim: main.cpp now only does process concerns
// (env log level, argv, stdin/stdout fds); everything both hosts must run
// identically lives here, so the two can never drift.
#pragma once

#include <nlohmann/json.hpp>

#include "protocol/Dispatcher.h"

namespace onecad::worker {

// Route OCCT's default messenger off std::cout: the cerr printer in the sidecar,
// the Log.h bridge when a library-mode sink is installed (capi/OcctLogPrinter.h).
void route_occt_diagnostics();

// SCHEMA §6 hello.result payload.
nlohmann::json make_hello_result();

// `--selftest`: validate the hello payload + run one SketchUpsert in-process.
// Returns the process exit code (0 == OK).
int run_selftest();

// Emit the unsolicited hello, then run the reader/kernel/solver/status loop over
// the given fds until EOF, Shutdown or protocol error. Returns the exit code the
// sidecar process exits with (0 clean, 2 protocol loss). With `close_fds` the
// call owns both fds and closes each exactly once before returning (library
// mode); the sidecar passes false and leaves stdin/stdout to process exit.
// `journal` (library only; empty for the sidecar) is installed on the
// Dispatcher before the loop starts.
int run_dispatch_loop(int in_fd, int out_fd, bool close_fds = false,
                      onecad::protocol::JournalHooks journal = {});

}  // namespace onecad::worker
