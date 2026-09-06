// test_fallback_payload.cpp — WP-H: the CONTENT of the §3.4 serialization
// fallback frames, and the producer-side sanitiser that should keep them rare.
//
// `test_dispatcher_fallback.cpp` (the H0 probe) pins the two structural
// properties: the fallback resp ships no binary tail, and an unserialisable
// event stays an event. This file pins what those substitutes SAY, because Rust
// parses them:
//
//   * the terminal substitute is an `OP_FAILED` error resp, message "response
//     serialization failed", with the ORIGINAL stamp (seq/jobId) preserved;
//   * the event substitute keeps `event`, `id`, `jobId` and `stepIndex`, and its
//     payload is exactly `{stepIndex, status:"Failed", diagnostics:[{severity:
//     "error", code:"EVENT_SERIALIZATION_FAILED", message}]}` — the minimum
//     Rust's `parse_plan_step` accepts, so the step surfaces as FAILED rather
//     than as a PROTOCOL_ERROR;
//   * `protocol::sanitize_non_finite` turns a degenerate producer value into
//     `null` so the frame serializes normally and the rest of the payload
//     survives (the fallback is the backstop for a bug, not the routine path).
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
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::protocol::Dispatcher;
using onecad::protocol::Envelope;
using onecad::protocol::Frame;
using onecad::protocol::HandlerContext;
using onecad::protocol::ReadStatus;

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

void write_req(int fd, std::uint64_t id, const std::string& verb) {
    Frame f;
    f.json = onecad::protocol::serialize(Envelope::request(id, verb));
    onecad::protocol::write_frame(fd, f);
}

std::vector<json> drain(int fd) {
    std::vector<json> out;
    for (;;) {
        onecad::protocol::ReadResult rr = onecad::protocol::read_frame(fd);
        if (rr.status != ReadStatus::Ok) break;
        json j = json::parse(rr.frame.json, nullptr, false);
        if (!j.is_discarded()) out.push_back(std::move(j));
    }
    return out;
}

