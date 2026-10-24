// WorkerRuntime.cpp — see WorkerRuntime.h. The verb handlers, verb table and
// selftest below moved here from main.cpp unchanged (P0.3, ONECAD-258).
//
// stdout carries protocol frames ONLY. All diagnostics go to stderr via WLOG_*
// (or to the library host's log callback — util/Log.h sink).
#include "capi/WorkerRuntime.h"

#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <optional>
#include <string>
#include <unistd.h>

#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <Standard_Version.hxx>

#include "io/Checkpoint.h"
#include "io/ExportGeometry.h"
#include "io/ExportStep.h"
#include "io/InspectStep.h"
#include "io/MeshExport.h"
#include "io/OcctStaticGuard.h"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "protocol/Limits.h"
#include "protocol/SolverLane.h"
#include "session/ClassifyElement.h"
#include "session/ElementIdentity.h"
#include "session/FaceProjection.h"
#include "session/ProjectToSketchPlane.h"
#include "session/MassProperties.h"
#include "session/BodyTopology.h"
#include "session/ExtractPrismProfile.h"
#include "session/PrepareOffsetFace.h"
#include "session/AnalyzeEdgeOpRange.h"
#include "session/PrepareEdgeOp.h"
#include "session/PlanExecutor.h"
#include "session/PreviewOp.h"
#include "session/PreviewPlan.h"
#include "session/Session.h"
#include "session/Tessellate.h"
#include "tess/MeshHandle.h"
#include "tess/Tessellate.h"
#include "util/Hashing.h"
#include "util/PolicyVersions.h"
#include "util/LittleEndian.h"
#include "util/Log.h"
#include "util/TestHooks.h"

#include "capi/OcctLogPrinter.h"

