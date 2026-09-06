// Dispatcher.h — verb registry + reader/kernel/solver threading for the worker.
//
// Threading model (W-WP3b: two worker lanes behind one reader):
//   * The caller's thread runs the stdin reader loop (blocking read_frame).
//   * The KERNEL thread pops OCCT/modeling jobs from its queue (single-writer
//     rule for the OCCT lane).
//   * The STATUS thread (kernel-hardening WP-H) serves the liveness probe
//     `GetWorkerHead` from its OWN queue, so a long kernel job can never delay
//     it. It is an implementation detail with NO wire representation: SCHEMA
//     §5.1 still has two lanes, and the status verbs touch no geometry (they
//     read the session's scalar head plus `inflight_snapshot()` below).
//   * The SOLVER lane thread pops Sketch* jobs from its OWN queue so PlaneGCS
//     drags never queue behind modeling (plan: "solver lane in V1"). Its mailbox
//     is LATEST-WINS per gesture for SolveDrag (only the newest unprocessed
//     target survives; superseded ones get a terminal CANCELLED/superseded resp
//     so the one-resp-per-id contract holds). Non-drag Sketch verbs are FIFO.
//   * All three loops write terminal frames to stdout under a shared write mutex, so
//     frame bytes never interleave; each emitted frame is stamped with the §3
//     stamp — the session head (documentRevision/workerEpoch/snapshotId, via the
//     stamp source) plus a monotonic `seq` — under that same lock (§2).
//   * Cancel frames flip the atomic CancelToken registered under the target id.
//
// Contract:
//   * exactly one terminal resp per req.
//   * unknown verb => terminal resp, error.code = "PROTOCOL_ERROR" (well-framed-
//     illegal sub-case, SCHEMA §8; never a process exit).
//   * bad magic / protocol loss => loop returns exit code 2 (no resync).
//   * a handler may request clean shutdown via HandlerContext::request_shutdown.
#pragma once

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <deque>
#include <functional>
#include <mutex>
#include <optional>
#include <queue>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "protocol/Envelope.h"
#include "util/Cancel.h"

namespace onecad::protocol {

// Handed to each handler. Lets a handler observe cancellation, request a clean
// process shutdown (used by the "Shutdown" verb), and stream non-terminal frames
// (used by ExecutePlan to emit per-step `event` frames before its terminal resp).
struct HandlerContext {
    CancelToken& cancel;
    std::function<void(int exit_code)> request_shutdown;
    // Stamp + write a non-terminal worker frame (event/progress) on this lane's
    // output. Serialized with terminal resps under the single write mutex, so an
    // ExecutePlan planStep never interleaves mid-frame with a solver resp.
    std::function<void(Envelope& frame)> emit;
    // True iff the LAST frame handed to `emit` could not be serialized and the
    // §3.4 substitute went out in its place (kernel lane only; empty elsewhere,
    // which reads as "went out verbatim"). A producer whose event is the ONLY
    // channel for a step's evidence MUST treat a substitution as that step
    // failing — the substitute carries no `bodyEvents` and no `elementMapDelta`,
    // so a step reported Ok behind one would have Rust publish a lineage it never
    // received (SCHEMA §3.4 / §7.2). See PlanExecutor::execute_ops.
    std::function<bool()> last_emit_substituted;
};

// A handler maps a request to its single terminal response. `bin` is the
// request frame's binary tail. Handlers run on the kernel or solver thread.
using Handler =
    std::function<Envelope(const Envelope& req, const std::vector<std::uint8_t>& bin,
                           HandlerContext& ctx)>;

class Dispatcher {
public:
    Dispatcher() = default;

    // Register (or replace) the handler for a verb routed to the KERNEL lane.
    void register_verb(std::string verb, Handler handler);

    // Register (or replace) the handler for a verb routed to the SOLVER lane
    // (Sketch* verbs). SolveDrag on this lane is coalesced latest-wins.
    void register_solver_verb(std::string verb, Handler handler);

    // Register (or replace) the handler for a verb routed to the STATUS thread
    // (`GetWorkerHead`, SCHEMA §7.1). A status handler MUST be O(1)-ish and must
    // never touch a `TopoDS_Shape`: it runs while the kernel lane is mid-op and
    // its whole purpose is to answer inside the §7.1 100 ms promise.
    void register_status_verb(std::string verb, Handler handler);

    // Source of the §3 session-head stamp (documentRevision/workerEpoch/
    // snapshotId) applied to every worker frame. Set by main from WorkerSession;
    // when unset the head is all-zero (pre-session). `seq` is always assigned by
    // the Dispatcher and is NOT taken from the source.
    void set_stamp_source(std::function<Stamp()> source);

    // Run the full reader/kernel/solver loop over the given fds until EOF,
    // shutdown, or protocol error. If `hello` is non-null it is emitted as the
    // unsolicited first frame (SCHEMA §6, seq 0) before the reader loop starts.
    // Returns the process exit code (0/2).
    int run(int in_fd, int out_fd, const Envelope* hello = nullptr);

    // Execute a single request synchronously in-process, no threads/fds.
    // Used by --selftest. Returns the terminal response.
    Envelope dispatch_once(const Envelope& req,
                           const std::vector<std::uint8_t>& bin = {});

