// test_extract_prism_profile.cpp — Component Library WP-C: the §7.8
// `ExtractPrismProfile` verb (is this body a prism, and if so what is its
// canonical end-cap profile?) plus the geometry it measures.
//
// The exact numbers live HERE and not in the NDJSON fixtures: `json_subset`
// compares numbers exactly and is the wrong instrument for a millimetre — the
// same contract/corpus split the OffsetFace pair and `analyze_edge_op_range`
// use. The fixtures pin the protocol SHAPE; this binary pins the measurements.
//
// The load-bearing case is POSE INDEPENDENCE: the same physical profile,
// ingested from a stick rotated 37° about its own axis, must canonicalise to the
// same face. Selecting the in-plane frame on a raw eigenvector sign or on the
// source's world orientation would make the blob a function of the input file's
// pose, and content addressing would silently stop deduplicating two ingests of
// one extrusion.
//
// No framework: exit code == failure count.
#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <limits>
#include <string>
#include <utility>
#include <vector>

#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepGProp.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRep_Tool.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <GProp_GProps.hxx>
#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "io/BrepCodec.h"
#include "nlohmann/json.hpp"
#include "ops/ExtrudeOp.h"
#include "ops/OpCommon.h"
#include "ops/OpTypes.h"
#include "protocol/Envelope.h"
#include "session/BodyStore.h"
#include "session/ExtractPrismProfile.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::BodyStore;
using onecad::session::PrismAnalysis;
using onecad::session::Session;
namespace em = onecad::elementmap;
namespace io = onecad::io;
namespace ops = onecad::ops;
namespace ses = onecad::session;

namespace {

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}
void check_near(double got, double want, double tol, const std::string& msg) {
    if (!(std::abs(got - want) <= tol)) {
        std::fprintf(stderr, "FAIL: %s (got %.17g want %.17g tol %g)\n", msg.c_str(), got, want,
                     tol);
        ++g_failures;
    }
}

constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

// 20x20 square with a centred Ø5 hole: 400 − π·2.5².
constexpr double kProfileArea = 380.3650459150638;
constexpr double kStickLength = 500.0;

void redirect_occt_to_stderr() {
    Handle(Message_Messenger) messenger = Message::DefaultMessenger();
    messenger->RemovePrinters(STANDARD_TYPE(Message_PrinterOStream));
    messenger->AddPrinter(new Message_PrinterOStream("cerr", false, Message_Info));
}

std::string temp_path(const std::string& name) {
    return (std::filesystem::temp_directory_path() / name).string();
}

json line_ent(const std::string& id, double x0, double y0, double x1, double y1) {
    return json{{"id", id}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y1}}};
}

// The 20x20-with-hole sketch, in the exact form the canonical fixture sends.
json stick_sketch(const std::string& sid) {
    return json{{"sketchId", sid},
                {"plane", {{"kind", "XY"}}},
                {"entities",
                 json::array({line_ent("e1", 0, 0, 20, 0), line_ent("e2", 20, 0, 20, 20),
                              line_ent("e3", 20, 20, 0, 20), line_ent("e4", 0, 20, 0, 0),
                              json{{"id", "c1"},
                                   {"type", "Circle"},
                                   {"center", {10.0, 10.0}},
                                   {"radius", 2.5}}})},
                {"constraints", json::array()}};
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    CancelToken cancel;
    ops::OpContext make(BodyStore& b, em::ElementMapPartition& p) {
        return ops::OpContext{b, &sketches, p, &last_sketch, false, json::object(), &cancel};
    }
};

// The vendor stick: the 20x20-with-hole profile extruded `kStickLength` mm,
// built through the REAL extrude op so the shape under test is the one a user's
// STEP import produces.
TopoDS_Shape build_stick() {
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk", stick_sketch("sk")});
    c.last_sketch = "sk";
    ops::OpContext ctx = c.make(bodies, part);
    const json op = {{"opType", "Extrude"},
                     {"opId", "ope"},
                     {"params",
                      {{"sketchId", "sk"},
                       {"distance", kStickLength},
                       {"extrudeMode", "Blind"},
                       {"booleanMode", "NewBody"}}}};
    const ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status == ops::OpOutcome::Status::Ok, "fixture: stick extruded");
    const onecad::session::BodyRecord* rec = bodies.get("body_ope");
    return rec != nullptr ? rec->geom : TopoDS_Shape();
}

