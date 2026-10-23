#include "protocol/Dispatcher.h"

#include <algorithm>
#include <cerrno>
#include <chrono>
#include <sys/socket.h>
#include <thread>
#include <unistd.h>
#include <utility>

#include <Standard_Failure.hxx>

#include "protocol/Frame.h"
#include "util/LaneThread.h"
#include "util/Log.h"
#include "util/TestHooks.h"

namespace onecad::protocol {

namespace {

std::uint64_t read_u64(const nlohmann::json& p, const char* key) {
    if (p.is_object() && p.contains(key) && p[key].is_number()) {
        return p[key].get<std::uint64_t>();
    }
    return 0;
}

// SCHEMA §3.4 serialization fallback. `env` could not be serialized; build the
// substitute frame OF THE SAME KIND, or nullopt when the kind is informational
// (`progress`) and the frame is simply dropped. The caller drops the binary tail.
std::optional<Envelope> serialization_fallback(const Envelope& env, const EnvelopeError& ex) {
    switch (env.type) {
        case MsgType::Resp: {
            WLOG_ERROR("failed to serialize response for id %llu: %s",
                       static_cast<unsigned long long>(env.id), ex.what());
            Envelope fallback = Envelope::error_response(
                env.id, ErrorInfo{"OP_FAILED", "response serialization failed", false});
            fallback.stamp = env.stamp;  // preserve the head + assigned seq
            return fallback;
        }
        case MsgType::Event: {
            // The request's terminal resp still follows, so this MUST stay an
            // event: turning it into a terminal would give the id two terminals.
            // The payload is the §3.4 minimum a `planStep` reader accepts — Rust's
            // parse_plan_step demands `stepIndex` and cross-checks it — so the step
            // surfaces as FAILED rather than as a protocol violation.
            WLOG_ERROR("failed to serialize event '%s' for id %llu: %s",
                       env.event_name.value_or("<unnamed>").c_str(),
                       static_cast<unsigned long long>(env.id), ex.what());
            Envelope fallback;
            fallback.type = MsgType::Event;
            fallback.id = env.id;
            fallback.event_name = env.event_name;
            fallback.step_index = env.step_index;
            fallback.result = nlohmann::json{
                {"stepIndex", env.step_index.has_value() ? nlohmann::json(*env.step_index)
                                                         : nlohmann::json(nullptr)},
                {"status", "Failed"},
                {"diagnostics", nlohmann::json::array({nlohmann::json{
                                    {"severity", "error"},
                                    {"code", "EVENT_SERIALIZATION_FAILED"},
                                    {"message", ex.what()}}})}};
            fallback.stamp = env.stamp;  // preserve the head, jobId and assigned seq
            return fallback;
        }
        case MsgType::Progress:
            WLOG_ERROR("dropping unserialisable progress frame for id %llu: %s",
                       static_cast<unsigned long long>(env.id), ex.what());
            return std::nullopt;
        default:
            // hello / chunk / an inbound kind echoed back: §3.4 defines no
            // substitute, and inventing one would be a wire shape nothing reads.
            // NOTE for whoever ships bulk streaming: a DROPPED `chunk` would hang
            // the stream on the Rust side (it waits for `bytesTotal`), so a chunk
            // emitter must define its own substitute — or abort the stream — here
            // rather than inherit this drop. No chunk emitter exists today.
            WLOG_ERROR("dropping unserialisable '%s' frame for id %llu: %s",
                       to_string(env.type).c_str(), static_cast<unsigned long long>(env.id),
                       ex.what());
            return std::nullopt;
    }
}

}  // namespace

void Dispatcher::register_verb(std::string verb, Handler handler) {
    handlers_[std::move(verb)] = std::move(handler);
}

void Dispatcher::register_solver_verb(std::string verb, Handler handler) {
    solver_verbs_.insert(verb);
    handlers_[std::move(verb)] = std::move(handler);
}

void Dispatcher::register_status_verb(std::string verb, Handler handler) {
    status_verbs_.insert(verb);
    handlers_[std::move(verb)] = std::move(handler);
}

void Dispatcher::set_stamp_source(std::function<Stamp()> source) {
    stamp_source_ = std::move(source);
}

bool Dispatcher::inflight_begin(const Job& job, bool solver) {
    InflightRecord rec;
    rec.verb = job.env.verb;
    rec.id = job.env.id;
    // `jobId` is present on the plan verbs only (SCHEMA §7.2); absence is
    // reported as an omitted key, never as 0.
    if (job.env.args.is_object() && job.env.args.contains("jobId") &&
        job.env.args["jobId"].is_number()) {
        rec.job_id = job.env.args["jobId"].get<std::uint64_t>();
    }
    rec.started = std::chrono::steady_clock::now();
    rec.last_progress = rec.started;  // §7.1: sinceProgressMs == ageMs until it emits
    std::lock_guard<std::mutex> lk(inflight_mu_);
    if (inflight_.size() >= 2 || inflight_.count(rec.id) != 0) {
        shutdown_requested_.store(true, std::memory_order_relaxed);
        return false;
    }
    rec.solver = solver;
    inflight_.emplace(rec.id, std::move(rec));
    return true;
}

void Dispatcher::inflight_note_progress(std::uint64_t id) {
    // Called for every NON-TERMINAL frame the kernel job emits (progress / event
    // / chunk, §7.1). When bulk streaming lands, the credit wait must call this
    // too: time blocked on §5.3 credit is Rust's wait, not the worker's, so it
    // counts as progress and a credit-starved sender can never read as wedged.
    std::lock_guard<std::mutex> lk(inflight_mu_);
    const auto it = inflight_.find(id);
    if (it != inflight_.end()) it->second.last_progress = std::chrono::steady_clock::now();
}

void Dispatcher::inflight_end(std::uint64_t id) {
    std::lock_guard<std::mutex> lk(inflight_mu_);
    inflight_.erase(id);
}

std::vector<Dispatcher::Inflight> Dispatcher::inflight_jobs_snapshot() const {
    std::lock_guard<std::mutex> lk(inflight_mu_);
    const auto now = std::chrono::steady_clock::now();
    auto ms_since = [now](std::chrono::steady_clock::time_point t) {
        const auto d = std::chrono::duration_cast<std::chrono::milliseconds>(now - t).count();
        return d > 0 ? static_cast<std::uint64_t>(d) : std::uint64_t{0};
    };
    std::vector<Inflight> jobs;
    for (const auto& [id, record] : inflight_) {
        jobs.push_back(Inflight{record.verb, id, record.job_id, ms_since(record.started),
                                ms_since(record.last_progress), record.solver});
    }
    std::sort(jobs.begin(), jobs.end(), [](const Inflight& a, const Inflight& b) {
        return a.id < b.id;
    });
    return jobs;
}

std::optional<Dispatcher::Inflight> Dispatcher::inflight_snapshot() const {
    for (const auto& job : inflight_jobs_snapshot()) {
        if (!job.solver) return job;
    }
    return std::nullopt;
}

namespace {

// The crash-circuit prefix token of ExecutePlan op `position`, by the same rule
// as Rust's plan_op_keys (src-tauri/src/worker/manager.rs): prefixHashes[i],
// else expectedBaseHash. In-process journal only (JournalHooks).
std::string journal_prefix_hash(const nlohmann::json& args, std::size_t position) {
    if (!args.is_object()) return {};
    const auto ph = args.find("prefixHashes");
    if (ph != args.end() && ph->is_array() && position < ph->size() && (*ph)[position].is_string()) {
        return (*ph)[position].get<std::string>();
    }
    const auto base = args.find("expectedBaseHash");
    return (base != args.end() && base->is_string()) ? base->get<std::string>() : std::string{};
}

}  // namespace

Envelope Dispatcher::execute(const Job& job, const std::function<void(Envelope&)>& emit,
                             std::function<bool()> substituted) {
    const Envelope& req = job.env;

    auto it = handlers_.find(req.verb);
    if (it == handlers_.end()) {
        // Unknown verb: well-framed but protocol-illegal (SCHEMA §8) — a terminal
        // PROTOCOL_ERROR resp, NOT a process exit. UNSUPPORTED is reserved for a
        // KNOWN verb with an unsupported op/param.
        WLOG_ERROR("unknown verb '%s' (id %llu)", req.verb.c_str(),
                   static_cast<unsigned long long>(req.id));
        return Envelope::error_response(
            req.id, ErrorInfo{"PROTOCOL_ERROR", "unknown verb: " + req.verb,
                              /*retriable=*/false});
    }

    HandlerContext ctx{
        *job.cancel,
        [this](int code) {
            exit_code_.store(code, std::memory_order_relaxed);
            shutdown_requested_.store(true, std::memory_order_relaxed);
        },
        emit,
        std::move(substituted),
        {},
    };
    if (journal_.plan_op_begin) {
        // `req` outlives the handler call that invokes this synchronously.
        ctx.op_begin = [this, &req](std::uint64_t step_index, std::size_t position,
                                    const std::string& op_id) {
            journal_.plan_op_begin(req.id, step_index, position, op_id,
                                   journal_prefix_hash(req.args, position));
        };
    }

    const auto started = std::chrono::steady_clock::now();
    auto elapsed_ms = [started] {
        return std::chrono::duration_cast<std::chrono::milliseconds>(
                   std::chrono::steady_clock::now() - started)
            .count();
    };

    try {
#if ONECAD_WORKER_HOOKS
        if (req.verb == "SketchUpsert" || req.verb == "SketchRegions") {
            const auto delay = std::min<std::uint64_t>(read_u64(req.args, "__testSolverDelayMs"), 10000);
            const auto until = started + std::chrono::milliseconds(delay);
            while (std::chrono::steady_clock::now() < until) {
                if (!req.args.value("__testSolverIgnoreCancel", false) && job.cancel->cancelled()) {
                    return Envelope::error_response(req.id, ErrorInfo{"CANCELLED", "cancelled", false});
                }
                std::this_thread::sleep_for(std::chrono::milliseconds(1));
            }
        }
#endif
        if (solver_verbs_.count(req.verb) != 0 && job.cancel->cancelled()) {
            return Envelope::error_response(req.id, ErrorInfo{"CANCELLED", "cancelled", false});
        }
        Envelope resp = it->second(req, job.bin, ctx);
        WLOG_DEBUG("verb '%s' id %llu ok in %lld ms", req.verb.c_str(),
                   static_cast<unsigned long long>(req.id),
                   static_cast<long long>(elapsed_ms()));
        return resp;
    } catch (const Standard_Failure& f) {
        // `GetMessageString()` is shared by OCCT 7.9 and 8.0; `what()` and
        // `ExceptionType()` are not available on the comparison kernel.
        const char* msg = f.GetMessageString();
        const std::string message = (msg && *msg) ? msg : "Standard_Failure";
        WLOG_ERROR("handler for verb '%s' id %llu threw Standard_Failure after %lld ms: %s",
                   req.verb.c_str(), static_cast<unsigned long long>(req.id),
                   static_cast<long long>(elapsed_ms()), message.c_str());
        return Envelope::error_response(
            req.id, ErrorInfo{"OP_FAILED", message, /*retriable=*/false});
    } catch (const std::exception& ex) {
        WLOG_ERROR("handler for verb '%s' id %llu threw after %lld ms: %s", req.verb.c_str(),
                   static_cast<unsigned long long>(req.id),
                   static_cast<long long>(elapsed_ms()), ex.what());
        // A handler failure is a recoverable op failure (SCHEMA §8 OP_FAILED):
        // the session is untouched (all work was in scratch).
        return Envelope::error_response(
            req.id, ErrorInfo{"OP_FAILED", ex.what(), /*retriable=*/false});
    }
}

bool Dispatcher::stamp_and_write(int out_fd, Envelope& resp) {
    std::lock_guard<std::mutex> lk(write_mu_);
    if (stamp_source_) {
        const Stamp head = stamp_source_();  // §3 session-head fencing tokens
        resp.stamp.document_revision = head.document_revision;
        resp.stamp.worker_epoch = head.worker_epoch;
        resp.stamp.snapshot_id = head.snapshot_id;
    }
    resp.stamp.seq = out_seq_++;  // §2: monotonic across every emitted frame

    Frame f;
    bool verbatim = true;
    try {
        f.json = serialize(resp);
        f.bin = resp.out_bin;  // the tail belongs to the SERIALIZED envelope only
    } catch (const EnvelopeError& ex) {
        verbatim = false;
        // §3.4 serialization fallback: the frame KIND is preserved and the binary
        // tail is DROPPED. The substitute's `bin` table is empty, so shipping
        // `resp.out_bin` would put unaddressable bytes on the wire (§5.1: the tail
        // is addressed by name/off/len ONLY) — `f.bin` is therefore left empty
        // here, which is why the assignment above lives on the success path.
        f.bin.clear();
        const std::optional<Envelope> substitute = serialization_fallback(resp, ex);
        if (!substitute.has_value()) return false;  // §3.3 progress: informational, dropped
        try {
            f.json = serialize(*substitute);
        } catch (const EnvelopeError& fatal) {
            // The substitute is built from literals + the exception text; a failure
            // here means the stamp itself is unserialisable, which no frame can
            // survive. Drop it rather than emit a malformed frame.
            WLOG_ERROR("serialization fallback for id %llu failed too: %s",
                       static_cast<unsigned long long>(resp.id), fatal.what());
            return false;
        }
    }
    if (!write_frame(out_fd, f)) {
        WLOG_ERROR("write_frame failed (broken stdout); stopping lanes");
        shutdown_requested_.store(true, std::memory_order_relaxed);
    }
    return verbatim;
}

void Dispatcher::kernel_loop(int out_fd) {
    for (;;) {
        Job job;
        {
            std::unique_lock<std::mutex> lk(queue_mu_);
            queue_cv_.wait(lk, [this] { return !queue_.empty() || kernel_stop_; });
            if (queue_.empty()) {
                return;  // stop requested and drained
            }
            job = std::move(queue_.front());
            queue_.pop();
        }

        // §7.1: the in-flight record opens at DEQUEUE and closes after the
        // terminal frame is written, so the status thread reports exactly the job
        // this lane is actually running.
        if (!inflight_begin(job)) {
            refuse_job(out_fd, job, "active job identity or capacity violation");
            close_reader_fd();
            return;
        }
        // Per-job, kernel-thread-local: ASSIGNED (not or-ed) on every emit, so it
        // always describes the LAST frame this job streamed — which is what the
        // producer asks about immediately after streaming it (SCHEMA §3.4).
        bool last_emit_substituted = false;
        Envelope resp = execute(
            job,
            [this, out_fd, &last_emit_substituted, id = job.env.id](Envelope& e) {
                inflight_note_progress(id);
                last_emit_substituted = !stamp_and_write(out_fd, e);
            },
            [&last_emit_substituted] { return last_emit_substituted; });
        finish_job(out_fd, job, resp);

        // If a handler asked to shut down, unblock the reader by closing stdin.
        if (shutdown_requested_.load(std::memory_order_relaxed)) {
            close_reader_fd();
            return;
        }
    }
}

void Dispatcher::solver_loop(int out_fd) {
    for (;;) {
        Job job;
        {
            std::unique_lock<std::mutex> lk(solver_mu_);
            solver_cv_.wait(lk, [this] { return !solver_queue_.empty() || solver_stop_; });
            if (solver_queue_.empty()) {
                return;  // stop requested and drained
            }
            job = std::move(solver_queue_.front());
            solver_queue_.pop_front();
        }

        if (!inflight_begin(job, true)) {
            refuse_job(out_fd, job, "active job identity or capacity violation");
            close_reader_fd();
            return;
        }
        Envelope resp = execute(job, [this, out_fd, id = job.env.id](Envelope& e) {
            inflight_note_progress(id);
            stamp_and_write(out_fd, e);
        });
        finish_job(out_fd, job, resp);
        if (shutdown_requested_.load(std::memory_order_relaxed)) {
            close_reader_fd();
            return;
        }
    }
}

void Dispatcher::finish_job(int out_fd, const Job& job, Envelope& response) {
    {
        std::lock_guard<std::mutex> lk(tokens_mu_);
        tokens_.erase(job.env.id);
    }
    stamp_and_write(out_fd, response);
    inflight_end(job.env.id);
    if (journal_.job_end) journal_.job_end(job.env.id, job.env.verb);
}

void Dispatcher::refuse_job(int out_fd, const Job& job, const char* message) {
    Envelope response = Envelope::error_response(
        job.env.id, ErrorInfo{"PROTOCOL_ERROR", message, false});
    finish_job(out_fd, job, response);
}

void Dispatcher::close_reader_fd() {
#ifdef ONECAD_WORKER_LIBRARY
    // In-process the fd NUMBER must stay reserved while run() is live: the
    // reader holds it by value and may be inside (or about to re-enter) read(),
    // retrying on EINTR. A close() here would let another thread of the HOST
    // reuse the number at once (a second worker's socketpair, a file), and the
    // reader would then read someone else's stream. shutdown(SHUT_RD) wakes the
    // blocked read with EOF instead; close_input() closes after run() returned.
    const int live = in_fd_.load(std::memory_order_acquire);
    if (live < 0) return;
    if (::shutdown(live, SHUT_RD) == 0 || errno != ENOTSOCK) return;
    // A pipe cannot be shut down: only close() can wake its reader, with the
    // hazard above (documented in capi/onecad_worker.h: pass a socket).
#endif
    // Exactly once, whichever loop gets here first (`exchange` on the atomic).
    const int fd = in_fd_.exchange(-1, std::memory_order_acq_rel);
    if (fd >= 0) ::close(fd);
}

void Dispatcher::close_input() {
    const int fd = in_fd_.exchange(-1, std::memory_order_acq_rel);
    if (fd >= 0) ::close(fd);
}

void Dispatcher::status_loop(int out_fd) {
    // The liveness lane (SCHEMA §7.1). Identical in shape to the other two loops
    // and deliberately trivial: it takes `write_mu_` (so `seq` stays monotonic
    // across all three) and whatever lock the handler needs for the scalar head,
    // and NOTHING else — no queue of the other lanes, no geometry.
    for (;;) {
        Job job;
        {
            std::unique_lock<std::mutex> lk(status_mu_);
            status_cv_.wait(lk, [this] { return !status_queue_.empty() || status_stop_; });
            if (status_queue_.empty()) {
                return;  // stop requested and drained
            }
            job = std::move(status_queue_.front());
            status_queue_.pop();
        }

        Envelope resp = execute(job, [this, out_fd](Envelope& e) { stamp_and_write(out_fd, e); });
        {
            std::lock_guard<std::mutex> lk(tokens_mu_);
            tokens_.erase(job.env.id);
        }
        stamp_and_write(out_fd, resp);

        // A broken stdout can be seen FIRST here — this lane answers every 5 s
        // ping, so without this the process would stay alive logging one failed
        // write per ping while the kernel lane sits idle and never notices.
        if (shutdown_requested_.load(std::memory_order_relaxed)) {
            close_reader_fd();
            return;
        }
    }
}

void Dispatcher::enqueue_solver_job(Job job, int out_fd) {
    std::vector<std::uint64_t> to_cancel;  // superseded request ids
    bool enqueue = true;
    {
        std::lock_guard<std::mutex> lk(solver_mu_);
        if (job.is_drag) {
            // Latest-wins: at most one unprocessed drag per gesture survives.
            for (auto it = solver_queue_.begin(); it != solver_queue_.end(); ++it) {
                if (it->is_drag && it->drag_gesture == job.drag_gesture) {
                    if (it->drag_seq < job.drag_seq) {
                        to_cancel.push_back(it->env.id);  // drop older
                        solver_queue_.erase(it);
                    } else {
                        to_cancel.push_back(job.env.id);  // incoming stale
                        enqueue = false;
                    }
                    break;
                }
            }
        }
        if (enqueue) {
            if (journal_.job_begin) journal_.job_begin(job.env.id, job.env.verb);
            solver_queue_.push_back(std::move(job));
        }
    }
    if (enqueue) {
        solver_cv_.notify_one();
    }
    // Terminal-respond superseded drags (CANCELLED/superseded) — never dropped
    // (SCHEMA §3.5/§5.4: the terminal frame is always sent).
    for (std::uint64_t id : to_cancel) {
        {
            std::lock_guard<std::mutex> lk(tokens_mu_);
            tokens_.erase(id);
        }
        Envelope resp = Envelope::error_response(
            id, ErrorInfo{"CANCELLED", "superseded", /*retriable=*/false});
        stamp_and_write(out_fd, resp);
        if (journal_.job_end) journal_.job_end(id, "SolveDrag");
    }
}

int Dispatcher::run(int in_fd, int out_fd, const Envelope* hello) {
    in_fd_.store(in_fd, std::memory_order_relaxed);
    kernel_stop_ = false;
    solver_stop_ = false;
    status_stop_ = false;

    // SCHEMA §6: emit the unsolicited hello (seq 0) before reading any request.
    if (hello != nullptr) {
        Envelope h = *hello;
        stamp_and_write(out_fd, h);
    }

    // Stops and joins the lanes on EVERY exit from run(), an exception included
    // (a failed lane-thread ctor, bad_alloc in the reader): a joinable
    // lane thread destroyed during unwinding is std::terminate, which in the
    // in-process library aborts the HOST app (P0.3 ONECAD-258). The normal path
    // calls stop_and_join() explicitly, exactly where it always joined.
    struct Lanes {
        Dispatcher& d;
        util::LaneThread kernel, solver, status;
        void stop_and_join() {
            {
                std::lock_guard<std::mutex> lk(d.queue_mu_);
                d.kernel_stop_ = true;
            }
            d.queue_cv_.notify_all();
            {
                std::lock_guard<std::mutex> lk(d.solver_mu_);
                d.solver_stop_ = true;
            }
            d.solver_cv_.notify_all();
            {
                std::lock_guard<std::mutex> lk(d.status_mu_);
                d.status_stop_ = true;
            }
            d.status_cv_.notify_all();
            if (kernel.joinable()) kernel.join();
            if (solver.joinable()) solver.join();
            if (status.joinable()) status.join();
        }
        ~Lanes() { stop_and_join(); }
    } lanes{*this, {}, {}, {}};
    lanes.kernel = util::LaneThread([this, out_fd] { kernel_loop(out_fd); }, lane_stack_bytes_);
    lanes.solver = util::LaneThread([this, out_fd] { solver_loop(out_fd); }, lane_stack_bytes_);
    lanes.status = util::LaneThread([this, out_fd] { status_loop(out_fd); }, lane_stack_bytes_);

    int exit_code = 0;
    for (;;) {
        ReadResult rr = read_frame(in_fd);

        if (shutdown_requested_.load(std::memory_order_relaxed)) {
            exit_code = exit_code_.load(std::memory_order_relaxed);
            break;
        }

        if (rr.status == ReadStatus::Eof) {
            exit_code = 0;
            break;
        }
        if (rr.status == ReadStatus::BadMagic || rr.status == ReadStatus::ProtocolError) {
            WLOG_ERROR("protocol: %s", rr.error.c_str());
            exit_code = 2;
            break;
        }

        Envelope env;
        try {
            env = parse(rr.frame.json);
        } catch (const EnvelopeError& ex) {
            WLOG_ERROR("protocol: malformed envelope: %s", ex.what());
            exit_code = 2;
            break;
        }

        if (env.type == MsgType::Cancel) {
            std::lock_guard<std::mutex> lk(tokens_mu_);
            auto it = tokens_.find(env.id);
            if (it != tokens_.end()) {
                it->second->cancel();
            } else {
                WLOG_WARN("cancel for unknown/finished id %llu ignored",
                          static_cast<unsigned long long>(env.id));
            }
            continue;
        }

        if (env.type == MsgType::Credit) {
            // The worker emits no bulk streams yet; credit is a no-op (SCHEMA §5.3).
            continue;
        }

        if (env.type != MsgType::Req) {
            WLOG_WARN("ignoring non-request frame type on stdin (id %llu)",
                      static_cast<unsigned long long>(env.id));
            continue;
        }

        const bool solver_routed = solver_verbs_.count(env.verb) != 0;
        // §7.1: the liveness probe takes its own lane so it is never queued behind
        // a kernel job. Routing is by verb name at ENQUEUE; nothing else moves.
        const bool status_routed = status_verbs_.count(env.verb) != 0;

        Job job;
        job.cancel = std::make_shared<CancelToken>();
        if (solver_routed && env.verb == "SolveDrag") {
            job.is_drag = true;
            job.drag_gesture = read_u64(env.args, "gestureId");
            job.drag_seq = read_u64(env.args, "seq");
        }
        job.env = std::move(env);
        job.bin = std::move(rr.frame.bin);
        {
            std::lock_guard<std::mutex> lk(tokens_mu_);
            tokens_[job.env.id] = job.cancel;
        }

        if (solver_routed) {
            enqueue_solver_job(std::move(job), out_fd);
        } else if (status_routed) {
            {
                std::lock_guard<std::mutex> lk(status_mu_);
                status_queue_.push(std::move(job));
            }
            status_cv_.notify_one();
        } else {
            {
                std::lock_guard<std::mutex> lk(queue_mu_);
                if (journal_.job_begin) journal_.job_begin(job.env.id, job.env.verb);
                queue_.push(std::move(job));
            }
            queue_cv_.notify_one();
        }
    }

    // Drain + join both lanes.
    lanes.stop_and_join();

    return exit_code;
}

Envelope Dispatcher::dispatch_once(const Envelope& req,
                                   const std::vector<std::uint8_t>& bin) {
    Job job;
    job.env = req;
    job.bin = bin;
    job.cancel = std::make_shared<CancelToken>();
    // Synchronous single-shot path (--selftest): no streaming, drop any events.
    return execute(job, [](Envelope&) {});
}

}  // namespace onecad::protocol
