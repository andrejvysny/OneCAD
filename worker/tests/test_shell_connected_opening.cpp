#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>

#include "ops/ShellOp.h"
#include "session/ShapeMetrics.h"

namespace {
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
using nlohmann::json;
constexpr double kBossX = -16.433137753393243;
constexpr double kBossY = 13.694281461161037;
int failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++failures;
    }
}

// Geometrically identical to the saved native QA plate+hole+boss. The app test
// replays the original container so this control cannot hide authoring differences.
TopoDS_Shape plate_hole_boss() {
    const auto plate = BRepPrimAPI_MakeBox(gp_Pnt(-40, 0, 0), 40, 60, 4).Shape();
    const auto hole = BRepPrimAPI_MakeCylinder(
        gp_Ax2(gp_Pnt(-20, 30, -1), gp_Dir(0, 0, 1)), 3, 6).Shape();
    const auto boss = BRepPrimAPI_MakeCylinder(
        gp_Ax2(gp_Pnt(kBossX, kBossY, 4), gp_Dir(0, 0, 1)), 3, 8).Shape();
    return BRepAlgoAPI_Fuse(BRepAlgoAPI_Cut(plate, hole).Shape(), boss).Shape();
}

TopoDS_Face horizontal_face(const TopoDS_Shape& shape, double z) {
    for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
        GProp_GProps props;
        BRepGProp::SurfaceProperties(it.Current(), props);
        if (std::abs(props.CentreOfMass().Z() - z) < 1e-7) return TopoDS::Face(it.Current());
    }
    return {};
}

struct Fixture {
    onecad::session::BodyStore bodies;
    em::ElementMapPartition partition;
    onecad::CancelToken cancel;
    TopoDS_Shape before = plate_hole_boss();
    ops::OpContext context{bodies, nullptr, partition, nullptr, false, json::object(), &cancel};

    Fixture(double z, ops::ValidationMode mode) {
        bodies.create("plate", "source", before);
        partition.mint("plate", "opening", em::km::ElementKind::Face,
                       horizontal_face(before, z), before, json::object());
        context.validation_mode = mode;
    }

    ops::OpOutcome shell(double thickness) {
        return ops::execute_shell(
            context,
            {{"inputs", {{{"primary", {{"bodyId", "plate"}, {"elementId", "opening"},
                                         {"kind", "face"}}}}}},
             {"params", {{"targetBodyId", "plate"}, {"thickness", thickness}}}}, "shell");
    }
};

void check_unchanged(const Fixture& fixture, const ops::OpOutcome& outcome) {
    check(outcome.status == ops::OpOutcome::Status::Failed, "invalid opening/thickness refused");
    check(outcome.body_events.empty() && outcome.body_ids.empty() && outcome.delta.empty(),
          "refusal publishes no body events or partition delta");
    check(fixture.bodies.get("plate")->geom.IsEqual(fixture.before),
          "refusal preserves original BRep");
    const auto* entry = fixture.partition.find("opening");
    check(entry && entry->shape.IsEqual(horizontal_face(fixture.before,
          em::ElementMapPartition::describe(entry->shape).center.Z())),
          "refusal preserves selected face identity");
}

void test_disconnected_opening(double thickness, ops::ValidationMode mode) {
    Fixture fixture(4, mode);
    const auto outcome = fixture.shell(thickness);
    check_unchanged(fixture, outcome);
    check(outcome.error_message.find("Selected opening disconnects the remaining shell.") == 0,
          "disconnected retained skin has a truthful actionable refusal");
    check(outcome.error_message.find("thickness") == std::string::npos,
          "connectivity refusal does not suggest changing thickness");
    check(!outcome.diagnostics.empty() &&
          outcome.diagnostics.front().value("code", "") == "SHELL_DISCONNECTED_RETAINED_SKIN",
          "connectivity failure has a stable diagnostic code");
}

void check_point(const TopoDS_Shape& shape, const gp_Pnt& point, TopAbs_State expected,
                 const std::string& message) {
    BRepClass3d_SolidClassifier classifier(shape, point, 1e-7);
    check(classifier.State() == expected, message);
}

void check_shell_geometry(const TopoDS_Shape& shape, double thickness, double opening_z) {
    check(BRepCheck_Analyzer(shape).IsValid(), "alternate opening publishes valid BRep");
    TopTools_IndexedMapOfShape solids;
    TopExp::MapShapes(shape, TopAbs_SOLID, solids);
    check(solids.Extent() == 1, "alternate opening publishes one solid");
    for (TopExp_Explorer it(shape, TopAbs_SHELL); it.More(); it.Next()) {
        check(BRep_Tool::IsClosed(it.Current()), "solid boundary is closed");
    }
    check_point(shape, gp_Pnt(kBossX, kBossY, 6), TopAbs_OUT, "boss contains a real cavity");
    check_point(shape, gp_Pnt(kBossX + 3 - thickness / 2, kBossY, 6), TopAbs_IN,
                "boss retains requested radial wall");
    check_point(shape, gp_Pnt(kBossX + 3 - 1.5 * thickness, kBossY, 6), TopAbs_OUT,
                "boss cavity reaches inward side of requested wall");
    check_point(shape, gp_Pnt(-30, 45, 2 - thickness / 4), TopAbs_OUT,
                "plate contains a real cavity");
    const double skin_z = opening_z == 0 ? 4 - thickness / 2 : thickness / 2;
    check_point(shape, gp_Pnt(-30, 45, skin_z), TopAbs_IN, "plate retains opposite skin");
    check_point(shape, gp_Pnt(-40 + thickness / 2, 45, 2), TopAbs_IN,
                "outer plate wall remains material");
}

void test_valid_opening(double opening_z, double thickness, ops::ValidationMode mode) {
    Fixture fixture(opening_z, mode);
    const auto outcome = fixture.shell(thickness);
    check(outcome.status == ops::OpOutcome::Status::Ok,
          "connected alternate opening builds: " + outcome.error_message);
    if (outcome.status != ops::OpOutcome::Status::Ok) return;
    if (opening_z == 0) {
        check(outcome.needs_repair.size() == 1 &&
              outcome.needs_repair.front().value("reason", "") == "ambiguous",
              "bottom opening retains fail-closed ambiguity between its two rim images");
    } else {
        check(outcome.needs_repair.empty(), "boss cap opening preserves unambiguous history");
    }
    check(outcome.body_ids == std::vector<std::string>{"plate"} && outcome.body_events.size() == 1,
          "alternate opening preserves target identity");
    const auto shape = fixture.bodies.get("plate")->geom;
    const double volume = onecad::session::shape_volume(shape);
    check(volume > 0 && volume < onecad::session::shape_volume(fixture.before) - 100,
          "alternate opening removes substantial material");
    check_shell_geometry(shape, thickness, opening_z);
}
} // namespace

int main() {
    for (auto mode : {ops::ValidationMode::PreviewInteractive,
                      ops::ValidationMode::CommitAuthoritative}) {
        for (double thickness : {.5, 1., 2.}) {
            test_disconnected_opening(thickness, mode);
            test_valid_opening(0, thickness, mode);
        }
        for (double thickness : {.5, 1.}) test_valid_opening(12, thickness, mode);
        for (double thickness : {2., 3.}) {
            Fixture fixture(12, mode);
            check_unchanged(fixture, fixture.shell(thickness));
        }
    }
    return failures ? 1 : 0;
}
