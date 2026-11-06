#include <cstdio>
#include <memory>

#include <BRepPrimAPI_MakeBox.hxx>
#include <BRep_Builder.hxx>
#include <TopoDS_Face.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

#include "session/SourceHistory.h"
#include "session/SourceResolution.h"
#include "session/ElementIdentity.h"
#include "session/Session.h"
#include "session/HistoryHash.h"
#include "session/PreviewOp.h"
#include "session/PlanExecutor.h"

using namespace onecad;

static std::shared_ptr<io::SourceClaimBatch> make_batch(const TopoDS_Shape& face) {
    auto batch = std::make_shared<io::SourceClaimBatch>();
    if (!batch->admit(1)) return {};
    io::SourceFaceClaim claim;
    claim.face = face;
    claim.body_id = "body_00000000-0000-0000-0000-000000000001";
    claim.producer_id = "00000000-0000-0000-0000-000000000001";
    claim.source_sha256 = std::string(64, 'a');
    claim.codec = "step";
    claim.codec_version = "occt-step-stream-v1";
    claim.reader_policy = "step-v1";
    claim.kernel_fingerprint = std::string(16, 'a');
    claim.step_root_entity = 1;
    claim.step_face_entity = 2;
    if (!batch->append(std::move(claim))) return {};
    return batch;
}

static bool invalidation_releases_live_geometry() {
    io::SourceClaimBudget budget(io::kSourceIdentityBudgetBytes);
    {
        BRep_Builder builder;
        TopoDS_Face face;
        builder.MakeFace(face);
        Handle(TopoDS_TShape) tracked = face.TShape();
        auto batch = make_batch(face);
        if (!batch) return false;
        session::SourceLedger ledger(budget);
        if (!ledger.append(batch)) return false;
        const auto birth = ledger.rows()[0].birth;
        if (!ledger.bind("el_lifetime", birth)) return false;
        auto checkpoint = ledger;
        face.Nullify();
        const auto held = tracked->GetRefCount();
        auto pressure = budget.acquire(io::kSourceIdentityBudgetBytes - budget.used());
        if (!pressure || ledger.remap([](const auto& row) { return std::optional<TopoDS_Shape>(row.live_face); })) return false;
        ledger.clear();
        pressure.reset();
        if (!ledger.rows().empty() || !ledger.bound("el_lifetime") || tracked->GetRefCount() != held) return false;
        checkpoint.clear();
        if (tracked->GetRefCount() != 1) return false;
    }
    return budget.used() == 0;
}