void fallback_frames_say_what_rust_parses() {
    int p2c[2], c2p[2];
    if (pipe(p2c) != 0 || pipe(c2p) != 0) {
        std::fprintf(stderr, "FAIL: pipe() failed\n");
        ++g_failures;
        return;
    }

    Dispatcher d;
    d.register_verb("BadResp", [](const Envelope& req, const std::vector<std::uint8_t>&,
                                  HandlerContext&) -> Envelope {
        Envelope r = Envelope::ok_response(
            req.id, json{{"value", std::numeric_limits<double>::quiet_NaN()}});
        r.stamp.job_id = 91;
        return r;
    });
    d.register_verb("BadEvent", [](const Envelope& req, const std::vector<std::uint8_t>&,
                                   HandlerContext& ctx) -> Envelope {
        Envelope ev = Envelope::event(
            req.id, "planStep", 4,
            json{{"stepIndex", 4}, {"evidence", -std::numeric_limits<double>::infinity()}});
        ev.stamp.job_id = 91;
        if (ctx.emit) ctx.emit(ev);
        return Envelope::ok_response(req.id, json{{"planPrepared", true}});
    });

    std::thread runner([&] { d.run(p2c[0], c2p[1], nullptr); });
    write_req(p2c[1], 1, "BadResp");
    write_req(p2c[1], 2, "BadEvent");
    close(p2c[1]);
    runner.join();
    close(c2p[1]);
    const std::vector<json> frames = drain(c2p[0]);
    close(c2p[0]);
    close(p2c[0]);

    for (const json& f : frames) std::fprintf(stderr, "  frame: %s\n", f.dump().c_str());

    const json* resp = nullptr;
    const json* event = nullptr;
    for (const json& f : frames) {
        if (f.value("id", std::uint64_t{0}) == 1 && f.value("t", std::string{}) == "resp") resp = &f;
        if (f.value("id", std::uint64_t{0}) == 2 && f.value("t", std::string{}) == "event") {
            event = &f;
        }
    }

    CHECK(resp != nullptr, "no substitute resp for the unserialisable response");
    if (resp != nullptr) {
        const json err = resp->value("error", json::object());
        CHECK(!resp->value("ok", true), "the substitute must be an ERROR resp");
        CHECK(err.value("code", std::string{}) == "OP_FAILED", "error.code = %s",
              err.value("code", std::string("<none>")).c_str());
        CHECK(err.value("message", std::string{}) == "response serialization failed",
              "error.message = %s", err.value("message", std::string("<none>")).c_str());
        CHECK(resp->value("jobId", std::uint64_t{0}) == 91,
              "the ORIGINAL stamp (jobId 91) must survive, got %s", resp->dump().c_str());
        CHECK(resp->contains("seq"), "the assigned seq must survive");
    }

    CHECK(event != nullptr, "no substitute event for the unserialisable planStep");
    if (event != nullptr) {
        CHECK(event->value("event", std::string{}) == "planStep", "event = %s",
              event->value("event", std::string("<none>")).c_str());
        CHECK(event->value("stepIndex", std::uint64_t{99}) == 4, "hoisted stepIndex = %s",
              event->dump().c_str());
        CHECK(event->value("jobId", std::uint64_t{0}) == 91, "jobId must be preserved");
        const json payload = event->value("payload", json::object());
        const json diags = payload.value("diagnostics", json::array());
        const std::string message =
            (diags.is_array() && diags.size() == 1) ? diags[0].value("message", std::string{})
                                                    : std::string{};
        // THE CONTRACT (§3.4): exactly these three keys, in this shape. The
        // message is the encoder's own reason, so it is read back rather than
        // frozen — but it must be non-empty.
        const json expected =
            json{{"stepIndex", 4},
                 {"status", "Failed"},
                 {"diagnostics", json::array({json{{"severity", "error"},
                                                   {"code", "EVENT_SERIALIZATION_FAILED"},
                                                   {"message", message}}})}};
        CHECK(payload == expected, "payload = %s", payload.dump().c_str());
        CHECK(!message.empty(), "the substitute must carry the encoder's reason as `message`");
        CHECK(diags.is_array() && diags.size() == 1 && !diags[0].contains("reasonCode"),
              "EVENT_SERIALIZATION_FAILED is a diagnostic `code`, never a publication reasonCode");
    }

    std::size_t terminals = 0;
    for (const json& f : frames) {
        if (f.value("id", std::uint64_t{0}) == 2 && f.value("t", std::string{}) == "resp") {
            ++terminals;
        }
    }
    CHECK(terminals == 1, "exactly one terminal per id, got %zu", terminals);
}

void producer_sanitiser_keeps_the_frame() {
    json payload = {{"stepIndex", 2},
                    {"evidence", {{"volume", std::numeric_limits<double>::quiet_NaN()},
                                  {"ratios", json::array({1.5, std::numeric_limits<double>::infinity()})}}},
                    {"bodyEvents", json::array()}};
    onecad::protocol::sanitize_non_finite(payload);
    CHECK(payload["evidence"]["volume"].is_null(), "NaN must become null, got %s",
          payload["evidence"]["volume"].dump().c_str());
    CHECK(payload["evidence"]["ratios"][0].get<double>() == 1.5, "a finite sibling must survive");
    CHECK(payload["evidence"]["ratios"][1].is_null(), "Inf must become null");
    CHECK(payload["stepIndex"].get<int>() == 2, "integers are untouched");

    Envelope ev = Envelope::event(7, "planStep", 2, payload);
    std::string wire;
    try {
        wire = onecad::protocol::serialize(ev);
    } catch (const onecad::protocol::EnvelopeError& ex) {
        CHECK(false, "a sanitised payload must serialize, got: %s", ex.what());
        return;
    }
    std::fprintf(stderr, "  sanitised planStep: %s\n", wire.c_str());
}

