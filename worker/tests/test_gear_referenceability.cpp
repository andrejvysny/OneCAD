// test_gear_referenceability.cpp — WP-I I0(c) probe, SCHEMA §7.3 gear
// referenceability through the REAL mint path.
//
// `GearOp.h` already refuses a tooth pick at `AcquireElementIds`. But
// `AcquireElementIds` mints NOTHING (`ElementIdentity.cpp:136-163` returns an
// empty `elementId` — Rust mints the id), so that refusal is advisory. The mint
// that actually installs a persistent, op-referenceable binding is
// `Session::bind_element_ids` → `stage_binding` (`Session.cpp:56-81`), and that
// path has no gear check at all: a caller that skips the advisory verb and binds
// a tooth-flank `TopoKey` directly gets a durable id for geometry whose identity
// changes with the tooth count — the silent-wrong-bind the ladder exists to
// prevent.
//
// This file drives a REAL gear body into a published `Session` head (the gear op
// itself, not a stand-in solid), then binds a flank / a bore / a cap and asks
// `QueryElement` what stuck.
//
//   * FLANK  → expected REFUSED at bind, or `present:false` after it. RED today.
//   * BORE   → bound and present. Positive control, green today and after.
//   * CAP    → bound and present. Positive control (§7.3 allow-list amendment).
//
// The same rule reaches EDGES and VERTICES (§7.3): one is referenceable iff
// EVERY face adjacent to it is. So the bore rim circle and the vertices on it
// stay referenceable, while a tooth-profile edge — including the cap's own outer
// boundary, which is shared with the flanks — and every vertex out at the teeth
// are refused with the same `reasonCode`.
//
// No framework: exit code == failure count. Mirrors
// `test_element_identity_gate.cpp`'s Session-driving shape verbatim.
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRep_Tool.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/GearOp.h"
#include "ops/OpTypes.h"
#include "protocol/Envelope.h"
#include "session/BodyStore.h"
#include "session/ClassifyElement.h"
#include "session/ElementIdentity.h"
#include "session/HistoryHash.h"
#include "session/PlanExecutor.h"
#include "session/ScratchJob.h"
#include "session/Session.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::protocol::Envelope;
using onecad::session::ScratchJob;
using onecad::session::Session;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;

