// test_status_thread.cpp — WP-H: `GetWorkerHead` is answered from the STATUS
// thread while the kernel lane is busy (SCHEMA §7.1).
//
// What the wire promises, and what each half here proves:
//
//   (A) REAL WORKER, real verb registry: with a 2 s `Debug.Busy` occupying the
//       kernel lane, a concurrent `GetWorkerHead` answers well inside the §7.1
//       100 ms budget and its result carries `inflight` (the busy verb, its id,
//       `sinceProgressMs == ageMs` because that verb emits nothing) plus
//       `hasRestoredBase: false`. Before WP-H this request sat in the kernel
//       queue for the whole 2 s and the §8 ping rule read it as a hung process.
//
//   (B) IN-PROCESS dispatcher, so the test can make a job emit on demand: the
//       same probe against a verb that emits a non-terminal frame 1 s in —
//       afterwards `sinceProgressMs < ageMs`, which is exactly the signal §8's
//       wedged-op rule keys on (a long op that streams progress is never killed).
//
// The measured number is a ROUND TRIP (write req → read resp) over a drained
// pipe, so it is a pessimistic reading of the §7.1 "dequeue → write attempt"
// budget, never an optimistic one.
//
// No test framework: exit code == failure count. Usage: <worker-path>.
#include <sys/wait.h>
#include <unistd.h>

#include <chrono>
#include <cstdio>
#include <string>
#include <thread>

#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "protocol/Frame.h"

using nlohmann::json;
using onecad::protocol::Dispatcher;
using onecad::protocol::Envelope;
using onecad::protocol::Frame;
using onecad::protocol::HandlerContext;
using onecad::protocol::ReadStatus;
using Clock = std::chrono::steady_clock;