double volume_of(const TopoDS_Shape& s) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(s, props);
    return props.Mass();
}

// A plain rectangular prism, built without the sketch lane so the pose test
// controls every coordinate it compares.
TopoDS_Shape rect_prism(double half_x, double half_y, double length) {
    BRepBuilderAPI_MakePolygon poly;
    poly.Add(gp_Pnt(-half_x, -half_y, 0.0));
    poly.Add(gp_Pnt(half_x, -half_y, 0.0));
    poly.Add(gp_Pnt(half_x, half_y, 0.0));
    poly.Add(gp_Pnt(-half_x, half_y, 0.0));
    poly.Close();
    const TopoDS_Face face = BRepBuilderAPI_MakeFace(poly.Wire()).Face();
    return BRepPrimAPI_MakePrism(face, gp_Vec(0.0, 0.0, length)).Shape();
}

TopoDS_Shape square_prism(double half, double length) {
    return rect_prism(half, half, length);
}

// The largest coordinate disagreement between two canonical blobs, after sorting
// each vertex list — the pose-independence measure. Byte equality is NOT the
// instrument: a real rotation and its canonical undo are not bit-exact in
// IEEE754, so two poses of one profile agree to rounding, never to the bit.
double canonical_vertex_delta(const std::vector<std::uint8_t>& a,
                              const std::vector<std::uint8_t>& b);

std::vector<std::uint8_t> bake(const TopoDS_Shape& body, const PrismAnalysis& a) {
    std::vector<std::uint8_t> bytes;
    const std::string err = ses::bake_canonical_profile(body, a, bytes);
    check(err.empty(), "bake: " + (err.empty() ? std::string("ok") : err));
    return bytes;
}

namespace {

std::vector<std::array<double, 3>> blob_vertices(const std::vector<std::uint8_t>& blob) {
    const std::string path = temp_path("onecad_wpc_cmp.brep");
    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    out.write(reinterpret_cast<const char*>(blob.data()),
              static_cast<std::streamsize>(blob.size()));
    out.close();
    const io::BrepShapeResult read = io::read_brep_shape(path);
    std::error_code rm;
    std::filesystem::remove(path, rm);
    std::vector<std::array<double, 3>> pts;
    if (!read.ok()) return pts;
    TopTools_IndexedMapOfShape vertices;
    TopExp::MapShapes(read.shape, TopAbs_VERTEX, vertices);
    for (int i = 1; i <= vertices.Extent(); ++i) {
        const gp_Pnt p = BRep_Tool::Pnt(TopoDS::Vertex(vertices(i)));
        pts.push_back({p.X(), p.Y(), p.Z()});
    }
    return pts;
}

}  // namespace

// Hausdorff, not a sorted pairing: two lists that agree to 1e-15 can still SORT
// differently (a last-bit x difference reorders equal-x corners), and pairing by
// index would then report a whole edge length as the error.
double canonical_vertex_delta(const std::vector<std::uint8_t>& a,
                              const std::vector<std::uint8_t>& b) {
    const std::vector<std::array<double, 3>> pa = blob_vertices(a);
    const std::vector<std::array<double, 3>> pb = blob_vertices(b);
    if (pa.empty() || pa.size() != pb.size()) return std::numeric_limits<double>::infinity();
    double worst = 0.0;
    for (const std::array<double, 3>& p : pa) {
        double nearest = std::numeric_limits<double>::infinity();
        for (const std::array<double, 3>& q : pb) {
            nearest = std::min(nearest, gp_Pnt(p[0], p[1], p[2]).Distance(gp_Pnt(q[0], q[1], q[2])));
        }
        worst = std::max(worst, nearest);
    }
    return worst;
}

// ── the stick ────────────────────────────────────────────────────────────────

