// test_dispatcher_fallback.cpp — WP-H H0 probe (d): the Dispatcher's
// serialization fallback (`Dispatcher::stamp_and_write`).
//
// Two properties the fallback MUST hold, driven through the real reader/kernel
// loop over a pipe pair (so `stamp_and_write` is exercised on its production path):
//
//   1. WIRE FIDELITY — a fallback error frame carries NO binary tail. The
//      fallback envelope has an empty `bin` section table, so shipping the
//      original `out_bin` bytes would give a frame whose `binLen != 0` with no
//      table addressing them (SCHEMA §3/§5.1: the tail is addressed by name/off/len
//      ONLY). `Dispatcher.cpp` clears `f.bin` inside the catch and then
//      unconditionally overwrites it with `resp.out_bin` on the next line, so the
//      clear is dead code.
//
//   2. FRAME KIND — an unserialisable NON-TERMINAL `event` must be replaced by an
//      event of the same kind carrying an error diagnostic, never by a terminal
//      `resp`. Today the fallback builds `Envelope::error_response`, so an
//      unserialisable planStep becomes a terminal resp for the request id — and
//      the handler's own terminal resp then follows, breaking the
//      exactly-one-terminal-resp-per-id contract (Dispatcher.h).
//
// No test framework: exit code == failure count.
#include <unistd.h>

#include <cmath>
#include <cstdio>
#include <limits>
#include <string>
#include <thread>
#include <vector>

#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "protocol/Frame.h"

using nlohmann::json;
using onecad::protocol::BinSection;
using onecad::protocol::Dispatcher;
using onecad::protocol::Envelope;
using onecad::protocol::Frame;
using onecad::protocol::HandlerContext;
using onecad::protocol::MsgType;
using onecad::protocol::ReadStatus;