namespace {

using onecad::protocol::Dispatcher;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::protocol::SolverLane;
using onecad::session::Session;
using onecad::session::WorkerHead;

constexpr int kProtocolVersion = 1;
constexpr const char* kWorkerVersion = "0.1.0";
constexpr int kQuantizationVersion = 1;
constexpr auto kSolverPolicyVersion = onecad::policy::kSolverPolicyVersion;

// occt.fingerprint (SCHEMA §2/§6): 64-bit FNV-1a over reproducibility metadata,
// rendered as 16 lowercase hex chars ($hex64). Operation-local OCCT options stay
// in history hashes; this seed covers only the global kernel compatibility axis.
std::string occt_fingerprint() {
    const std::string seed = ONECAD_OCCT_FINGERPRINT_SEED;
    std::uint64_t h = 14695981039346656037ULL;  // FNV-1a 64-bit offset basis
    for (unsigned char c : seed) {
        h ^= c;
        h *= 1099511628211ULL;  // FNV-1a 64-bit prime
    }
    char buf[17];
    std::snprintf(buf, sizeof(buf), "%016llx", static_cast<unsigned long long>(h));
    return std::string(buf);
}

// SCHEMA §6 hello.result payload (shared by the unsolicited hello frame and the
// --selftest content check).
nlohmann::json make_hello_result_impl() {
    const std::string occt_version = OCC_VERSION_COMPLETE;  // e.g. "7.9.3"
    return {
        {"protocolVersion", kProtocolVersion},
        {"workerVersion", kWorkerVersion},
        {"occt",
         {
             {"version", occt_version},
             {"fingerprint", occt_fingerprint()},
         }},
        {"quantizationVersion", kQuantizationVersion},
        {"solverPolicyVersion", kSolverPolicyVersion},
        // One entry per capability this build actually dispatches. The list had
        // drifted: six ops (hole/shell/both patterns/mirror/transform/offsetFace)
        // shipped without ever being announced, and the three verbs added in the
        // 2026-08-13/14 tranche (ClassifyElement, ExportGeometry, QueryBodyTopology)
        // would have joined them. Nothing VERIFIES this list — the manifest compares
        // version axes only and the hello fixture matches it as `$any` — which is
        // exactly why it rotted, so it is completed here rather than extended by
        // three and left half-true.
        {"capabilities",
         nlohmann::json::array({"op.sketch", "op.extrude", "op.revolve", "op.loft", "op.fillet", "op.chamfer",
                                "op.boolean", "op.boolean.tools", "op.hole", "op.hole.points",
                                "op.shell", "op.linearPattern",
                                "op.circularPattern", "op.mirrorBody", "op.transformBody",
                                "op.offsetFace", "op.importStep", "op.placeComponent",
                                "op.detachComponent", "solver.planegcs", "constraint.tangentAtPoint", "tessellate.mesh1",
                                "io.step", "io.step.import", "io.geometry.export",
                                "query.classifyElement", "query.bodyTopology",
                                "plan.preview", "editOperandContext"})},
        {"limits",
         {{"chunkSize", onecad::protocol::kChunkSize},
          {"initialBulkCredit", onecad::protocol::kInitialBulkCredit}}},
    };
}

// --- lifecycle verbs (SCHEMA §7.1) ------------------------------------------
//
// W-WP4: OpenSession/CloseSession/ResetSession/GetWorkerHead now drive a REAL
// per-document `Session` (head + bodies + sketches + scratch), superseding the
// pre-W-WP4 flag placeholder. OpenSession adopts the request's fencing tokens and
// resets the document to the empty-prefix head; GetWorkerHead reports the real
// head incl. `historyPrefixHash` + `hasScratch`; ResetSession bumps the epoch.

Envelope handle_open_session(Session& session, const Envelope& req) {
    const nlohmann::json& args = req.args;
    const std::string document_id = args.value("documentId", std::string{});
    const std::uint64_t document_revision = args.value("documentRevision", std::uint64_t{0});
    const std::uint64_t worker_epoch = args.value("workerEpoch", std::uint64_t{0});
    const std::string mode = args.value("mode", std::string{"determinism"});
    session.open(document_id, document_revision, worker_epoch, mode);
    nlohmann::json result = {
        {"sessionOpen", true},
        {"workerHead", {{"documentRevision", document_revision}, {"snapshotId", 0}}},
    };
    return Envelope::ok_response(req.id, std::move(result));
}

Envelope handle_close_session(Session& session, const Envelope& req) {
    session.close();
    return Envelope::ok_response(req.id, nlohmann::json{{"sessionClosed", true}});
}

Envelope handle_reset_session(Session& session, const Envelope& req) {
    const std::uint64_t new_epoch = session.reset();
    return Envelope::ok_response(
        req.id, nlohmann::json{{"reset", true}, {"workerEpoch", new_epoch}});
}

// GetWorkerHead (SCHEMA §7.1) — the reconciliation probe AND the §8 liveness
// ping, which is why the Dispatcher routes it to its STATUS thread: it must
// answer inside 100 ms while the kernel lane is mid-op. Everything it reads is a
// scalar — the session's head under `mu_` plus the dispatcher's in-flight record
// — and it touches no `TopoDS_Shape`, which is what makes that promise keepable.
nlohmann::json inflight_json(const Dispatcher::Inflight& job) {
    nlohmann::json result{{"verb", job.verb}, {"id", job.id}, {"ageMs", job.age_ms},
                          {"sinceProgressMs", job.since_progress_ms}};
    if (job.job_id.has_value()) result["jobId"] = *job.job_id;
    return result;
}

Envelope handle_get_worker_head(Session& session, const Envelope& req,
                                const std::vector<Dispatcher::Inflight>& inflight) {
    const WorkerHead head = session.head();
    nlohmann::json kernel = nullptr;
    nlohmann::json jobs = nlohmann::json::array();
    for (const auto& job : inflight) {
        auto record = inflight_json(job);
        if (!job.solver) kernel = record;
        record["kind"] = job.solver ? "solver" : "kernel";
        jobs.push_back(std::move(record));
    }
    nlohmann::json result = {
        {"documentRevision", head.document_revision},
        {"workerEpoch", head.worker_epoch},
        {"snapshotId", head.snapshot_id},
        {"historyPrefixHash", head.history_prefix_hash},
        {"hasScratch", head.has_scratch},
        {"hasRestoredBase", head.has_restored_base},
        {"inflight", std::move(kernel)},
        {"inflightJobs", std::move(jobs)},
    };
    return Envelope::ok_response(req.id, std::move(result));
}

Envelope handle_shutdown(const Envelope& req, const std::vector<std::uint8_t>&,
                         HandlerContext& ctx) {
    ctx.request_shutdown(0);
    return Envelope::ok_response(req.id, nlohmann::json{{"goodbye", true}});
}

#if ONECAD_WORKER_HOOKS
// Debug.Busy — busy-SPINS the kernel thread for `durationMs` (default 200ms).
// A test hook (util/TestHooks.h): always in the sidecar, where a Rust core never
// sends it in production; in the in-process library only with
// ONECAD_WORKER_TEST_HOOKS. Used by the solver benchmark to prove drag latency is
// unaffected by a busy kernel lane, and by the wedge drills.
Envelope handle_debug_busy(const Envelope& req, const std::vector<std::uint8_t>&,
                           HandlerContext&) {
    const int duration_ms =
        (req.args.is_object() && req.args.contains("durationMs") &&
         req.args["durationMs"].is_number())
            ? req.args["durationMs"].get<int>()
            : 200;
    const auto deadline =
        std::chrono::steady_clock::now() + std::chrono::milliseconds(duration_ms);
    volatile std::uint64_t spin = 0;
    while (std::chrono::steady_clock::now() < deadline) {
        spin = spin + 1;  // occupy the core, not just yield
    }
    nlohmann::json result = {{"busySpunMs", duration_ms}};
    return Envelope::ok_response(req.id, std::move(result));
}
#endif

void register_verbs(Dispatcher& dispatcher, SolverLane& solver_lane, Session& session) {
    dispatcher.register_verb(
        "OpenSession",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return handle_open_session(session, r);
        });
    dispatcher.register_verb(
        "CloseSession",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return handle_close_session(session, r);
        });
    dispatcher.register_verb(
        "ResetSession",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return handle_reset_session(session, r);
        });
    // §7.1 (WP-H): the STATUS thread, not the kernel lane. A `GetWorkerHead`
    // queued behind a large STEP import used to read as a hung worker and get the
    // process SIGKILLed by the §8 ping rule (finding session-executor-1).
    dispatcher.register_status_verb(
        "GetWorkerHead",
        [&session, &dispatcher](const Envelope& r, const std::vector<std::uint8_t>&,
                                HandlerContext&) {
            return handle_get_worker_head(session, r, dispatcher.inflight_jobs_snapshot());
        });
    // --- W-WP4: transactional regen (kernel lane, single-writer) ---
    dispatcher.register_verb(
        "ExecutePlan",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext& ctx) {
            return onecad::session::handle_execute_plan(session, r, ctx);
        });
    dispatcher.register_verb(
        "AcceptPrepared",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_accept_prepared(session, r);
        });
    dispatcher.register_verb(
        "DiscardPrepared",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_discard_prepared(session, r);
        });
    // --- W-WP5: geometry + element identity (SCHEMA §7.5/§7.6) ---
    // MODEL-OPS W3: drag-time preview. Kernel lane (OCCT single-writer), no
    // fencing, no scratch — see session/PreviewOp.h.
    dispatcher.register_verb(
        "PreviewOp",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext& ctx) {
            return onecad::session::handle_preview_op(session, r, ctx.cancel);
        });
    // ONECAD-369: full-replay edit preview. Kernel lane; touches no session slot
    // (no scratch, no restored base, no snapshot id) — see session/PreviewPlan.h.
    dispatcher.register_verb(
        "PreviewPlan",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext& ctx) {
            return onecad::session::handle_preview_plan(session, r, ctx);
        });
    dispatcher.register_verb(
        "Tessellate",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_tessellate(session, r);
        });
    dispatcher.register_verb(
        "AcquireElementIds",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_acquire_element_ids(session, r);
        });
    dispatcher.register_verb(
        "BindElementIds",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_bind_element_ids(session, r);
        });
    dispatcher.register_verb(
        "QueryElement",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_query_element(session, r);
        });
    // --- COMPONENT-LIBRARY P0.1: interactive surface classification for the
    //     placement/mate-snap solver (SCHEMA §7.5). Read-only, current head,
    //     no snapshotId — a continuously re-issued LIVE hover query, unlike
    //     QueryElement's pick-time snapshot addressing (Invariant 4). ---
    dispatcher.register_verb(
        "ClassifyElement",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_classify_element(session, r);
        });
    // --- WP-C1: exact mass properties (SCHEMA §7.5). Read-only, addressed by
    //     bodyId against a head copy — no fence, no scratch, no minting. ---
    dispatcher.register_verb(
        "QueryMassProperties",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_query_mass_properties(session, r);
        });
    dispatcher.register_verb(
        "QueryBodyTopology",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_query_body_topology(session, r);
        });
    dispatcher.register_verb(
        "ResolveRefs",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_resolve_refs(session, r);
        });
    // --- SKETCH-ON-FACE W1: face-boundary projection (SCHEMA §7.6). Read-only;
    //     addressed like QueryElement (head copy, `present:false` for stale). ---
    dispatcher.register_verb(
        "ProjectFaceBoundary",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_project_face_boundary(session, r);
        });
    // --- WP-P: project picked body edges (or a picked face's whole boundary)
    //     into the ACTIVE sketch's plane (SCHEMA §7.6). Head COPY like its
    //     neighbour above, but SNAPSHOT-FENCED (stale ⇒ STALE_PREVIEW): the
    //     answer is committed into a document sketch and the `projectedHash` it
    //     mints is the baseline every later staleness check compares against. ---
    dispatcher.register_verb(
        "ProjectToSketchPlane",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_project_to_sketch_plane(session, r);
        });
    // --- OFFSET-FACE W1: the read-only `op.offsetFace` authoring handshake
    //     (SCHEMA §7.6). Head COPY, no minting — but SNAPSHOT-FENCED, because its
    //     answer is frozen into a document record (stale ⇒ STALE_PREVIEW). ---
    dispatcher.register_verb(
        "PrepareOffsetFace",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_prepare_offset_face(session, r);
        });
    dispatcher.register_verb(
        "PrepareEdgeOp",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_prepare_edge_op(session, r);
        });
    // --- WP4: the feasible radius/distance range for a prepared edge closure
    //     (SCHEMA §7.6). Read-only and snapshot-fenced like PrepareEdgeOp, but
    //     it RUNS BUILDS — so unlike its neighbours it forwards `ctx.cancel`,
    //     which is what lets a §3.5 cancel and the wall deadline reach the
    //     search between (and, for fillet, inside) probes. ---
    dispatcher.register_verb(
        "AnalyzeEdgeOpRange",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext& ctx) {
            return onecad::session::handle_analyze_edge_op_range(session, r, ctx.cancel);
        });
    // --- W-WP6: STEP export (SCHEMA §7.8, D2) ---
    dispatcher.register_verb(
        "ExportStep",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::io::handle_export_step(session, r);
        });
    // --- Component Library WP-3.2: geometry export in the §7.3 REPLAY codecs
    //     (SCHEMA §7.8). The inverse of InspectStep's conversion lane — this one
    //     bakes a body already in the session, which is what an `embedded` /
    //     `document` component source is made of. ---
    dispatcher.register_verb(
        "ExportGeometry",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::io::handle_export_geometry(session, r);
        });
    // --- Component Library WP-C: the "vendor a STEP extrusion" ingest probe
    //     (SCHEMA §7.8). Read-only with respect to SESSION state — no mint, no
    //     publish, no scratch — but SNAPSHOT-FENCED like PrepareOffsetFace,
    //     because its answer is frozen into a package. It writes the canonical
    //     end-cap FACE a §7.3 `profile` source reads back. ---
    dispatcher.register_verb(
        "ExtractPrismProfile",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::session::handle_extract_prism_profile(session, r);
        });
    // --- STEP-IMPORT WP-A W1: read-only STEP probe + brep conversion lane
    //     (SCHEMA §7.8). No session argument by design: no head, no scratch, no
    //     fence, no publish — the import itself is the §7.3 `ImportStep` op. ---
    dispatcher.register_verb(
        "InspectStep",
        [](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext& ctx) {
            return onecad::io::handle_inspect_step(r, ctx.cancel);
        });
    // --- M5a: mesh export (STL / OBJ, SCHEMA §7.8) ---
    dispatcher.register_verb(
        "ExportStl",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::io::handle_export_stl(session, r);
        });
    dispatcher.register_verb(
        "ExportObj",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::io::handle_export_obj(session, r);
        });
    // --- M5a: checkpoints (SCHEMA §7.7) ---
    dispatcher.register_verb(
        "SaveCheckpoint",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::io::handle_save_checkpoint(session, r);
        });
    dispatcher.register_verb(
        "RestoreCheckpoint",
        [&session](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return onecad::io::handle_restore_checkpoint(session, r);
        });
    dispatcher.register_verb("Shutdown", handle_shutdown);
