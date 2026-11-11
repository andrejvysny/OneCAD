// test_frame_nonfinite.cpp — kernel audit 2026-11-11 finding 20.
// A well-formed JSON number literal that overflows a double (`1e400`) makes
// nlohmann raise `out_of_range` (406), not `parse_error`. The envelope reader only
// caught `parse_error`, so the exception escaped the dispatch loop and the worker
// died by std::terminate (SIGABRT) — even when the number sat in an argument the
// verb ignores.
//
// Contract (SCHEMA §4 + §1 no-resync): such a frame is a malformed frame. The
// worker writes no response, logs on stderr and exits CLEANLY with status 2. It is
// never killed by a signal. Usage: <worker-path>. No framework.
#include <sys/wait.h>
#include <unistd.h>

#include <csignal>
#include <cstdio>
#include <string>

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

// `raw` is written byte for byte: `serialize` would (rightly) refuse to produce it.
void run_case(const std::string& worker, const std::string& label, const std::string& raw) {
    Worker w;
    if (!spawn(worker, w)) { check(false, label + ": spawn"); return; }
    json resp;
    check(recv(w, resp) && resp.value("t", std::string{}) == "hello", label + ": hello");
    Frame open;
    open.json = onecad::protocol::serialize(Envelope::request(
        1, "OpenSession", json{{"documentId", "doc_1"}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    onecad::protocol::write_frame(w.to, open);
    check(recv(w, resp) && resp.value("ok", false), label + ": session opened");

    Frame bad;
    bad.json = raw;
    onecad::protocol::write_frame(w.to, bad);
    check(!recv(w, resp), label + ": no response frame for an unparseable request");

    close(w.to);
    int status = 0;
    waitpid(w.pid, &status, 0);
    close(w.from);
    if (WIFSIGNALED(status)) std::fprintf(stderr, "  %s: killed by signal %d\n", label.c_str(), WTERMSIG(status));
    check(!WIFSIGNALED(status), label + ": the worker is not killed by a signal");
    check(WIFEXITED(status) && WEXITSTATUS(status) == 2,
          label + ": clean malformed-frame exit status 2 (got " +
              std::to_string(WIFEXITED(status) ? WEXITSTATUS(status) : -1) + ")");
}
}  // namespace

int main(int argc, char** argv) {
    if (argc < 2) {
        std::fprintf(stderr, "usage: %s <worker-path>\n", argv[0]);
        return 2;
    }
    signal(SIGPIPE, SIG_IGN);
    run_case(argv[1], "ignored argument",
             R"({"v":1,"t":"req","id":2,"verb":"GetWorkerHead","args":{"junk":1e400}})");
    run_case(argv[1], "op parameter",
             R"({"v":1,"t":"req","id":2,"verb":"ExecutePlan","args":{"jobId":9,"documentRevision":1,)"
             R"("workerEpoch":3,"expectedBaseHash":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",)"
             R"("prefixHashes":["a"],"targetStep":0,"ops":[{"opType":"Extrude","opId":"op0","stepIndex":0,)"
             R"("params":{"distance":-1e400}}]}})");
    run_case(argv[1], "request id", R"({"v":1,"t":"req","id":1e400,"verb":"GetWorkerHead","args":{}})");
    if (g_failures == 0) std::fprintf(stderr, "test_frame_nonfinite: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