void test_stick_measurements(const TopoDS_Shape& stick) {
    const PrismAnalysis a = ses::analyze_prism(stick, nullptr);
    check(a.is_prism, "stick: is a prism (" + a.refusal + ")");
    check(a.axis.IsEqual(gp_Dir(0, 0, 1), 1e-12), "stick: axis is +Z (longest bbox dimension)");
    check_near(a.length_mm, kStickLength, 1e-9, "stick: lengthMm is plane-to-plane");
    check_near(a.area_mm2, kProfileArea, 1e-9, "stick: areaMm2 == 400 - pi*2.5^2");
    check_near(a.volume_ratio, 1.0, 1e-9, "stick: volumeRatio == 1");
    check(a.outer_edge_count == 4, "stick: outerEdgeCount == 4, got " +
                                       std::to_string(a.outer_edge_count));
    check(a.inner_wire_count == 1, "stick: innerWireCount == 1, got " +
                                       std::to_string(a.inner_wire_count));
    std::fprintf(stderr, "  stick: len=%.17g area=%.17g ratio=%.17g cap=f:%d\n", a.length_mm,
                 a.area_mm2, a.volume_ratio, a.end_cap_ordinal);
}

// The canonical contract the §7.3 `profile` arm re-checks: plane through z = 0,
// orientation-corrected normal +Z, area centroid at the origin.
void test_canonical_face_shape(const TopoDS_Shape& stick) {
    const PrismAnalysis a = ses::analyze_prism(stick, nullptr);
    const std::string path = temp_path("onecad_wpc_canonical.brep");
    const std::vector<std::uint8_t> bytes = bake(stick, a);
    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    out.write(reinterpret_cast<const char*>(bytes.data()),
              static_cast<std::streamsize>(bytes.size()));
    out.close();

    const io::BrepShapeResult read = io::read_brep_shape(path);
    check(read.ok(), "canonical: readable back (" + read.error + ")");
    check(read.shape.ShapeType() == TopAbs_FACE, "canonical: the blob is ONE bare face");
    if (read.shape.ShapeType() != TopAbs_FACE) return;
    const TopoDS_Face face = TopoDS::Face(read.shape);
    check(face.Orientation() == TopAbs_FORWARD, "canonical: stored orientation is FORWARD");

    gp_Pln plane;
    gp_Dir normal;
    check(ops::planar_face_plane_normal(face, plane, normal), "canonical: planar");
    check_near(plane.Distance(gp_Pnt(0, 0, 0)), 0.0, 1e-6, "canonical: plane through z = 0");
    check_near((normal.XYZ() - gp_XYZ(0, 0, 1)).Modulus(), 0.0, 1e-6, "canonical: normal is +Z");

    GProp_GProps props;
    BRepGProp::SurfaceProperties(face, props);
    check_near(props.Mass(), kProfileArea, 1e-9, "canonical: area survives the transform");
    const gp_Pnt centre = props.CentreOfMass();
    check_near(centre.XYZ().Modulus(), 0.0, 1e-9, "canonical: area centroid at the origin");
    std::error_code rm;
    std::filesystem::remove(path, rm);
}

// Determinism: the same head and the same request twice ⇒ the same bytes.
void test_bake_is_byte_identical(const TopoDS_Shape& stick) {
    const PrismAnalysis a = ses::analyze_prism(stick, nullptr);
    const std::vector<std::uint8_t> first = bake(stick, a);
    const std::vector<std::uint8_t> second = bake(stick, a);
    check(!first.empty() && first == second, "determinism: two bakes are byte-identical");
}

// ── pose independence ────────────────────────────────────────────────────────

