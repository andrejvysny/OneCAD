// test_dispatcher_lanes.cpp — two in-process library guarantees of the
// Dispatcher, driven through its real reader/lane loop over a socketpair:
//
//   1. JOURNAL WINDOW — `JournalHooks::job_end` fires only AFTER the job's
//      terminal frame was written. Before, it fired between the handler's
//      return and the write, so a crash while serializing a large result was
//      attributed to nothing (ADR-0016 finding C). Asserted by reading, from
//      inside job_end, the bytes already waiting on the host end.
//   2. LANE STACK — `set_lane_stack_bytes` reaches all three lane threads (the
//      library asks for 8 MiB; util/LaneThread.h), and 0 keeps a working
//      default lane (the sidecar).
//
// No framework: exit code == failure count.
#include <pthread.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <unistd.h>

#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "protocol/Frame.h"

using nlohmann::json;
using onecad::protocol::Dispatcher;
using onecad::protocol::Envelope;
using onecad::protocol::Frame;
using onecad::protocol::HandlerContext;
using onecad::protocol::JournalHooks;
using onecad::protocol::ReadStatus;

namespace {

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

constexpr std::size_t kLaneStack = std::size_t{8} << 20;

std::size_t current_stack_bytes() {
#if defined(__APPLE__)
    return pthread_get_stacksize_np(pthread_self());
#else
    pthread_attr_t attr;
    std::size_t size = 0;
    if (pthread_getattr_np(pthread_self(), &attr) == 0) {
        pthread_attr_getstacksize(&attr, &size);
        pthread_attr_destroy(&attr);
    }
    return size;
#endif
}

void send(int fd, std::uint64_t id, const std::string& verb) {
    Frame f;
    f.json = onecad::protocol::serialize(Envelope::request(id, verb));
    check(onecad::protocol::write_frame(fd, f), "write_frame(" + verb + ")");
}

// Reports the stack of whichever lane runs it.
Envelope stack_handler(const Envelope& req, const std::vector<std::uint8_t>&, HandlerContext&) {
    return Envelope::ok_response(req.id, json{{"stack", current_stack_bytes()}});
}

struct Outcome {
    std::vector<int> pending_at_job_end;  // bytes readable on the host end at each job_end
    std::vector<json> results;            // terminal results, by arrival
};

// The result frame must fit the socket buffers: the host reads it only AFTER
// job_end fired, so a frame larger than the buffers would block the writer.
constexpr int kSocketBuffer = 256 * 1024;
constexpr std::size_t kBigResult = 64 * 1024;

void widen_buffers(int fd) {
    setsockopt(fd, SOL_SOCKET, SO_SNDBUF, &kSocketBuffer, sizeof(kSocketBuffer));
    setsockopt(fd, SOL_SOCKET, SO_RCVBUF, &kSocketBuffer, sizeof(kSocketBuffer));
}

// Runs one Dispatcher over a socketpair: three probe jobs (one per lane) plus a
// kernel job with a large result, then EOF. Returns what the hooks and the
// host end observed.
Outcome run_dispatcher(std::size_t lane_stack) {
    int sv[2];
    check(socketpair(AF_UNIX, SOCK_STREAM, 0, sv) == 0, "socketpair");
    widen_buffers(sv[0]);
    widen_buffers(sv[1]);
    const int host = sv[0];
    const int worker_in = sv[1];
    const int worker_out = dup(sv[1]);

    Outcome out;
    std::mutex mu;
    std::condition_variable ended;
    Dispatcher d;
    d.register_verb("KernelStack", stack_handler);
    d.register_solver_verb("SolverStack", stack_handler);
    d.register_status_verb("StatusStack", stack_handler);
    d.register_verb("BigResult", [](const Envelope& req, const std::vector<std::uint8_t>&,
                                    HandlerContext&) {
        return Envelope::ok_response(req.id, json{{"blob", std::string(kBigResult, 'x')}});
    });
    JournalHooks hooks;
    hooks.job_end = [&](std::uint64_t, const std::string&) {
        int pending = 0;
        ioctl(host, FIONREAD, &pending);
        {
            std::lock_guard<std::mutex> lk(mu);
            out.pending_at_job_end.push_back(pending);
        }
        ended.notify_all();
    };
    d.set_journal_hooks(std::move(hooks));
    d.set_lane_stack_bytes(lane_stack);

    std::thread runner([&] { d.run(worker_in, worker_out, nullptr); });
    // One job at a time, each terminal read before the next request. For a
    // kernel/solver job the host does not read until job_end fired, so FIONREAD there
    // sees exactly what the lane had written by then (0 with the old order).
    const std::string verbs[] = {"KernelStack", "SolverStack", "StatusStack", "BigResult"};
    std::uint64_t id = 1;
    for (const std::string& verb : verbs) {
        std::size_t ends_before;
        { std::lock_guard<std::mutex> lock(mu); ends_before = out.pending_at_job_end.size(); }
        send(host, id++, verb);
        if (verb != "StatusStack") {
            std::unique_lock<std::mutex> lk(mu);
            check(ended.wait_for(lk, std::chrono::seconds(10),
                                 [&] { return out.pending_at_job_end.size() > ends_before; }),
                  "job_end fired for " + verb);
        }
        onecad::protocol::ReadResult rr = onecad::protocol::read_frame(host);
        check(rr.status == ReadStatus::Ok, "terminal for " + verb);
        if (rr.status == ReadStatus::Ok) {
            out.results.push_back(json::parse(rr.frame.json, nullptr, false));
        }
    }
    // All terminals are consumed: close the peer to deliver EOF without needing
    // shutdown(), which some sandboxed hosts forbid. Lanes then drain + join.
    ::close(host);
    runner.join();
    d.close_input();
    ::close(worker_out);
    return out;
}

void test_journal_window() {
    const Outcome out = run_dispatcher(0);
    // KernelStack, SolverStack and BigResult each have their own lifecycle.
    // The status-only probe remains outside the crash journal.
    check(out.pending_at_job_end.size() == 3, "journal: one job_end per kernel/solver job");
    for (std::size_t i = 0; i < out.pending_at_job_end.size(); ++i) {
        check(out.pending_at_job_end[i] > 0,
              "journal: job_end #" + std::to_string(i) +
                  " fired after its terminal frame was written (pending bytes " +
                  std::to_string(out.pending_at_job_end[i]) + ")");
    }
    if (out.pending_at_job_end.size() == 3) {
        check(out.pending_at_job_end[2] > static_cast<int>(kBigResult),
              "journal: the large result was fully serialized and written before job_end");
    }
}

void test_lane_stacks() {
    const Outcome sized = run_dispatcher(kLaneStack);
    check(sized.results.size() == 4, "stack: every probe answered");
    for (std::size_t i = 0; i + 1 < sized.results.size(); ++i) {
        const std::uint64_t stack = sized.results[i]["result"].value("stack", std::uint64_t{0});
        check(stack >= kLaneStack, "stack: lane " + std::to_string(i) + " runs on >= 8 MiB (got " +
                                       std::to_string(stack) + ")");
    }
    const Outcome dflt = run_dispatcher(0);
    check(dflt.results.size() == 4, "stack: default lanes answer too");
}

}  // namespace

int main() {
    test_journal_window();
    test_lane_stacks();
    if (g_failures == 0) std::fprintf(stderr, "dispatcher_lanes: OK\n");
    return g_failures;
}
