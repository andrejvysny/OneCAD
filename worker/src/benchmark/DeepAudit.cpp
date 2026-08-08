#include "benchmark/DeepAudit.h"

#include <algorithm>
#include <cmath>
#include <numeric>
#include <vector>

#include <BOPAlgo_CheckerSI.hxx>
#include <BOPDS_DS.hxx>
#include <BRepBndLib.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_IndexedMap.hxx>
#include <NCollection_List.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <nlohmann/json.hpp>

#include "kernel/validation/ShapeAudit.h"

namespace onecad::benchmark {
namespace {

constexpr int kMaximumCount = 10000000;

struct ToleranceStats {
  std::vector<double> values;

  nlohmann::json json() const {
    if (values.empty())
      return {{"count", 0}, {"maximum", 0.0}, {"mean", 0.0}, {"p95", 0.0}};
    std::vector<double> sorted = values;
    std::sort(sorted.begin(), sorted.end());
    const std::size_t index = static_cast<std::size_t>(
        std::ceil(0.95 * static_cast<double>(sorted.size()))) - 1;
    const double sum = std::accumulate(sorted.begin(), sorted.end(), 0.0);
    const std::size_t count = std::min<std::size_t>(sorted.size(), kMaximumCount);
    return {{"count", count}, {"maximum", sorted.back()},
            {"mean", sum / static_cast<double>(sorted.size())},
            {"p95", sorted[index]}};
  }
};

ToleranceStats tolerances(const TopoDS_Shape &shape, TopAbs_ShapeEnum type) {
  ToleranceStats stats;
  NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> map;
  TopExp::MapShapes(shape, type, map);
  for (int i = 1; i <= map.Extent(); ++i) {
    if (type == TopAbs_VERTEX)
      stats.values.push_back(BRep_Tool::Tolerance(TopoDS::Vertex(map(i))));
    else if (type == TopAbs_EDGE)
      stats.values.push_back(BRep_Tool::Tolerance(TopoDS::Edge(map(i))));
    else
      stats.values.push_back(BRep_Tool::Tolerance(TopoDS::Face(map(i))));
  }
  return stats;
}

int topology_count(const TopoDS_Shape &shape, TopAbs_ShapeEnum type) {
  NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> map;
  TopExp::MapShapes(shape, type, map);
  return std::min(map.Extent(), kMaximumCount);
}

double bounding_diagonal(const TopoDS_Shape &shape, nlohmann::json &bounds) {
  Bnd_Box box;
  BRepBndLib::Add(shape, box);
  if (box.IsVoid()) {
    bounds = {{"minimum", {0.0, 0.0, 0.0}},
              {"maximum", {0.0, 0.0, 0.0}}, {"diagonal", 0.0}};
    return 0.0;
  }
  double xmin, ymin, zmin, xmax, ymax, zmax;
  box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
  const double diagonal = std::hypot(xmax - xmin, std::hypot(ymax - ymin, zmax - zmin));
  bounds = {{"minimum", {xmin, ymin, zmin}}, {"maximum", {xmax, ymax, zmax}},
            {"diagonal", diagonal}};
  return diagonal;
}

nlohmann::json mass_properties(const TopoDS_Shape &shape) {
  GProp_GProps volume;
  GProp_GProps surface;
  BRepGProp::VolumeProperties(shape, volume);
  BRepGProp::SurfaceProperties(shape, surface);
  if (std::abs(volume.Mass()) <= 1.0e-300) {
    return {{"volume", 0.0}, {"area", std::max(0.0, surface.Mass())},
            {"centroid", {0.0, 0.0, 0.0}},
            {"inertia", {0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0}}};
  }
  const gp_Pnt center = volume.CentreOfMass();
  const gp_Mat inertia = volume.MatrixOfInertia();
  return {{"volume", std::abs(volume.Mass())}, {"area", std::max(0.0, surface.Mass())},
          {"centroid", {center.X(), center.Y(), center.Z()}},
          {"inertia", {inertia.Value(1, 1), inertia.Value(1, 2), inertia.Value(1, 3),
                       inertia.Value(2, 1), inertia.Value(2, 2), inertia.Value(2, 3),
                       inertia.Value(3, 1), inertia.Value(3, 2), inertia.Value(3, 3)}}};
}

nlohmann::json manifold_audit(const TopoDS_Shape &shape) {
  NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> edges;
  NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> faces;
  TopExp::MapShapes(shape, TopAbs_EDGE, edges);
  TopExp::MapShapes(shape, TopAbs_FACE, faces);
  std::vector<int> uses(static_cast<std::size_t>(edges.Extent() + 1), 0);
  for (int i = 1; i <= faces.Extent(); ++i) {
    for (TopExp_Explorer it(faces(i), TopAbs_EDGE); it.More(); it.Next()) {
      const int index = edges.FindIndex(it.Current());
      if (index > 0)
        ++uses[static_cast<std::size_t>(index)];
    }
  }
  int open = 0;
  int non_manifold = 0;
  for (int i = 1; i <= edges.Extent(); ++i) {
    if (BRep_Tool::Degenerated(TopoDS::Edge(edges(i))))
      continue;
    const int count = uses[static_cast<std::size_t>(i)];
    open += count < 2 ? 1 : 0;
    non_manifold += count > 2 ? 1 : 0;
  }
  return {{"closedManifold", open == 0 && non_manifold == 0},
          {"openEdgeCount", open}, {"nonManifoldEdgeCount", non_manifold}};
}

nlohmann::json self_interference(const TopoDS_Shape &shape) {
  BOPAlgo_CheckerSI checker;
  checker.AddArgument(shape);
  checker.SetLevelOfCheck(9);
  checker.SetRunParallel(false);
  checker.Perform();
  if (checker.HasErrors())
    return {{"checked", false}, {"count", 0}, {"error", "checker failed"}};
  return {{"checked", true},
          {"count", checker.DS().Interferences().Extent()},
          {"mode", "full-single-threaded"}};
}

double edge_length(const TopoDS_Edge &edge) {
  GProp_GProps props;
  BRepGProp::LinearProperties(edge, props);
  return props.Mass();
}

double face_area(const TopoDS_Face &face) {
  GProp_GProps props;
  BRepGProp::SurfaceProperties(face, props);
  return props.Mass();
}

nlohmann::json micro_topology(const TopoDS_Shape &shape, double diagonal) {
  int micro_edges = 0;
  int sliver_faces = 0;
  double minimum_edge = 1.0;
  double minimum_face = 1.0;
  const double safe_diagonal = std::max(diagonal, 1.0e-300);
  for (TopExp_Explorer it(shape, TopAbs_EDGE); it.More(); it.Next()) {
    const double normalized = edge_length(TopoDS::Edge(it.Current())) / safe_diagonal;
    minimum_edge = std::min(minimum_edge, normalized);
    if (normalized < 1.0e-9 && micro_edges < kMaximumCount)
      ++micro_edges;
  }
  for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
    const double normalized = face_area(TopoDS::Face(it.Current())) /
                              (safe_diagonal * safe_diagonal);
    minimum_face = std::min(minimum_face, normalized);
    if (normalized < 1.0e-12 && sliver_faces < kMaximumCount)
      ++sliver_faces;
  }
  return {{"microEdgeCount", micro_edges}, {"sliverFaceCount", sliver_faces},
          {"minimumEdgeRatio", minimum_edge}, {"minimumFaceRatio", minimum_face}};
}

nlohmann::json topology_counts(const TopoDS_Shape &shape) {
  return {{"solids", topology_count(shape, TopAbs_SOLID)},
          {"shells", topology_count(shape, TopAbs_SHELL)},
          {"faces", topology_count(shape, TopAbs_FACE)},
          {"edges", topology_count(shape, TopAbs_EDGE)},
          {"vertices", topology_count(shape, TopAbs_VERTEX)}};
}

nlohmann::json null_audit() {
  const nlohmann::json distribution =
      {{"count", 0}, {"maximum", 0.0}, {"mean", 0.0}, {"p95", 0.0}};
  return {{"productionAudit", "fail"}, {"exactValid", false},
          {"selfInterferenceFree", false}, {"closedManifold", false},
          {"counts", {{"solids", 0}, {"shells", 0}, {"faces", 0},
                      {"edges", 0}, {"vertices", 0}}},
          {"massProperties", {{"volume", 0.0}, {"area", 0.0},
                              {"centroid", {0.0, 0.0, 0.0}},
                              {"inertia", {0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                                           0.0, 0.0, 0.0}}}},
          {"bounds", {{"minimum", {0.0, 0.0, 0.0}},
                      {"maximum", {0.0, 0.0, 0.0}}, {"diagonal", 0.0}}},
          {"tolerances", {{"vertex", distribution}, {"edge", distribution},
                           {"face", distribution}}},
          {"microTopology", {{"microEdgeCount", 0}, {"minimumEdgeRatio", 0.0},
                             {"sliverFaceCount", 0}, {"minimumFaceRatio", 0.0}}}};
}

} // namespace

