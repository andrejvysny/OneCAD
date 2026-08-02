// test_import_step.cpp — STEP-IMPORT WP-A W1: the §7.3 `ImportStep` op (both
// codecs) driven through the real ExecutePlan transaction, plus the read-only
// §7.8 `InspectStep` verb and its brep conversion lane.
//
// The two lanes are asserted to be OBSERVATIONALLY IDENTICAL: the same fixture
// imported via `sourceCodec:"step"` and via the brep bytes `InspectStep` produced
// from it must yield the same body ordinals, the same per-ordinal volumes, and the
// same `geometry` signature. That equivalence is the whole point of the brep
// replay policy — if it ever breaks, a document authored against brep bytes would
// silently regenerate different geometry than the STEP it was imported from.
//
// Fixtures are GENERATED at run time (tests/step_fixture_util.h) — nothing binary
// is committed. Volumes are analytic constants, so they are asserted exactly (to
// 1e-6 relative), not "approximately".
//
// stdout hygiene: like test_wp6_exportstep, this binary calls OCCT in-process
// without main.cpp's startup, so it replicates redirect_occt_to_stderr() and
// asserts that ZERO bytes reach the real fd 1 for the whole run — the STEP reader
// is chatty, and unredirected chatter would corrupt the frame stream in the real
// worker. No framework: exit code == failure count.
#include <fcntl.h>
#include <unistd.h>

#include <cstdint>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <string>
#include <utility>
#include <vector>

#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "io/BrepCodec.h"
#include "io/InspectStep.h"
#include "io/StepRead.h"
#include "nlohmann/json.hpp"
#include "ops/ImportOp.h"
#include "ops/OpTypes.h"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "session/BodyStore.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "session/ShapeMetrics.h"
#include "session/Signatures.h"
#include "step_fixture_util.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::BodyStore;
using onecad::session::Session;