    // SCHEMA §7.1 `GetWorkerHead.result.inflight`: the KERNEL-lane job being
    // executed right now, in wire units. Solver-lane and status jobs are never
    // reported. `age_ms` runs from the dequeue; `since_progress_ms` from the last
    // non-terminal frame the job emitted (== `age_ms` while it has emitted none).
    struct Inflight {
        std::string verb;
        std::uint64_t id = 0;
        std::optional<std::uint64_t> job_id;
        std::uint64_t age_ms = 0;
        std::uint64_t since_progress_ms = 0;
    };

    // Snapshot of the in-flight kernel job, or nullopt when the lane is idle.
    // Thread-safe (guarded by `inflight_mu_`); called from the status thread.
    std::optional<Inflight> inflight_snapshot() const;

private:
    struct Job {
        Envelope env;
        std::vector<std::uint8_t> bin;
        CancelTokenPtr cancel;
        // Latest-wins coalescing hints (SolveDrag only).
        bool is_drag = false;
        std::uint64_t drag_gesture = 0;
        std::uint64_t drag_seq = 0;
    };

    // Execute one job's handler, translating unknown verbs and handler
    // exceptions into recoverable error responses. `emit` writes any non-terminal
    // frames the handler streams (wired to `stamp_and_write` on the lane's fd);
    // `substituted` reports whether the last such write was replaced by the §3.4
    // substitute (empty on the lanes that stream nothing).
    Envelope execute(const Job& job, const std::function<void(Envelope&)>& emit,
                     std::function<bool()> substituted = {});

    // Close stdin once, from whichever loop first observes a shutdown, to
    // unblock the blocking reader.
    void close_reader_fd();

    void kernel_loop(int out_fd);
    void solver_loop(int out_fd);
    void status_loop(int out_fd);

    // In-flight bookkeeping for the kernel lane (WP-H). `begin` at dequeue,
    // `note_progress` on every non-terminal frame the job emits (and on any wait
    // that is NOT the worker's own — see the credit note in the .cpp), `end`
    // after its terminal frame is written.
    void inflight_begin(const Job& job);
    void inflight_note_progress();
    void inflight_end();

    // Enqueue onto the solver mailbox with latest-wins coalescing for drags.
    // Superseded drags are terminal-responded CANCELLED/superseded on `out_fd`.
    void enqueue_solver_job(Job job, int out_fd);

    // Serialize + stamp (monotonic seq) + write a terminal resp under the write
    // mutex, copying any handler binary (`out_bin`) into the frame tail. Returns
    // false when the envelope did NOT go out verbatim — the §3.4 fallback wrote a
    // substitute in its place, or dropped an informational frame.
    bool stamp_and_write(int out_fd, Envelope& resp);

    // Registration happens before `run()` starts any thread, so the maps below
    // are read-only for the lifetime of the loops and need no lock.
    std::unordered_map<std::string, Handler> handlers_;
    std::unordered_set<std::string> solver_verbs_;  // routing set (subset of handlers_)
    std::unordered_set<std::string> status_verbs_;  // routing set (subset of handlers_)

    // §3 session-head stamp source (documentRevision/workerEpoch/snapshotId).
    std::function<Stamp()> stamp_source_;

    // Kernel work queue (reader -> kernel).
    std::mutex queue_mu_;
    std::condition_variable queue_cv_;
    std::queue<Job> queue_;
    bool kernel_stop_ = false;

    // Solver mailbox (reader -> solver lane); deque so drags can be coalesced.
    std::mutex solver_mu_;
    std::condition_variable solver_cv_;
    std::deque<Job> solver_queue_;
    bool solver_stop_ = false;

    // Status mailbox (reader -> status thread); never holds anything but a
    // status verb, so it is never behind a kernel job.
    std::mutex status_mu_;
    std::condition_variable status_cv_;
    std::queue<Job> status_queue_;
    bool status_stop_ = false;

    // Single writer discipline across ALL THREE loops + monotonic output seq
    // (§2). The status thread takes this lock too, so its answer is serialized
    // with — never interleaved into — a kernel or solver frame.
    std::mutex write_mu_;
    std::uint64_t out_seq_ = 0;

    // §7.1 in-flight record for the KERNEL lane, guarded by `inflight_mu_`
    // (written by the kernel thread, read by the status thread — never by both
    // under any other lock, so it can never be behind a running op).
    struct InflightRecord {
        std::string verb;
        std::uint64_t id = 0;
        std::optional<std::uint64_t> job_id;
        std::chrono::steady_clock::time_point started;
        std::chrono::steady_clock::time_point last_progress;
    };
    mutable std::mutex inflight_mu_;
    std::optional<InflightRecord> inflight_;

    // Active cancel tokens keyed by request id (reader sets, lane clears).
    std::mutex tokens_mu_;
    std::unordered_map<std::uint64_t, CancelTokenPtr> tokens_;

    // Shutdown coordination.
    std::atomic<bool> shutdown_requested_{false};
    std::atomic<int> exit_code_{0};
    // Closed by whichever loop first observes a shutdown, to unblock the reader.
    // ATOMIC because more than one loop can observe it: `exchange(-1)` makes the
    // close happen exactly once even if the kernel and status threads race.
    std::atomic<int> in_fd_{-1};
};

}  // namespace onecad::protocol
