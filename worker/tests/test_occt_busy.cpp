// test_occt_busy.cpp — every process-wide OCCT lock site refuses RETRIABLE with
// the OCCT_GLOBALS_BUSY prefix FIRST (io/OcctBusy.h; ADR-0016 finding A).
//
// In the in-process library a sibling worker can hold `occt_globals_mutex` /
// `ocaf_mutex` (OcctStaticGuard.h). Before this rule only ExportStep and
// InspectStep reported the wait as retriable, InspectStep put "InspectStep: "
// in front of the prefix, and ImportStep / PlaceComponent turned it into an
// `opFailed` STEP — a model error the document would record — and xbf
// ExportGeometry into a non-retriable failure. Here a helper thread holds the
// lock (as a wedged sibling would) and every site is asserted to:
//   * answer OP_FAILED, retriable:true, message STARTING with the prefix;
//   * for a plan: refuse the WHOLE request (no planStep, no scratch, session
//     head untouched), and succeed once the lock is released.
// No framework: exit code == failure count.
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <future>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>

#include "io/BrepCodec.h"
#include "io/ExportGeometry.h"
#include "io/ExportStep.h"
#include "io/InspectStep.h"
#include "io/OcafApp.h"
#include "io/OcctBusy.h"
#include "io/OcctStaticGuard.h"
#include "io/StepRead.h"
#include "io/XcafCodec.h"
#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "session/PlanExecutor.h"
#include "session/PreviewOp.h"
#include "session/PreviewPlan.h"
#include "session/Session.h"
#include "step_fixture_util.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::Session;
namespace io = onecad::io;

namespace {

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

constexpr const char* kEmpty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
constexpr const char* kNext = "1111111111111111111111111111111111111111111111111111111111111111";

// Holds `m` on its own thread (a timed_mutex must not be re-locked by its
// owner) until destroyed — the sibling worker wedged inside a STEP call.
class LockHolder {
public:
    explicit LockHolder(std::timed_mutex& m) : m_(m) {
        std::promise<void> held;
        std::future<void> ready = held.get_future();
        thread_ = std::thread([this, &held] {
            m_.lock();
            held.set_value();
            std::unique_lock<std::mutex> lk(mu_);
            cv_.wait(lk, [this] { return release_; });
            m_.unlock();
        });
        ready.wait();
    }
    ~LockHolder() {
        {
            std::lock_guard<std::mutex> lk(mu_);
            release_ = true;
        }
        cv_.notify_all();
        thread_.join();
    }
    LockHolder(const LockHolder&) = delete;
    LockHolder& operator=(const LockHolder&) = delete;

private:
    std::timed_mutex& m_;
    std::mutex mu_;
    std::condition_variable cv_;
    bool release_ = false;
    std::thread thread_;
};

std::string describe(const Envelope& r) {
    if (r.ok.value_or(false) || !r.error.has_value()) return "ok";
    return r.error->code + " retriable=" + (r.error->retriable ? "true" : "false") + " '" +
           r.error->message + "'";
}

// The busy shape: OP_FAILED, retriable, prefix first, `context` named.
void check_busy(const Envelope& r, const std::string& what, const std::string& context) {
    const bool shaped = !r.ok.value_or(true) && r.error.has_value() &&
                        r.error->code == "OP_FAILED" && r.error->retriable &&
                        io::is_occt_busy(r.error->message);
    check(shaped, what + ": retriable OP_FAILED starting with OCCT_GLOBALS_BUSY (got " +
                      describe(r) + ")");
    if (shaped && !context.empty()) {
        check(r.error->message.find(context) != std::string::npos,
              what + ": message names '" + context + "' (" + r.error->message + ")");
    }
}

std::string tmp_path(const char* name) {
    return (std::filesystem::temp_directory_path() / name).string();
}

bool write_bytes(const std::string& path, const std::vector<std::uint8_t>& bytes) {
    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    out.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
    return static_cast<bool>(out);
}

struct Fixtures {
    std::string step, brep, xbf;
};

// STEP from the shared generator; brep and xbf from its solids (no lock held).
bool make_fixtures(Fixtures& fx) {
    const int failures_before = g_failures;
    fx.step = tmp_path("onecad_busy.step");
    fx.brep = tmp_path("onecad_busy.brep");
    fx.xbf = tmp_path("onecad_busy.xbf");
    const std::string err = stepfx::write_step_fixture(stepfx::make_single_box(), fx.step);
    check(err.empty(), "fixture: STEP written (" + err + ")");
    const io::StepReadResult read = io::read_step(fx.step);
    check(read.ok() && read.solids.size() == 1, "fixture: STEP reads back one solid");
    if (!err.empty() || !read.ok()) return false;
    std::vector<std::uint8_t> brep, xbf;
    check(io::write_brep_compound(read.solids, brep).empty() && write_bytes(fx.brep, brep),
          "fixture: brep written");
    check(io::write_xcaf_document(read.solids, {io::SolidAttributes{}}, xbf).empty() &&
              write_bytes(fx.xbf, xbf),
          "fixture: xbf written");
    return g_failures == failures_before;
}

json import_op(const std::string& op_id, const std::string& codec, const std::string& path) {
    json params = {{"sourceSha256", std::string(64, 'a')}, {"sourceName", "busy.step"},
                   {"sourceCodec", codec}, {"healPolicy", "v1"}, {"unitScale", 1.0},
                   {"path", path}};
    if (codec == "brep") params["brepFormat"] = io::kBrepFormatVersion;
    if (codec == io::kXcafCodecName) params["brepFormat"] = io::kXcafFormatVersion;
    return json{{"opType", "ImportStep"}, {"opId", op_id}, {"stepIndex", 0}, {"params", params}};
}

json place_op(const std::string& op_id, const std::string& codec, const std::string& path) {
    json source = {{"kind", "embedded"}, {"sha256", std::string(64, 'b')}, {"codec", codec},
                   {"path", path}};
    if (codec == io::kXcafCodecName) source["brepFormat"] = io::kXcafFormatVersion;
    return json{{"opType", "PlaceComponent"},
                {"opId", op_id},
                {"stepIndex", 0},
                {"params",
                 {{"componentId", "onecad.test.busy"},
                  {"componentVersion", "1.0.0"},
                  {"componentRevision", "sha256:" + std::string(64, '0')},
                  {"source", source},
                  {"placement", {{"translate", {0.0, 0.0, 0.0}}}}}}};
}

struct PlanRun {
    Envelope resp;
    std::size_t step_events = 0;
};

PlanRun run_plan(Session& s, std::uint64_t job_id, const json& op) {
    PlanRun run;
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {},
                       [&run](Envelope& f) {
                           if (f.event_name.has_value() && *f.event_name == "planStep") {
                               ++run.step_events;
                           }
                       },
                       {}, {}};
    const json args = {{"jobId", job_id},       {"documentRevision", 0},
                       {"workerEpoch", 3},      {"expectedBaseHash", kEmpty},
                       {"prefixHashes", {kNext}}, {"targetStep", 0},
                       {"ops", json::array({op})}};
    run.resp = onecad::session::handle_execute_plan(
        s, Envelope::request(job_id, "ExecutePlan", args), ctx);
    return run;
}

