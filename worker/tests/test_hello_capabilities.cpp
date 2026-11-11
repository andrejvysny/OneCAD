// test_hello_capabilities.cpp — kernel audit 2026-11-11 finding 25 (+ review).
// Rust gates optional operations on the hello `capabilities` (SCHEMA §6), but
// nothing verified the list: Sweep, Gear and FeaturePattern dispatched while
// `op.sweep`, `op.gear` and `op.featurePattern` were never announced.
//
// Pinned here, against the real sidecar: every modeling opType the executor
// DISPATCHES (proved per op by an ExecutePlan that does not answer
// "unsupported opType") is announced as `op.<opType in camelCase>`, and the
// leading `importSourceIdentityV1` entry the hello fixture pins stays first.
// Usage: <worker-path>. No framework.
#include <sys/wait.h>
#include <unistd.h>

#include <algorithm>
#include <cctype>
#include <csignal>
#include <cstdio>
#include <string>
#include <vector>

#include "nlohmann/json.hpp"
#include "protocol/Envelope.h"
#include "protocol/Frame.h"

using nlohmann::json;
using onecad::protocol::Envelope;
using onecad::protocol::Frame;
using onecad::protocol::ReadStatus;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}

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

bool recv(const Worker& w, json& out) {
    auto rr = onecad::protocol::read_frame(w.from);
    if (rr.status != ReadStatus::Ok) return false;
    out = json::parse(rr.frame.json);
    return true;
}

// The terminal response to request `id` (planStep events before it are skipped).
json request(const Worker& w, std::uint64_t id, const std::string& verb, const json& args) {
    Frame f;
    f.json = onecad::protocol::serialize(Envelope::request(id, verb, args));
    onecad::protocol::write_frame(w.to, f);
    json frame;
    while (recv(w, frame)) {
        if (frame.value("t", "") == "resp" && frame.value("id", 0ULL) == id) return frame;
    }
    return json::object();
}

bool dispatches(const Worker& w, std::uint64_t id, const std::string& op_type) {
    const json plan = {{"jobId", id}, {"documentRevision", id}, {"workerEpoch", 3},
                       {"expectedBaseHash",
                        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"},
                       {"prefixHashes", json::array({"a"})}, {"targetStep", 0},
                       {"ops", json::array({json{{"opType", op_type}, {"opId", "op0"},
                                                 {"stepIndex", 0}, {"params", json::object()}}})}};
    const json resp = request(w, id, "ExecutePlan", plan);
    request(w, id + 1000, "DiscardPrepared",
            json{{"jobId", id}, {"documentRevision", id}, {"workerEpoch", 3}});
    return resp.dump().find("unsupported opType") == std::string::npos && !resp.empty();
}
}  // namespace

int main(int argc, char** argv) {
    if (argc < 2) {
        std::fprintf(stderr, "usage: %s <worker-path>\n", argv[0]);
        return 2;
    }
    signal(SIGPIPE, SIG_IGN);
    Worker w;
    if (!spawn(argv[1], w)) { std::fprintf(stderr, "spawn failed\n"); return 2; }
    json hello;
    check(recv(w, hello) && hello.value("t", "") == "hello", "hello received");
    const json caps = hello["result"].value("capabilities", json::array());
    check(!caps.empty() && caps[0] == "importSourceIdentityV1",
          "importSourceIdentityV1 stays the first capability (hello fixture pin)");
    auto announced = [&](const std::string& c) {
        return std::find(caps.begin(), caps.end(), json(c)) != caps.end();
    };

    check(request(w, 1, "OpenSession", json{{"documentId", "doc_1"}, {"documentRevision", 0},
                                            {"workerEpoch", 3}})
              .value("ok", false),
          "session opened");
    const std::vector<std::string> op_types = {
        "Extrude",       "Boolean",         "Revolve",    "Loft",          "Sweep",
        "Fillet",        "Chamfer",         "Shell",      "OffsetFace",    "Hole",
        "Gear",          "LinearPattern",   "CircularPattern", "MirrorBody", "ImportStep",
        "TransformBody", "PlaceComponent",  "DetachComponent", "FeaturePattern"};
    std::uint64_t id = 10;
    for (const std::string& op : op_types) {
        std::string cap = "op." + op;
        cap[3] = static_cast<char>(std::tolower(static_cast<unsigned char>(cap[3])));
        check(dispatches(w, ++id, op), op + " dispatches (control)");
        check(announced(cap), op + " dispatches, so hello announces " + cap);
    }
    check(!dispatches(w, ++id, "NoSuchOp"), "an unknown opType is unsupported (control)");

    close(w.to);
    int status = 0;
    waitpid(w.pid, &status, 0);
    close(w.from);
    if (g_failures == 0) std::fprintf(stderr, "test_hello_capabilities: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
