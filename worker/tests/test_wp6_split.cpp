// test_wp6_split.cpp — M5a boolean split children (`body_<opId>:<k>`, SCHEMA §2/D1).
// A Cut that BISECTS a box into two disconnected pieces must mint two deterministic
// children `body_<boolOpId>:0` / `:1` (ordered by the worker's geometric key), emit a
// Created bodyEvent for each + a Deleted for the parent, and expose exact volumes.
//
// Geometry: box A = [0,40]×[0,20]×[0,25] (vol 20000). Tool = a slab x∈[15,25]
// overshooting A in y and z. Cut(A, tool) removes x∈[15,25] all the way through,
// leaving left x∈[0,15] (vol 7500, centroid x=7.5) and right x∈[25,40] (vol 7500,
// centroid x=32.5). Equal volumes ⇒ the centroid tiebreak fixes the order: child :0
// is the LEFT piece (lower centroid x). Ids are stable across a replay (pure fn of
// the opId + k).
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "nlohmann/json.hpp"
#include "ops/OpCommon.h"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "session/ShapeMetrics.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::Session;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

json rect(const std::string& p, double x0, double y0, double x1, double y1) {
    return json::array({{{"id", p + "1"}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y0}}},
                        {{"id", p + "2"}, {"type", "Line"}, {"p0", {x1, y0}}, {"p1", {x1, y1}}},
                        {{"id", p + "3"}, {"type", "Line"}, {"p0", {x1, y1}}, {"p1", {x0, y1}}},
                        {{"id", p + "4"}, {"type", "Line"}, {"p0", {x0, y1}}, {"p1", {x0, y0}}}});
}
json sketch_op(const std::string& op_id, int step, const std::string& sid, const json& ents) {
    return json{{"opType", "Sketch"}, {"opId", op_id}, {"stepIndex", step},
                {"params", {{"sketchId", sid}, {"plane", {{"kind", "XY"}}},
                            {"entities", ents}, {"constraints", json::array()}}}};
}

// Run the split plan into `s`; `body_events` collects the boolean step's bodyEvents.
// (Session holds a mutex ⇒ non-movable, so it is passed by reference, never
// reassigned.)
void run_plan(Session& s, std::vector<json>& body_events) {
    s.open("doc", 0, 3, "determinism");
    json ops = json::array({
        sketch_op("op0", 0, "sk_a", rect("a", 0, 0, 40, 20)),
        json{{"opType", "Extrude"}, {"opId", "op1"}, {"stepIndex", 1},
             {"params", {{"sketchId", "sk_a"}, {"distance", 25.0}, {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}},
        sketch_op("op2", 2, "sk_t", rect("t", 15, -10, 25, 30)),
        json{{"opType", "Extrude"}, {"opId", "op3"}, {"stepIndex", 3},
             {"params", {{"sketchId", "sk_t"}, {"distance", 100.0}, {"extrudeMode", "Symmetric"}, {"booleanMode", "NewBody"}}}},
        json{{"opType", "Boolean"}, {"opId", "op4"}, {"stepIndex", 4},
             {"params", {{"operation", "Cut"}, {"targetBodyId", "body_op1"}, {"toolBodyId", "body_op3"}}}},
    });

    CancelToken tok;
    auto capture = [&body_events](Envelope& ev) {
        if (ev.event_name == "planStep" && ev.result.contains("bodyEvents")) {
            body_events.clear();
            for (const auto& be : ev.result["bodyEvents"]) body_events.push_back(be);
        }
    };
    HandlerContext ctx{tok, [](int) {}, capture};
    json args = {{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3},
                 {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({"a", "b", "c", "d", "e"})},
                 {"targetStep", 4}, {"ops", ops}};
    onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
}

double vol_of(Session& s, const std::string& bid) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* rec = bodies.get(bid);
    if (!rec) return -1.0;
    return onecad::session::shape_volume(rec->geom);
}

// The emitted `rankKey` for a body id, or an empty vector when the event carried
// none (SCHEMA §7.2: absence = "no claim").
std::vector<long long> rank_key_of(const std::vector<json>& events, const std::string& bid) {
    for (const json& be : events) {
        if (be.value("bodyId", "") != bid || !be.contains("rankKey")) continue;
        return be["rankKey"].get<std::vector<long long>>();
    }
    return {};
}

// Recompute the §7.2 rank key straight from OCCT for `bid` — the independent
// oracle the emitted key must match exactly (VF-B6).
std::vector<long long> recomputed_key(Session& s, const std::string& bid) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* rec = bodies.get(bid);
    if (!rec) return {};
    const std::vector<onecad::ops::RankedSolid> ranked = onecad::ops::ranked_solids(rec->geom);
    if (ranked.size() != 1) return {};
    return std::vector<long long>(ranked[0].key.begin(), ranked[0].key.end());
}
}  // namespace