int main() {
    if (!invalidation_releases_live_geometry()) return 43;
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(2, 3, 4).Shape();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    auto batch = make_batch(faces(1));
    if (!batch) return 1;
    session::SourceLedger source;
    if (!source.append(batch) || !batch->items()[0].face.IsNull() || !source.lookup("body_00000000-0000-0000-0000-000000000001", faces(1))) return 3;
    if (batch->append(io::SourceFaceClaim{})) return 34;
    try {
        (void)batch->mutable_items();
        return 35;
    } catch (const std::logic_error&) {}
    try {
        io::SourceClaimBatch stolen(std::move(*batch));
        return 41;
    } catch (const std::logic_error&) {}
    const auto identity = source.lookup("body_00000000-0000-0000-0000-000000000001", faces(1));
    if (!source.bind("el_a", identity) || source.bind("el_b", identity) ||
        !source.bind("el_a", identity)) return 8;
    auto other_batch = std::make_shared<io::SourceClaimBatch>();
    if (!other_batch->admit(1)) return 19;
    io::SourceFaceClaim other_claim;
    other_claim.face = faces(1);
    other_claim.body_id = "body_00000000-0000-0000-0000-000000000002";
    other_claim.producer_id = "00000000-0000-0000-0000-000000000002";
    other_claim.source_sha256 = std::string(64, 'b');
    other_claim.codec = "step";
    other_claim.codec_version = "occt-step-stream-v1";
    other_claim.reader_policy = "step-v1";
    other_claim.kernel_fingerprint = std::string(16, 'a');
    other_claim.step_root_entity = 1;
    other_claim.step_face_entity = 3;
    if (!other_batch->append(std::move(other_claim))) return 20;
    auto global = source;
    if (!global.append(other_batch)) return 21;
    const auto alias = global.lookup("body_00000000-0000-0000-0000-000000000002", faces(1));
    if (!alias || global.bind("el_b", alias)) return 22;
    session::SourceLedger checkpoint = source;
    session::BodyStore bodies;
    bodies.create("body_00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000001", box);
    const std::vector<session::BodyEvent> events{{"modified", "body_00000000-0000-0000-0000-000000000001", {}}};
    session::TopologyBodyHistory history;
    history.body_id = "body_00000000-0000-0000-0000-000000000001";
    history.coverage = session::HistoryCoverage::Proven;
    history.survivors.push_back({faces(1), faces(1)});
    if (!session::apply_source_history(source, bodies, events, {history}) ||
        !source.lookup("body_00000000-0000-0000-0000-000000000001", faces(1))) return 4;
    history.survivors.push_back({faces(1), faces(2)});
    if (!session::apply_source_history(source, bodies, events, {history}) ||
        source.lookup("body_00000000-0000-0000-0000-000000000001", faces(1))) return 5;
    if (!checkpoint.lookup("body_00000000-0000-0000-0000-000000000001", faces(1)) || !source.bound("el_a") ||
        !source.bound("el_a")->face.IsNull()) return 6;
    history.survivors.clear();
    history.successors.push_back({{faces(1), faces(2)}, faces(1)});
    if (!session::apply_source_history(checkpoint, bodies, events, {history}) ||
        checkpoint.lookup("body_00000000-0000-0000-0000-000000000001", faces(1)) || !checkpoint.bound("el_a")) return 7;
    session::SourceLedger fresh;
    auto fresh_batch = make_batch(faces(1));
    if (!fresh_batch || !fresh.append(fresh_batch)) return 9;
    session::Session host;
    host.open("doc", 0, 3, "determinism");
    auto fence = host.fence_and_clone(1, 0, 3, session::kEmptyPrefixHash);
    session::ScratchJob job;
    job.job_id = 1;
    job.bodies = fence.cloned_bodies;
    job.partition = fence.cloned_partition;
    job.prepared_snapshot_id = fence.prepared_snapshot_id;
    job.history_prefix_hash = std::string(64, 'a');
    job.bodies.create("body_00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000001", box);
    job.topology_owners.source_ledger() = fresh;
    host.store_prepared(std::move(job));
    const auto accepted = host.accept_prepared(1, 0, 3);
    if (!accepted.ok) return 10;
    using nlohmann::json;
    using protocol::Envelope;
    const auto initial_cert = io::source_identity_json(*fresh.rows()[0].birth);
    const json fresh_ref = {{"refId", "fresh"}, {"primary", {{"bodyId", "body_00000000-0000-0000-0000-000000000001"},
        {"elementId", "el_fresh"}, {"kind", "face"}}}, {"importSourceIdentity", initial_cert}};
    const auto fresh_resolution = session::handle_resolve_refs(host, Envelope::request(15, "ResolveRefs",
        {{"snapshotId", accepted.snapshot_id}, {"refs", json::array({fresh_ref})}}));
    if (fresh_resolution.ok != true || fresh_resolution.result["resolutions"][0]["outcome"] != "autoBind" ||
        fresh_resolution.result["resolutions"][0]["topoKey"] != "f:1" || host.partition_copy().size() != 0 ||
        host.topology_owners_copy().source_ledger().bound("el_fresh")) return 44;
    auto fresh_bad = fresh_ref;
    fresh_bad["importSourceIdentity"]["sourceSha256"] = std::string(64, 'b');
    const auto bad_resolution = session::handle_resolve_refs(host, Envelope::request(16, "ResolveRefs",
        {{"snapshotId", accepted.snapshot_id}, {"refs", json::array({fresh_bad})}}));
    if (bad_resolution.ok != true || bad_resolution.result["resolutions"][0]["outcome"] != "needsRepair" ||
        host.partition_copy().size() != 0) return 45;
    const json legacy_binding = {{"bodyId", "body_00000000-0000-0000-0000-000000000001"},
        {"topoKey", "f:1"}, {"elementId", "el_a"}, {"kind", "face"}};
    const auto legacy = session::handle_bind_element_ids(host, Envelope::request(9, "BindElementIds",
        {{"snapshotId", accepted.snapshot_id}, {"bindings", json::array({legacy_binding})}}));
    if (legacy.ok != true || host.topology_owners_copy().source_ledger().bound("el_a")) return 32;
    const json unresolved_ref = {{"refId", "r"}, {"primary", {{"bodyId", "body_00000000-0000-0000-0000-000000000001"},
        {"elementId", "el_a"}, {"kind", "face"}}}, {"importSourceIdentity", initial_cert}};
    const auto prepared_fence = host.fence_and_clone(9, 0, 3, std::string(64, 'a'));
    if (prepared_fence.status != session::FenceOutcome::Status::Ok) return 36;
    session::ScratchJob pending;
    pending.job_id = 9;
    pending.prepared_snapshot_id = prepared_fence.prepared_snapshot_id;
    pending.bodies = prepared_fence.cloned_bodies;
    pending.partition = prepared_fence.cloned_partition;
    pending.topology_owners = prepared_fence.cloned_topology_owners;
    pending.history_prefix_hash = std::string(64, 'a');
    host.store_prepared(std::move(pending));
    const auto partition_before = host.partition_copy().size();
    const auto legacy_query = session::handle_query_element(host, Envelope::request(10, "QueryElement",
        {{"snapshotId", accepted.snapshot_id}, {"elementId", "el_a"}}));
    if (legacy_query.ok != true || legacy_query.result.contains("importSourceIdentity")) return 37;
    const auto unbound_resolution = session::handle_resolve_refs(host, Envelope::request(11, "ResolveRefs",
        {{"snapshotId", accepted.snapshot_id}, {"refs", json::array({unresolved_ref})}}));
    if (unbound_resolution.ok != true || unbound_resolution.result["resolutions"][0]["outcome"] != "autoBind" ||
        unbound_resolution.result["resolutions"][0]["topoKey"] != "f:1" ||
        host.topology_owners_copy().source_ledger().bound("el_a") ||
        host.partition_copy().size() != partition_before) return 38;
    const auto blocked_acquire = session::handle_acquire_element_ids(host, Envelope::request(12, "AcquireElementIds",
        {{"bodyId", "body_00000000-0000-0000-0000-000000000001"}, {"picks", json::array({{{"topoKey", "f:1"}}})}}));
    if (blocked_acquire.ok != true || blocked_acquire.result["ids"].size() != 1 ||
        blocked_acquire.result["ids"][0].contains("importSourceIdentity") ||
        host.topology_owners_copy().source_ledger().bound("el_a") ||
        host.partition_copy().size() != partition_before) return 39;
    auto blocked_binding = legacy_binding;
    blocked_binding["importSourceIdentity"] = initial_cert;
    const auto blocked_bind = session::handle_bind_element_ids(host, Envelope::request(14, "BindElementIds",
        {{"snapshotId", accepted.snapshot_id}, {"bindings", json::array({blocked_binding})}}));
    if (blocked_bind.ok != false || host.topology_owners_copy().source_ledger().bound("el_a")) return 42;
    if (!host.discard_prepared(9)) return 40;
    const auto acquired = session::handle_acquire_element_ids(host, Envelope::request(1, "AcquireElementIds",
        {{"bodyId", "body_00000000-0000-0000-0000-000000000001"}, {"picks", json::array({{{"topoKey", "f:1"}}})}}));
    if (acquired.ok != true || acquired.result["ids"].size() != 1 ||
        acquired.result["ids"][0].contains("importSourceIdentity") ||
        acquired.result["ids"][0]["elementId"] != "el_a" ||
        host.topology_owners_copy().source_ledger().bound("el_a")) return 11;
    const auto cert = initial_cert;
    json binding = {{"bodyId", "body_00000000-0000-0000-0000-000000000001"}, {"topoKey", "f:1"},
                    {"elementId", "el_a"}, {"kind", "face"}, {"importSourceIdentity", cert}};
    const auto bind = session::handle_bind_element_ids(host, Envelope::request(2, "BindElementIds",
        {{"snapshotId", accepted.snapshot_id}, {"bindings", json::array({binding})}}));
    if (bind.ok != true) return 12;
    const auto certified_pick = session::handle_acquire_element_ids(host, Envelope::request(13, "AcquireElementIds",
        {{"bodyId", "body_00000000-0000-0000-0000-000000000001"}, {"picks", json::array({{{"topoKey", "f:1"}}})}}));
    if (certified_pick.ok != true || certified_pick.result["ids"][0]["importSourceIdentity"] != cert ||
        !host.topology_owners_copy().source_ledger().bound("el_a")) return 33;
    const auto queried = session::handle_query_element(host, Envelope::request(6, "QueryElement",
        {{"snapshotId", accepted.snapshot_id}, {"elementId", "el_a"}}));
    if (queried.ok != true || queried.result["importSourceIdentity"] != cert) return 28;
    json ref = {{"refId", "r"}, {"primary", {{"bodyId", "body_00000000-0000-0000-0000-000000000001"},
        {"elementId", "el_a"}, {"kind", "face"}}}, {"importSourceIdentity", cert}};
    const auto resolve = [&](const json& value) {
        return session::handle_resolve_refs(host, Envelope::request(3, "ResolveRefs",
            {{"snapshotId", accepted.snapshot_id}, {"refs", json::array({value})}}));
    };
    if (resolve(ref).result["resolutions"][0]["outcome"] != "unchanged") return 13;
    auto competing_ref = ref;
    competing_ref["primary"]["elementId"] = "el_competing";
    if (resolve(competing_ref).result["resolutions"][0]["outcome"] != "needsRepair") return 46;
    auto missing = ref;
    missing.erase("importSourceIdentity");
    if (resolve(missing).result["resolutions"][0]["outcome"] != "needsRepair") return 14;
    auto bad = ref;
    bad["importSourceIdentity"]["sourceSha256"] = std::string(64, 'b');
    if (resolve(bad).result["resolutions"][0]["outcome"] != "needsRepair") return 15;
    const json shell = {{"opType", "Shell"}, {"opId", "preview_shell"},
        {"inputs", json::array({ref})}, {"params", {{"targetBodyId", "body_00000000-0000-0000-0000-000000000001"},
        {"thickness", 0.2}, {"openFaces", json::array({"el_a"})}}}};
    const auto preview = session::handle_preview_op(host, Envelope::request(7, "PreviewOp", {{"op", shell}}));
    if (preview.ok != true || !preview.result["needsRepair"].empty() ||
        preview.result["changedBodies"].empty()) return 23;
    session::ScratchJob committing;
    committing.bodies.create("body_00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000001", box);
    committing.topology_owners.source_ledger() = fresh;
    std::string last_sketch;
    onecad::CancelToken cancel;
    const auto committed = session::execute_candidate_op(committing, shell, "shell", last_sketch, cancel);
    if (committed.status != session::CandidateResult::Status::Ok ||
        committing.bodies.get("body_00000000-0000-0000-0000-000000000001")->geom.IsSame(box)) return 30;
    session::ScratchJob rejected;
    rejected.bodies.create("body_00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000001", box);
    rejected.topology_owners.source_ledger() = fresh;
    auto bad_shell = shell;
    bad_shell["inputs"][0] = bad;
    const auto refused = session::execute_candidate_op(rejected, bad_shell, "shell", last_sketch, cancel);
    if (refused.status != session::CandidateResult::Status::NeedsRepair ||
        !rejected.bodies.get("body_00000000-0000-0000-0000-000000000001")->geom.IsSame(box) ||
        rejected.partition.contains("el_a")) return 31;
    auto wrong_preview_op = shell;
    wrong_preview_op["inputs"][0] = bad;
    const auto wrong_preview = session::handle_preview_op(host, Envelope::request(8, "PreviewOp", {{"op", wrong_preview_op}}));
    if (wrong_preview.ok != true || wrong_preview.result["needsRepair"].empty() || !wrong_preview.out_bin.empty()) return 24;
    const auto saved = host.save_checkpoint(0);
    const auto budget_before_checkpoint = io::source_claim_budget().used();
    const auto saved_again = host.save_checkpoint(1);
    if (io::source_claim_budget().used() != budget_before_checkpoint ||
        !saved_again.topology_owners.source_ledger().bound("el_a")) return 25;
    const auto restored = host.restore_checkpoint(0, saved.history_prefix_hash, "cp");
    if (!restored.restored) return 26;
    const session::BaseCheckpointRef base{0, "cp"};
    const auto restored_fence = host.fence_and_clone(5, 0, 3, saved.history_prefix_hash, &base);
    if (restored_fence.status != session::FenceOutcome::Status::Ok ||
        !restored_fence.cloned_topology_owners.source_ledger().bound("el_a")) return 27;
    auto other = binding;
    other["elementId"] = "el_b";
    if (session::handle_bind_element_ids(host, Envelope::request(4, "BindElementIds",
        {{"snapshotId", accepted.snapshot_id}, {"bindings", json::array({other})}})).ok != false) return 16;
    session::ScratchJob preflight;
    preflight.bodies.create("body_00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000001", box);
    preflight.topology_owners.source_ledger() = fresh;
    elementmap::ElementMapDelta delta;
    std::vector<json> repairs;
    if (!session::resolve_source_refs(preflight, {{"params", {{"face", ref}}}}, "op", delta, repairs) ||
        !preflight.partition.contains("el_a")) return 17;
    repairs.clear();
    if (session::resolve_source_refs(preflight, {{"params", {{"face", missing}}}}, "op", delta, repairs) ||
        repairs.empty()) return 18;
    repairs.clear();
    if (session::resolve_source_refs(preflight, {{"params", {{"openFaces", json::array({"el_a"})}}}},
                                     "op", delta, repairs) || repairs.empty()) return 29;
    std::fprintf(stderr, "source_ledger: OK\n");
    return 0;
}