#if ONECAD_WORKER_HOOKS
    dispatcher.register_verb("Debug.Busy", handle_debug_busy);
#endif
    solver_lane.register_verbs(dispatcher);  // Sketch* verbs -> solver lane
}

// Validate the hello payload + run a SketchUpsert through the solver lane
// in-process; return exit code (0 == OK).
int run_selftest_impl() {
    Dispatcher dispatcher;
    Session session;
    SolverLane solver_lane(session.sketches());
    register_verbs(dispatcher, solver_lane, session);

    // The hello is unsolicited (SCHEMA §6), not a verb — validate its payload.
    const nlohmann::json hello = make_hello_result_impl();
    const bool hello_ok =
        hello.value("protocolVersion", 0) == kProtocolVersion && hello.contains("occt") &&
        hello["occt"].value("version", std::string{}) == ONECAD_OCCT_VERSION &&
        std::string(ONECAD_OCCT_BUILD_ID).size() > 0 &&
        hello["occt"].value("fingerprint", std::string{}).size() == 16;
    if (!hello_ok) {
        WLOG_ERROR("selftest FAILED: unexpected hello payload: %s", hello.dump().c_str());
        return 1;
    }

    // Exercise the solver lane in-process: a triangle upsert must solve + report.
    nlohmann::json args = {
        {"sketchId", "selftest"},
        {"plane", {{"kind", "XY"}}},
        {"entities",
         {{{"id", "l1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {10, 0}}},
          {{"id", "l2"}, {"type", "Line"}, {"p0", {10, 0}}, {"p1", {5, 8}}},
          {{"id", "l3"}, {"type", "Line"}, {"p0", {5, 8}}, {"p1", {0, 0}}}}},
        {"constraints", nlohmann::json::array()},
    };
    Envelope up = dispatcher.dispatch_once(Envelope::request(1, "SketchUpsert", args));
    if (!up.ok.value_or(false) || !up.result.value("upserted", false)) {
        WLOG_ERROR("selftest FAILED: SketchUpsert response: %s",
                   onecad::protocol::serialize(up).c_str());
        return 1;
    }

    WLOG_INFO("selftest OK (occt %s, build %s, fingerprint %s, upsert dof=%d)",
              hello["occt"]["version"].get<std::string>().c_str(),
              ONECAD_OCCT_BUILD_ID,
              hello["occt"]["fingerprint"].get<std::string>().c_str(),
              up.result.value("dof", -1));
    return 0;
}

}  // namespace

namespace onecad::worker {

void route_occt_diagnostics() {
    // NOT io::occt_globals_mutex(): a worker wedged inside a STEP call holds that
    // one forever, and a new in-process worker must still start (send its hello).
    // In the library nothing else edits the default printer list (OcctMessengerGuard
    // is a no-op there), so this mutex only serializes concurrent worker starts.
    static std::mutex route_mutex;
    const std::lock_guard<std::mutex> lock(route_mutex);
    Handle(Message_Messenger) messenger = Message::DefaultMessenger();
#ifdef ONECAD_WORKER_LIBRARY
    // Already routed by an earlier or concurrent run: mutate nothing, so a
    // starting worker never edits the printer list another worker prints through.
    const Message_SequenceOfPrinters& printers = messenger->Printers();
    if (printers.Length() == 1 &&
        printers.First()->IsKind(STANDARD_TYPE(onecad::capi::LogBridgePrinter))) {
        return;
    }
#endif
    // Drop the default std::cout printer (and a bridge a previous library run
    // left behind, so a re-run never doubles every OCCT line).
    messenger->RemovePrinters(STANDARD_TYPE(Message_PrinterOStream));
    messenger->RemovePrinters(STANDARD_TYPE(onecad::capi::LogBridgePrinter));
    messenger->AddPrinter(onecad::capi::make_diagnostic_printer());
}

nlohmann::json make_hello_result() { return make_hello_result_impl(); }

int run_selftest() { return run_selftest_impl(); }

int run_dispatch_loop(int in_fd, int out_fd, bool close_fds,
                      onecad::protocol::JournalHooks journal, std::size_t lane_stack_bytes) {
    // Library mode owns both fds from the first statement: they are closed on
    // EVERY exit, an exception included, so the host end always sees EOF.
    // Until run() owns the reader fd, `closer` closes both.
    struct FdCloser {
        bool active;
        int in_fd;
        int out_fd;
        ~FdCloser() {
            if (!active) return;
            ::close(in_fd);
            ::close(out_fd);
        }
    } closer{close_fds, in_fd, out_fd};
    WLOG_INFO("onecad-worker %s starting (protocol v%d, occt %s)", kWorkerVersion,
              kProtocolVersion, OCC_VERSION_COMPLETE);
    Dispatcher dispatcher;
    Session session;
    SolverLane solver_lane(session.sketches());
    register_verbs(dispatcher, solver_lane, session);
    // Every worker frame is stamped from the session head (SCHEMA §3).
    dispatcher.set_stamp_source([&session] { return session.head_stamp(); });
    dispatcher.set_journal_hooks(std::move(journal));
    dispatcher.set_lane_stack_bytes(lane_stack_bytes);

    const Envelope hello = Envelope::hello(make_hello_result_impl());
    // From here the Dispatcher's exactly-once close owns the reader fd.
    closer.active = false;
    const auto close_after_run = [&] {
        if (!close_fds) return;
        dispatcher.close_input();  // no-op if the Shutdown path already closed it
        ::close(out_fd);
    };
    int code = 0;
    try {
        code = dispatcher.run(in_fd, out_fd, &hello);
    } catch (...) {
        close_after_run();
        throw;
    }
    close_after_run();
    WLOG_INFO("onecad-worker exiting with code %d", code);
    return code;
}

}  // namespace onecad::worker