nlohmann::json deep_audit(const TopoDS_Shape &shape) {
  if (shape.IsNull())
    return null_audit();
  const auto production = onecad::kernel::validation::audit_shape(shape);
  nlohmann::json bounds;
  const double diagonal = bounding_diagonal(shape, bounds);
  const nlohmann::json mass = mass_properties(shape);
  nlohmann::json out = {{"productionAudit", production.publishable() ? "pass" : "fail"},
                        {"exactValid", false}, {"selfInterferenceFree", false},
                        {"closedManifold", false}, {"counts", topology_counts(shape)},
                        {"massProperties", mass}, {"bounds", bounds},
                        {"tolerances", {{"vertex", tolerances(shape, TopAbs_VERTEX).json()},
                                        {"edge", tolerances(shape, TopAbs_EDGE).json()},
                                        {"face", tolerances(shape, TopAbs_FACE).json()}}},
                        {"microTopology", micro_topology(shape, diagonal)}};
  try {
    BRepCheck_Analyzer exact(shape, true, false, true);
    exact.SetParallel(false);
    exact.SetExactMethod(true);
    const nlohmann::json interference = self_interference(shape);
    const nlohmann::json manifold = manifold_audit(shape);
    // BRepCheck_Analyzer reports a topologically-empty container (e.g. a
    // solid with zero shells) as valid, since it only checks constraints on
    // sub-shapes that exist. A container with no faces is degenerate
    // regardless, so gate exactValid on that too.
    out["exactValid"] = exact.IsValid() && out["counts"]["faces"].get<int>() > 0;
    out["selfInterferenceFree"] = interference.value("checked", false) &&
                                  interference.value("count", 1) == 0;
    out["closedManifold"] = manifold.value("closedManifold", false);
  } catch (const Standard_Failure &failure) {
    (void)failure;
  } catch (const std::exception &error) {
    (void)error;
  }
  return out;
}

} // namespace onecad::benchmark
