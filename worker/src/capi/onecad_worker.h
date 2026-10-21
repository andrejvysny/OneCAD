/* onecad_worker.h — C ABI for running the OneCAD OCCT worker IN-PROCESS.
 *
 * The same worker runs as a library behind the same OCW1 wire
 * (protocol/SCHEMA.md): the host hands it one end of a socketpair and speaks
 * OCW1 to the other end exactly as it speaks to the sidecar's stdio. This is
 * the iPadOS kernel host (ADR-0018, docs/ios/ARCHITECTURE.md; iPadOS forbids
 * child processes) and a macOS test host for onecad-app's `embedded-kernel`
 * backend. The shipping macOS host stays the sidecar, for crash and hang
 * isolation. Build procedures and the identity manifest: docs/ios/KERNEL.md.
 *
 * Built by the CMake target `onecad_worker_lib` (static, ONECAD_WORKER_LIBRARY
 * defined: the `__crash` std::abort chaos hook is compiled out, and so are the
 * `Debug.Busy` verb and the `__slow` op hook unless ONECAD_WORKER_TEST_HOOKS is
 * defined — worker/src/util/TestHooks.h).
 *
 * C++ callers see every entry point `noexcept`: no C++ exception ever leaves
 * the library (an escape inside the loop becomes ONECAD_WORKER_EXIT_INTERNAL).
 */
#ifndef ONECAD_WORKER_H
#define ONECAD_WORKER_H

#include <stdint.h>

