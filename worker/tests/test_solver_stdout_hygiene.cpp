// PlaneGCS diagnostics must never enter the sidecar's OCW1 frame stream,
// including buffered text flushed only when the process exits.
#include <sys/wait.h>
#include <unistd.h>

#include <csignal>
#include <cstdio>
#include <stdexcept>
#include <string>

#include "protocol/Envelope.h"
#include "protocol/Frame.h"

using nlohmann::json;
using onecad::protocol::Envelope;
using onecad::protocol::Frame;
using onecad::protocol::ReadStatus;

namespace {
void require(bool condition, const std::string& message) {
    if (!condition) throw std::runtime_error(message);
}

struct Worker {
    pid_t pid = -1;
    int to = -1;
    int from = -1;

    ~Worker() {
        if (to >= 0) close(to);
        if (from >= 0) close(from);
        if (pid > 0) {
            kill(pid, SIGKILL);
            waitpid(pid, nullptr, 0);
        }
    }
};

void spawn(const char* path, Worker& worker) {
    int input[2], output[2];
    require(pipe(input) == 0, "stdin pipe failed");
    if (pipe(output) != 0) {
        close(input[0]);
        close(input[1]);
        throw std::runtime_error("stdout pipe failed");
    }
    worker.pid = fork();
    if (worker.pid == 0) {
        if (dup2(input[0], STDIN_FILENO) < 0 || dup2(output[1], STDOUT_FILENO) < 0) _exit(126);
        close(input[0]);
        close(input[1]);
        close(output[0]);
        close(output[1]);
        char* const args[] = {const_cast<char*>(path), nullptr};
        execv(path, args);
        _exit(127);
    }
    close(input[0]);
    close(output[1]);
    worker.to = input[1];
    worker.from = output[0];
    require(worker.pid > 0, "fork failed");
}

json receive(const Worker& worker, int& frames) {
    const auto response = onecad::protocol::read_frame(worker.from);
    require(response.status == ReadStatus::Ok,
            "frame " + std::to_string(frames + 1) + ": " + response.error);
    require(response.frame.bin.empty(), "unexpected binary tail");
    const json value = json::parse(response.frame.json);
    require(value.value("v", 0) == 1, "wrong protocol version");
    require(value.value("seq", -1) == frames, "wrong output sequence");
    ++frames;
    return value;
}

json request(const Worker& worker, int& frames, int id, const std::string& verb,
             const json& args = json::object()) {
    Frame frame;
    frame.json = onecad::protocol::serialize(Envelope::request(id, verb, args));
    require(onecad::protocol::write_frame(worker.to, frame), "request write failed");
    const json response = receive(worker, frames);
    require(response.value("t", "") == "resp", "unexpected frame type");
    require(response.value("id", 0) == id, "wrong response id");
    require(response.value("ok", false), "request failed: " + response.dump());
    return response.at("result");
}

json redundant_sketch() {
    return {{"sketchId", "redundant"}, {"plane", {{"kind", "XY"}}},
            {"entities", {{{"id", "p1"}, {"type", "Point"}, {"at", {0, 0}}},
                          {{"id", "p2"}, {"type", "Point"}, {"at", {10, 0}}},
                          {{"id", "L"}, {"type", "Line"}, {"p0Ref", "p1"}, {"p1Ref", "p2"}}}},
            {"constraints", {{{"id", "h1"}, {"type", "Horizontal"}, {"entities", {"L"}}},
                             {{"id", "h2"}, {"type", "Horizontal"}, {"entities", {"L"}}}}}};
}

void run_case(const char* path, int upserts) {
    Worker worker;
    spawn(path, worker);
    int frames = 0;
    require(receive(worker, frames).value("t", "") == "hello", "missing hello");
    const json args = redundant_sketch();
    for (int id = 1; id <= upserts; ++id) {
        const json result = request(worker, frames, id, "SketchUpsert", args);
        require(result.value("upserted", false), "sketch was not stored");
        require(result.value("sketchRevision", 0) == id, "wrong sketch revision");
        require(result.value("state", "") == "OverConstrained", "redundancy was lost");
        require(result.value("dof", -1) == 3, "redundancy changed degrees of freedom");
        require(result.at("conflicting").empty(), "consistent redundancy became conflicting");
        require(result.value("maxResidual", -1.0) == 0.0, "horizontal line moved off constraint");
    }
    request(worker, frames, upserts + 1, "Shutdown");
    close(worker.to);
    worker.to = -1;
    const auto tail = onecad::protocol::read_frame(worker.from);
    require(tail.status == ReadStatus::Eof, "bytes after Shutdown: " + tail.error);
    int status = 0;
    require(waitpid(worker.pid, &status, 0) == worker.pid, "waitpid failed");
    worker.pid = -1;
    require(WIFEXITED(status) && WEXITSTATUS(status) == 0, "worker exited unsuccessfully");
    require(frames == upserts + 2, "wrong total frame count");
    std::fprintf(stderr, "solver stdout hygiene: upserts=%d frames=%d clean EOF\n", upserts, frames);
}
}  // namespace

int main(int argc, char** argv) {
    if (argc != 2) {
        std::fprintf(stderr, "usage: %s <worker-path>\n", argv[0]);
        return 2;
    }
    std::signal(SIGPIPE, SIG_IGN);
    int failures = 0;
    // One solve catches exit-time flushing; 512 crosses stdio's buffer boundary.
    for (const int upserts : {1, 512}) {
        try {
            run_case(argv[1], upserts);
        } catch (const std::exception& error) {
            std::fprintf(stderr, "FAIL solver stdout hygiene (%d upserts): %s\n", upserts, error.what());
            ++failures;
        }
    }
    return failures;
}
