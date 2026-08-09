// test_element_identity_gate.cpp — VF-M3: `AcquireElementIds` must refuse evidence
// it cannot honestly resolve, instead of binding the least-wrong sub-shape.
//
// ── The hazard ───────────────────────────────────────────────────────────────
// Two independent ways the verb used to mint a persistent, op-referencable
// ElementId for geometry the user never picked:
//
//   1. STALE SNAPSHOT. `snapshotId` rode in the request and was IGNORED. A
//      `TopoKey` is a 1-based ordinal into `TopExp::MapShapes` — snapshot-scoped
//      evidence (SCHEMA §9) — so a pick captured before a regen names a different
//      face after it. The verb happily resolved "f:1" against the NEW head.
//
//   2. ANCHOR FALLBACK. With no usable topoKey the verb fell back to
//      `nearest_subshape`, which ranks candidates by distance to their bbox centre
//      and ALWAYS returns the least-bad one. A world point that missed the body by
//      a body-width still bound a face.
//
// Both are the H5-B class this migration exists to kill: a silent wrong bind is
// strictly worse than a loud refusal, because the wrong id is then persisted into
// an operation's inputs and every later regen re-resolves it *cleanly*.
//
// ── What this asserts ────────────────────────────────────────────────────────
//   1. head-snapshot pick with a real topoKey  → ok, binds f:1.
//   2. stale snapshotId (head+1, and head-1)   → REF_UNRESOLVED, detail {requested, head}.
//   3. absent snapshotId                       → still ok (no claim; legacy callers).
//   4. anchor 40 mm off a 10 mm box            → pick DROPPED (empty ids array).
//   5. anchor ON a face of that same box       → still binds (the veto is proportional
//                                                to the candidate, not a blanket ban).
//   6. anchor at a face CORNER                 → still binds (a corner is legitimately
//                                                far from the face centre — this is the
//                                                case the 0.85 descriptor gate would
//                                                have wrongly dropped).
//
// No framework: exit code == failure count.
#include <cstdint>
#include <cstdio>
#include <string>

#include <BRepBuilderAPI_Transform.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax1.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "nlohmann/json.hpp"
#include "protocol/Envelope.h"
#include "session/ElementIdentity.h"
#include "session/HistoryHash.h"
#include "session/ScratchJob.h"
#include "session/Session.h"

using nlohmann::json;
using onecad::protocol::Envelope;
using onecad::session::ScratchJob;
using onecad::session::Session;

namespace {

// A 10 mm cube: every face's bbox diagonal is 10·√2 ≈ 14.14, so the proportional
// anchor bound is ≈ 15.14 mm. A 40 mm miss is far outside it; a corner hit (≈7.07
// from the face centre) is comfortably inside.
constexpr double kBox = 10.0;
constexpr const char* kBody = "body_a";

int g_failures = 0;

void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

// Drive a Session to a published head holding one 10 mm cube. Returns the head
// snapshot id (the worker mints it at AcceptPrepared).
std::uint64_t publish_cube(Session& session, bool transformed = false) {
    session.open("doc_1", /*documentRevision=*/0, /*workerEpoch=*/3, "determinism");
    auto fence = session.fence_and_clone(/*jobId=*/1, /*documentRevision=*/0, /*workerEpoch=*/3,
                                         onecad::session::kEmptyPrefixHash);
    ScratchJob job;
    job.job_id = 1;
    job.bodies = fence.cloned_bodies;
    job.partition = fence.cloned_partition;
    job.prepared_snapshot_id = fence.prepared_snapshot_id;
    job.history_prefix_hash = std::string(64, 'a');
    TopoDS_Shape cube = BRepPrimAPI_MakeBox(kBox, kBox, kBox).Shape();
    if (transformed) {
        gp_Trsf placement;
        placement.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), 0.37);
        placement.SetTranslationPart(gp_Vec(23.0, -11.0, 7.0));
        cube = BRepBuilderAPI_Transform(cube, placement, true).Shape();
    }
    job.bodies.create(kBody, "op_a", cube);
    session.store_prepared(std::move(job));
    const auto accepted = session.accept_prepared(/*jobId=*/1, /*documentRevision=*/0,
                                                  /*workerEpoch=*/3);
    check(accepted.ok, "setup: AcceptPrepared published the cube");
    return accepted.snapshot_id;
}

Envelope acquire(Session& session, json args) {
    return onecad::session::handle_acquire_element_ids(
        session, Envelope::request(/*id=*/7, "AcquireElementIds", std::move(args)));
}

Envelope bind(Session& session, std::uint64_t snapshot, json bindings) {
    return onecad::session::handle_bind_element_ids(
        session, Envelope::request(8, "BindElementIds",
                                   json{{"snapshotId", snapshot},
                                        {"bindings", std::move(bindings)}}));
}