// ─────────────────────────────────────────────────────────────────────────────
// A SUBSTITUTED planStep must FAIL its step (SCHEMA §3.4 / §7.2, WP-H)
// ─────────────────────────────────────────────────────────────────────────────
//
// The substitute carries none of the step's `bodyEvents` or `elementMapDelta`, so
// a terminal that still called the step Ok would have Rust publish a snapshot
// whose lineage it never received — a body created by a step Rust believes it
// knows nothing about (adversarial review 2026-09-05).
//
// Two halves, because one deterministic trigger cannot cover both:
//   (i)  the WIRING — the Dispatcher tells the producer, through
//        `HandlerContext::last_emit_substituted`, that the frame it just handed
//        to `emit` was replaced, and the flag describes the LAST frame only;
//   (ii) the RULE — given that report, `PlanExecutor` undoes the step, reports it
//        failed with the §3.4 diagnostic and prepares m−1.
//
// (The review's own scenario — a body id that is not valid UTF-8 — never reaches
// (ii) today: `handle_execute_plan` throws `json.exception.type_error.316` while
// building the step's evidence, which `Dispatcher::execute` turns into an
// `OP_FAILED` terminal with the session untouched. Loud already, by a different
// mechanism; the rule below is what covers every OTHER unserialisable payload.)

void the_dispatcher_reports_a_substitution_to_the_producer() {
    int p2c[2], c2p[2];
    if (pipe(p2c) != 0 || pipe(c2p) != 0) {
        std::fprintf(stderr, "FAIL: pipe() failed\n");
        ++g_failures;
        return;
    }

    Dispatcher d;
    d.register_verb("EmitBadThenGood",
                    [](const Envelope& req, const std::vector<std::uint8_t>&,
                       HandlerContext& ctx) -> Envelope {
                        Envelope bad = Envelope::event(
                            req.id, "planStep", 0,
                            json{{"stepIndex", 0},
                                 {"x", std::numeric_limits<double>::quiet_NaN()}});
                        if (ctx.emit) ctx.emit(bad);
                        const bool after_bad =
                            ctx.last_emit_substituted && ctx.last_emit_substituted();
                        Envelope good = Envelope::event(req.id, "planStep", 1,
                                                        json{{"stepIndex", 1}});
                        if (ctx.emit) ctx.emit(good);
                        const bool after_good =
                            ctx.last_emit_substituted && ctx.last_emit_substituted();
                        return Envelope::ok_response(
                            req.id, json{{"afterBad", after_bad}, {"afterGood", after_good}});
                    });

    std::thread runner([&] { d.run(p2c[0], c2p[1], nullptr); });
    write_req(p2c[1], 3, "EmitBadThenGood");
    close(p2c[1]);
    runner.join();
    close(c2p[1]);
    const std::vector<json> frames = drain(c2p[0]);
    close(c2p[0]);
    close(p2c[0]);

    for (const json& f : frames) {
        if (f.value("t", std::string{}) == "resp" && f.value("id", std::uint64_t{0}) == 3) {
            const json r = f.value("result", json::object());
            std::fprintf(stderr, "  substitution report: %s\n", r.dump().c_str());
            CHECK(r.value("afterBad", false),
                  "the producer must be told its event was replaced, got %s", r.dump().c_str());
            CHECK(!r.value("afterGood", true),
                  "the flag describes the LAST frame: a good emit must clear it, got %s",
                  r.dump().c_str());
        }
    }
}

constexpr const char* kEmptyPrefix =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

