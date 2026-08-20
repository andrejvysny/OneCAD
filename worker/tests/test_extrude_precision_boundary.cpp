// test_extrude_precision_boundary.cpp — WP1 G1: Extrude's two authoring-resolution
// guards, asserted AT the boundary.
//
// Both the Blind/Symmetric distance guard and the ToFace coincidence guard read
// `GeometryPrecisionContext::authoring_resolution()`. Every pre-existing extrude
// refusal case sits at 0, or at a value ANY plausible threshold refuses, so none of
// them can prove the guards are still wired to the context — perturbing the context
// constant left the whole extrude suite green. These cases STRADDLE the boundary:
// one value just below is refused BY NAME, one just above clears the guard, so they
// flip the moment the context moves.
//
// The upper case asserts the GUARD, not the build. OCCT is entitled to refuse a
// 1.05 µm prism for its own reasons; it is not entitled to refuse it with the
// guard's message.
//
// In-process via `execute_extrude` on real OCCT. No framework: exit code == failure
// count.
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/ExtrudeOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
using onecad::session::BodyStore;

namespace {

int g_failures = 0;

void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

/// A w×h rectangle on XY. World map (u,v)->(-v,u,0): world x∈[-h,0], y∈[0,w].
json rect_sketch(const std::string& sid, double w, double h) {
    json sk;
    sk["sketchId"] = sid;
    sk["plane"] = json{{"kind", "XY"}};
    sk["entities"] = json::array({
        json{{"id", "e1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {w, 0}}},
        json{{"id", "e2"}, {"type", "Line"}, {"p0", {w, 0}}, {"p1", {w, h}}},
        json{{"id", "e3"}, {"type", "Line"}, {"p0", {w, h}}, {"p1", {0, h}}},
        json{{"id", "e4"}, {"type", "Line"}, {"p0", {0, h}}, {"p1", {0, 0}}},
    });
    sk["constraints"] = json::array();
    return sk;
}

/// The FACE whose descriptor centre is nearest (cx,cy,cz).
TopoDS_Shape face_by_center(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(faces(i));
        const double dx = d.center.X() - cx, dy = d.center.Y() - cy, dz = d.center.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) {
            best_d2 = d2;
            best = faces(i);
        }
    }
    return best;
}

json face_ref(const std::string& body_id, const TopoDS_Shape& face, double ax, double ay,
              double az) {
    return json{{"primary", {{"bodyId", body_id}, {"elementId", "el_tf"}, {"kind", "face"}}},
                {"intent",
                 {{"kind", "face"},
                  {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                     em::ElementMapPartition::describe(face))}}},
                {"anchor", {{"worldPoint", {ax, ay, az}}}}};
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& bodies, em::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

// ── Blind distance: ExtrudeOp's `authoring_resolution()` distance guard ────────
void blind_distance_boundary() {
    const std::string needle = "Extrude distance too small";
    const auto extrude_by = [&](double distance) {
        BodyStore bodies;
        em::ElementMapPartition part;
        Ctx c;
        c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
        c.last_sketch = "sk1";
        ops::OpContext ctx = c.make(bodies, part);
        json op = {{"opType", "Extrude"},
                   {"opId", "opeb"},
                   {"params",
                    {{"sketchId", "sk1"},
                     {"extrudeMode", "Blind"},
                     {"booleanMode", "NewBody"},
                     {"distance", distance}}}};
        return ops::execute_extrude(ctx, op, "opeb");
    };

    const ops::OpOutcome below = extrude_by(9.9e-4);
    check(below.status == ops::OpOutcome::Status::Failed &&
              below.error_message.find(needle) != std::string::npos,
          "blind boundary: 9.9e-4 mm is refused by name, got: " + below.error_message);

    const ops::OpOutcome above = extrude_by(1.05e-3);
    check(above.error_message.find(needle) == std::string::npos,
          "blind boundary: 1.05e-3 mm clears the guard, got: " + above.error_message);
}

// ── ToFace coincidence: the same floor, read at the ToFace resolution site ─────
void to_face_coincidence_boundary() {
    const std::string needle = "coincides with the sketch plane";
    // The profile is at z=0; a target box whose BOTTOM face sits at z=`gap` puts the
    // resolved ToFace distance exactly at `gap`.
    const auto extrude_to = [&](double gap) {
        const TopoDS_Shape target =
            BRepPrimAPI_MakeBox(gp_Pnt(-10, 0, gap), 10.0, 10.0, 10.0).Shape();
        BodyStore bodies;
        bodies.create("body_target", "op_t", target);
        em::ElementMapPartition part;
        const TopoDS_Shape bottom = face_by_center(target, -5, 5, gap);
        Ctx c;
        c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
        c.last_sketch = "sk1";
        ops::OpContext ctx = c.make(bodies, part);
        json op = {{"opType", "Extrude"},
                   {"opId", "opet"},
                   {"params",
                    {{"sketchId", "sk1"},
                     {"extrudeMode", "ToFace"},
                     {"booleanMode", "NewBody"},
                     {"targetFace", face_ref("body_target", bottom, -5.0, 5.0, gap)}}}};
        return ops::execute_extrude(ctx, op, "opet");
    };

    const ops::OpOutcome below = extrude_to(9.9e-4);
    check(below.status == ops::OpOutcome::Status::Failed &&
              below.error_message.find(needle) != std::string::npos,
          "toFace boundary: a target 9.9e-4 mm away is refused by name, got: " +
              below.error_message);

    const ops::OpOutcome above = extrude_to(1.05e-3);
    check(above.error_message.find(needle) == std::string::npos,
          "toFace boundary: a target 1.05e-3 mm away clears the guard, got: " +
              above.error_message);
}

}  // namespace

int main() {
    blind_distance_boundary();
    to_face_coincidence_boundary();
    if (g_failures == 0) std::fprintf(stderr, "extrude_precision_boundary: OK\n");
    // An exit status that is a multiple of 256 reports PASS to the shell.
    return g_failures == 0 ? 0 : 1;
}