void check_pose_independent(const TopoDS_Shape& upright, const std::string& what) {
    gp_Trsf spin;
    spin.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), 37.0 * M_PI / 180.0);
    const TopoDS_Shape rotated =
        BRepBuilderAPI_Transform(upright, spin, /*Copy=*/Standard_True).Shape();

    const PrismAnalysis a0 = ses::analyze_prism(upright, nullptr);
    const PrismAnalysis a1 = ses::analyze_prism(rotated, nullptr);
    check(a0.is_prism && a1.is_prism, what + ": both sticks are prisms");
    check_near(a1.area_mm2, a0.area_mm2, 1e-9, what + ": same section area");

    const std::vector<std::uint8_t> b0 = bake(upright, a0);
    const std::vector<std::uint8_t> b1 = bake(rotated, a1);
    const double delta = canonical_vertex_delta(b0, b1);
    std::fprintf(stderr, "  pose[%s]: bytes %zu vs %zu, maxdelta=%.3g\n", what.c_str(), b0.size(),
                 b1.size(), delta);
    check(b0.size() == b1.size(), what + ": the two canonical blobs are the same size");
    // Not byte equality: a 37° rotation and its canonical undo are not bit-exact
    // in IEEE754, so the honest claim is that the two poses agree to ROUNDING.
    // The frame CHOICE itself is exact — a different choice would land the
    // vertices millimetres apart, not 1e-15 apart.
    check(delta <= 1e-9, what + ": the canonical faces agree to rounding");
}

// A square's second-moment tensor is a multiple of the identity, so this is the
// DEGENERATE branch: no eigenvector carries information and the frame has to
// come from the shape's own vertices. Get it wrong and the two ingests differ by
// 37°, which is what breaks content-address dedup.
void test_pose_independence() {
    check_pose_independent(square_prism(10.0, 300.0), "isotropic square");
    check_pose_independent(rect_prism(15.0, 5.0, 300.0), "anisotropic rectangle");
}

// ── refusals ─────────────────────────────────────────────────────────────────

// A cross-drilled stick has two equal end caps and still is not a prism: the
// swept volume overstates it, and the measured ratio says by how much.
void test_cross_drilled_refuses() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 20, 20, 100).Shape();
    const TopoDS_Shape drill =
        BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(-1, 10, 50), gp_Dir(1, 0, 0)), 4.0, 22.0).Shape();
    const TopoDS_Shape drilled = BRepAlgoAPI_Cut(box, drill).Shape();

    const PrismAnalysis a = ses::analyze_prism(drilled, nullptr);
    check(!a.is_prism, "cross-drilled: refused");
    check(a.volume_ratio < 1.0 && a.volume_ratio > 0.9,
          "cross-drilled: volumeRatio is measured and below 1");
    std::fprintf(stderr, "  cross-drilled: ratio=%.6f msg=%s\n", a.volume_ratio,
                 a.refusal.c_str());
}

// THE ONE BOTH MEASUREMENTS ACCEPT. A 20x20 stick fused end to end onto a 40x10
// stick has caps of EQUAL area (400 mm²) and a volume that is exactly
// endCapArea x length, so the volume gate reads 1.0 and the equal-area gate reads
// 0. Only the step face at the junction says this is two prisms, and baking the
// 20x20 square as "the section" would re-extrude a part that is not the one
// anybody measured.
void test_fused_unequal_sections_refuse() {
    const TopoDS_Shape lower = rect_prism(10.0, 10.0, 250.0);  // 20x20, z 0..250
    gp_Trsf lift;
    lift.SetTranslation(gp_Vec(0.0, 0.0, 250.0));
    const TopoDS_Shape upper = BRepBuilderAPI_Transform(
                                   rect_prism(20.0, 5.0, 250.0), lift, /*Copy=*/Standard_True)
                                   .Shape();  // 40x10, z 250..500
    BRepAlgoAPI_Fuse fuse(lower, upper);
    fuse.Build();
    check(fuse.IsDone(), "fused sticks: fixture built");
    const TopoDS_Shape fused = fuse.Shape();

    const PrismAnalysis a = ses::analyze_prism(fused, nullptr);
    std::fprintf(stderr, "  fused: prism=%d ratio=%.9f areaDelta=%.3g msg=%s\n",
                 static_cast<int>(a.is_prism), a.volume_ratio, a.area_delta, a.refusal.c_str());
    check(!a.is_prism, "fused sticks: refused");
    check(a.refusal.find("strictly between the end caps") != std::string::npos,
          "fused sticks: the message names the step face between the caps");

    // The negative control: BOTH measurements accept this body, so the structural
    // test is the only thing standing between it and a wrong bake.
    const double volume = volume_of(fused);
    check_near(volume, 400.0 * 500.0, 1e-6, "fused sticks: volume IS endCapArea x length");
}

