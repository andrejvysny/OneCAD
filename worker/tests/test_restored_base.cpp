// test_restored_base.cpp — WP-H: `RestoreCheckpoint` installs a RESTORED BASE,
// not a head, and `ExecutePlan` consumes it through the fourth fence case
// (SCHEMA §7.7 / §7.2 `baseCheckpoint` / §7.1 `hasRestoredBase`).
//
// The hazard this closes (finding session-executor-2, measured red-first in
// `src-tauri/tests/restore_fencing.rs`): a restore used to roll the LIVE head
// back, so an unfenced `Tessellate` or export between the restore and the plan
// it seeded served rolled-back geometry while Rust still believed the head was
// the post-edit one — and the head's `snapshotId` moved under Rust's feet.
//
// Covered here, in one session so the ORDER is part of the assertion:
//   1. a restore leaves snapshotId / historyPrefixHash / bodies alone and turns
//      `hasRestoredBase` on;
//   2. the OPTIONAL reader fence refuses a stale `snapshotId` — and an export
//      refused this way leaves NO file behind (§7.6/§7.8);
//   3. a plan naming a checkpoint the slot does not hold is a `PROTOCOL_ERROR`,
//      and that failed prepare LEAVES the base (the next plan decides);
//   4. a plan naming the slot fences `expectedBaseHash` against the BASE's hash
//      (which is NOT the head's here), clones the scratch from the base, keeps
//      the slot while prepared, and drops it at AcceptPrepared — publishing the
//      base plus the plan's own new step;
//   5. `DiscardPrepared` leaves the base; a plan WITHOUT `baseCheckpoint` drops
//      it before it is fenced; `ResetSession` clears it.
//
// No test framework: exit code == failure count.
#include <cstdio>
#include <string>
#include <vector>

#include <sys/stat.h>

#include "io/Checkpoint.h"
#include "io/MeshExport.h"
#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "session/Signatures.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::Session;

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

constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