Envelope query(Session& session, std::uint64_t snapshot, const std::string& element_id) {
    return onecad::session::handle_query_element(
        session, Envelope::request(9, "QueryElement",
                                   json{{"snapshotId", snapshot},
                                        {"elementId", element_id}}));
}

Envelope resolve(Session& session, std::uint64_t snapshot, json refs) {
    return onecad::session::handle_resolve_refs(
        session, Envelope::request(10, "ResolveRefs",
                                   json{{"snapshotId", snapshot}, {"refs", std::move(refs)}}));
}

json binding(const char* topo_key, const char* element_id, const char* kind) {
    return json{{"bodyId", kBody},
                {"topoKey", topo_key},
                {"elementId", element_id},
                {"kind", kind}};
}

json primary_ref(const char* ref_id, const char* element_id, const char* kind) {
    return json{{"refId", ref_id},
                {"primary", json{{"bodyId", kBody},
                                 {"elementId", element_id},
                                 {"kind", kind}}}};
}

bool is_error(const Envelope& resp, const std::string& code) {
    return resp.ok.has_value() && !*resp.ok && resp.error.has_value() && resp.error->code == code;
}

std::size_t id_count(const Envelope& resp) {
    if (!resp.ok.has_value() || !*resp.ok) return 0;
    if (!resp.result.contains("ids") || !resp.result["ids"].is_array()) return 0;
    return resp.result["ids"].size();
}

// --- 1/2/3: the stale-snapshot gate ---
void test_snapshot_gate() {
    Session session;
    const std::uint64_t head = publish_cube(session);
    check(head > 0, "setup: head snapshot id is positive");

    const json picks = json::array({json{{"topoKey", "f:1"}}});

    const Envelope fresh =
        acquire(session, json{{"snapshotId", head}, {"bodyId", kBody}, {"picks", picks}});
    check(id_count(fresh) == 1, "head snapshotId: pick resolves");
    if (id_count(fresh) == 1) {
        check(fresh.result["ids"][0]["topoKey"] == "f:1", "head snapshotId: binds f:1");
    }

    const Envelope ahead =
        acquire(session, json{{"snapshotId", head + 1}, {"bodyId", kBody}, {"picks", picks}});
    check(is_error(ahead, "REF_UNRESOLVED"), "future snapshotId → REF_UNRESOLVED");
    if (ahead.error.has_value() && ahead.error->detail.has_value()) {
        const json& d = *ahead.error->detail;
        check(d.value("requested", 0ULL) == head + 1, "error detail carries the requested id");
        check(d.value("head", 0ULL) == head, "error detail carries the head id");
    } else {
        check(false, "stale snapshotId error carries a {requested, head} detail");
    }

    const Envelope stale =
        acquire(session, json{{"snapshotId", head - 1}, {"bodyId", kBody}, {"picks", picks}});
    check(is_error(stale, "REF_UNRESOLVED"), "stale snapshotId → REF_UNRESOLVED");

    // Absent snapshotId is "no claim" — tolerated, so the gate cannot break a
    // caller that never addressed a snapshot.
    const Envelope unscoped = acquire(session, json{{"bodyId", kBody}, {"picks", picks}});
    check(id_count(unscoped) == 1, "absent snapshotId: still resolves (no claim)");
}

// --- 4/5/6: the anchor-fallback sanity veto ---
void test_anchor_veto() {
    Session session;
    const std::uint64_t head = publish_cube(session);

    // No topoKey ⇒ the anchor branch owns the resolution.
    auto anchor_pick = [](double x, double y, double z) {
        return json::array({json{{"anchor", json{{"worldPoint", json::array({x, y, z})}}}}});
    };

    const Envelope way_off = acquire(
        session,
        json{{"snapshotId", head}, {"bodyId", kBody}, {"picks", anchor_pick(40.0, 40.0, 40.0)}});
    check(id_count(way_off) == 0,
          "anchor 40 mm off every candidate on a 10 mm box → pick DROPPED "
          "(pre-fix: bound the nearest face)");

    const Envelope on_face = acquire(
        session,
        json{{"snapshotId", head}, {"bodyId", kBody}, {"picks", anchor_pick(5.0, 5.0, kBox)}});
    check(id_count(on_face) == 1, "anchor on the top face centre still binds");

    // A CORNER of the top face: 7.07 mm from that face's centre — the case a
    // descriptor-score gate would have dropped, and the reason the veto is
    // proportional to the candidate's own size rather than absolute.
    const Envelope corner = acquire(
        session,
        json{{"snapshotId", head}, {"bodyId", kBody}, {"picks", anchor_pick(0.0, 0.0, kBox)}});
    check(id_count(corner) == 1, "anchor at a face corner still binds");
}