// A taper has a constant-looking axis and two parallel caps of DIFFERENT area.
void test_taper_refuses() {
    BRepBuilderAPI_MakePolygon big;
    big.Add(gp_Pnt(-10, -10, 0));
    big.Add(gp_Pnt(10, -10, 0));
    big.Add(gp_Pnt(10, 10, 0));
    big.Add(gp_Pnt(-10, 10, 0));
    big.Close();
    BRepBuilderAPI_MakePolygon small;
    small.Add(gp_Pnt(-8, -8, 100));
    small.Add(gp_Pnt(8, -8, 100));
    small.Add(gp_Pnt(8, 8, 100));
    small.Add(gp_Pnt(-8, 8, 100));
    small.Close();
    BRepOffsetAPI_ThruSections loft(/*isSolid=*/Standard_True, /*ruled=*/Standard_True);
    loft.AddWire(big.Wire());
    loft.AddWire(small.Wire());
    loft.Build();
    check(loft.IsDone(), "taper: fixture built");

    const PrismAnalysis a = ses::analyze_prism(loft.Shape(), nullptr);
    check(!a.is_prism, "taper: refused");
    // Still caught by VOLUME, with the area gate relaxed from 1e-9 to 1e-6: a
    // taper that matters is nowhere near either threshold.
    check(a.volume_ratio < 0.99, "taper: the volume gate is the one that catches it");
    std::fprintf(stderr, "  taper: ratio=%.6f areaDelta=%.3g msg=%s\n", a.volume_ratio,
                 a.area_delta, a.refusal.c_str());
}

// A cube has no distinguished axis, so the bbox tie rule answers X — never a
// last-bit guess between three equal extents.
void test_cube_tie_falls_to_x() {
    const TopoDS_Shape cube = BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 10, 10, 10).Shape();
    const PrismAnalysis a = ses::analyze_prism(cube, nullptr);
    check(a.is_prism, "cube: a cube IS a prism (" + a.refusal + ")");
    check(a.axis.IsEqual(gp_Dir(1, 0, 0), 1e-12), "cube: the bbox tie falls to X");
    check_near(a.length_mm, 10.0, 1e-9, "cube: length 10");
    check_near(a.area_mm2, 100.0, 1e-9, "cube: area 100");

    const gp_Dir hint(0, 0, 1);
    const PrismAnalysis hinted = ses::analyze_prism(cube, &hint);
    check(hinted.axis.IsEqual(gp_Dir(0, 0, 1), 1e-12), "cube: an axisHint overrides the tie rule");
}

// ── the verb ─────────────────────────────────────────────────────────────────

void publish_stick(Session& s) {
    s.open("doc", 0, 3, "determinism");
    const json plan_ops = json::array(
        {json{{"opType", "Sketch"}, {"opId", "op0"}, {"stepIndex", 0}, {"params", stick_sketch("sk")}},
         json{{"opType", "Extrude"},
              {"opId", "op1"},
              {"stepIndex", 1},
              {"params",
               {{"sketchId", "sk"},
                {"distance", kStickLength},
                {"extrudeMode", "Blind"},
                {"booleanMode", "NewBody"}}}}});
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    const json args = {{"jobId", 1},
                       {"documentRevision", 0},
                       {"workerEpoch", 3},
                       {"expectedBaseHash", kEmpty},
                       {"prefixHashes", json::array({"a", "b"})},
                       {"targetStep", 1},
                       {"ops", plan_ops}};
    onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    const Envelope acc = onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    check(acc.ok.value_or(false), "verb fixture: stick published");
}

