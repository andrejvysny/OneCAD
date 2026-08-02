// test_step_read_determinism.cpp — STEP-IMPORT WP-A W0 determinism gate.
//
// The identity spine (CLAUDE.md) rests on a read producing the SAME solids in
// the SAME order every time. Two probes, because they fail for different reasons:
//
//   IN-PROCESS double read. Catches a pipeline that carries state between calls —
//   a leaked Interface_Static knob, a cached OCCT session, an accumulating
//   messenger. Cheap, and the first thing to break when a stage becomes
//   conditional.
//
//   CROSS-PROCESS double read. The real probe. Address-space layout is
//   randomized per process, so any ordering that ultimately derives from a
//   pointer — OCCT map bucket order, allocator addresses, a std::unordered_*
//   iteration — differs between runs. In-process comparison CANNOT see that: both
//   reads share one heap and one ASLR slide. This test re-executes ITSELF twice
//   in `digest` mode and requires byte-identical stdout.
//
// `digest` mode contract: argv[1] == "digest", argv[2] == fixture path. Writes
// EXACTLY one line to stdout (the canonical digest, '\n'-terminated) and exits
// with the failure count. fd 1 is pointed at /dev/null for the duration of the
// read so the line is the only thing on it — a hygiene bug in read_step would
// then show up as a byte count on fd 1 in test_step_read, not as corrupted
// output here (each test probes one thing).
//
// No framework: exit code == failure count.
#include <fcntl.h>
#include <unistd.h>

#include <array>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <string>

#include "io/StepRead.h"
#include "step_fixture_util.h"

using onecad::io::StepReadPolicy;
using onecad::io::StepReadResult;

namespace {

int g_failures = 0;

void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

// Shell-quote a path so a space in the build directory cannot split the command.
std::string shell_quote(const std::string& s) {
    std::string out = "'";
    for (char c : s) {
        if (c == '\'') {
            out += "'\\''";
        } else {
            out += c;
        }
    }
    out += "'";
    return out;
}

// Run `cmd`, returning its stdout verbatim. Empty on spawn failure.
std::string capture_command(const std::string& cmd) {
    std::string out;
    FILE* pipe = ::popen(cmd.c_str(), "r");
    if (pipe == nullptr) return out;
    std::array<char, 4096> buf{};
    std::size_t n = 0;
    while ((n = std::fread(buf.data(), 1, buf.size(), pipe)) > 0) out.append(buf.data(), n);
    ::pclose(pipe);
    return out;
}

// digest mode: read the fixture with fd 1 muted, then emit the one line.
int run_digest_mode(const std::string& fixture) {
    std::fflush(stdout);
    const int devnull = ::open("/dev/null", O_WRONLY);
    const int saved = ::dup(STDOUT_FILENO);
    if (devnull >= 0) {
        ::dup2(devnull, STDOUT_FILENO);
        ::close(devnull);
    }

    const StepReadResult r = onecad::io::read_step(fixture, StepReadPolicy{});
    const std::string line = stepfx::digest_result(r);

    std::fflush(stdout);
    if (saved >= 0) {
        ::dup2(saved, STDOUT_FILENO);
        ::close(saved);
    }

    if (!r.ok()) {
        std::fprintf(stderr, "digest mode: read failed: %s\n", r.error->c_str());
        return 1;
    }
    // The ONLY sanctioned stdout write in this binary. fwrite, not printf — the
    // stdout-hygiene grep is scoped to worker/src, but there is no reason to
    // teach the pattern here either.
    const std::string payload = line + "\n";
    std::fwrite(payload.data(), 1, payload.size(), stdout);
    std::fflush(stdout);
    return 0;
}

}  // namespace

int main(int argc, char** argv) {
    if (argc >= 3 && std::string(argv[1]) == "digest") {
        return run_digest_mode(argv[2]);
    }

    const std::filesystem::path dir = std::filesystem::current_path();
    const std::string fixture = (dir / "w0_step_determinism.step").string();
    check(stepfx::write_step_fixture(stepfx::make_multi_solid(), fixture).empty(),
          "fixture: multi-solid written");

    // --- In-process double read --------------------------------------------
    const StepReadResult a = onecad::io::read_step(fixture, StepReadPolicy{});
    const StepReadResult b = onecad::io::read_step(fixture, StepReadPolicy{});
    check(a.ok(), "in-process: read #1 succeeded");
    check(b.ok(), "in-process: read #2 succeeded");
    check(a.solids.size() == 3, "in-process: read #1 recovered 3 solids");

    const std::string digest_a = stepfx::digest_result(a);
    const std::string digest_b = stepfx::digest_result(b);
    check(digest_a == digest_b, "in-process: two reads produce byte-identical digests");
    if (digest_a != digest_b) {
        std::fprintf(stderr, "  #1: %s\n  #2: %s\n", digest_a.c_str(), digest_b.c_str());
    }

    // A THIRD read after an unrelated read of a different file, to catch
    // cross-call state that only shows up once another document has been through
    // the same reader.
    const std::string other = (dir / "w0_step_determinism_other.step").string();
    check(stepfx::write_step_fixture(stepfx::make_single_box(), other).empty(),
          "fixture: interleave box written");
    (void)onecad::io::read_step(other, StepReadPolicy{});
    const StepReadResult c = onecad::io::read_step(fixture, StepReadPolicy{});
    check(stepfx::digest_result(c) == digest_a,
          "in-process: digest unchanged after an interleaved read of a different file");

    // --- Cross-process double read (the ASLR / map-ordering probe) ----------
    // argv[0] is the absolute target path under CTest; fall back to a
    // ./-relative form for a bare shell invocation from the binary's directory.
    std::string self = argv[0] != nullptr ? argv[0] : "";
    if (self.find('/') == std::string::npos) self = "./" + self;
    const std::string cmd = shell_quote(self) + " digest " + shell_quote(fixture) + " 2>/dev/null";

    const std::string run1 = capture_command(cmd);
    const std::string run2 = capture_command(cmd);
    check(!run1.empty(), "cross-process: run #1 produced output");
    check(!run2.empty(), "cross-process: run #2 produced output");
    check(run1 == run2, "cross-process: two independent processes produce byte-identical digests");
    if (run1 != run2) {
        std::fprintf(stderr, "  p1: %s  p2: %s\n", run1.c_str(), run2.c_str());
    }

    // The subprocess digest must also equal the in-process one — otherwise the
    // two lanes could each be self-consistent and still disagree.
    std::string run1_trimmed = run1;
    while (!run1_trimmed.empty() && (run1_trimmed.back() == '\n' || run1_trimmed.back() == '\r')) {
        run1_trimmed.pop_back();
    }
    check(run1_trimmed == digest_a, "cross-process: subprocess digest == in-process digest");
    if (run1_trimmed != digest_a) {
        std::fprintf(stderr, "  sub: %s\n  in : %s\n", run1_trimmed.c_str(), digest_a.c_str());
    }

    std::fprintf(stderr, "step_read_determinism: digest = %s\n", digest_a.c_str());

    std::error_code ec;
    std::filesystem::remove(fixture, ec);
    std::filesystem::remove(other, ec);
    if (g_failures == 0) std::fprintf(stderr, "step_read_determinism: OK\n");
    return g_failures;
}
