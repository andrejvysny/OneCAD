#include <cstdio>
#include <set>
#include <string>

#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "kernel/geometry/EdgeChainer.h"

namespace {

int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++failures;
}

TopoDS_Shape split_prism() {
  BRepBuilderAPI_MakePolygon polygon;
  polygon.Add(gp_Pnt(0, 0, 0));
  polygon.Add(gp_Pnt(5, 0, 0));
  polygon.Add(gp_Pnt(10, 0, 0));
  polygon.Add(gp_Pnt(10, 10, 0));
  polygon.Add(gp_Pnt(0, 10, 0));
  polygon.Close();
  return BRepPrimAPI_MakePrism(BRepBuilderAPI_MakeFace(polygon.Wire()).Face(),
                               gp_Vec(0, 0, 10))
      .Shape();
}

std::set<int> chained_ordinals(const TopoDS_Shape &body,
                               const TopoDS_Edge &seed,
                               const TopTools_IndexedMapOfShape &edges) {
  std::set<int> result;
  const auto chain = onecad::core::modeling::EdgeChainer::buildChain(seed, body);
  for (const TopoDS_Edge &edge : chain.edges)
    result.insert(edges.FindIndex(edge));
  return result;
}

template <typename Builder>
std::set<int> contour_ordinals(const Builder &builder, const TopoDS_Edge &seed,
                               const TopTools_IndexedMapOfShape &edges) {
  std::set<int> result;
  const int contour = builder.Contour(seed);
  if (contour <= 0)
    return result;
  for (int index = 1; index <= builder.NbEdges(contour); ++index)
    result.insert(edges.FindIndex(builder.Edge(contour, index)));
  return result;
}

std::string names(const std::set<int> &ordinals) {
  std::string result;
  for (const int ordinal : ordinals) {
    if (!result.empty())
      result += ",";
    result += std::to_string(ordinal);
  }
  return result;
}

void compare_contours() {
  const TopoDS_Shape body = split_prism();
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(body, TopAbs_EDGE, edges);
  TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
  TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  bool saw_expansion = false;
  bool saw_ineligible = false;
  for (int ordinal = 1; ordinal <= edges.Extent(); ++ordinal) {
    const TopoDS_Edge seed = TopoDS::Edge(edges(ordinal));
    const std::set<int> expected = chained_ordinals(body, seed, edges);

    BRepFilletAPI_MakeFillet fillet(body);
    fillet.Add(1.0, seed);
    const std::set<int> fillet_contour = contour_ordinals(fillet, seed, edges);

    BRepFilletAPI_MakeChamfer chamfer(body);
    const int face_index = edge_faces.FindIndex(seed);
    check(face_index > 0 && !edge_faces(face_index).IsEmpty(),
          "solid edge has adjacent face");
    chamfer.Add(1.0, 1.0, seed,
                TopoDS::Face(edge_faces(face_index).First()));
    const std::set<int> chamfer_contour = contour_ordinals(chamfer, seed, edges);

    check(fillet_contour.empty() == chamfer_contour.empty(),
          "Fillet and Chamfer agree whether edge is operable");
    if (fillet_contour.empty() && chamfer_contour.empty()) {
      saw_ineligible = true;
      continue;
    }
    check(expected == fillet_contour,
          "EdgeChainer equals OCCT Fillet contour for edge " +
              std::to_string(ordinal) + " (chain=" + names(expected) +
              ", OCCT=" + names(fillet_contour) + ")");
    check(expected == chamfer_contour,
          "EdgeChainer equals OCCT Chamfer contour for edge " +
              std::to_string(ordinal) + " (chain=" + names(expected) +
              ", OCCT=" + names(chamfer_contour) + ")");
    saw_expansion = saw_expansion || expected.size() > 1;
  }
  check(saw_expansion, "fixture contains a tangent-expanded contour");
  check(saw_ineligible, "fixture contains an edge both OCCT builders refuse");
}

} // namespace

int main() {
  compare_contours();
  return failures;
}