#ifdef __cplusplus
#define ONECAD_WORKER_NOEXCEPT noexcept
extern "C" {
#else
#define ONECAD_WORKER_NOEXCEPT
#endif

/* Bumped on any change to the functions or structs below (and every consumer —
 * crates/onecad-embedded-worker, onecad-embedded-kernel.json `abiVersion` —
 * moves with it). 2: onecad_worker_run_config and
 * the crash-journal callback (onecad_worker_run is unchanged). 3: the journal
 * callback takes an event struct (op position, prefix hash, fingerprint) and
 * the config gains occt_lock_wait_ms. */
#define ONECAD_WORKER_ABI_VERSION 3

/* Log levels, identical to onecad::log::Level (worker/src/util/Log.h). */
#define ONECAD_WORKER_LOG_ERROR 0
#define ONECAD_WORKER_LOG_WARN 1
#define ONECAD_WORKER_LOG_INFO 2
#define ONECAD_WORKER_LOG_DEBUG 3

/* Library-only exit code: invalid arguments or an exception escaped the loop. */
#define ONECAD_WORKER_EXIT_INTERNAL 3

/* One formatted diagnostic line: no timestamp, no level prefix, no newline at
 * all — a multi-line message (OCCT messenger texts often are) is split into one
 * call per line and blank lines are dropped. Called from ANY worker thread (reader, kernel, solver, status, and
 * OCCT/PlaneGCS helper threads), serialized by the worker's log mutex — the
 * callback must not call back into the worker and should return quickly. */
typedef void (*onecad_worker_log_fn)(int level, const char* msg, void* ctx);

int onecad_worker_abi_version(void) ONECAD_WORKER_NOEXCEPT;

/* Runs the worker's reader/kernel/solver/status loop — the exact loop the
 * sidecar's main() runs — reading OCW1 frames from `in_fd` and writing them to
 * `out_fd`. The unsolicited hello (SCHEMA §6) is the first frame written.
 *
 * Blocks until the input reaches EOF, a `Shutdown` request is served, or a
 * protocol error occurs, AND every worker lane has finished its current op.
 * Returns the code the sidecar process would exit with (0 clean, 2 protocol
 * loss), or ONECAD_WORKER_EXIT_INTERNAL.
 *
 * Threads: the calling thread runs the reader loop (give it a large stack;
 * onecad-app uses 8 MiB). The kernel, solver and status lanes are started with
 * an explicit 8 MiB stack (worker/src/util/LaneThread.h).
 *
 * WEDGE: an op that never returns blocks this call forever (the lanes are
 * joined, never abandoned — there is no safe way to stop a thread inside OCCT).
 * The host must time-box its wait after closing its end and treat expiry as
 * "relaunch required" (docs/ios/ARCHITECTURE.md "In-process failure policy").
 *
 * fds: must be distinct (dup() a socket to get two). Ownership of BOTH
 * transfers to this call; each is closed exactly once, after every worker lane
 * has stopped, just before it returns. If `out_fd` is a socket, SO_NOSIGPIPE is
 * set on it so a vanished host end is an EPIPE (orderly shutdown), never a
 * process-killing SIGPIPE. `in_fd` SHOULD be a socket: on Shutdown the worker
 * wakes its reader with shutdown(SHUT_RD) and keeps the fd number reserved
 * until return. A pipe can only be woken by close(), which frees the number
 * while the reader may still be about to read it; another host thread could
 * then reuse the number and the stale read would consume its bytes.
 *
 * Logging: `log_cb` (NULL = stderr, like the sidecar) and `log_level` replace
 * the sidecar's ONECAD_WORKER_LOG env var; the env is NOT read. The sink is
 * PROCESS-GLOBAL: while several calls run concurrently, EVERY worker's lines go
 * to the most recently started live call's callback (lines carry no worker
 * identity); when a call returns, the sink falls back to the newest call still
 * running, or to stderr. A callback is never invoked after its own call has
 * returned. The level is process-global too (last call wins). OCCT's default
 * messenger and the OCAF application's messenger are routed to the same sink.
 *
 * NOT routed by the library (the host must handle it): OCCT's direct writes to
 * the C++ `cout` stream (BinTools and ~10 other TKBRep files in 8.0.1) still
 * reach the host's stdout. The Rust spike swaps that stream's streambuf
 * (spikes/inproc-worker/csrc/cout_bridge.cpp); it cannot live under worker/src
 * while scripts/check-worker-stdout-hygiene.sh bans the identifier there.
 * PlaneGCS's stdout diagnostics are silenced in library builds (NoDebug).
 *
 * CONCURRENCY: several calls may run at once in one process. STEP import,
 * inspect and export (and the XCAF codec) are serialized process-wide
 * (io::occt_globals_mutex, io::ocaf_mutex). Those locks are taken with a
 * bounded wait (onecad_worker_config.occt_lock_wait_ms, default 20 s): a call
 * that cannot get one fails with a RETRIABLE OP_FAILED whose message starts
 * with "OCCT_GLOBALS_BUSY" instead of blocking its kernel lane — every verb
 * (InspectStep, ExportStep, ExportGeometry) and every ExecutePlan / PreviewPlan
 * / PreviewOp whose op (ImportStep, PlaceComponent) hit the lock; a plan is
 * refused as a whole, never recorded as an opFailed step
 * (worker/src/io/OcctBusy.h). A worker wedged
 * INSIDE a STEP call holds them for the rest of the process's life, so after
 * such a wedge every worker's STEP/XCAF calls fail busy until the app is
 * relaunched; modeling, meshing and starting new workers are unaffected
 * (starting a worker never takes these locks). */
int onecad_worker_run(int in_fd, int out_fd, onecad_worker_log_fn log_cb, void* ctx,
                      int log_level) ONECAD_WORKER_NOEXCEPT;

/* Crash-journal events (ABI 2). */
#define ONECAD_WORKER_JOURNAL_JOB_BEGIN 0
#define ONECAD_WORKER_JOURNAL_PLAN_OP_BEGIN 1
#define ONECAD_WORKER_JOURNAL_JOB_END 2

/* One journal event. Every pointer is valid only for the duration of the call.
 * PLAN_OP_BEGIN describes the op about to run in THIS job's own args, so the
 * host can build the crash-circuit key (Rust plan_op_keys:
 * prefixHash|opId|fingerprint) without pre-arming anything per plan:
 *   name        = opId            step_index  = the op's stepIndex
 *   op_position = its index in args.ops (== its prefixHashes index; NOT the
 *                 stepIndex: an incremental plan's first op has stepIndex k>0)
 *   prefix_hash = args.prefixHashes[op_position], or args.expectedBaseHash
 *                 when the array is short (the same fallback Rust uses)
 * JOB_BEGIN / JOB_END: name = verb, step_index = op_position = 0,
 * prefix_hash = "". occt_fingerprint = the hello's occt.fingerprint, always. */
typedef struct onecad_worker_journal_event {
    uint32_t struct_size; /* sizeof(onecad_worker_journal_event) */
    int event;
    uint64_t request_id; /* the OCW1 request id */
    const char* name;
    uint64_t step_index;
    uint64_t op_position;
    const char* prefix_hash;
    const char* occt_fingerprint;
} onecad_worker_journal_event;

/* Called SYNCHRONOUSLY on the worker's kernel thread: JOB_BEGIN when a kernel
 * job is dequeued, PLAN_OP_BEGIN before each ExecutePlan / PreviewPlan op's work
 * starts, JOB_END after the job's terminal frame was written (or its write
 * failed) — so result serialization is inside the window. Whatever the host
 * persists inside the callback is on record before the
 * named work runs, so a crash that kills the process inside that work can be
 * attributed without having read a single frame (in-process, frames still in
 * the socket buffer die with the process). The kernel lane is blocked while
 * the callback runs; it must not call into the worker. Kernel lane only:
 * solver-lane (Sketch*) and status-lane jobs are not journaled. */
typedef void (*onecad_worker_journal_fn)(const onecad_worker_journal_event* event, void* ctx);

typedef struct onecad_worker_config {
    uint32_t struct_size; /* sizeof(onecad_worker_config) */
    int in_fd;
    int out_fd;
    onecad_worker_log_fn log_cb; /* NULL = stderr */
    void* log_ctx;
    int log_level;
    onecad_worker_journal_fn journal_cb; /* NULL = no journal events */
    void* journal_ctx; /* must stay valid until the call returns */
    /* Bounded wait for the process-wide OCCT locks (see CONCURRENCY above), ms.
     * 0 = default (20000). PROCESS-GLOBAL like log_level: last call wins. */
    uint32_t occt_lock_wait_ms;
} onecad_worker_config;

/* onecad_worker_run with the full configuration; identical contract. Returns
 * ONECAD_WORKER_EXIT_INTERNAL when `config` is NULL (nothing is closed: the fds
 * are unknown) or when `struct_size` is smaller than this header's struct (both
 * fds closed). On every other exit path — invalid fds, an exception before or
 * during the loop — both fds are closed exactly once before the call returns. */
int onecad_worker_run_config(const onecad_worker_config* config) ONECAD_WORKER_NOEXCEPT;

#ifdef __cplusplus
}
#endif

#endif /* ONECAD_WORKER_H */