int main() {
    Session s1;
    std::vector<json> body_events;
    run_plan(s1, body_events);

    // The boolean step's bodyEvents: parent deleted, two children created, tool deleted.
    int created = 0, deleted = 0;
    bool has_c0 = false, has_c1 = false, parent_deleted = false;
    for (const json& be : body_events) {
        const std::string kind = be.value("kind", "");
        const std::string bid = be.value("bodyId", "");
        if (kind == "created") ++created;
        if (kind == "deleted") ++deleted;
        if (kind == "created" && bid == "body_op4:0") has_c0 = true;
        if (kind == "created" && bid == "body_op4:1") has_c1 = true;
        if (kind == "deleted" && bid == "body_op1") parent_deleted = true;
    }
    check(created == 2, "split: two children Created");
    check(has_c0 && has_c1, "split: children are body_op4:0 + body_op4:1 (deterministic)");
    check(parent_deleted, "split: parent body_op1 Deleted");
    check(deleted == 2, "split: parent + tool both Deleted");

    // The parent + tool are gone; only the two children survive at head.
    const onecad::session::BodyStore bodies = s1.bodies_copy();
    check(!bodies.contains("body_op1"), "split: parent body removed from head");
    check(!bodies.contains("body_op3"), "split: tool body removed from head");
    check(bodies.contains("body_op4:0") && bodies.contains("body_op4:1"),
          "split: both children present at head");

    // Volumes are exact (7500 each) and ORDERED by centroid: :0 is the left piece.
    const double v0 = vol_of(s1, "body_op4:0");
    const double v1 = vol_of(s1, "body_op4:1");
    check(std::abs(v0 - 7500.0) < 1e-3, "split: child :0 volume == 7500 (exact box arithmetic)");
    check(std::abs(v1 - 7500.0) < 1e-3, "split: child :1 volume == 7500 (exact box arithmetic)");

    // --- VF-B6 rankKey: the ordinal's geometric evidence rides the bodyEvent ---
    // Each child carries the key it was RANKED by: (volume, cx, cy, cz, faceCount)
    // at 1e-6. Equal volumes (7500 each) ⇒ the centroid decides, so :0 must be the
    // LEFT piece (cx = 7.5) and :1 the RIGHT (cx = 32.5) — the same fact the ordinal
    // assertions above pin, now published as machine-readable evidence.
    const std::vector<long long> k0 = rank_key_of(body_events, "body_op4:0");
    const std::vector<long long> k1 = rank_key_of(body_events, "body_op4:1");
    check(k0.size() == 5 && k1.size() == 5, "rankKey: both children carry a 5-tuple key");
    if (k0.size() == 5 && k1.size() == 5) {
        check(k0[0] == 7500000000LL && k1[0] == 7500000000LL,
              "rankKey: quantized volume == llround(7500 * 1e6) for both children");
        // Equal volumes ⇒ the centroid decides. The sketch's u axis (the axis the cut
        // runs across) lands on world Y for an XY-plane sketch, so index 2 is the
        // component that separates the pieces: 7.5 for the near piece, 32.5 for the
        // far one. Index 1 (world X) is shared — the tie the volume left open is
        // broken by the FIRST differing centroid component, exactly as the
        // lexicographic key promises.
        check(k0[1] == k1[1], "rankKey: the shared centroid component ties");
        check(k0[2] == 7500000LL, "rankKey: child :0 split-axis centroid == 7.5 (near piece)");
        check(k1[2] == 32500000LL, "rankKey: child :1 split-axis centroid == 32.5 (far piece)");
        check(k0 < k1, "rankKey: the emitted keys are in ascending ordinal order");
        check(k0[4] == 6 && k1[4] == 6, "rankKey: face count == 6 per box piece");
        // The emitted key IS the GProp recomputation — no drift between the sort key
        // and the published evidence.
        check(k0 == recomputed_key(s1, "body_op4:0"),
              "rankKey: child :0 key matches a fresh GProp recomputation");
        check(k1 == recomputed_key(s1, "body_op4:1"),
              "rankKey: child :1 key matches a fresh GProp recomputation");
    }

    // Ids stable across replay (pure function of opId + ordinal).
    Session s2;
    std::vector<json> body_events2;
    run_plan(s2, body_events2);
    const onecad::session::BodyStore bodies2 = s2.bodies_copy();
    check(bodies2.contains("body_op4:0") && bodies2.contains("body_op4:1"),
          "split: child ids identical across a replay");
    check(std::abs(vol_of(s2, "body_op4:0") - 7500.0) < 1e-3, "split: replay child :0 volume stable");

    // Byte-determinism of the evidence across two independent sessions.
    check(rank_key_of(body_events2, "body_op4:0") == k0 &&
              rank_key_of(body_events2, "body_op4:1") == k1,
          "rankKey: byte-identical across a fresh replay (Invariant 5)");

    if (g_failures == 0) std::fprintf(stderr, "wp6_split: OK\n");
    return g_failures;
}
