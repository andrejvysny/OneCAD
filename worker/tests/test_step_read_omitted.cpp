// test_step_read_omitted.cpp — kernel audit 2026-11-11 finding 15.
// STEP import is solids-only (SCHEMA §7.3 ImportStep). `promote_solids` dropped
// open shells and faces outside any shell; the drop was reported only when the
// root produced NO solid (STEP_ROOT_SKIPPED). A surface part next to a solid in
// one transferred root therefore vanished without a word.
//
// Fixture (written with OCCT here, nothing binary committed): one compound root
// holding a 10 mm box SOLID, an OPEN shell (five faces of a second box) and a
// FREE planar face, all disjoint. Expected: exactly the box is imported (no
// surface is promoted to a body), and STEP_GEOMETRY_OMITTED names the loss.
// Control: a lone box carries no such diagnostic. No framework.
#include <cmath>
#include <cstdio>
#include <filesystem>
#include <string>

#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRep_Builder.hxx>
#include <GProp_GProps.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Shell.hxx>
#include <gp_Pln.hxx>

#include "io/StepRead.h"
#include "step_fixture_util.h"

using onecad::io::StepReadPolicy;
using onecad::io::StepReadResult;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}

const onecad::io::StepReadDiagnostic* find(const StepReadResult& r, const char* code) {
    for (const auto& d : r.diagnostics) {
        if (d.code == code) return &d;
    }
    return nullptr;
}

TopoDS_Shape mixed_root() {
    BRep_Builder builder;
    TopoDS_Compound root;
    builder.MakeCompound(root);
    builder.Add(root, BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 10, 10, 10).Shape());

    TopoDS_Shell open;
    builder.MakeShell(open);
    int kept = 0;
    const TopoDS_Shape second = BRepPrimAPI_MakeBox(gp_Pnt(50, 0, 0), 10, 10, 10).Shape();
    for (TopExp_Explorer fe(second, TopAbs_FACE); fe.More() && kept < 5; fe.Next(), ++kept) {
        builder.Add(open, fe.Current());
    }
    builder.Add(root, open);
    builder.Add(root, BRepBuilderAPI_MakeFace(gp_Pln(gp_Pnt(0, 100, 0), gp_Dir(0, 0, 1)), 0, 10, 0, 10).Face());
    return root;
}
}  // namespace

int main() {
    const std::filesystem::path dir = std::filesystem::temp_directory_path();
    const std::string mixed_path = (dir / "onecad_step_mixed_omitted.step").string();
    const std::string box_path = (dir / "onecad_step_box_control.step").string();
    check(stepfx::write_step_fixture(mixed_root(), mixed_path).empty(), "fixture: mixed root written");
    check(stepfx::write_step_fixture(stepfx::make_single_box(), box_path).empty(), "fixture: box written");

    const StepReadResult mixed = onecad::io::read_step(mixed_path, StepReadPolicy{});
    check(mixed.ok(), "mixed: read succeeded");
    check(mixed.solids.size() == 1, "mixed: exactly the box is imported (got " +
                                        std::to_string(mixed.solids.size()) + " solids)");
    if (mixed.solids.size() == 1) {
        GProp_GProps props;
        BRepGProp::VolumeProperties(mixed.solids[0], props);
        check(std::abs(props.Mass() - 1000.0) < 1e-6, "mixed: the imported solid is the 10 mm box");
    }
    const auto* omitted = find(mixed, "STEP_GEOMETRY_OMITTED");
    check(omitted != nullptr, "mixed: STEP_GEOMETRY_OMITTED names the dropped surface geometry");
    if (omitted != nullptr) std::fprintf(stderr, "  %s: %s\n", omitted->code.c_str(), omitted->message.c_str());
    check(find(mixed, onecad::io::step_diag::kRootSkipped) == nullptr,
          "mixed: the root still yields a solid, so it is not a skipped root");

    const StepReadResult box = onecad::io::read_step(box_path, StepReadPolicy{});
    check(box.ok() && box.solids.size() == 1, "control: lone box imported");
    check(find(box, "STEP_GEOMETRY_OMITTED") == nullptr,
          "control: a lone solid carries no omission diagnostic");

    std::error_code ec;
    std::filesystem::remove(mixed_path, ec);
    std::filesystem::remove(box_path, ec);
    if (g_failures == 0) std::fprintf(stderr, "test_step_read_omitted: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