namespace {

constexpr const char* kGearBody = "body_opgear";

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(onecad::session::BodyStore& bodies, em::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

// m=2, z=20 involute spur gear, 5 mm thick, on an explicit +Z frame at the
// origin, with a Ø10 axle bore. Reference diameter 40, root diameter 35 (root
// radius 17.5), tip diameter 44 — so the bore at r=5 and the caps at z=0/5 are
// the three face families §7.3's allow-list keeps, and everything else is tooth.
json gear_op() {
    const json block = {{"teeth", 20},
                        {"module", 2.0},
                        {"height", 5.0},
                        {"pressureAngleDeg", 20.0},
                        {"shift", 0.0},
                        {"helixAngleDeg", 0.0},
                        {"doubleHelix", false},
                        {"propertiesFromTool", false},
                        {"undercut", false},
                        {"backlash", 0.0},
                        {"clearance", 0.25},
                        {"head", 0.0},
                        {"sampleCount", 12},
                        {"axleHole", true},
                        {"axleHoleDiameter", 10.0}};
    return json{{"opType", "Gear"},
                {"opId", "opgear"},
                {"inputs", json::array()},
                {"params",
                 {{"recipe", "involuteExternal"},
                  {"placement",
                   {{"face", nullptr},
                    {"frame",
                     {{"origin", {0.0, 0.0, 0.0}},
                      {"axis", {0.0, 0.0, 1.0}},
                      {"xDir", {1.0, 0.0, 0.0}}}},
                    {"point", {0.0, 0.0, 0.0}}}},
                  {"involuteExternal", block}}}};
}

// Drive a Session to a published head holding ONE gear body, built by the real
// `execute_gear` against the job's own store, so any per-body metadata the op
// records at publish rides along exactly as it would in production.
std::uint64_t publish_gear(Session& session, TopoDS_Shape& gear_out) {
    session.open("doc_1", /*documentRevision=*/0, /*workerEpoch=*/3, "determinism");
    auto fence = session.fence_and_clone(/*jobId=*/1, /*documentRevision=*/0, /*workerEpoch=*/3,
                                         onecad::session::kEmptyPrefixHash);
    ScratchJob job;
    job.job_id = 1;
    job.bodies = fence.cloned_bodies;
    job.partition = fence.cloned_partition;
    job.prepared_snapshot_id = fence.prepared_snapshot_id;
    job.history_prefix_hash = std::string(64, 'a');
    // SCHEMA §7.3's gear classifier is PLAN-DERIVED (a body is a gear body when
    // `body_<opId>` names a `Gear` op of the plan — D1), so the session rebuilds
    // its gear map at `AcceptPrepared` from `ScratchJob::plan`. `handle_execute_
    // _plan` sets it from the ExecutePlan args; this fixture drives the executor
    // directly, so it sets the same field with the same one-op plan.
    job.plan = json{{"ops", json::array({gear_op()})}};

    Ctx c;
    ops::OpContext ctx = c.make(job.bodies, job.partition);
    const ops::OpOutcome built = ops::execute_gear(ctx, gear_op(), "opgear");
    check(built.error_code.empty(), "setup: the gear op built (" + built.error_message + ")");
    const onecad::session::BodyRecord* rec = job.bodies.get(kGearBody);
    check(rec != nullptr, "setup: the gear published body_opgear");
    if (rec != nullptr) gear_out = rec->geom;

    session.store_prepared(std::move(job));
    const auto accepted = session.accept_prepared(/*jobId=*/1, /*documentRevision=*/0,
                                                  /*workerEpoch=*/3);
    check(accepted.ok, "setup: AcceptPrepared published the gear");
    return accepted.snapshot_id;
}

Envelope bind_element(Session& session, std::uint64_t snapshot, const std::string& topo_key,
                      const std::string& element_id, const char* kind) {
    return onecad::session::handle_bind_element_ids(
        session, Envelope::request(8, "BindElementIds",
                                   json{{"snapshotId", snapshot},
                                        {"bindings", json::array({json{{"bodyId", kGearBody},
                                                                       {"topoKey", topo_key},
                                                                       {"elementId", element_id},
                                                                       {"kind", kind}}})}}));
}

Envelope bind_face(Session& session, std::uint64_t snapshot, const std::string& topo_key,
              const std::string& element_id) {
    return bind_element(session, snapshot, topo_key, element_id, "face");
}

Envelope query(Session& session, std::uint64_t snapshot, const std::string& element_id) {
    return onecad::session::handle_query_element(
        session, Envelope::request(9, "QueryElement",
                                   json{{"snapshotId", snapshot}, {"elementId", element_id}}));
}

// The three face families, located by CLASSIFICATION rather than by ordinal —
// the gear's face order is an OCCT implementation detail this probe must not pin.
struct GearFaces {
    std::string flank;  // a tooth face: neither planar nor cylindrical
    std::string bore;   // the axle bore: cylindrical, coaxial, radius < root radius
    std::string cap;    // an end cap: planar, normal along the axis
};

GearFaces classify_gear_faces(const TopoDS_Shape& gear, double root_radius) {
    GearFaces out;
    NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> faces;
    TopExp::MapShapes(gear, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        const std::string key = "f:" + std::to_string(i);
        const json c = onecad::session::classify_shape(faces(i));
        const std::string type = c.value("surfaceType", "");
        if (type == "plane") {
            if (!out.cap.empty()) continue;
            if (!c.contains("frame")) continue;
            const json& n = c["frame"]["normal"];
            if (std::abs(n[2].get<double>()) > 0.999) out.cap = key;
        } else if (type == "cylinder") {
            if (!out.bore.empty()) continue;
            if (!c.contains("frame")) continue;
            const json& f = c["frame"];
            const double radius = f.value("radius", 0.0);
            const double ox = f["origin"][0].get<double>();
            const double oy = f["origin"][1].get<double>();
            if (std::abs(f["axis"][2].get<double>()) > 0.999 &&
                std::sqrt(ox * ox + oy * oy) < 1e-6 && radius < root_radius) {
                out.bore = key;
            }
        } else if (out.flank.empty()) {
            out.flank = key;
        }
    }
    return out;
}

// The edge/vertex families, again located by GEOMETRY rather than by the
// production predicate — a probe that asked the rule what the rule says would
// prove nothing. The bore rim is the circle at the bore radius; a tooth edge is
// any curve the classifier calls neither a line nor a circle (a cap's involute
// boundary); a vertex is sorted by its distance from the gear axis.
struct GearSubElements {
    std::string bore_rim_edge;    // circle at r = bore radius: bore cylinder + a cap
    std::string tooth_edge;       // an involute curve on a cap's outer boundary
    std::string bore_rim_vertex;  // a vertex at r = bore radius
    std::string tooth_vertex;     // a vertex out beyond the root circle
};

GearSubElements classify_gear_sub_elements(const TopoDS_Shape& gear, double bore_radius,
                                           double root_radius) {
    GearSubElements out;
    NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> edges;
    TopExp::MapShapes(gear, TopAbs_EDGE, edges);
    for (int i = 1; i <= edges.Extent(); ++i) {
        const std::string key = "e:" + std::to_string(i);
        const json c = onecad::session::classify_shape(edges(i));
        const std::string type = c.value("curveType", "");
        if (type == "circle" && out.bore_rim_edge.empty() && c.contains("frame")) {
            if (std::abs(c["frame"].value("radius", 0.0) - bore_radius) < 1e-6) {
                out.bore_rim_edge = key;
            }
        } else if (type == "other" && out.tooth_edge.empty()) {
            out.tooth_edge = key;
        }
    }
    NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> vertices;
    TopExp::MapShapes(gear, TopAbs_VERTEX, vertices);
    for (int i = 1; i <= vertices.Extent(); ++i) {
        const std::string key = "v:" + std::to_string(i);
        const gp_Pnt p = BRep_Tool::Pnt(TopoDS::Vertex(vertices(i)));
        const double r = std::sqrt(p.X() * p.X() + p.Y() * p.Y());
        if (out.bore_rim_vertex.empty() && std::abs(r - bore_radius) < 1e-6) {
            out.bore_rim_vertex = key;
        }
        if (out.tooth_vertex.empty() && r > root_radius) out.tooth_vertex = key;
    }
    return out;
}

// ── I0(c): the REAL mint refuses a tooth face. RED today. ────────────────────
void test_flank_is_refused_by_the_real_mint() {
    Session session;
    TopoDS_Shape gear;
    const std::uint64_t head = publish_gear(session, gear);
    if (gear.IsNull()) return;
    const GearFaces f = classify_gear_faces(gear, 17.5);
    std::fprintf(stderr, "  [I0-c] gear faces: flank=%s bore=%s cap=%s\n",
                 f.flank.empty() ? "(none)" : f.flank.c_str(),
                 f.bore.empty() ? "(none)" : f.bore.c_str(),
                 f.cap.empty() ? "(none)" : f.cap.c_str());
    check(!f.flank.empty(), "I0(c) fixture: the gear has a non-planar, non-cylindrical tooth face");
    if (f.flank.empty()) return;

    const Envelope installed = bind_face(session, head, f.flank, "el_flank");
    const bool bind_ok = installed.ok.has_value() && *installed.ok;
    const Envelope found = query(session, head, "el_flank");
    const bool present = found.ok.has_value() && *found.ok && found.result.value("present", false);
    std::fprintf(stderr, "  [I0-c] BindElementIds(flank %s) ok=%s → QueryElement present=%s\n",
                 f.flank.c_str(), bind_ok ? "true" : "false", present ? "true" : "false");
    check(!bind_ok || !present,
          "I0(c): a tooth flank must NOT be mintable — BindElementIds refuses it, or the id is "
          "not present afterwards (a tooth's identity changes with the tooth count)");
}

// ── I0(c) positive controls: the bore and a cap ARE referenceable. GREEN. ────
void test_bore_and_cap_are_referenceable() {
    Session session;
    TopoDS_Shape gear;
    const std::uint64_t head = publish_gear(session, gear);
    if (gear.IsNull()) return;
    const GearFaces f = classify_gear_faces(gear, 17.5);
    check(!f.bore.empty(), "I0(c) fixture: the gear has an axle bore face");
    check(!f.cap.empty(), "I0(c) fixture: the gear has an end-cap face");
    if (f.bore.empty() || f.cap.empty()) return;

    const Envelope bore = bind_face(session, head, f.bore, "el_bore");
    check(bore.ok.has_value() && *bore.ok, "I0(c) control: the axle bore binds");
    check(query(session, head, "el_bore").result.value("present", false),
          "I0(c) control: the axle bore is present after the bind");

    const Envelope cap = bind_face(session, head, f.cap, "el_cap");
    check(cap.ok.has_value() && *cap.ok, "I0(c) control: an end cap binds");
    check(query(session, head, "el_cap").result.value("present", false),
          "I0(c) control: the end cap is present after the bind");
}

// ── §7.3 edges and vertices: the NEIGHBOURHOOD rule. ────────────────────────
// A tooth-profile edge and a vertex out at the teeth are refused for the same
// reason their faces are; the bore rim and the vertices on it survive, because
// every face around them (the bore cylinder, a cap) does.
void test_edges_and_vertices_follow_their_neighbourhood() {
    Session session;
    TopoDS_Shape gear;
    const std::uint64_t head = publish_gear(session, gear);
    if (gear.IsNull()) return;
    const GearSubElements sub = classify_gear_sub_elements(gear, /*bore_radius=*/5.0,
                                                           /*root_radius=*/17.5);
    std::fprintf(stderr, "  [I0-c] gear sub-elements: boreRim=%s toothEdge=%s boreVertex=%s "
                         "toothVertex=%s\n",
                 sub.bore_rim_edge.empty() ? "(none)" : sub.bore_rim_edge.c_str(),
                 sub.tooth_edge.empty() ? "(none)" : sub.tooth_edge.c_str(),
                 sub.bore_rim_vertex.empty() ? "(none)" : sub.bore_rim_vertex.c_str(),
                 sub.tooth_vertex.empty() ? "(none)" : sub.tooth_vertex.c_str());
    check(!sub.bore_rim_edge.empty(), "I0(c) fixture: the gear has a bore rim circle");
    check(!sub.tooth_edge.empty(), "I0(c) fixture: the gear has an involute profile edge");
    check(!sub.bore_rim_vertex.empty(), "I0(c) fixture: the gear has a vertex on the bore rim");
    check(!sub.tooth_vertex.empty(), "I0(c) fixture: the gear has a vertex out at the teeth");
    if (sub.bore_rim_edge.empty() || sub.tooth_edge.empty() || sub.bore_rim_vertex.empty() ||
        sub.tooth_vertex.empty()) {
        return;
    }

    // REFUSED: a tooth-profile edge — its adjacent faces include a flank.
    const Envelope tooth_edge =
        bind_element(session, head, sub.tooth_edge, "el_tooth_edge", "edge");
    check(tooth_edge.ok.has_value() && !*tooth_edge.ok,
          "I0(c): a tooth-profile edge is REFUSED at bind");
    check(!query(session, head, "el_tooth_edge").result.value("present", false),
          "I0(c): the refused tooth edge minted nothing");

    // REFUSED: a vertex out at the teeth.
    const Envelope tooth_vertex =
        bind_element(session, head, sub.tooth_vertex, "el_tooth_vertex", "vertex");
    check(tooth_vertex.ok.has_value() && !*tooth_vertex.ok,
          "I0(c): a vertex on the tooth profile is REFUSED at bind");
    check(!query(session, head, "el_tooth_vertex").result.value("present", false),
          "I0(c): the refused tooth vertex minted nothing");

    // BOUND: the bore rim circle — bore cylinder + cap, both referenceable.
    const Envelope rim = bind_element(session, head, sub.bore_rim_edge, "el_bore_rim", "edge");
    check(rim.ok.has_value() && *rim.ok, "I0(c) control: the bore rim circle binds");
    check(query(session, head, "el_bore_rim").result.value("present", false),
          "I0(c) control: the bore rim circle is present after the bind");

    // BOUND: a vertex on that rim.
    const Envelope rim_vertex =
        bind_element(session, head, sub.bore_rim_vertex, "el_bore_vertex", "vertex");
    check(rim_vertex.ok.has_value() && *rim_vertex.ok,
          "I0(c) control: a vertex on the bore rim binds");
    check(query(session, head, "el_bore_vertex").result.value("present", false),
          "I0(c) control: the bore-rim vertex is present after the bind");
}

// ── The PLAN-STEP rung. Everything above goes through `BindElementIds`; these
// three drive `execute_candidate_op`, which is where `CandidateFilter`,
// `gear_info_for` and the tracked-entry halt actually live. ─────────────────

// A ScratchJob holding the published gear, shaped exactly as `handle_execute_plan`
// builds one — `plan` included, since the §7.3 classifier is plan-derived.
onecad::session::ScratchJob gear_job(TopoDS_Shape& gear_out) {
    onecad::session::ScratchJob job;
    job.job_id = 1;
    job.plan = json{{"ops", json::array({gear_op()})}};
    Ctx c;
    ops::OpContext ctx = c.make(job.bodies, job.partition);
    const ops::OpOutcome built = ops::execute_gear(ctx, gear_op(), "opgear");
    check(built.error_code.empty(), "plan-step setup: the gear op built");
    if (const onecad::session::BodyRecord* rec = job.bodies.get(kGearBody)) gear_out = rec->geom;
    return job;
}

// One op input carrying the FROZEN evidence a stored ref really has: the typed
// descriptor plus the world anchor, both measured off the live sub-shape — the
// shape `ladder_ref_from_input` parses.
json stored_ref(const TopoDS_Shape& gear, const TopoDS_Shape& sub, const char* kind,
                const std::string& element_id) {
    const auto descriptor = em::ElementMapPartition::describe(sub, gear);
    const gp_Pnt c = descriptor.center;
    return json{{"primary", {{"bodyId", kGearBody}, {"elementId", element_id}, {"kind", kind}}},
                {"intent",
                 {{"kind", kind},
                  {"descriptor", em::ElementMapPartition::descriptor_to_json(descriptor)}}},
                {"anchor", {{"worldPoint", {c.X(), c.Y(), c.Z()}}}}};
}

json fillet_op(const std::string& op_id, const json& input, const std::string& element_id) {
    return json{{"opType", "Fillet"},
                {"opId", op_id},
                {"stepIndex", 1},
                {"inputs", json::array({input})},
                {"params",
                 {{"mode", "Fillet"},
                  {"radius", 0.2},
                  {"edgeIds", json::array({element_id})},
                  {"chainTangentEdges", false},
                  {"targetBodyId", kGearBody}}}};
}

bool has_gear_diagnostic(const onecad::session::CandidateResult& result) {
    for (const json& d : result.diagnostics) {
        if (d.value("reasonCode", std::string()) == "GEAR_FACE_NOT_REFERENCEABLE") return true;
    }
    return false;
}

// ── The DESCRIPTOR rung: a stored ref naming a tooth edge cannot bind, because
// the tooth is not in the pool the ladder scores. The bore rim is the control. ─
void test_plan_step_refuses_a_stored_tooth_ref() {
    TopoDS_Shape gear;
    onecad::session::ScratchJob job = gear_job(gear);
    if (gear.IsNull()) return;
    const GearSubElements sub = classify_gear_sub_elements(gear, 5.0, 17.5);
    if (sub.tooth_edge.empty() || sub.bore_rim_edge.empty()) return;

    onecad::CancelToken cancel;
    std::string last_sketch;
    const TopoDS_Shape tooth = em::ElementMapPartition::shape_for_topokey(gear, sub.tooth_edge);
    const onecad::session::CandidateResult refused = onecad::session::execute_candidate_op(
        job, fillet_op("opfil", stored_ref(gear, tooth, "edge", "el_stored_tooth"),
                       "el_stored_tooth"),
        "opfil", last_sketch, cancel);
    std::fprintf(stderr,
                 "  [I0-c] plan step on a stored tooth edge: needsRepair=%zu diagnostic=%s "
                 "tracked=%s\n",
                 refused.needs_repair.size(), has_gear_diagnostic(refused) ? "yes" : "no",
                 job.partition.contains("el_stored_tooth") ? "yes" : "no");
    check(refused.status == onecad::session::CandidateResult::Status::NeedsRepair,
          "plan step: a stored ref on a tooth edge halts needsRepair");
    check(!refused.needs_repair.empty() &&
              refused.needs_repair[0].value("ladderFailed", std::string()) == "descriptor",
          "plan step: the halt is a LADDER outcome, not a synthesized one");
    check(has_gear_diagnostic(refused),
          "plan step: the GEAR_FACE_NOT_REFERENCEABLE diagnostic rides the step");
    check(!job.partition.contains("el_stored_tooth"),
          "plan step: NOTHING was bound for the refused ref");
    // Every candidate the ladder offered must itself be referenceable — the pool
    // was narrowed, so a tooth face can never be proposed as the repair target.
    const ops::GearBodyInfo info = *ops::gear_body_info(job.plan, kGearBody, gear);
    bool tooth_offered = false;
    for (const json& c : refused.needs_repair[0].value("candidates", json::array())) {
        const TopoDS_Shape s =
            em::ElementMapPartition::shape_for_topokey(gear, c.value("topoKey", std::string()));
        if (!ops::gear_element_referenceable(info, gear, s)) tooth_offered = true;
    }
    check(!tooth_offered, "plan step: no refused sub-element is offered as a candidate");

    // CONTROL: the same op on the bore rim resolves and the op runs.
    TopoDS_Shape gear2;
    onecad::session::ScratchJob job2 = gear_job(gear2);
    if (gear2.IsNull()) return;
    const TopoDS_Shape rim = em::ElementMapPartition::shape_for_topokey(gear2, sub.bore_rim_edge);
    std::string last_sketch2;
    const onecad::session::CandidateResult allowed = onecad::session::execute_candidate_op(
        job2, fillet_op("opfil2", stored_ref(gear2, rim, "edge", "el_stored_rim"), "el_stored_rim"),
        "opfil2", last_sketch2, cancel);
    // The bind is asserted on the DELTA, not on the surviving partition: a Fillet
    // CONSUMES the edge it rounds, so the entry the resolution minted is removed
    // again by the op's own history in the same step (SCHEMA §10).
    bool rim_bound = false;
    for (const auto& added : allowed.delta.added) {
        if (added.element_id == "el_stored_rim") rim_bound = true;
    }
    std::fprintf(stderr,
                 "  [I0-c] plan step on the bore rim: status=%d needsRepair=%zu bound=%s "
                 "bodyEvents=%zu\n",
                 static_cast<int>(allowed.status), allowed.needs_repair.size(),
                 rim_bound ? "yes" : "no", allowed.body_events.size());
    check(rim_bound, "plan step control: the bore-rim ref RESOLVES and is minted");
    check(allowed.status == onecad::session::CandidateResult::Status::Ok,
          "plan step control: the op then runs");
    check(!allowed.body_events.empty(), "plan step control: the op published its result");
    check(!has_gear_diagnostic(allowed),
          "plan step control: no gear diagnostic on a legitimate ref");
}

// ── The TRACKED rung: an id already bound to a tooth face (a document written
// before this build) halts with an EMPTY candidate list and its partition entry
// UNTOUCHED. Re-running the narrowed ladder for it would be free to repoint the
// stored id at a cap or a bore — a wrong bind reached from the other side. ────
void test_tracked_tooth_id_halts_without_repointing() {
    TopoDS_Shape gear;
    onecad::session::ScratchJob job = gear_job(gear);
    if (gear.IsNull()) return;
    const GearFaces f = classify_gear_faces(gear, 17.5);
    if (f.flank.empty()) return;
    const TopoDS_Shape flank = em::ElementMapPartition::shape_for_topokey(gear, f.flank);

    // The legacy binding: minted directly, the way a pre-WP-I Bind installed one.
    job.partition.mint(kGearBody, "el_legacy_tooth", em::km::ElementKind::Face, flank, gear,
                       json::object());
    const em::PartitionEntry* before = job.partition.find("el_legacy_tooth");
    check(before != nullptr, "tracked rung fixture: the legacy tooth binding exists");
    if (before == nullptr) return;
    const std::string key_before = before->topo_key;

    onecad::CancelToken cancel;
    std::string last_sketch;
    const json op = json{{"opType", "OffsetFace"},
                         {"opId", "opoff"},
                         {"stepIndex", 1},
                         {"inputs", json::array({stored_ref(gear, flank, "face",
                                                            "el_legacy_tooth")})},
                         {"params",
                          {{"faceIds", json::array({"el_legacy_tooth"})},
                           {"distance", 0.5},
                           {"targetBodyId", kGearBody}}}};
    const onecad::session::CandidateResult halted =
        onecad::session::execute_candidate_op(job, op, "opoff", last_sketch, cancel);
    const em::PartitionEntry* after = job.partition.find("el_legacy_tooth");
    std::fprintf(stderr,
                 "  [I0-c] tracked tooth id: needsRepair=%zu candidates=%zu topoKey %s -> %s\n",
                 halted.needs_repair.size(),
                 halted.needs_repair.empty()
                     ? 0
                     : halted.needs_repair[0].value("candidates", json::array()).size(),
                 key_before.c_str(), after == nullptr ? "(dropped)" : after->topo_key.c_str());
    check(halted.status == onecad::session::CandidateResult::Status::NeedsRepair,
          "tracked rung: a stored id on a tooth face halts needsRepair");
    check(!halted.needs_repair.empty() &&
              halted.needs_repair[0].value("reason", std::string()) == "no-candidates" &&
              halted.needs_repair[0].value("candidates", json::array()).empty(),
          "tracked rung: the halt offers NO candidates — a record defect, not a rebind");
    check(has_gear_diagnostic(halted),
          "tracked rung: the GEAR_FACE_NOT_REFERENCEABLE diagnostic rides the step");
    check(after != nullptr && after->topo_key == key_before,
          "tracked rung: the stored binding is left exactly where it was, never repointed");
}

// ── The ROOT-RADIUS BOUNDARY. `root_radius` is an ANALYTIC rootDiameter/2 while
// a candidate's radius is measured off built geometry, so an exact `<` at the
// root land decides by float luck. The rule uses a one-authoring-resolution band;
// this pins both sides of it on a bare cylinder (a null body skips the
// membership test, so only the geometric rule is under test). ────────────────
void test_root_radius_uses_a_band_not_an_exact_compare() {
    ops::GearBodyInfo info;
    info.axis = gp_Ax1(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0));
    info.root_radius = 17.5;
    info.gear_op_id = "opgear";

    const auto lateral_face_at = [](double radius) {
        const TopoDS_Shape cyl =
            BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0)), radius,
                                     10.0)
                .Shape();
        NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> faces;
        TopExp::MapShapes(cyl, TopAbs_FACE, faces);
        for (int i = 1; i <= faces.Extent(); ++i) {
            if (onecad::session::classify_shape(faces(i)).value("surfaceType", std::string()) ==
                "cylinder") {
                return faces(i);
            }
        }
        return TopoDS_Shape();
    };

    struct Case {
        double radius;
        bool referenceable;
        const char* what;
    };
    for (const Case& c : {Case{5.0, true, "the Ø10 axle bore is well inside the root circle"},
                          Case{17.0, true, "a counterbore half a millimetre inside it"},
                          Case{17.4995, false, "a cylinder INSIDE the 1e-3 mm band"},
                          Case{17.5, false, "a root-land cylinder at exactly the root radius"},
                          Case{22.0, false, "a tip cylinder outside the root circle"}}) {
        const TopoDS_Shape face = lateral_face_at(c.radius);
        check(!face.IsNull(), std::string("root band fixture: a cylinder at r=") +
                                  std::to_string(c.radius) + " has a lateral face");
        if (face.IsNull()) continue;
        const bool got = ops::gear_face_referenceable(info, TopoDS_Shape(), face);
        std::fprintf(stderr, "  [I0-c] root band r=%.4f (root 17.5) -> %s\n", c.radius,
                     got ? "referenceable" : "refused");
        check(got == c.referenceable, std::string("root band: ") + c.what);
    }
}

}  // namespace

int main() {
    test_flank_is_refused_by_the_real_mint();
    test_bore_and_cap_are_referenceable();
    test_edges_and_vertices_follow_their_neighbourhood();
    test_plan_step_refuses_a_stored_tooth_ref();
    test_tracked_tooth_id_halts_without_repointing();
    test_root_radius_uses_a_band_not_an_exact_compare();
    if (g_failures == 0) std::fprintf(stderr, "gear_referenceability: OK\n");
    return g_failures;
}