// A plan refused busy must leave no trace: no step event, no scratch, the head
// exactly as before (so Rust has nothing to discard and may simply retry).
void check_plan_busy(Session& s, std::uint64_t job_id, const json& op, const std::string& what,
                     const std::string& context) {
    const onecad::session::WorkerHead before = s.head();
    const PlanRun run = run_plan(s, job_id, op);
    check_busy(run.resp, what, context);
    check(run.step_events == 0, what + ": no planStep event for a refused plan");
    const onecad::session::WorkerHead after = s.head();
    check(!after.has_scratch, what + ": no scratch stored (session intact)");
    check(before.snapshot_id == after.snapshot_id &&
              before.history_prefix_hash == after.history_prefix_hash,
          what + ": head untouched");
}

void test_plans(const Fixtures& fx) {
    Session s;
    s.open("doc", 0, 3, "determinism");
    {
        const LockHolder globals(io::occt_globals_mutex());
        check_plan_busy(s, 1, import_op("imp_step", "step", fx.step), "ExecutePlan ImportStep(step)",
                        "ImportStep source");
        check_plan_busy(s, 2, place_op("pc_step", "step", fx.step),
                        "ExecutePlan PlaceComponent(step)", "PlaceComponent");
    }
    {
        const LockHolder ocaf(io::ocaf_mutex());
        check_plan_busy(s, 3, import_op("imp_xbf", io::kXcafCodecName, fx.xbf),
                        "ExecutePlan ImportStep(xbf)", "ImportStep source");
        check_plan_busy(s, 4, place_op("pc_xbf", io::kXcafCodecName, fx.xbf),
                        "ExecutePlan PlaceComponent(xbf)", "PlaceComponent");
    }
    // Retriable means retriable: the same plan succeeds once the holder lets go.
    const PlanRun retry = run_plan(s, 5, import_op("imp_step", "step", fx.step));
    check(retry.resp.ok.value_or(false) &&
              retry.resp.result.value("stoppedReason", std::string{}) == "completed",
          "ExecutePlan ImportStep(step): the retry after release completes (" +
              describe(retry.resp) + ")");
}

