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

#include <BRepPrimAPI_MakeBox.hxx>
#include <TopoDS_Shape.hxx>

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
std::uint64_t publish_cube(Session& session) {
    session.open("doc_1", /*documentRevision=*/0, /*workerEpoch=*/3, "determinism");
    auto fence = session.fence_and_clone(/*jobId=*/1, /*documentRevision=*/0, /*workerEpoch=*/3,
                                         onecad::session::kEmptyPrefixHash);
    ScratchJob job;
    job.job_id = 1;
    job.bodies = fence.cloned_bodies;
    job.partition = fence.cloned_partition;
    job.prepared_snapshot_id = fence.prepared_snapshot_id;
    job.history_prefix_hash = std::string(64, 'a');
    job.bodies.create(kBody, "op_a", BRepPrimAPI_MakeBox(kBox, kBox, kBox).Shape());
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

}  // namespace

int main() {
    test_snapshot_gate();
    test_anchor_veto();
    if (g_failures == 0) std::fprintf(stderr, "element_identity_gate: OK\n");
    return g_failures;
}
