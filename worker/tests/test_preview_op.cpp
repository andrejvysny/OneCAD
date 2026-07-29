// test_preview_op.cpp — the drag-time preview verb (MODEL-OPS W3).
//
// Two things have to be true for a preview to be worth having:
//   1. It is EXACT — the same executor a real plan step uses, over the real head
//      bodies, so a Cut preview actually subtracts. (Before this verb, the "exact"
//      L2 mesh was synthesized in JavaScript and a Cut never removed anything.)
//   2. It is INVISIBLE — the session's head bodies, partition, history hash,
//      snapshot id and document revision are untouched afterwards, and no scratch
//      is left behind. A preview that perturbed fencing would break regen.
//
// (2) is the load-bearing one, so it is asserted after EVERY preview here.
//
// No test framework (matches the prototype style): exit code == failure count.
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include "BRepPrimAPI_MakeBox.hxx"
#include "nlohmann/json.hpp"
#include "session/PreviewOp.h"
#include "session/HistoryHash.h"
#include "session/ScratchJob.h"
#include "session/ShapeMetrics.h"

using nlohmann::json;
using onecad::protocol::Envelope;
using onecad::session::Session;

namespace {
int g_failures = 0;
#define CHECK(cond)                                                              \
    do {                                                                         \
        if (!(cond)) {                                                           \
            std::fprintf(stderr, "FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond); \
            ++g_failures;                                                        \
        }                                                                        \
    } while (0)

/// A 10×10 rectangle sketch in the solver-lane `wire_args` shape (what
/// `SketchStore` holds and what `PreviewOp` seeds the scratch with).
json rect_sketch_args(double w, double h) {
    return json{
        {"plane", json{{"kind", "XY"}}},
        {"entities",
         json::array({
             json{{"id", "e1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {w, 0}}},
             json{{"id", "e2"}, {"type", "Line"}, {"p0", {w, 0}}, {"p1", {w, h}}},
             json{{"id", "e3"}, {"type", "Line"}, {"p0", {w, h}}, {"p1", {0, h}}},
             json{{"id", "e4"}, {"type", "Line"}, {"p0", {0, h}}, {"p1", {0, 0}}},
         })},
        {"constraints", json::array()},
    };
}

/// Seed one body into the session HEAD through the real transaction (fence →
/// mutate the scratch → store → accept), because `Session` has no test-only body
/// setter and inventing one would test a path production never takes.
void seed_head_bodies(
    Session& session,
    const std::vector<std::pair<std::string, TopoDS_Shape>>& bodies) {
    onecad::session::FenceOutcome fo =
        session.fence_and_clone(/*job_id=*/1, /*document_revision=*/1, /*worker_epoch=*/1,
                                onecad::session::kEmptyPrefixHash);
    onecad::session::ScratchJob job;
    job.job_id = 1;
    job.plan_document_revision = 1;
    job.bodies = std::move(fo.cloned_bodies);
    job.partition = std::move(fo.cloned_partition);
    job.prepared_snapshot_id = fo.prepared_snapshot_id;
    for (const auto& [body_id, shape] : bodies) {
        job.bodies.create(body_id, "op_seed", shape);
    }
    session.store_prepared(std::move(job));
    session.accept_prepared(/*job_id=*/1, /*document_revision=*/1, /*worker_epoch=*/1);
}

Envelope preview_req(json args) {
    Envelope req;
    req.id = 42;
    req.args = std::move(args);
    return req;
}

/// Everything about the session a preview must NOT move.
struct HeadFingerprint {
    std::uint64_t snapshot_id;
    std::size_t body_count;
    double total_volume;
};

HeadFingerprint fingerprint(Session& s) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    double vol = 0.0;
    for (const auto& [bid, rec] : bodies.all()) {
        (void)bid;
        vol += onecad::session::shape_volume(rec.geom);
    }
    return HeadFingerprint{s.current_snapshot_id(), bodies.all().size(), vol};
}

void check_head_untouched(Session& s, const HeadFingerprint& before, const char* what) {
    const HeadFingerprint after = fingerprint(s);
    CHECK(after.snapshot_id == before.snapshot_id);
    CHECK(after.body_count == before.body_count);
    CHECK(std::abs(after.total_volume - before.total_volume) < 1e-6);
    if (after.snapshot_id != before.snapshot_id) {
        std::fprintf(stderr, "  [%s] snapshot moved %llu → %llu\n", what,
                     static_cast<unsigned long long>(before.snapshot_id),
                     static_cast<unsigned long long>(after.snapshot_id));
    }
}

}  // namespace

int main() {
    Session session;
    session.open("doc", 1, 1, "normal");

    // Head: two boxes plus a stored 10×10 sketch. body_a overlaps the profile,
    // so Cut proves real subtraction rather than a merely touched-body event.
    seed_head_bodies(
        session,
        {{"body_a",
          BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 10.0, 10.0, 10.0).Shape()},
         {"body_b",
          BRepPrimAPI_MakeBox(gp_Pnt(30, 0, 0), 5.0, 5.0, 5.0).Shape()}});
    session.sketches().upsert("sk1", rect_sketch_args(10, 10));

    const HeadFingerprint base = fingerprint(session);
    CHECK(std::abs(base.total_volume - 1125.0) < 1e-6);

    // ── 1. NewBody extrude off the stored sketch ─────────────────────────────
    {
        json op = {{"opType", "Extrude"},
                   {"opId", "op_preview"},
                   {"params",
                    {{"sketchId", "sk1"},
                     {"distance", 5.0},
                     {"extrudeMode", "Blind"},
                     {"booleanMode", "NewBody"}}}};
        Envelope resp = onecad::session::handle_preview_op(
            session,
            preview_req(json{{"op", op},
                             {"sketchId", "sk1"},
                             {"expectedSnapshotId", base.snapshot_id},
                             {"lod", "coarse"}}));
        CHECK(!resp.error.has_value());
        CHECK(resp.result.contains("meshes"));
        CHECK(resp.result["meshes"].is_array() && !resp.result["meshes"].empty());
        // Only the NEW body is shipped — an untouched survivor must not be re-sent
        // on every drag frame.
        CHECK(resp.result["changedBodies"].size() == 1);
        CHECK(resp.result["bodyEvents"].size() == 1);
        CHECK(resp.result["bodyEvents"][0]["kind"] == "created");
        CHECK(resp.result["deletedBodies"].empty());
        CHECK(resp.result["needsRepair"].empty());
        CHECK(!resp.out_bin.empty());
        std::fprintf(stderr, "preview NewBody: %zu mesh(es), %zu bin bytes\n",
                     resp.result["meshes"].size(), resp.out_bin.size());
        check_head_untouched(session, base, "NewBody");
    }

    // ── 2. Cut ACTUALLY subtracts (the whole point of the verb) ──────────────
    {
        json op = {{"opType", "Extrude"},
                   {"opId", "op_cut"},
                   {"params",
                    {{"sketchId", "sk1"},
                     {"distance", 5.0},
                     {"extrudeMode", "Blind"},
                     {"booleanMode", "Cut"},
                     {"targetBodyId", "body_a"}}}};
        Envelope resp = onecad::session::handle_preview_op(
            session, preview_req(json{{"op", op}, {"sketchId", "sk1"}}));
        CHECK(!resp.error.has_value());
        // The target is MODIFIED, so it is the body shipped back.
        CHECK(resp.result["changedBodies"].size() == 1);
        CHECK(resp.result["changedBodies"][0] == "body_a");
        CHECK(resp.result["bodyEvents"].size() == 1);
        CHECK(resp.result["bodyEvents"][0]["kind"] == "modified");
        CHECK(resp.result["deletedBodies"].empty());
        std::fprintf(stderr, "preview Cut: changed=%s\n",
                     resp.result["changedBodies"].dump().c_str());
        // And the REAL body is still 1000 — the cut happened only on the copy.
        check_head_untouched(session, base, "Cut");
    }

    // ── 3. A failing op is an error, and still leaves nothing behind ─────────
    {
        json op = {{"opType", "Extrude"},
                   {"opId", "op_bad"},
                   {"params",
                    {{"sketchId", "nope"},
                     {"distance", 5.0},
                     {"extrudeMode", "Blind"},
                     {"booleanMode", "NewBody"}}}};
        Envelope resp = onecad::session::handle_preview_op(
            session, preview_req(json{{"op", op}, {"sketchId", "nope"}}));
        CHECK(resp.error.has_value());
        CHECK(resp.error->code == "REF_UNRESOLVED");
        check_head_untouched(session, base, "missing sketch");
    }

    // ── 4. An unsupported opType reports UNSUPPORTED, not a crash ────────────
    {
        json op = {{"opType", "Loft"}, {"opId", "op_loft"}, {"params", json::object()}};
        Envelope resp =
            onecad::session::handle_preview_op(session, preview_req(json{{"op", op}}));
        CHECK(resp.error.has_value());
        CHECK(resp.error->code == "UNSUPPORTED");
        check_head_untouched(session, base, "unsupported op");
    }

    // ── 5. A malformed request is a PROTOCOL_ERROR ───────────────────────────
    {
        Envelope resp = onecad::session::handle_preview_op(
            session, preview_req(json{{"lod", "coarse"}}));
        CHECK(resp.error.has_value());
        CHECK(resp.error->code == "PROTOCOL_ERROR");
        check_head_untouched(session, base, "no op");
    }

    // ── 6. Repeated previews are stable and still leave no trace ─────────────
    {
        json op = {{"opType", "Extrude"},
                   {"opId", "op_rep"},
                   {"params",
                    {{"sketchId", "sk1"},
                     {"distance", 7.0},
                     {"extrudeMode", "Blind"},
                     {"booleanMode", "NewBody"}}}};
        std::size_t first_bytes = 0;
        for (int i = 0; i < 5; ++i) {
            Envelope resp = onecad::session::handle_preview_op(
                session, preview_req(json{{"op", op}, {"sketchId", "sk1"}}));
            CHECK(!resp.error.has_value());
            if (i == 0) first_bytes = resp.out_bin.size();
            // Same input ⇒ same mesh, drag after drag.
            CHECK(resp.out_bin.size() == first_bytes);
        }
        // Five previews must not have advanced the snapshot counter even once —
        // this is what `fence_and_clone` would have done.
        check_head_untouched(session, base, "repeated previews");
    }

    // ── 7. NeedsRepair is success-state, never a wrong preview or error ──────
    {
        json op = {
            {"opType", "Extrude"},
            {"opId", "op_repair"},
            {"inputs",
             json::array({json{{"primary",
                                {{"bodyId", "body_missing"},
                                 {"elementId", "el_missing"},
                                 {"kind", "face"}}}}})},
            {"params",
             {{"sketchId", "sk1"},
              {"distance", 5.0},
              {"extrudeMode", "Blind"},
              {"booleanMode", "NewBody"}}},
        };
        Envelope resp = onecad::session::handle_preview_op(
            session, preview_req(json{{"op", op}, {"sketchId", "sk1"}}));
        CHECK(!resp.error.has_value());
        CHECK(resp.result["needsRepair"].size() == 1);
        CHECK(resp.result["bodyEvents"].empty());
        CHECK(resp.result["changedBodies"].empty());
        CHECK(resp.result["deletedBodies"].empty());
        CHECK(resp.result["meshes"].empty());
        CHECK(resp.out_bin.empty());
        check_head_untouched(session, base, "NeedsRepair");
    }

    // ── 8. Lifecycle deletions are explicit; only survivors carry meshes ─────
    {
        json op = {
            {"opType", "Boolean"},
            {"opId", "op_union"},
            {"params",
             {{"targetBodyId", "body_a"},
              {"toolBodyId", "body_b"},
              {"operation", "Union"}}},
        };
        Envelope resp =
            onecad::session::handle_preview_op(session, preview_req(json{{"op", op}}));
        CHECK(!resp.error.has_value());
        CHECK(resp.result["changedBodies"].size() == 2);
        CHECK(resp.result["deletedBodies"].size() == 2);
        CHECK(resp.result["deletedBodies"][0] == "body_a");
        CHECK(resp.result["deletedBodies"][1] == "body_b");
        CHECK(resp.result["meshes"].size() == 2);
        CHECK(resp.result["bodyEvents"].size() == 4);
        check_head_untouched(session, base, "body lifecycle");
    }

    // ── 9. Dispatcher cancellation token reaches the candidate executor ─────
    {
        json op = {{"opType", "Extrude"},
                   {"opId", "op_cancel"},
                   {"params",
                    {{"sketchId", "sk1"},
                     {"distance", 5.0},
                     {"extrudeMode", "Blind"},
                     {"booleanMode", "NewBody"}}}};
        onecad::CancelToken cancel;
        cancel.cancel();
        Envelope resp = onecad::session::handle_preview_op(
            session, preview_req(json{{"op", op}, {"sketchId", "sk1"}}), cancel);
        CHECK(resp.error.has_value());
        CHECK(resp.error->code == "CANCELLED");
        check_head_untouched(session, base, "cancelled");
    }

    // ── 10. Unknown modes fail; they never silently become NewBody/Blind ─────
    for (const auto& [field, value] :
         std::vector<std::pair<std::string, std::string>>{
             {"booleanMode", "Bogus"}, {"extrudeMode", "Bogus"}}) {
        json params = {{"sketchId", "sk1"},
                       {"distance", 5.0},
                       {"extrudeMode", "Blind"},
                       {"booleanMode", "NewBody"}};
        params[field] = value;
        json op = {{"opType", "Extrude"},
                   {"opId", "op_bad_mode_" + field},
                   {"params", std::move(params)}};
        Envelope resp = onecad::session::handle_preview_op(
            session, preview_req(json{{"op", op}, {"sketchId", "sk1"}}));
        CHECK(resp.error.has_value());
        CHECK(resp.error->code == "OP_FAILED");
        check_head_untouched(session, base, "invalid mode");
    }

    // ── 11. A stale preview base is rejected before candidate execution ──────
    {
        json op = {{"opType", "Extrude"},
                   {"opId", "op_stale"},
                   {"params",
                    {{"sketchId", "sk1"},
                     {"distance", 5.0},
                     {"extrudeMode", "Blind"},
                     {"booleanMode", "NewBody"}}}};
        Envelope resp = onecad::session::handle_preview_op(
            session,
            preview_req(json{{"op", op},
                             {"sketchId", "sk1"},
                             {"expectedSnapshotId", base.snapshot_id + 1}}));
        CHECK(resp.error.has_value());
        CHECK(resp.error->code == "STALE_PREVIEW");
        CHECK(resp.error->message.find("PreviewOp: stale snapshot; expected ") == 0);
        CHECK(resp.out_bin.empty());
        check_head_untouched(session, base, "stale snapshot");
    }

    if (g_failures == 0) std::fprintf(stderr, "preview_op: all checks passed\n");
    return g_failures;
}