void test_preview_plan(const Fixtures& fx) {
    Session s;
    s.open("doc", 0, 3, "determinism");
    const json args = {{"jobId", std::uint64_t{7}},
                       {"workerEpoch", std::uint64_t{3}},
                       {"expectedSnapshotId", s.head().snapshot_id},
                       {"ops", json::array({import_op("prev_step", "step", fx.step)})}};
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}, {}, {}};
    const LockHolder globals(io::occt_globals_mutex());
    const Envelope r = onecad::session::handle_preview_plan(
        s, Envelope::request(7, "PreviewPlan", args), ctx);
    check_busy(r, "PreviewPlan ImportStep(step)", "ImportStep source");
}

// PreviewOp runs one candidate op on a head copy: the same retriable refusal.
void test_preview_op(const Fixtures& fx) {
    Session s;
    s.open("doc", 0, 3, "determinism");
    const CancelToken tok;
    const LockHolder globals(io::occt_globals_mutex());
    const Envelope r = onecad::session::handle_preview_op(
        s, Envelope::request(8, "PreviewOp", json{{"op", place_op("pv_step", "step", fx.step)}}),
        tok);
    check_busy(r, "PreviewOp PlaceComponent(step)", "PlaceComponent");
}

// A session with one published body (brep import takes no OCCT lock).
void seed_body(Session& s, const Fixtures& fx) {
    s.open("doc", 0, 3, "determinism");
    const PlanRun run = run_plan(s, 10, import_op("seed", "brep", fx.brep));
    check(run.resp.ok.value_or(false), "seed: brep import prepared (" + describe(run.resp) + ")");
    const Envelope acc = onecad::session::handle_accept_prepared(
        s, Envelope::request(11, "AcceptPrepared",
                             json{{"jobId", 10}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    check(acc.ok.value_or(false), "seed: accepted");
}

void test_verbs(const Fixtures& fx) {
    Session s;
    seed_body(s, fx);
    CancelToken tok;
    const std::string step_out = tmp_path("onecad_busy_export.step");
    const std::string xbf_out = tmp_path("onecad_busy_export.xbf");
    std::error_code ec;
    std::filesystem::remove(step_out, ec);
    std::filesystem::remove(xbf_out, ec);
    {
        const LockHolder globals(io::occt_globals_mutex());
        check_busy(io::handle_inspect_step(
                       Envelope::request(20, "InspectStep", json{{"path", fx.step}}), tok),
                   "InspectStep (StepRead)", "InspectStep");
        check_busy(io::handle_export_step(
                       s, Envelope::request(21, "ExportStep", json{{"path", step_out}})),
                   "ExportStep (occt_globals_mutex)", "");
    }
    {
        const LockHolder ocaf(io::ocaf_mutex());
        check_busy(io::handle_export_step(
                       s, Envelope::request(22, "ExportStep", json{{"path", step_out}})),
                   "ExportStep (ocaf_mutex)", "");
        check_busy(io::handle_export_geometry(
                       s, Envelope::request(23, "ExportGeometry",
                                            json{{"path", xbf_out},
                                                 {"bodyIds", {"body_seed"}},
                                                 {"codec", io::kXcafCodecName}})),
                   "ExportGeometry (xbf)", "ExportGeometry");
        // The STEP read is free here; the XCAF attribute pass needs ocaf_mutex.
        check_busy(io::handle_inspect_step(
                       Envelope::request(24, "InspectStep",
                                         json{{"path", fx.step}, {"includeGeometry", true}}),
                       tok),
                   "InspectStep (XCAF attribute pass)", "InspectStep");
    }
    check(!std::filesystem::exists(step_out, ec), "ExportStep: a refused export writes no file");
    check(!std::filesystem::exists(xbf_out, ec), "ExportGeometry: a refused export writes no file");
}

void test_prefix_rule() {
    check(io::is_occt_busy(io::kOcctGlobalsBusy), "prefix: the bare token is busy");
    check(!io::is_occt_busy(std::string("InspectStep: ") + io::kOcctGlobalsBusy),
          "prefix: a message that merely CONTAINS the token is not the contract");
    check(io::busy_message("OCCT_GLOBALS_BUSY: x", "ctx") == "OCCT_GLOBALS_BUSY: x (ctx)",
          "prefix: context is appended, never prepended");
}

}  // namespace

int main() {
    Handle(Message_Messenger) messenger = Message::DefaultMessenger();
    messenger->RemovePrinters(STANDARD_TYPE(Message_PrinterOStream));
    messenger->AddPrinter(new Message_PrinterOStream("cerr", false, Message_Info));
    // A held lock is refused after this bound instead of the production 20 s.
    io::set_occt_lock_wait_ms(50);

    test_prefix_rule();
    Fixtures fx;
    if (make_fixtures(fx)) {
        test_plans(fx);
        test_preview_plan(fx);
        test_preview_op(fx);
        test_verbs(fx);
    }
    std::error_code ec;
    for (const std::string& p : {fx.step, fx.brep, fx.xbf}) std::filesystem::remove(p, ec);
    if (g_failures == 0) std::fprintf(stderr, "occt_busy: OK\n");
    return g_failures;
}