namespace {

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
void check_eq(const std::string& got, const std::string& want, const std::string& msg) {
    if (got != want) {
        std::fprintf(stderr, "FAIL: %s\n  got  %s\n  want %s\n", msg.c_str(), got.c_str(),
                     want.c_str());
        ++g_failures;
    }
}
// Volumes are analytic; 1e-6 RELATIVE keeps the cylinder honest without pinning
// an absolute tolerance that only suits the boxes.
void check_volume(double got, double want, const std::string& msg) {
    const double tol = std::abs(want) * 1e-6 + 1e-9;
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.9f want %.9f)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
constexpr const char* kNext =
    "1111111111111111111111111111111111111111111111111111111111111111";

// Mirrors main.cpp's redirect_occt_to_stderr() (src/main.cpp step 2).
void redirect_occt_to_stderr() {
    Handle(Message_Messenger) messenger = Message::DefaultMessenger();
    messenger->RemovePrinters(STANDARD_TYPE(Message_PrinterOStream));
    messenger->AddPrinter(new Message_PrinterOStream("cerr", Standard_False, Message_Info));
}

std::string tmp_path(const char* name) {
    return (std::filesystem::temp_directory_path() / name).string();
}

// --- ExecutePlan driver -----------------------------------------------------

struct PlanRun {
    bool ok = false;                 // terminal resp ok
    std::string error_code;          // §8 code when !ok
    std::string stopped_reason;
    std::string step_message;        // perStepResults[0].message (opFailed channel)
    std::vector<json> step_events;   // streamed planStep payloads
    bool accepted = false;
};

// One-op ImportStep plan → ExecutePlan → (on success) AcceptPrepared. Returns the
// observable outcome; the caller inspects the session for published bodies.
PlanRun run_import_plan(Session& s, std::uint64_t job_id, const std::string& op_id,
                        const json& params, CancelToken& tok) {
    PlanRun run;
    json ops = json::array({json{{"opType", "ImportStep"},
                                 {"opId", op_id},
                                 {"stepIndex", 0},
                                 {"params", params}}});
    HandlerContext ctx{tok, [](int) {}, [&run](Envelope& f) {
                           if (f.event_name.has_value() && *f.event_name == "planStep") {
                               run.step_events.push_back(f.result);
                           }
                       }};
    json args = {{"jobId", job_id},      {"documentRevision", 0},
                 {"workerEpoch", 3},     {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({kNext})}, {"targetStep", 0},
                 {"ops", ops}};
    const Envelope resp = onecad::session::handle_execute_plan(
        s, Envelope::request(job_id, "ExecutePlan", args), ctx);

    run.ok = resp.ok.value_or(false);
    if (!run.ok) {
        if (resp.error.has_value()) run.error_code = resp.error->code;
        return run;
    }
    run.stopped_reason = resp.result.value("stoppedReason", std::string{});
    if (resp.result.contains("perStepResults") && resp.result["perStepResults"].is_array() &&
        !resp.result["perStepResults"].empty()) {
        run.step_message = resp.result["perStepResults"][0].value("message", std::string{});
    }
    if (run.stopped_reason == "completed") {
        const Envelope acc = onecad::session::handle_accept_prepared(
            s, Envelope::request(job_id, "AcceptPrepared",
                                 json{{"jobId", job_id}, {"documentRevision", 0}, {"workerEpoch", 3}}));
        run.accepted = acc.ok.value_or(false);
    } else {
        // A failed/needsRepair plan still leaves a scratch (publish ≤ m−1); Rust
        // discards it. Do the same so the session can take another plan.
        onecad::session::handle_discard_prepared(
            s, Envelope::request(job_id + 1000, "DiscardPrepared", json{{"jobId", job_id}}));
    }
    return run;
}

json step_params(const std::string& path, double unit_scale = 1.0) {
    json p = {{"sourceSha256", std::string(64, 'a')}, {"sourceName", "fixture.step"},
              {"sourceCodec", "step"}, {"healPolicy", "v1"}, {"unitScale", unit_scale},
              {"path", path}};
    return p;
}

json brep_params(const std::string& path, double unit_scale = 1.0,
                 int brep_format = onecad::io::kBrepFormatVersion) {
    json p = {{"sourceSha256", std::string(64, 'b')}, {"sourceName", "fixture.step"},
              {"sourceCodec", "brep"}, {"healPolicy", "v1"}, {"unitScale", unit_scale},
              {"brepFormat", brep_format}, {"path", path}};
    return p;
}

// Published bodies in ORDINAL order. BodyStore iterates ids ascending, and the
// minted ids are `body_<opId>` / `body_<opId>:<k>`, so ascending id order IS
// ordinal order for k < 10 (every fixture here has ≤ 3 solids).
std::vector<TopoDS_Shape> ordinal_shapes(const BodyStore& bodies) {
    std::vector<TopoDS_Shape> out;
    for (const std::string& id : bodies.ids()) out.push_back(bodies.get(id)->geom);
    return out;
}

// `Session` holds a mutex, so it is neither copyable nor movable — open in place.
void open_session(Session& s) { s.open("doc", 0, 3, "determinism"); }

// --- scenarios --------------------------------------------------------------

// Exactly one solid ⇒ the PLAIN `body_<opId>` form (D1), never `:0`.
void test_single_solid_step(const std::string& path) {
    Session s;
    open_session(s);
    CancelToken tok;
    const PlanRun run = run_import_plan(s, 1, "op0", step_params(path), tok);
    check(run.ok && run.stopped_reason == "completed", "single: plan completed");
    check(run.accepted, "single: plan accepted");
    check(run.step_events.size() == 1, "single: one planStep streamed");
    if (run.step_events.size() == 1) {
        const json& ev = run.step_events[0];
        check(ev["bodyEvents"] == json::array({json{{"kind", "created"}, {"bodyId", "body_op0"}}}),
              "single: one created event, no deleted parent (got " + ev["bodyEvents"].dump() + ")");
        check(ev["elementMapDelta"]["added"].empty() && ev["elementMapDelta"]["relabeled"].empty() &&
                  ev["elementMapDelta"]["removed"].empty(),
              "single: empty elementMapDelta (fresh partition)");
        check(ev["signatures"]["geometry"].is_string() &&
                  ev["signatures"]["geometry"].get<std::string>().size() == 16,
              "single: geometry signature emitted");
    }
    const BodyStore bodies = s.bodies_copy();
    check(bodies.size() == 1 && bodies.contains("body_op0"),
          "single: exactly one body, plain id body_op0");
    if (bodies.contains("body_op0")) {
        check_volume(onecad::session::shape_volume(bodies.get("body_op0")->geom),
                     stepfx::kBoxVolume, "single: exact box volume 24000");
    }
}

// N > 1 ⇒ ordered `body_<opId>:<k>` children whose ORDER is W0's read order.
void test_multi_solid_step(const std::string& path) {
    const onecad::io::StepReadResult ref = onecad::io::read_step(path);
    check(ref.ok() && ref.solids.size() == 3, "multi: reference read yields 3 solids");

    Session s;
    open_session(s);
    CancelToken tok;
    const PlanRun run = run_import_plan(s, 2, "op0", step_params(path), tok);
    check(run.ok && run.stopped_reason == "completed", "multi: plan completed");

    const BodyStore bodies = s.bodies_copy();
    check(bodies.size() == 3, "multi: three bodies published");
    for (int k = 0; k < 3; ++k) {
        check(bodies.contains("body_op0:" + std::to_string(k)),
              "multi: minted body_op0:" + std::to_string(k));
    }
    check(!bodies.contains("body_op0"), "multi: no plain body_op0 (N>1 mints children only)");
    if (run.step_events.size() == 1) {
        check(run.step_events[0]["bodyEvents"].size() == 3,
              "multi: three created events, no deleted parent");
        for (const json& e : run.step_events[0]["bodyEvents"]) {
            check(e["kind"] == "created", "multi: every body event is 'created'");
        }
    }

    // The ordinal order IS the W0 digest order — the sharpest available probe
    // (per-solid metrics + face-map permutation + the geometry signature).
    const std::vector<TopoDS_Shape> got = ordinal_shapes(bodies);
    check_eq(stepfx::digest_solids(got), stepfx::digest_solids(ref.solids),
             "multi: ordinal order + geometry match the W0 read digest");
    if (got.size() == 3 && ref.solids.size() == 3) {
        for (std::size_t k = 0; k < 3; ++k) {
            check_volume(onecad::session::shape_volume(got[k]),
                         onecad::session::shape_volume(ref.solids[k]),
                         "multi: ordinal " + std::to_string(k) + " volume matches the read");
        }
    }
}

// unitScale is a UNIFORM scale about the origin ⇒ volumes go as the cube.
void test_unit_scale(const std::string& path, const char* what, const json& params) {
    const onecad::io::StepReadResult ref = onecad::io::read_step(path);
    Session s;
    open_session(s);
    CancelToken tok;
    const PlanRun run = run_import_plan(s, 3, "op0", params, tok);
    check(run.ok && run.stopped_reason == "completed", std::string(what) + ": plan completed");
    const std::vector<TopoDS_Shape> got = ordinal_shapes(s.bodies_copy());
    check(got.size() == ref.solids.size(), std::string(what) + ": solid count preserved");
    for (std::size_t k = 0; k < got.size() && k < ref.solids.size(); ++k) {
        check_volume(onecad::session::shape_volume(got[k]),
                     onecad::session::shape_volume(ref.solids[k]) * 8.0,
                     std::string(what) + ": ordinal " + std::to_string(k) + " volume x8");
    }
}

// A malformed file is a RECOVERABLE step failure: the session survives and takes
// the next plan. This is the invariant that keeps a hostile file from being a DoS.
void test_garbage_file(const std::string& step_path, const std::string& garbage_path,
                       const std::string& good_path) {
    Session s;
    open_session(s);
    CancelToken tok;
    const PlanRun bad = run_import_plan(s, 4, "op0", step_params(garbage_path), tok);
    check(bad.ok, "garbage: terminal resp is ok:true (op failure is per-STEP, not a frame error)");
    check_eq(bad.stopped_reason, "opFailed", "garbage: stoppedReason opFailed");
    check(!bad.step_message.empty(), "garbage: perStepResults carries the reason");
    check(bad.step_message.find(std::string(64, 'a')) != std::string::npos,
          "garbage: the reason names the source sha (got '" + bad.step_message + "')");
    check(s.bodies_copy().size() == 0, "garbage: nothing published");
    check(s.head().snapshot_id == 0, "garbage: head untouched");

    // Session still healthy: a following op succeeds.
    const PlanRun good = run_import_plan(s, 5, "op1", step_params(good_path), tok);
    check(good.ok && good.stopped_reason == "completed", "garbage: session alive, next op succeeds");
    check(s.bodies_copy().contains("body_op1"), "garbage: the following import published");

    // Wrong bytes for the codec: a STEP file fed to the brep lane.
    Session s2;
    open_session(s2);
    const PlanRun wrong = run_import_plan(s2, 6, "op0", brep_params(step_path), tok);
    check(wrong.ok && wrong.stopped_reason == "opFailed",
          "wrong-codec: STEP bytes on the brep lane fail the step, not the session");
    check(s2.bodies_copy().size() == 0, "wrong-codec: nothing published");
}

// Param validation — each is a data problem, so each is OP_FAILED, never a frame
// error and never a silent fallback to a different pipeline.
void test_param_guards(const std::string& path) {
    CancelToken tok;
    struct Case { const char* what; json params; };
    json bad_policy = step_params(path);
    bad_policy["healPolicy"] = "v2";
    json no_path = step_params(path);
    no_path.erase("path");
    json bad_scale = step_params(path);
    bad_scale["unitScale"] = 0.0;
    json bad_codec = step_params(path);
    bad_codec["sourceCodec"] = "iges";
    json no_format = brep_params(path);
    no_format.erase("brepFormat");
    json wrong_format = brep_params(path, 1.0, 99);

    const std::vector<Case> cases = {{"healPolicy v2", bad_policy}, {"missing path", no_path},
                                     {"unitScale 0", bad_scale},    {"unknown codec", bad_codec},
                                     {"brep without brepFormat", no_format},
                                     {"brepFormat mismatch", wrong_format}};
    std::uint64_t job = 10;
    for (const Case& c : cases) {
        Session s;
        open_session(s);
        const PlanRun run = run_import_plan(s, job++, "op0", c.params, tok);
        check(run.ok && run.stopped_reason == "opFailed",
              std::string("guard: ") + c.what + " ⇒ OP_FAILED");
        check(s.bodies_copy().size() == 0, std::string("guard: ") + c.what + " publishes nothing");
    }
    // The missing-path failure must name the content address, not just the path.
    Session s;
    open_session(s);
    const PlanRun run = run_import_plan(s, job, "op0", no_path, tok);
    check(run.step_message.find(std::string(64, 'a')) != std::string::npos,
          "guard: missing path names the sha (got '" + run.step_message + "')");
}

// Cancellation. A mid-transfer interrupt is inherently racy to provoke on a
// fixture that reads in microseconds, so the two halves are asserted separately:
// (a) `read_step` honors an already-fired token — the token is wired into
// `TransferRoots` via ops::CancelProgress, so the same path serves a real
// mid-read cancel; (b) the op MAPS that outcome to CANCELLED (session intact),
// not to OP_FAILED (which would be the wrong §8 class).
void test_cancel(const std::string& path) {
    CancelToken fired;
    fired.cancel();

    const onecad::io::StepReadResult r = onecad::io::read_step(path, onecad::io::StepReadPolicy{}, &fired);
    check(r.cancelled, "cancel: read_step reports cancelled");
    check(!r.ok() && r.solids.empty(), "cancel: cancelled read yields no solids");

    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::ops::OpContext ctx{bodies, &sketches, part, &last_sketch, false, json::object(), &fired};
    const json op = {{"opType", "ImportStep"}, {"opId", "op0"}, {"params", step_params(path)}};
    const onecad::ops::OpOutcome oc = onecad::ops::execute_import_step(ctx, op, "op0");
    check(oc.status == onecad::ops::OpOutcome::Status::Cancelled,
          "cancel: ImportStep maps a fired token to CANCELLED (not OP_FAILED)");
    check(bodies.size() == 0, "cancel: no body created on a cancelled import");
}

// InspectStep: the probe payload + the conversion lane's binary tail, and the
// read-only guarantee (no session argument exists, so this asserts the caller's
// head is untouched across the call).
void test_inspect(const std::string& path, const std::string& brep_out) {
    Session s;
    open_session(s);
    CancelToken tok;
    const PlanRun seed = run_import_plan(s, 20, "seed", step_params(path), tok);
    check(seed.accepted, "inspect: seeded a published body to observe head drift against");
    const onecad::session::WorkerHead before = s.head();
    const std::string sig_before = onecad::session::geometry_signature(s.bodies_copy());

    const Envelope resp = onecad::io::handle_inspect_step(
        Envelope::request(1, "InspectStep", json{{"path", path}, {"includeBrep", true}}), tok);
    check(resp.ok.value_or(false), "inspect: ok");
    if (!resp.ok.value_or(false)) return;

    check(resp.result.value("solidCount", 0) == 3, "inspect: solidCount 3");
    check_eq(resp.result.value("sourceUnit", std::string{}), "MM", "inspect: sourceUnit MM");
    check(resp.result.value("brepFormat", 0) == onecad::io::kBrepFormatVersion,
          "inspect: brepFormat is the pinned BinTools version");
    check(resp.result["productNames"].is_array(), "inspect: productNames array present");
    check(resp.result["diagnostics"].is_array(), "inspect: diagnostics array present");
    const json& bbox = resp.result["bbox"];
    check(bbox["min"].size() == 3 && bbox["max"].size() == 3, "inspect: bbox min/max are 3-vectors");
    check(bbox["max"][1].get<double>() > 200.0,
          "inspect: bbox spans the offset cylinder (y > 200)");

    check(resp.bin.size() == 1 && resp.bin[0].name == "brep", "inspect: one 'brep' bin section");
    check(!resp.out_bin.empty() && resp.bin[0].off == 0 &&
              resp.bin[0].len == resp.out_bin.size(),
          "inspect: bin section addresses the whole tail");

    const onecad::session::WorkerHead after = s.head();
    check(before.document_revision == after.document_revision &&
              before.worker_epoch == after.worker_epoch &&
              before.snapshot_id == after.snapshot_id &&
              before.history_prefix_hash == after.history_prefix_hash &&
              before.has_scratch == after.has_scratch,
          "inspect: read-only — GetWorkerHead identical before/after");
    check_eq(onecad::session::geometry_signature(s.bodies_copy()), sig_before,
             "inspect: read-only — bodies untouched");

    std::ofstream out(brep_out, std::ios::binary | std::ios::trunc);
    out.write(reinterpret_cast<const char*>(resp.out_bin.data()),
              static_cast<std::streamsize>(resp.out_bin.size()));
    out.close();
}

// The equivalence that licenses brep replay: same fixture, same opId, two codecs,
// identical published geometry.
void test_brep_lane(const std::string& step_path, const std::string& brep_path) {
    CancelToken tok;
    Session a;
    open_session(a);
    const PlanRun step_run = run_import_plan(a, 30, "op0", step_params(step_path), tok);
    check(step_run.ok && step_run.stopped_reason == "completed", "brep: step-codec run completed");

    Session b;
    open_session(b);
    const PlanRun brep_run = run_import_plan(b, 31, "op0", brep_params(brep_path), tok);
    check(brep_run.ok && brep_run.stopped_reason == "completed", "brep: brep-codec run completed");
    check(!brep_run.step_events.empty(), "brep: planStep streamed");

    const BodyStore ba = a.bodies_copy();
    const BodyStore bb = b.bodies_copy();
    check(ba.ids() == bb.ids(), "brep: identical BodyIds (same ordinals for the same opId)");
    check_eq(onecad::session::geometry_signature(bb), onecad::session::geometry_signature(ba),
             "brep: identical geometry signature across codecs");
    check_eq(stepfx::digest_solids(ordinal_shapes(bb)), stepfx::digest_solids(ordinal_shapes(ba)),
             "brep: identical ordinal digest across codecs");

    // …and the scale escape hatch works off the CANONICAL (unscaled) brep bytes.
    Session c;
    open_session(c);
    const PlanRun scaled = run_import_plan(c, 32, "op0", brep_params(brep_path, 2.0), tok);
    check(scaled.ok && scaled.stopped_reason == "completed", "brep: unitScale run completed");
    const std::vector<TopoDS_Shape> ref = ordinal_shapes(ba);
    const std::vector<TopoDS_Shape> got = ordinal_shapes(c.bodies_copy());
    check(got.size() == ref.size(), "brep: unitScale preserves the solid count");
    for (std::size_t k = 0; k < got.size() && k < ref.size(); ++k) {
        check_volume(onecad::session::shape_volume(got[k]),
                     onecad::session::shape_volume(ref[k]) * 8.0,
                     "brep: unitScale=2 ordinal " + std::to_string(k) + " volume x8");
    }
}

// Run `fn` with the process's real stdout captured to a temp file; return the
// byte count written to it (must be 0 — SCHEMA §3 stdout hygiene).
template <typename Fn>
std::uintmax_t capture_stdout_bytes(Fn&& fn) {
    std::fflush(stdout);
    const std::string tmp = tmp_path("onecad_import_stdout.tmp");
    const int cap_fd = ::open(tmp.c_str(), O_CREAT | O_WRONLY | O_TRUNC, 0600);
    const int saved_fd = ::dup(STDOUT_FILENO);
    ::dup2(cap_fd, STDOUT_FILENO);
    ::close(cap_fd);
    fn();
    std::fflush(stdout);
    ::dup2(saved_fd, STDOUT_FILENO);
    ::close(saved_fd);
    std::error_code ec;
    const std::uintmax_t bytes = std::filesystem::file_size(tmp, ec);
    if (!ec && bytes > 0) {
        // Print what leaked — in the real worker these bytes would sit in the
        // middle of an OCW1 frame, so "which chatter" is the whole diagnosis.
        std::ifstream leaked(tmp, std::ios::binary);
        std::string head(static_cast<std::size_t>(bytes > 400 ? 400 : bytes), '\0');
        leaked.read(head.data(), static_cast<std::streamsize>(head.size()));
        std::fprintf(stderr, "stdout leak (%llu bytes), first chunk:\n%s\n",
                     static_cast<unsigned long long>(bytes), head.c_str());
    }
    std::filesystem::remove(tmp, ec);
    return ec ? 0 : bytes;
}

}  // namespace

