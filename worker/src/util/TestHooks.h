// TestHooks.h — whether this build compiles in the worker's test-only hooks:
// the `Debug.Busy` verb (WorkerRuntime.cpp) and the `__slow` op hook
// (PlanExecutor.cpp).
//
// The sidecar keeps them unconditionally: ctest, the NDJSON fixtures and the
// Rust chaos drills drive them, and a sidecar hook can only hurt its own
// process. The in-process library (ONECAD_WORKER_LIBRARY) runs inside the HOST
// app, so it compiles them out unless the build opts in with
// ONECAD_WORKER_TEST_HOOKS (scripts/build-ios-worker.sh --test-hooks, recorded
// as `testHooks` in onecad-embedded-kernel.json). A release app must not answer
// `Debug.Busy`: an unknown verb is a terminal PROTOCOL_ERROR there.
#pragma once

#if !defined(ONECAD_WORKER_LIBRARY) || defined(ONECAD_WORKER_TEST_HOOKS)
#define ONECAD_WORKER_HOOKS 1
#else
#define ONECAD_WORKER_HOOKS 0
#endif