void test_verb(Session& s) {
    const std::uint64_t head = s.current_snapshot_id();
    const std::string path = temp_path("onecad_wpc_verb.brep");

    const Envelope ok = ses::handle_extract_prism_profile(
        s, Envelope::request(9, "ExtractPrismProfile",
                             json{{"snapshotId", head}, {"bodyId", "body_op1"}, {"path", path}}));
    check(ok.ok.value_or(false), "verb: ok");
    check(ok.result.value("written", false), "verb: written");
    check(ok.result.value("codec", std::string()) == "brep", "verb: codec echo");
    check(ok.result.value("format", 0) == io::kBrepFormatVersion, "verb: format echo");
    check(ok.result["refusal"].is_null(), "verb: no refusal");
    check(ok.result["sha256"].is_string() && ok.result["sha256"].get<std::string>().size() == 64,
          "verb: sha256 of the bytes written");
    check(ok.result["prism"]["outerEdgeCount"].get<int>() == 4, "verb: outerEdgeCount");
    check(ok.result["prism"]["innerWireCount"].get<int>() == 1, "verb: innerWireCount");
    check_near(ok.result["prism"]["areaMm2"].get<double>(), kProfileArea, 1e-9, "verb: areaMm2");
    std::error_code ec;
    check(std::filesystem::file_size(path, ec) == ok.result.value("bytes", std::uint64_t{0}),
          "verb: reported bytes == file size");

    // The same head twice ⇒ the same digest and the same file.
    const Envelope again = ses::handle_extract_prism_profile(
        s, Envelope::request(10, "ExtractPrismProfile",
                             json{{"snapshotId", head}, {"bodyId", "body_op1"}, {"path", path}}));
    check(again.result["sha256"] == ok.result["sha256"], "verb: the digest is reproducible");

    // No `path` ⇒ analysis only, nothing written.
    const Envelope probe = ses::handle_extract_prism_profile(
        s, Envelope::request(11, "ExtractPrismProfile",
                             json{{"snapshotId", head}, {"bodyId", "body_op1"}}));
    check(probe.ok.value_or(false) && !probe.result.value("written", true),
          "verb: no path ⇒ analysed, not written");
    check(probe.result["sha256"].is_null(), "verb: nothing written ⇒ sha256 null");

    const Envelope stale = ses::handle_extract_prism_profile(
        s, Envelope::request(12, "ExtractPrismProfile",
                             json{{"snapshotId", head + 7}, {"bodyId", "body_op1"}}));
    check(stale.error.has_value() && stale.error->code == "STALE_PREVIEW",
          "verb: a stale snapshotId is STALE_PREVIEW");

    const Envelope missing = ses::handle_extract_prism_profile(
        s, Envelope::request(13, "ExtractPrismProfile",
                             json{{"snapshotId", head}, {"bodyId", "body_nope"}}));
    check(missing.error.has_value() && missing.error->code == "REF_UNRESOLVED",
          "verb: an unknown bodyId is REF_UNRESOLVED");

    const Envelope degenerate = ses::handle_extract_prism_profile(
        s, Envelope::request(14, "ExtractPrismProfile",
                             json{{"snapshotId", head},
                                  {"bodyId", "body_op1"},
                                  {"axisHint", json::array({0.0, 0.0, 0.0})}}));
    check(degenerate.error.has_value() && degenerate.error->code == "PROTOCOL_ERROR",
          "verb: a degenerate axisHint is PROTOCOL_ERROR");

    // Read-only: the head did not move across any of the above.
    check(s.current_snapshot_id() == head, "verb: the head is untouched");
    check(!s.has_scratch(), "verb: no scratch was created");
    std::error_code rm;
    std::filesystem::remove(path, rm);
}

}  // namespace

int main() {
    redirect_occt_to_stderr();

    const TopoDS_Shape stick = build_stick();
    std::fprintf(stderr, "  stick volume = %.17g\n", volume_of(stick));
    test_stick_measurements(stick);
    test_canonical_face_shape(stick);
    test_bake_is_byte_identical(stick);
    test_pose_independence();
    test_cross_drilled_refuses();
    test_fused_unequal_sections_refuse();
    test_taper_refuses();
    test_cube_tie_falls_to_x();

    Session s;
    publish_stick(s);
    test_verb(s);

    if (g_failures == 0) std::fprintf(stderr, "extract_prism_profile: OK\n");
    return g_failures;
}
