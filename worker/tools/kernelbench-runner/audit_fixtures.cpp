#include <cstdlib>
#include <iostream>
#include <string>

#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRep_Builder.hxx>
#include <Standard_Failure.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Solid.hxx>
#include <gp_Pln.hxx>
#include <nlohmann/json.hpp>

#include "benchmark/DeepAudit.h"

namespace {

using nlohmann::json;

void require(bool condition, const std::string &message) {
  if (!condition) {
    std::cerr << "kernelbench audit fixture: " << message << '\n';
    std::exit(1);
  }
}

void run_fixture(const std::string &name, void (*fixture)()) {
  try {
    fixture();
  } catch (const Standard_Failure &failure) {
    require(false, name + ": " + failure.what());
  } catch (const std::exception &error) {
    require(false, name + ": " + error.what());
  }
}

TopoDS_Shape compound(const TopoDS_Shape &first, const TopoDS_Shape &second) {
  BRep_Builder builder;
  TopoDS_Compound result;
  builder.MakeCompound(result);
  builder.Add(result, first);
  builder.Add(result, second);
  return result;
}

void null_and_non_solid_fixtures() {
  const json null_audit = onecad::benchmark::deep_audit(TopoDS_Shape());
  require(null_audit["productionAudit"] == "fail", "null must fail production audit");
  require(null_audit["counts"]["solids"] == 0, "null solid count");

  const TopoDS_Shape edge = BRepBuilderAPI_MakeEdge(gp_Pnt(0, 0, 0), gp_Pnt(1, 0, 0));
  const json edge_audit = onecad::benchmark::deep_audit(edge);
  require(edge_audit["productionAudit"] == "fail", "edge must not publish");
  require(!edge_audit["closedManifold"].get<bool>(), "edge must be open");
}

void invalid_solid_fixture() {
  BRep_Builder builder;
  TopoDS_Solid empty;
  builder.MakeSolid(empty);
  const json audit = onecad::benchmark::deep_audit(empty);
  require(audit["productionAudit"] == "fail", "empty solid must not publish");
  require(!audit["exactValid"].get<bool>(), "empty solid must fail exact audit");
  require(audit["counts"]["solids"] == 1, "empty solid count remains observable");
}

void open_and_multi_solid_fixtures() {
  const TopoDS_Shape face = BRepBuilderAPI_MakeFace(
      gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), 0.0, 10.0, 0.0, 10.0);
  const json face_audit = onecad::benchmark::deep_audit(face);
  require(!face_audit["closedManifold"].get<bool>(), "face must be open");

  const TopoDS_Shape first = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
  const TopoDS_Shape second = BRepPrimAPI_MakeBox(gp_Pnt(20, 0, 0), 10, 10, 10).Shape();
  const json multi = onecad::benchmark::deep_audit(compound(first, second));
  require(multi["counts"]["solids"] == 2, "multi-solid count");
  require(multi["productionAudit"] == "fail", "multi-solid must not publish");
}

void self_interference_fixture() {
  const TopoDS_Shape first = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
  const TopoDS_Shape second =
      BRepPrimAPI_MakeBox(gp_Pnt(5, 5, 5), 10, 10, 10).Shape();
  const json audit = onecad::benchmark::deep_audit(compound(first, second));
  require(!audit["selfInterferenceFree"].get<bool>(), "overlap interference");
}

void tolerance_fixture() {
  TopoDS_Shape shape = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
  BRep_Builder builder;
  for (TopExp_Explorer it(shape, TopAbs_VERTEX); it.More(); it.Next()) {
    TopoDS_Vertex vertex = TopoDS::Vertex(it.Current());
    builder.UpdateVertex(vertex, 0.01);
  }
  const json audit = onecad::benchmark::deep_audit(shape);
  require(audit["tolerances"]["vertex"]["count"] == 8, "vertex tolerance count");
  require(audit["tolerances"]["vertex"]["maximum"].get<double>() >= 0.01,
          "inflated tolerance maximum");
}

void micro_topology_fixture() {
  const TopoDS_Shape box = BRepPrimAPI_MakeBox(1.0e6, 1.0e6, 1.0e6).Shape();
  const TopoDS_Shape sliver = BRepBuilderAPI_MakeFace(
      gp_Pln(gp_Pnt(2.0e6, 0, 0), gp_Dir(0, 0, 1)), 0.0, 1.0e-4, 0.0, 1.0e-4);
  const TopoDS_Shape shape = compound(box, sliver);
  const json audit = onecad::benchmark::deep_audit(shape);
  require(audit["microTopology"]["microEdgeCount"].get<int>() > 0,
          "micro-edge metric");
  require(audit["microTopology"]["sliverFaceCount"].get<int>() > 0,
          "sliver-face metric");
}

} // namespace

int main() {
  run_fixture("null/non-solid", null_and_non_solid_fixtures);
  run_fixture("invalid-solid", invalid_solid_fixture);
  run_fixture("open/multi-solid", open_and_multi_solid_fixtures);
  run_fixture("self-interference", self_interference_fixture);
  run_fixture("tolerance", tolerance_fixture);
  run_fixture("micro-topology", micro_topology_fixture);
  return 0;
}