void test_bind_query_resolve_round_trip(bool transformed) {
    Session session;
    const std::uint64_t head = publish_cube(session, transformed);
    const json bindings = json::array({binding("f:1", "el_face", "face"),
                                       binding("e:1", "el_edge", "edge"),
                                       binding("v:1", "el_vertex", "vertex")});
    const Envelope installed = bind(session, head, bindings);
    check(installed.ok == true, "BindElementIds: face/edge/vertex batch succeeds");
    check(installed.result.value("bound", json::array()).size() == 3,
          "BindElementIds: result echoes all bindings");

    for (const json& expected : bindings) {
        const std::string id = expected["elementId"].get<std::string>();
        const Envelope found = query(session, head, id);
        check(found.ok == true && found.result.value("present", false), id + " query present");
        check(found.result.value("topoKey", "") == expected["topoKey"].get<std::string>(),
              id + " query preserves topoKey");
        check(found.result.value("kind", "") == expected["kind"].get<std::string>(),
              id + " query preserves kind");
    }

    const json refs = json::array({primary_ref("r_face", "el_face", "face"),
                                   primary_ref("r_edge", "el_edge", "edge"),
                                   primary_ref("r_vertex", "el_vertex", "vertex")});
    const Envelope resolved = resolve(session, head, refs);
    check(resolved.ok == true, "ResolveRefs: bound batch resolves");
    const json outcomes = resolved.result.value("resolutions", json::array());
    check(outcomes.size() == 3, "ResolveRefs: one result per primary-only ref");
    for (std::size_t i = 0; i < outcomes.size(); ++i) {
        check(outcomes[i].value("outcome", "") == "unchanged", "ResolveRefs: unchanged");
        check(outcomes[i].value("elementId", "") ==
                  refs[i]["primary"]["elementId"].get<std::string>(),
              "ResolveRefs: same elementId");
        check(outcomes[i].value("topoKey", "") == bindings[i]["topoKey"].get<std::string>(),
              "ResolveRefs: same topoKey");
    }
    check(bind(session, head, bindings).ok == true, "exact rebind is idempotent");
    check(session.partition_copy().size() == 3, "idempotent rebind adds no entries");
}

void test_identity_snapshot_fences() {
    Session session;
    const std::uint64_t head = publish_cube(session);
    const Envelope stale_bind =
        bind(session, head + 1, json::array({binding("f:1", "el_stale", "face")}));
    check(is_error(stale_bind, "REF_UNRESOLVED"), "stale BindElementIds refused");
    check(!query(session, head, "el_stale").result.value("present", true),
          "stale bind mutates nothing");
    check(is_error(query(session, head + 1, "el_missing"), "REF_UNRESOLVED"),
          "stale QueryElement refused");
    check(is_error(resolve(session, head + 1, json::array()), "REF_UNRESOLVED"),
          "stale ResolveRefs refused");
}

void test_invalid_batches_are_atomic() {
    Session session;
    const std::uint64_t head = publish_cube(session);
    const json wrong_kind = json::array({binding("v:1", "el_partial", "vertex"),
                                         binding("e:1", "el_wrong", "face")});
    check(bind(session, head, wrong_kind).ok == false, "kind/topoKey mismatch refused");
    check(!query(session, head, "el_partial").result.value("present", true),
          "invalid batch installs no valid prefix");

    check(bind(session, head, json::array({binding("f:1", "el_base", "face")})).ok == true,
          "setup: base binding installed");
    const json id_conflict = json::array({binding("v:1", "el_partial", "vertex"),
                                          binding("f:2", "el_base", "face")});
    check(bind(session, head, id_conflict).ok == false, "same id on different topoKey refused");
    check(!query(session, head, "el_partial").result.value("present", true),
          "id-conflict batch is atomic");

    const json topo_conflict = json::array({binding("v:1", "el_partial", "vertex"),
                                            binding("f:1", "el_other", "face")});
    check(bind(session, head, topo_conflict).ok == false,
          "same topoKey with different id refused");
    check(!query(session, head, "el_partial").result.value("present", true),
          "topo-conflict batch is atomic");
    const Envelope base = query(session, head, "el_base");
    check(base.result.value("present", false) && base.result.value("topoKey", "") == "f:1",
          "failed batches preserve prior binding");
}

}  // namespace

int main() {
    test_snapshot_gate();
    test_anchor_veto();
    test_bind_query_resolve_round_trip(false);
    test_bind_query_resolve_round_trip(true);
    test_identity_snapshot_fences();
    test_invalid_batches_are_atomic();
    if (g_failures == 0) std::fprintf(stderr, "element_identity_gate: OK\n");
    return g_failures;
}