namespace {
int g_failures = 0;

#define CHECK(cond, ...)                                                          \
    do {                                                                          \
        if (!(cond)) {                                                            \
            std::fprintf(stderr, "FAIL %s:%d: %s | ", __FILE__, __LINE__, #cond); \
            std::fprintf(stderr, __VA_ARGS__);                                    \
            std::fprintf(stderr, "\n");                                           \
            ++g_failures;                                                         \
        }                                                                         \
    } while (0)

// The §7.1 budget. Generous headroom is deliberate: this asserts the probe does
// not QUEUE behind the kernel job (2 s), not a scheduler-tight latency bound.
constexpr long long kBudgetMs = 100;

struct Worker { pid_t pid = -1; int to = -1, from = -1; };

bool spawn(const std::string& path, Worker& w) {
    int p2c[2], c2p[2];
    if (pipe(p2c) != 0 || pipe(c2p) != 0) return false;
    const pid_t pid = fork();
    if (pid < 0) return false;
    if (pid == 0) {
        dup2(p2c[0], STDIN_FILENO);
        dup2(c2p[1], STDOUT_FILENO);
        close(p2c[0]); close(p2c[1]); close(c2p[0]); close(c2p[1]);
        char* const argv[] = {const_cast<char*>(path.c_str()), nullptr};
        execv(path.c_str(), argv);
        _exit(127);
    }
    close(p2c[0]); close(c2p[1]);
    w.pid = pid; w.to = p2c[1]; w.from = c2p[0];
    return true;
}

void send_req(int fd, std::uint64_t id, const std::string& verb, json args = json::object()) {
    Frame f;
    f.json = onecad::protocol::serialize(Envelope::request(id, verb, std::move(args)));
    if (!onecad::protocol::write_frame(fd, f)) {
        std::fprintf(stderr, "FAIL: write_frame(%s) failed\n", verb.c_str());
        ++g_failures;
    }
}

// Read frames until one with `t == "resp"` and the given id; returns it (or a
// discarded json on EOF). Any frame read on the way is reported for the record.
json read_resp(int fd, std::uint64_t id) {
    for (;;) {
        onecad::protocol::ReadResult rr = onecad::protocol::read_frame(fd);
        if (rr.status != ReadStatus::Ok) return json(nullptr);
        json j = json::parse(rr.frame.json, nullptr, false);
        if (j.is_discarded()) return json(nullptr);
        if (j.value("t", std::string{}) == "resp" && j.value("id", std::uint64_t{0}) == id) {
            return j;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// (A) the real worker: the probe overtakes a 2 s kernel job
// ─────────────────────────────────────────────────────────────────────────────
void probe_overtakes_a_busy_kernel_lane(const std::string& worker_path) {
    Worker w;
    if (!spawn(worker_path, w)) {
        std::fprintf(stderr, "FAIL: could not spawn %s\n", worker_path.c_str());
        ++g_failures;
        return;
    }
    onecad::protocol::read_frame(w.from);  // the unsolicited hello (§6)

    send_req(w.to, 1, "OpenSession",
             json{{"documentId", "doc_status"}, {"documentRevision", 0}, {"workerEpoch", 3},
                  {"mode", "determinism"}});
    CHECK(read_resp(w.from, 1).value("ok", false), "OpenSession must succeed");

    // Occupy the KERNEL lane for 2 s.
    send_req(w.to, 2, "Debug.Busy", json{{"durationMs", 2000}});
    std::this_thread::sleep_for(std::chrono::milliseconds(250));  // let it dequeue

    const auto t0 = Clock::now();
    send_req(w.to, 3, "GetWorkerHead");
    const json head = read_resp(w.from, 3);
    const long long elapsed_ms =
        std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now() - t0).count();

    std::fprintf(stderr,
                 "status-thread(real worker): GetWorkerHead round trip %lld ms while Debug.Busy "
                 "held the kernel lane\n  result = %s\n",
                 elapsed_ms,
                 head.is_null() ? "<none>" : head.value("result", json()).dump().c_str());

    CHECK(!head.is_null() && head.value("ok", false), "GetWorkerHead must answer");
    CHECK(elapsed_ms < kBudgetMs,
          "GetWorkerHead took %lld ms — it queued behind the 2 s kernel job instead of being "
          "answered by the status thread",
          elapsed_ms);
    const json result = head.value("result", json::object());
    CHECK(result.value("hasRestoredBase", true) == false,
          "hasRestoredBase must be false with no pending restore");
    const json inflight = result.value("inflight", json(nullptr));
    CHECK(inflight.is_object(), "inflight must report the running kernel job, got %s",
          inflight.dump().c_str());
    if (inflight.is_object()) {
        CHECK(inflight.value("verb", std::string{}) == "Debug.Busy",
              "inflight.verb = %s", inflight.value("verb", std::string("<none>")).c_str());
        CHECK(inflight.value("id", std::uint64_t{0}) == 2, "inflight.id = %llu",
              static_cast<unsigned long long>(inflight.value("id", std::uint64_t{0})));
        CHECK(!inflight.contains("jobId"),
              "a verb with no jobId must OMIT the key, got %s", inflight.dump().c_str());
        // §7.1: sinceProgressMs == ageMs while the job has emitted nothing.
        CHECK(inflight.value("sinceProgressMs", std::uint64_t{1}) ==
                  inflight.value("ageMs", std::uint64_t{0}),
              "sinceProgressMs must equal ageMs for a silent job, got %s", inflight.dump().c_str());
        CHECK(inflight.value("ageMs", std::uint64_t{0}) >= 200,
              "ageMs must run from the DEQUEUE (~250 ms by now), got %llu",
              static_cast<unsigned long long>(inflight.value("ageMs", std::uint64_t{0})));
    }

    CHECK(read_resp(w.from, 2).value("ok", false), "the busy job still terminates normally");

    // The lane is idle again: `inflight` must go back to null.
    send_req(w.to, 4, "GetWorkerHead");
    const json idle = read_resp(w.from, 4).value("result", json::object());
    CHECK(idle.contains("inflight") && idle["inflight"].is_null(),
          "inflight must be null once the lane is idle, got %s", idle.dump().c_str());

    send_req(w.to, 5, "Shutdown");
    read_resp(w.from, 5);
    close(w.to);
    int status = 0;
    waitpid(w.pid, &status, 0);
    close(w.from);
}

// ─────────────────────────────────────────────────────────────────────────────
// (B) in-process: a job that EMITS moves sinceProgressMs off ageMs
// ─────────────────────────────────────────────────────────────────────────────
void progress_moves_since_progress() {
    int p2c[2], c2p[2];
    if (pipe(p2c) != 0 || pipe(c2p) != 0) {
        std::fprintf(stderr, "FAIL: pipe() failed\n");
        ++g_failures;
        return;
    }

    Dispatcher d;
    // A kernel job that runs 2 s and emits ONE non-terminal frame 1 s in.
    d.register_verb("ProbeEmitting",
                    [](const Envelope& req, const std::vector<std::uint8_t>&,
                       HandlerContext& ctx) -> Envelope {
                        std::this_thread::sleep_for(std::chrono::seconds(1));
                        Envelope ev = Envelope::event(req.id, "planStep", 0, json{{"stepIndex", 0}});
                        if (ctx.emit) ctx.emit(ev);
                        std::this_thread::sleep_for(std::chrono::seconds(1));
                        return Envelope::ok_response(req.id, json{{"done", true}});
                    });
    // The status verb renders the dispatcher's own bookkeeping (main.cpp renders
    // the same record into the §7.1 result shape; half (A) covers that side).
    d.register_status_verb("ProbeStatus",
                           [&d](const Envelope& req, const std::vector<std::uint8_t>&,
                                HandlerContext&) -> Envelope {
                               const auto inf = d.inflight_snapshot();
                               json r = json{{"inflight", json(nullptr)}};
                               if (inf.has_value()) {
                                   r["inflight"] = json{{"verb", inf->verb},
                                                        {"id", inf->id},
                                                        {"ageMs", inf->age_ms},
                                                        {"sinceProgressMs", inf->since_progress_ms},
                                                        {"hasJobId", inf->job_id.has_value()}};
                               }
                               return Envelope::ok_response(req.id, std::move(r));
                           });

    std::thread runner([&] { d.run(p2c[0], c2p[1], nullptr); });

    const auto started = Clock::now();
    send_req(p2c[1], 1, "ProbeEmitting", json{{"jobId", 77}});

    // Probe 1 — 250 ms in, before the job has emitted anything.
    std::this_thread::sleep_for(std::chrono::milliseconds(250));
    const auto t0 = Clock::now();
    send_req(p2c[1], 2, "ProbeStatus");
    const json a = read_resp(c2p[0], 2).value("result", json::object());
    const long long lat_a =
        std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now() - t0).count();

    // Probe 2 — 1.4 s in, i.e. ~400 ms after the job's only emitted frame.
    std::this_thread::sleep_for(std::chrono::milliseconds(1150));
    const auto t1 = Clock::now();
    send_req(p2c[1], 3, "ProbeStatus");
    const json b = read_resp(c2p[0], 3).value("result", json::object());
    const long long lat_b =
        std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now() - t1).count();

    std::fprintf(stderr,
                 "status-thread(in-process): before any emit %s (%lld ms); after the emit %s "
                 "(%lld ms)\n",
                 a.dump().c_str(), lat_a, b.dump().c_str(), lat_b);

    CHECK(lat_a < kBudgetMs, "probe 1 took %lld ms", lat_a);
    CHECK(lat_b < kBudgetMs, "probe 2 took %lld ms", lat_b);
    const json ia = a.value("inflight", json(nullptr));
    const json ib = b.value("inflight", json(nullptr));
    CHECK(ia.is_object() && ib.is_object(), "both probes must see the job in flight");
    if (ia.is_object() && ib.is_object()) {
        CHECK(ia.value("verb", std::string{}) == "ProbeEmitting", "inflight.verb = %s",
              ia.value("verb", std::string("<none>")).c_str());
        CHECK(ia.value("hasJobId", false), "a verb carrying args.jobId must report jobId");
        CHECK(ia.value("sinceProgressMs", std::uint64_t{1}) == ia.value("ageMs", std::uint64_t{0}),
              "before any emit sinceProgressMs must equal ageMs, got %s", ia.dump().c_str());
        // THE PROBE: the emitted frame is what §8's wedge deadline measures from.
        CHECK(ib.value("sinceProgressMs", std::uint64_t{0}) < ib.value("ageMs", std::uint64_t{0}),
              "after an emitted frame sinceProgressMs must be BELOW ageMs, got %s",
              ib.dump().c_str());
        CHECK(ib.value("ageMs", std::uint64_t{0}) > ia.value("ageMs", std::uint64_t{0}),
              "ageMs must keep running from the dequeue");
    }

    // Drain the job's terminal, then close stdin so the loops finish.
    read_resp(c2p[0], 1);
    close(p2c[1]);
    runner.join();
    close(c2p[1]);
    close(c2p[0]);
    close(p2c[0]);
    const long long total =
        std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now() - started).count();
    std::fprintf(stderr, "status-thread(in-process): job ran %lld ms\n", total);
}

}  // namespace

int main(int argc, char** argv) {
    if (argc < 2) {
        std::fprintf(stderr, "usage: %s <worker-path>\n", argv[0]);
        return 1;
    }
    probe_overtakes_a_busy_kernel_lane(argv[1]);
    progress_moves_since_progress();
    if (g_failures == 0) std::fprintf(stderr, "status_thread PASS\n");
    return g_failures;
}