json plan_args() {
    json entities = json::array({json{{"id", "e1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {10, 0}}},
                                 json{{"id", "e2"}, {"type", "Line"}, {"p0", {10, 0}}, {"p1", {10, 10}}},
                                 json{{"id", "e3"}, {"type", "Line"}, {"p0", {10, 10}}, {"p1", {0, 10}}},
                                 json{{"id", "e4"}, {"type", "Line"}, {"p0", {0, 10}}, {"p1", {0, 0}}}});
    json ops = json::array(
        {json{{"opType", "Sketch"}, {"opId", "op0"}, {"stepIndex", 0},
              {"params", {{"sketchId", "sk"}, {"plane", {{"kind", "XY"}}}, {"entities", entities},
                          {"constraints", json::array()}}}},
         json{{"opType", "Extrude"}, {"opId", "op1"}, {"stepIndex", 1},
              {"params", {{"sketchId", "sk"}, {"distance", 5.0}, {"extrudeMode", "Blind"},
                          {"booleanMode", "NewBody"}}}}});
    return json{{"jobId", 55},
                {"documentRevision", 0},
                {"workerEpoch", 3},
                {"expectedBaseHash", kEmptyPrefix},
                {"prefixHashes", json::array({"h0", "h1"})},
                {"targetStep", 1},
                {"ops", ops}};
}

void a_substituted_plan_step_fails_that_step() {
    onecad::session::Session session;
    session.open("doc", 0, 3, "determinism");

    onecad::CancelToken tok;
    std::size_t emitted = 0;
    // Step 0's event goes out verbatim; step 1's — the one that CREATES the body —
    // is reported as substituted, exactly as the Dispatcher would report it.
    HandlerContext ctx{tok, [](int) {}, [&](Envelope&) { ++emitted; },
                       [&] { return emitted == 2; }};

    const Envelope resp = onecad::session::handle_execute_plan(
        session, Envelope::request(5, "ExecutePlan", plan_args()), ctx);
    const json result = resp.result;
    std::fprintf(stderr, "substituted-step: %zu planStep(s); terminal result = %s\n", emitted,
                 result.dump().c_str());

    CHECK(emitted == 2, "both steps must have emitted (got %zu)", emitted);
    CHECK(resp.ok.value_or(false), "the plan still PREPARES at m-1");
    CHECK(result.value("stoppedReason", std::string{}) == "opFailed", "stoppedReason = %s",
          result.value("stoppedReason", std::string("<none>")).c_str());
    CHECK(result.value("lastValidStep", json(nullptr)) == json(0),
          "lastValidStep must be the step BEFORE the substituted one, got %s",
          result.value("lastValidStep", json(nullptr)).dump().c_str());
    const json per_step = result.value("perStepResults", json::array());
    CHECK(per_step.size() == 2, "both steps must be reported, got %s", per_step.dump().c_str());
    if (per_step.size() == 2) {
        CHECK(per_step[0].value("status", std::string{}) == "ok", "step 0 stays ok");
        CHECK(per_step[1].value("status", std::string{}) == "opFailed",
              "the substituted step must NOT be reported ok, got %s", per_step[1].dump().c_str());
        CHECK(!per_step[1].contains("bodyIds"),
              "the failed step claims no body, got %s", per_step[1].dump().c_str());
        const json diags = per_step[1].value("diagnostics", json::array());
        CHECK(diags.size() == 1 &&
                  diags[0].value("code", std::string{}) == "EVENT_SERIALIZATION_FAILED" &&
                  diags[0].value("severity", std::string{}) == "error" &&
                  diags[0].value("stage", std::string{}) == "wire",
              "the failed step carries the §3.4 diagnostic, got %s", diags.dump().c_str());
    }

    // …and nothing past step 0 was prepared: accepting publishes the sketch-only
    // scratch, so the extrude's body never reaches the head.
    CHECK(session.has_scratch(), "the plan prepared m-1, so a scratch exists");
    const Envelope accepted = onecad::session::handle_accept_prepared(
        session, Envelope::request(6, "AcceptPrepared",
                                   json{{"jobId", 55}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    CHECK(accepted.ok.value_or(false), "AcceptPrepared of the m-1 scratch");
    CHECK(session.bodies_copy().ids().empty(),
          "the substituted step's body must NOT reach the published head (got %zu bodies)",
          session.bodies_copy().ids().size());
}

}  // namespace

int main() {
    fallback_frames_say_what_rust_parses();
    producer_sanitiser_keeps_the_frame();
    the_dispatcher_reports_a_substitution_to_the_producer();
    a_substituted_plan_step_fails_that_step();
    if (g_failures == 0) std::fprintf(stderr, "fallback_payload PASS\n");
    return g_failures;
}