int main() {
    redirect_occt_to_stderr();  // stdout hygiene guard, mirrors main.cpp step 2

    const std::string single = tmp_path("onecad_import_single.step");
    const std::string multi = tmp_path("onecad_import_multi.step");
    const std::string garbage = tmp_path("onecad_import_garbage.step");
    const std::string brep = tmp_path("onecad_import_multi.brep");

    const std::string e1 = stepfx::write_step_fixture(stepfx::make_single_box(), single);
    const std::string e2 = stepfx::write_step_fixture(stepfx::make_multi_solid(), multi);
    check(e1.empty(), "fixture: single-solid STEP written (" + e1 + ")");
    check(e2.empty(), "fixture: multi-solid STEP written (" + e2 + ")");
    if (!e1.empty() || !e2.empty()) return g_failures;

    // Truncated STEP: a real header with the DATA section cut off mid-file.
    {
        std::ifstream in(multi, std::ios::binary);
        std::vector<char> head(400, '\0');
        in.read(head.data(), static_cast<std::streamsize>(head.size()));
        const std::streamsize got = in.gcount();
        std::ofstream out(garbage, std::ios::binary | std::ios::trunc);
        out.write(head.data(), got);
    }

    const std::uintmax_t stdout_bytes = capture_stdout_bytes([&]() {
        test_single_solid_step(single);
        test_multi_solid_step(multi);
        test_unit_scale(multi, "unitScale(step)", step_params(multi, 2.0));
        test_garbage_file(multi, garbage, single);
        test_param_guards(multi);
        test_cancel(multi);
        test_inspect(multi, brep);
        test_brep_lane(multi, brep);
    });
    check(stdout_bytes == 0, "stdout hygiene: zero bytes written to real stdout");

    std::error_code rm;
    for (const std::string& p : {single, multi, garbage, brep}) std::filesystem::remove(p, rm);
    if (g_failures == 0) std::fprintf(stderr, "import_step: OK\n");
    return g_failures;
}
