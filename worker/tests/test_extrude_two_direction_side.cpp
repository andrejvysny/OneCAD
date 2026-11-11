// test_extrude_two_direction_side.cpp — kernel audit 2026-11-11 finding 6 / review
// SR-03. A two-direction extrude's legs are non-negative lengths along their OWN
// side (SCHEMA §7.3 Extrude). The authored-parameter check could not see a ToFace
// leg whose RESOLVED distance is negative: leg 2 (direction −Z) aimed at a face at
// z = +20 resolved to −20, built (−Z)·(−20) = +20 Z on leg 1's side, absorbed leg 1
// and published a valid 2000 mm³ solid spanning z ∈ [0, 20] with nothing below.
//
// Pinned here:
//   * leg 2 ToFace on leg 1's side → EXTRUDE_TWO_DIRECTION_LEG_WRONG_SIDE, nothing built;
//   * the mirror case for leg 1 → the same refusal;
//   * a tilted wrong-side target → the same refusal;
//   * controls: correct-side two-direction (V = 2500, z ∈ [−20, 5]) and the
//     single-direction BACKWARD ToFace (V = 2000 below the sketch plane) stay valid.
//
// Profile: the 10×10 square at world x ∈ [−10, 0], y ∈ [0, 10], z = 0 (XY sketch,
// toWorld(u, v) = (−v, u, 0)). No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepBndLib.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <Bnd_Box.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/ExtrudeOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "session/ShapeMetrics.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
using onecad::session::BodyStore;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
void check_near(double got, double want, double abs_tol, const std::string& msg) {
    if (!(std::abs(got - want) <= abs_tol)) {
        std::fprintf(stderr, "FAIL: %s (got %.12g want %.12g)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}
constexpr double kQuarter = 3.14159265358979323846 / 12.0;  // 15°

json square_sketch() {
    return json{{"sketchId", "sk1"},
                {"plane", {{"kind", "XY"}}},
                {"entities", json::array({
                                 json{{"id", "e1"}, {"type", "Line"}, {"p0", {0.0, 0.0}}, {"p1", {10.0, 0.0}}},
                                 json{{"id", "e2"}, {"type", "Line"}, {"p0", {10.0, 0.0}}, {"p1", {10.0, 10.0}}},
                                 json{{"id", "e3"}, {"type", "Line"}, {"p0", {10.0, 10.0}}, {"p1", {0.0, 10.0}}},
                                 json{{"id", "e4"}, {"type", "Line"}, {"p0", {0.0, 10.0}}, {"p1", {0.0, 0.0}}},
                             })},
                {"constraints", json::array()}};
}

// A 60×60×40 block whose face through (0, 0, z0) has normal (sinθ, 0, cosθ) and
// covers the profile footprint; it grows away from the sketch plane (`up` = +n).
TopoDS_Shape plate(double theta, double z0, bool up) {
    const gp_Dir n(std::sin(theta), 0.0, std::cos(theta));
    const gp_Dir u(std::cos(theta), 0.0, -std::sin(theta));
    const gp_Ax2 frame(gp_Pnt(0, 0, z0).Translated(gp_Vec(u) * -30.0 +
                                                   gp_Vec(n.Crossed(u)) * (up ? -25.0 : 35.0)),
                       up ? n : n.Reversed(), u);
    return BRepPrimAPI_MakeBox(frame, 60.0, 60.0, 40.0).Shape();
}

TopoDS_Shape face_through(const TopoDS_Shape& shape, const gp_Pnt& point, double theta) {
    const gp_Dir n(std::sin(theta), 0.0, std::cos(theta));
    for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
        BRepAdaptor_Surface surface(TopoDS::Face(it.Current()), true);
        if (surface.GetType() != GeomAbs_Plane) continue;
        const gp_Pln pln = surface.Plane();
        if (pln.Axis().Direction().IsParallel(n, 1e-9) && std::abs(pln.Distance(point)) < 1e-9) {
            return it.Current();
        }
    }
    return {};
}

json face_ref(const std::string& body_id, const std::string& element_id, const TopoDS_Shape& face) {
    const km::ElementDescriptor d = em::ElementMapPartition::describe(face);
    return {{"primary", {{"bodyId", body_id}, {"elementId", element_id}, {"kind", "face"}}},
            {"intent", {{"kind", "face"}, {"descriptor", em::ElementMapPartition::descriptor_to_json(d)}}},
            {"anchor", {{"worldPoint", {d.center.X(), d.center.Y(), d.center.Z()}}}}};
}

struct Target {
    std::string body;
    double theta;
    double z0;
    bool up;
};

struct Result {
    ops::OpOutcome outcome;
    double volume = -1.0;
    double zmin = 0.0, zmax = 0.0;
};

// Runs one Extrude NewBody of the square with `params` merged in; `targets` are the
// blocks placed first (their faces through (0, 0, z0) are referenced by name).
Result run(const std::vector<Target>& targets, json params) {
    BodyStore bodies;
    for (const Target& t : targets) bodies.create(t.body, "op_" + t.body, plate(t.theta, t.z0, t.up));
    for (auto& [key, value] : params.items()) {
        if (!value.is_string() || value.get<std::string>().rfind("@", 0) != 0) continue;
        const std::string body = value.get<std::string>().substr(1);
        for (const Target& t : targets) {
            if (t.body != body) continue;
            const TopoDS_Shape face = face_through(bodies.get(body)->geom, gp_Pnt(0, 0, t.z0), t.theta);
            check(!face.IsNull(), "target face found on " + body);
            value = face_ref(body, "el_" + body, face);
        }
    }
    params["sketchId"] = "sk1";
    params["booleanMode"] = "NewBody";
    std::vector<std::pair<std::string, json>> sketches{{"sk1", square_sketch()}};
    std::string last = "sk1";
    onecad::CancelToken cancel;
    em::ElementMapPartition part;
    ops::OpContext ctx{bodies, &sketches, part, &last, false, json::object(), &cancel};
    Result r;
    r.outcome = ops::execute_extrude(ctx, json{{"opType", "Extrude"}, {"opId", "ope"}, {"params", params}},
                                     "ope");
    if (const auto* rec = bodies.get("body_ope")) {
        r.volume = onecad::session::shape_volume(rec->geom);
        Bnd_Box box;
        BRepBndLib::AddOptimal(rec->geom, box, false, false);
        double x0, y0, x1, y1;
        box.Get(x0, y0, r.zmin, x1, y1, r.zmax);
    }
    return r;
}

std::string code_of(const ops::OpOutcome& o) {
    return o.diagnostics.empty() ? std::string{} : o.diagnostics[0].value("code", std::string{});
}

void expect_wrong_side(const char* label, const Result& r, int leg) {
    check(r.outcome.status == ops::OpOutcome::Status::Failed,
          std::string(label) + ": refused (volume=" + std::to_string(r.volume) + ")");
    check(code_of(r.outcome) == "EXTRUDE_TWO_DIRECTION_LEG_WRONG_SIDE",
          std::string(label) + ": named refusal, got '" + code_of(r.outcome) + "'");
    if (!r.outcome.diagnostics.empty()) {
        const json ev = r.outcome.diagnostics[0].value("evidence", json::object());
        check(ev.contains("extrude") && ev["extrude"].value("leg", 0) == leg,
              std::string(label) + ": evidence names leg " + std::to_string(leg));
    }
    check(r.volume < 0.0, std::string(label) + ": nothing published");
}

void expect_built(const char* label, const Result& r, double volume, double zmin, double zmax) {
    check(r.outcome.status == ops::OpOutcome::Status::Ok,
          std::string(label) + ": Ok (" + r.outcome.error_message + ")");
    check_near(r.volume, volume, 1e-9 * volume, std::string(label) + ": volume");
    check_near(r.zmin, zmin, 1e-7, std::string(label) + ": z min");
    check_near(r.zmax, zmax, 1e-7, std::string(label) + ": z max");
}

}  // namespace

int main() {
    try {
        const Target above{"body_up", 0.0, 20.0, true};
        const Target below{"body_down", 0.0, -20.0, false};
        const json leg1_blind = {{"extrudeMode", "Blind"}, {"distance", 5.0}, {"twoDirections", true}};

        json p = leg1_blind;
        p.update({{"extrudeMode2", "ToFace"}, {"distance2", 0.0}, {"targetFace2", "@body_up"}});
        expect_wrong_side("leg 2 ToFace on leg 1's side", run({above}, p), 2);

        p = leg1_blind;
        p.update({{"extrudeMode2", "ToFace"}, {"distance2", 0.0}, {"targetFace2", "@body_down"}});
        expect_built("leg 2 ToFace on its own side", run({below}, p), 2500.0, -20.0, 5.0);

        p = {{"extrudeMode", "ToFace"}, {"distance", 0.0}, {"targetFace", "@body_down"},
             {"twoDirections", true}, {"extrudeMode2", "Blind"}, {"distance2", 5.0}};
        expect_wrong_side("leg 1 ToFace on leg 2's side", run({below}, p), 1);

        p = {{"extrudeMode", "ToFace"}, {"distance", 0.0}, {"targetFace", "@body_up"},
             {"twoDirections", true}, {"extrudeMode2", "Blind"}, {"distance2", 5.0}};
        expect_built("leg 1 ToFace on its own side", run({above}, p), 2500.0, -5.0, 20.0);

        // Tilted 15° target above the profile, referenced by leg 2: the trim path.
        p = leg1_blind;
        p.update({{"extrudeMode2", "ToFace"}, {"distance2", 0.0}, {"targetFace2", "@body_tilt"}});
        expect_wrong_side("leg 2 tilted ToFace on leg 1's side",
                          run({Target{"body_tilt", kQuarter, 20.0, true}}, p), 2);

        // Single direction: a target BEHIND the profile is a legal backward extrude.
        p = {{"extrudeMode", "ToFace"}, {"distance", 0.0}, {"targetFace", "@body_down"}};
        expect_built("single-direction backward ToFace", run({below}, p), 2000.0, -20.0, 0.0);
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: exception %s\n", e.what());
        ++g_failures;
    }
    if (g_failures == 0) std::fprintf(stderr, "test_extrude_two_direction_side: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