namespace {
int g_failures = 0;

#define CHECK(cond, ...)                                                         \
    do {                                                                         \
        if (!(cond)) {                                                           \
            std::fprintf(stderr, "FAIL %s:%d: %s | ", __FILE__, __LINE__, #cond); \
            std::fprintf(stderr, __VA_ARGS__);                                   \
            std::fprintf(stderr, "\n");                                          \
            ++g_failures;                                                        \
        }                                                                        \
    } while (0)

// One frame as captured off the dispatcher's output pipe.
struct Captured {
    std::string type;             // envelope `t`
    std::uint64_t id = 0;
    std::size_t bin_len = 0;      // the FRAME's binLen
    std::size_t bin_sections = 0; // entries in the envelope's `bin` table
    bool ok = false;
};

void write_req(int fd, std::uint64_t id, const std::string& verb) {
    Frame f;
    f.json = onecad::protocol::serialize(Envelope::request(id, verb));
    if (!onecad::protocol::write_frame(fd, f)) {
        std::fprintf(stderr, "FAIL: write_frame(req %s) failed\n", verb.c_str());
        ++g_failures;
    }
}

std::vector<Captured> drain(int fd) {
    std::vector<Captured> out;
    for (;;) {
        onecad::protocol::ReadResult rr = onecad::protocol::read_frame(fd);
        if (rr.status != ReadStatus::Ok) break;
        Captured c;
        c.bin_len = rr.frame.bin.size();
        json j = json::parse(rr.frame.json, nullptr, false);
        if (j.is_discarded()) {
            c.type = "<unparseable>";
        } else {
            c.type = j.value("t", std::string{});
            c.id = j.value("id", std::uint64_t{0});
            c.ok = j.value("ok", false);
            if (j.contains("bin") && j["bin"].is_array()) c.bin_sections = j["bin"].size();
        }
        out.push_back(std::move(c));
    }
    return out;
}

}  // namespace

int main() {
    int p2c[2];
    int c2p[2];
    if (pipe(p2c) != 0 || pipe(c2p) != 0) {
        std::fprintf(stderr, "FAIL: pipe() failed\n");
        return 1;
    }

    Dispatcher d;

    // (1) A terminal resp whose result cannot serialize (non-finite double), WITH a
    //     non-empty binary tail + section table — the exact shape a Tessellate-bearing
    //     PlanPrepared would have.
    d.register_verb("ProbeBadResp",
                    [](const Envelope& req, const std::vector<std::uint8_t>&,
                       HandlerContext&) -> Envelope {
                        Envelope r = Envelope::ok_response(
                            req.id,
                            json{{"value", std::numeric_limits<double>::quiet_NaN()}});
                        r.out_bin = {0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04};
                        r.bin.push_back(BinSection{"mesh:body_probe", 0, 8});
                        return r;
                    });

    // (2) A non-terminal `event` that cannot serialize, followed by the handler's own
    //     terminal resp.
    d.register_verb("ProbeBadEvent",
                    [](const Envelope& req, const std::vector<std::uint8_t>&,
                       HandlerContext& ctx) -> Envelope {
                        Envelope ev = Envelope::event(
                            req.id, "planStep", 0,
                            json{{"fraction",
                                  std::numeric_limits<double>::infinity()}});
                        ev.stamp.job_id = 42;
                        if (ctx.emit) ctx.emit(ev);
                        return Envelope::ok_response(req.id, json{{"planPrepared", true}});
                    });

    std::thread runner([&] { d.run(p2c[0], c2p[1], nullptr); });

    write_req(p2c[1], 1, "ProbeBadResp");
    write_req(p2c[1], 2, "ProbeBadEvent");
    close(p2c[1]);  // EOF -> the reader loop drains both lanes and returns

    runner.join();
    close(c2p[1]);  // so the drain below terminates at EOF

    const std::vector<Captured> frames = drain(c2p[0]);
    close(c2p[0]);
    close(p2c[0]);

    std::fprintf(stderr, "captured %zu frames:\n", frames.size());
    for (const Captured& c : frames) {
        std::fprintf(stderr, "  t=%s id=%llu ok=%d binLen=%zu binSections=%zu\n", c.type.c_str(),
                     static_cast<unsigned long long>(c.id), c.ok ? 1 : 0, c.bin_len,
                     c.bin_sections);
    }

    // ── (1) the fallback error frame must ship NO binary tail ──────────────────
    const Captured* bad_resp = nullptr;
    for (const Captured& c : frames) {
        if (c.id == 1) {
            bad_resp = &c;
            break;
        }
    }
    CHECK(bad_resp != nullptr, "no frame captured for the ProbeBadResp request");
    if (bad_resp != nullptr) {
        CHECK(bad_resp->type == "resp" && !bad_resp->ok,
              "expected a fallback error resp, got t=%s ok=%d", bad_resp->type.c_str(),
              bad_resp->ok ? 1 : 0);
        CHECK(bad_resp->bin_sections == 0, "fallback envelope carries %zu bin sections",
              bad_resp->bin_sections);
        // THE PROBE: the tail must be dropped with the section table.
        CHECK(bad_resp->bin_len == 0,
              "fallback error frame shipped binLen=%zu with %zu bin table entries — an "
              "unaddressable tail on the wire",
              bad_resp->bin_len, bad_resp->bin_sections);
    }

    // ── (2) an unserialisable EVENT must stay an event, not become a terminal resp ─
    std::vector<const Captured*> id2;
    for (const Captured& c : frames) {
        if (c.id == 2) id2.push_back(&c);
    }
    CHECK(!id2.empty(), "no frame captured for the ProbeBadEvent request");
    if (!id2.empty()) {
        // THE PROBE: the replacement for the unserialisable planStep keeps its kind.
        CHECK(id2.front()->type == "event",
              "unserialisable event was replaced by a '%s' frame (kind not preserved)",
              id2.front()->type.c_str());
        std::size_t terminals = 0;
        for (const Captured* c : id2) {
            if (c->type == "resp") ++terminals;
        }
        CHECK(terminals == 1,
              "exactly one terminal resp per id, got %zu for id 2 (the fallback turned a "
              "non-terminal event into a second terminal)",
              terminals);
    }

    if (g_failures == 0) {
        std::fprintf(stderr, "dispatcher_fallback PASS\n");
    }
    return g_failures;
}