json rect(double w, double h) {
    return json::array({{{"id", "e1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {w, 0}}},
                        {{"id", "e2"}, {"type", "Line"}, {"p0", {w, 0}}, {"p1", {w, h}}},
                        {{"id", "e3"}, {"type", "Line"}, {"p0", {w, h}}, {"p1", {0, h}}},
                        {{"id", "e4"}, {"type", "Line"}, {"p0", {0, h}}, {"p1", {0, 0}}}});
}

json sketch_op(const std::string& op_id, const std::string& sid, std::uint64_t step, double w,
               double h) {
    return json{{"opType", "Sketch"},
                {"opId", op_id},
                {"stepIndex", step},
                {"params",
                 {{"sketchId", sid}, {"plane", {{"kind", "XY"}}}, {"entities", rect(w, h)},
                  {"constraints", json::array()}}}};
}

json extrude_op(const std::string& op_id, const std::string& sid, std::uint64_t step, double d) {
    return json{{"opType", "Extrude"},
                {"opId", op_id},
                {"stepIndex", step},
                {"params",
                 {{"sketchId", sid}, {"distance", d}, {"extrudeMode", "Blind"},
                  {"booleanMode", "NewBody"}}}};
}

// One ExecutePlan, with the args a caller actually varies. `base` is the §7.2
// `baseCheckpoint` object (or null to omit the key entirely).
Envelope run_plan(Session& s, std::uint64_t job_id, const json& ops, const std::string& base_hash,
                  const json& prefix_hashes, const json& base = json()) {
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    json args = {{"jobId", job_id},
                 {"documentRevision", 0},
                 {"workerEpoch", 3},
                 {"expectedBaseHash", base_hash},
                 {"prefixHashes", prefix_hashes},
                 {"targetStep", ops.size()},
                 {"ops", ops}};
    if (!base.is_null()) args["baseCheckpoint"] = base;
    return onecad::session::handle_execute_plan(
        s, Envelope::request(job_id, "ExecutePlan", args), ctx);
}

void accept(Session& s, std::uint64_t job_id) {
    const Envelope a = onecad::session::handle_accept_prepared(
        s, Envelope::request(job_id, "AcceptPrepared",
                             json{{"jobId", job_id}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    CHECK(a.ok.value_or(false), "AcceptPrepared(job %llu) failed: %s",
          static_cast<unsigned long long>(job_id),
          a.error.has_value() ? a.error->message.c_str() : "<no error>");
}

bool file_exists(const std::string& path) {
    struct stat st{};
    return ::stat(path.c_str(), &st) == 0;
}

}  // namespace

int main() {
    Session s;
    s.open("doc", 0, 3, "determinism");

    // ── head A: a 40×20×25 box, head hash "hash_a" ────────────────────────────
    run_plan(s, 1, json::array({sketch_op("opA0", "skA", 0, 40, 20), extrude_op("op1", "skA", 1, 25)}),
             kEmpty, json::array({"hash_a0", "hash_a"}));
    accept(s, 1);
    const std::string sig_a = onecad::session::geometry_signature(s.bodies_copy());
    const Envelope save = onecad::io::handle_save_checkpoint(
        s, Envelope::request(2, "SaveCheckpoint", json{{"stepIndex", 1}}));
    CHECK(save.ok.value_or(false), "SaveCheckpoint must succeed");
    const std::string ckpt_hash = save.result.value("historyPrefixHash", std::string{});
    CHECK(ckpt_hash == "hash_a", "the checkpoint froze the head token, got '%s'",
          ckpt_hash.c_str());

    // ── head B: replace it with a 60×60×60 box, head hash "hash_b" ────────────
    run_plan(s, 3, json::array({sketch_op("opB0", "skB", 0, 60, 60), extrude_op("op4", "skB", 1, 60)}),
             kEmpty, json::array({"hash_b0", "hash_b"}));
    accept(s, 3);
    const std::string sig_b = onecad::session::geometry_signature(s.bodies_copy());
    CHECK(sig_a != sig_b, "precondition: the two heads differ");
    const onecad::session::WorkerHead before = s.head();
    CHECK(before.history_prefix_hash == "hash_b" && !before.has_restored_base,
          "precondition: head at hash_b with no pending base");

    // ── (1) the restore installs a BASE; the head does not move ───────────────
    const Envelope restore = onecad::io::handle_restore_checkpoint(
        s, Envelope::request(5, "RestoreCheckpoint",
                             json{{"stepIndex", 1},
                                  {"checkpointId", "probe_ckpt_1"},
                                  {"expectedHistoryPrefixHash", ckpt_hash},
                                  {"workerEpoch", 3}}));
    const onecad::session::WorkerHead after = s.head();
    std::fprintf(stderr, "restore: result=%s head snapshotId %llu -> %llu, hash '%s' -> '%s'\n",
                 restore.result.dump().c_str(),
                 static_cast<unsigned long long>(before.snapshot_id),
                 static_cast<unsigned long long>(after.snapshot_id),
                 before.history_prefix_hash.c_str(), after.history_prefix_hash.c_str());
    CHECK(restore.ok.value_or(false) && restore.result.value("restored", false),
          "RestoreCheckpoint must report restored:true");
    CHECK(!restore.result.value("driftDetected", true), "no drift expected");
    CHECK(restore.result.value("snapshotId", std::uint64_t{0}) == before.snapshot_id,
          "the result's snapshotId must echo the UNCHANGED head (%llu), got %llu",
          static_cast<unsigned long long>(before.snapshot_id),
          static_cast<unsigned long long>(restore.result.value("snapshotId", std::uint64_t{0})));
    CHECK(after.snapshot_id == before.snapshot_id, "a restore must not move the head snapshotId");
    CHECK(after.history_prefix_hash == before.history_prefix_hash,
          "a restore must not move the head token");
    CHECK(onecad::session::geometry_signature(s.bodies_copy()) == sig_b,
          "an unfenced reader must still see the HEAD's geometry after a restore");
    CHECK(after.has_restored_base, "hasRestoredBase must be true while a base is pending");

    // ── (2) the OPTIONAL reader fence, and a refused export writes nothing ────
    const std::string stl_path = "/tmp/onecad_wph_stale_export.stl";
    ::remove(stl_path.c_str());
    const Envelope stale_stl = onecad::io::handle_export_stl(
        s, Envelope::request(6, "ExportStl",
                             json{{"path", stl_path},
                                  {"snapshotId", after.snapshot_id + 99},
                                  {"lod", "coarse"}}));
    CHECK(!stale_stl.ok.value_or(true) && stale_stl.error.has_value() &&
              stale_stl.error->code == "STALE_PREVIEW",
          "a stale ExportStl must be STALE_PREVIEW, got %s",
          stale_stl.error.has_value() ? stale_stl.error->code.c_str() : "<ok>");
    CHECK(!file_exists(stl_path), "a STALE_PREVIEW export must leave NO file at %s",
          stl_path.c_str());
    const Envelope fresh_stl = onecad::io::handle_export_stl(
        s, Envelope::request(7, "ExportStl",
                             json{{"path", stl_path}, {"snapshotId", after.snapshot_id}}));
    CHECK(fresh_stl.ok.value_or(false), "a matching snapshotId must export normally");
    CHECK(file_exists(stl_path), "the matching export must write its file");
    ::remove(stl_path.c_str());
    const Envelope unfenced_stl = onecad::io::handle_export_stl(
        s, Envelope::request(8, "ExportStl", json{{"path", stl_path}}));
    CHECK(unfenced_stl.ok.value_or(false), "an ABSENT snapshotId keeps the pre-WP-H live-head path");
    ::remove(stl_path.c_str());

    // A PRESENT but malformed snapshotId must REFUSE, never fall through to the
    // live head: §4's tolerate-malformed rule would silently unfence the write
    // the caller asked to fence (adversarial review 2026-09-05).
    for (const json& bad : {json("1"), json(1.5), json(-1), json(nullptr)}) {
        const Envelope malformed = onecad::io::handle_export_stl(
            s, Envelope::request(20, "ExportStl",
                                 json{{"path", stl_path}, {"snapshotId", bad}}));
        CHECK(!malformed.ok.value_or(true) && malformed.error.has_value() &&
                  malformed.error->code == "PROTOCOL_ERROR",
              "snapshotId %s must be PROTOCOL_ERROR, got %s", bad.dump().c_str(),
              malformed.error.has_value() ? malformed.error->code.c_str() : "<ok>");
        CHECK(!file_exists(stl_path), "a refused malformed-fence export writes no file");
    }

    // ── (3) a plan naming an unknown checkpoint: PROTOCOL_ERROR, base kept ────
    const Envelope unknown = run_plan(s, 9, json::array(), ckpt_hash, json::array(),
                                      json{{"stepIndex", 1}, {"checkpointId", "ckpt_nope"}});
    CHECK(!unknown.ok.value_or(true) && unknown.error.has_value() &&
              unknown.error->code == "PROTOCOL_ERROR",
          "an unknown baseCheckpoint must be PROTOCOL_ERROR, got %s",
          unknown.error.has_value() ? unknown.error->code.c_str() : "<ok>");
    CHECK(s.has_restored_base(), "a FAILED prepare must leave the restored base in place");
    CHECK(!s.has_scratch(), "a refused plan stores nothing");

    // A present `baseCheckpoint` with an EMPTY checkpointId must be refused: an
    // empty id inside a fence would bind whatever sits at that step.
    const Envelope empty_id = run_plan(s, 19, json::array(), ckpt_hash, json::array(),
                                       json{{"stepIndex", 1}, {"checkpointId", ""}});
    CHECK(!empty_id.ok.value_or(true) && empty_id.error.has_value() &&
              empty_id.error->code == "PROTOCOL_ERROR",
          "an empty checkpointId must be PROTOCOL_ERROR, got %s",
          empty_id.error.has_value() ? empty_id.error->code.c_str() : "<ok>");
    CHECK(s.has_restored_base(), "that refusal leaves the base too");

    // A plan naming the right slot but the WRONG base hash is refused too — the
    // fence is against the BASE's hash, so the head's own token must not pass.
    const Envelope wrong_hash = run_plan(s, 10, json::array(), "hash_b", json::array(),
                                         json{{"stepIndex", 1}, {"checkpointId", "probe_ckpt_1"}});
    CHECK(!wrong_hash.ok.value_or(true) && wrong_hash.error.has_value() &&
              wrong_hash.error->code == "PROTOCOL_ERROR",
          "expectedBaseHash is fenced against the BASE, not the head");
    CHECK(s.has_restored_base(), "that failed prepare leaves the base too");

    // ── (4) the plan that names the slot: cloned from the base, published ─────
    const Envelope prepared =
        run_plan(s, 11, json::array({sketch_op("op8", "skC", 2, 5, 5), extrude_op("op9", "skC", 3, 5)}),
                 ckpt_hash, json::array({"hash_c0", "hash_c"}),
                 json{{"stepIndex", 1}, {"checkpointId", "probe_ckpt_1"}});
    CHECK(prepared.ok.value_or(false) && prepared.result.value("planPrepared", false),
          "the plan naming the restored base must prepare, got %s",
          prepared.error.has_value() ? prepared.error->message.c_str() : "<ok>");
    CHECK(s.has_scratch() && s.has_restored_base(),
          "between store_prepared and accept BOTH hasScratch and hasRestoredBase are true");
    accept(s, 11);
    CHECK(!s.has_restored_base(), "AcceptPrepared drops the base it published from");
    const std::vector<std::string> ids = s.bodies_copy().ids();
    CHECK(ids.size() == 2, "the published head is the BASE plus the plan's new body, got %zu bodies",
          ids.size());
    bool has_base_body = false;
    for (const std::string& id : ids) has_base_body = has_base_body || id == "body_op1";
    CHECK(has_base_body, "the base's body (body_op1) must survive into the published head");
    CHECK(s.head().history_prefix_hash == "hash_c", "the head adopts the plan's token");

    // ── (4b) an accept drops only the base ITS OWN plan used ─────────────────
    // A restore that lands while a baseCheckpoint plan is prepared parks a base for
    // the NEXT plan (§7.1 lifetime). Accepting the earlier plan must not eat it.
    const Envelope save7 = onecad::io::handle_save_checkpoint(
        s, Envelope::request(40, "SaveCheckpoint", json{{"stepIndex", 7}}));
    CHECK(save7.result.value("historyPrefixHash", std::string{}) == "hash_c",
          "the second checkpoint freezes the CURRENT head token");
    onecad::io::handle_restore_checkpoint(
        s, Envelope::request(41, "RestoreCheckpoint",
                             json{{"stepIndex", 1},
                                  {"checkpointId", "probe_ckpt_1"},
                                  {"expectedHistoryPrefixHash", ckpt_hash},
                                  {"workerEpoch", 3}}));
    const Envelope prep30 = run_plan(s, 30, json::array(), ckpt_hash, json::array(),
                                     json{{"stepIndex", 1}, {"checkpointId", "probe_ckpt_1"}});
    CHECK(prep30.ok.value_or(false), "base-only prepare against the step-1 base");
    onecad::io::handle_restore_checkpoint(
        s, Envelope::request(42, "RestoreCheckpoint",
                             json{{"stepIndex", 7},
                                  {"checkpointId", "probe_ckpt_7"},
                                  {"expectedHistoryPrefixHash", "hash_c"},
                                  {"workerEpoch", 3}}));
    accept(s, 30);
    CHECK(s.has_restored_base(),
          "the accept must drop only the base ITS plan used, not one parked after it");
    const Envelope prep31 = run_plan(s, 31, json::array(), "hash_c", json::array(),
                                     json{{"stepIndex", 7}, {"checkpointId", "probe_ckpt_7"}});
    CHECK(prep31.ok.value_or(false),
          "the surviving base is the step-7 one: a plan naming it must prepare, got %s",
          prep31.error.has_value() ? prep31.error->message.c_str() : "<ok>");
    onecad::session::handle_discard_prepared(
        s, Envelope::request(43, "DiscardPrepared", json{{"jobId", 31}}));

    // ── (5) DiscardPrepared leaves it; a plan without baseCheckpoint drops it ─
    const Envelope restore2 = onecad::io::handle_restore_checkpoint(
        s, Envelope::request(12, "RestoreCheckpoint",
                             json{{"stepIndex", 1},
                                  {"checkpointId", "probe_ckpt_1"},
                                  {"expectedHistoryPrefixHash", ckpt_hash},
                                  {"workerEpoch", 3}}));
    CHECK(restore2.result.value("restored", false) && s.has_restored_base(), "second restore");
    const Envelope prep2 = run_plan(s, 13, json::array(), ckpt_hash, json::array(),
                                    json{{"stepIndex", 1}, {"checkpointId", "probe_ckpt_1"}});
    CHECK(prep2.ok.value_or(false), "base-only prepare against the slot");
    onecad::session::handle_discard_prepared(
        s, Envelope::request(14, "DiscardPrepared", json{{"jobId", 13}}));
    CHECK(!s.has_scratch(), "DiscardPrepared drops the scratch");
    CHECK(s.has_restored_base(), "DiscardPrepared LEAVES the restored base (§7.1 lifetime)");

    const Envelope plain = run_plan(
        s, 15, json::array({sketch_op("opD0", "skD", 0, 8, 8), extrude_op("op16", "skD", 1, 8)}),
        kEmpty, json::array({"hash_d0", "hash_d"}));
    CHECK(plain.ok.value_or(false), "an ordinary from-0 plan still prepares");
    CHECK(!s.has_restored_base(),
          "a plan WITHOUT baseCheckpoint drops the pending base before it is fenced");
    accept(s, 15);

    // ── (6) ResetSession clears the slot (open() clears it on the same line) ─
    const Envelope restore3 = onecad::io::handle_restore_checkpoint(
        s, Envelope::request(17, "RestoreCheckpoint",
                             json{{"stepIndex", 1},
                                  {"checkpointId", "probe_ckpt_1"},
                                  {"expectedHistoryPrefixHash", ckpt_hash},
                                  {"workerEpoch", 3}}));
    CHECK(restore3.result.value("restored", false) && s.has_restored_base(),
          "precondition: a third restore parks the base again");
    s.reset();
    CHECK(!s.has_restored_base(), "ResetSession must clear the restored base");

    if (g_failures == 0) std::fprintf(stderr, "restored_base PASS\n");
    return g_failures;
}
