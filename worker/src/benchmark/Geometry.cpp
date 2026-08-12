#include "benchmark/Geometry.h"

#include "benchmark/GeometrySupportPair.h"

#include <algorithm>
#include <cmath>
#include <limits>
#include <tuple>

#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepGProp.hxx>
#include <BRepLib.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeWedge.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <NCollection_IndexedMap.hxx>
#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_List.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Shell.hxx>
#include <gp_Ax1.hxx>
#include <gp_Trsf.hxx>

namespace onecad::benchmark {
namespace {

std::uint64_t splitmix64(std::uint64_t &state) {
  state += 0x9e3779b97f4a7c15ULL;
  std::uint64_t value = state;
  value = (value ^ (value >> 30U)) * 0xbf58476d1ce4e5b9ULL;
  value = (value ^ (value >> 27U)) * 0x94d049bb133111ebULL;
  return value ^ (value >> 31U);
}

double unit_double(std::uint64_t &state) {
  return static_cast<double>(splitmix64(state) >> 11U) * 0x1.0p-53;
}

TopoDS_Face planar_face(const std::vector<gp_Pnt> &points) {
  BRepBuilderAPI_MakePolygon polygon;
  for (const gp_Pnt &point : points)
    polygon.Add(point);
  polygon.Close();
  return BRepBuilderAPI_MakeFace(polygon.Wire()).Face();
}

TopoDS_Shape make_pyramid(const std::vector<double> &dimensions) {
  const double x = dimensions[0] * 0.5;
  const double y = dimensions[1] * 0.5;
  const double z = dimensions[2];
  const gp_Pnt a(-x, -y, 0), b(x, -y, 0), c(x, y, 0), d(-x, y, 0);
  const gp_Pnt apex(0, 0, z);
  BRepBuilderAPI_Sewing sewing;
  sewing.Add(planar_face({a, d, c, b}));
  sewing.Add(planar_face({a, b, apex}));
  sewing.Add(planar_face({b, c, apex}));
  sewing.Add(planar_face({c, d, apex}));
  sewing.Add(planar_face({d, a, apex}));
  sewing.Perform();
  for (TopExp_Explorer it(sewing.SewedShape(), TopAbs_SHELL); it.More(); it.Next()) {
    TopoDS_Solid solid = BRepBuilderAPI_MakeSolid(TopoDS::Shell(it.Current())).Solid();
    BRepLib::OrientClosedSolid(solid);
    return solid;
  }
  return {};
}

bool make_shape(const CaseSpec &benchmark_case, TopoDS_Shape &out,
                std::string &error) {
  const std::string &recipe = benchmark_case.recipe;
  if (recipe == "supportPair")
    return make_support_pair(benchmark_case.parameters, out, error);
  const auto &d = benchmark_case.parameters.dimensions;
  if (d.size() != 3) {
    error = "recipe '" + recipe + "' is not generated yet";
    return false;
  }
  if (recipe == "box" || recipe == "valence3Corner")
    out = BRepPrimAPI_MakeBox(d[0], d[1], d[2]).Shape();
  else if (recipe == "valence4Corner")
    out = make_pyramid(d);
  else if (recipe == "overflowWedge")
    out = BRepPrimAPI_MakeWedge(d[0], d[1], d[2], d[0] * 0.25).Shape();
  else {
    error = "recipe '" + recipe + "' is not generated yet";
    return false;
  }
  return true;
}

struct EdgeInfo {
  TopoDS_Edge edge;
  gp_Pnt midpoint;
  gp_Vec chord;
  double length = 0.0;
};

EdgeInfo describe_edge(const TopoDS_Edge &edge) {
  TopoDS_Vertex first;
  TopoDS_Vertex last;
  TopExp::Vertices(edge, first, last);
  EdgeInfo info;
  info.edge = edge;
  if (first.IsNull() || last.IsNull())
    return info;
  const gp_Pnt a = BRep_Tool::Pnt(first);
  const gp_Pnt b = BRep_Tool::Pnt(last);
  info.midpoint.SetXYZ((a.XYZ() + b.XYZ()) * 0.5);
  info.chord = gp_Vec(a, b);
  info.length = info.chord.Magnitude();
  return info;
}

std::vector<EdgeInfo> sorted_edges(const TopoDS_Shape &shape) {
  NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> map;
  TopExp::MapShapes(shape, TopAbs_EDGE, map);
  std::vector<EdgeInfo> edges;
  for (int i = 1; i <= map.Extent(); ++i)
    edges.push_back(describe_edge(TopoDS::Edge(map(i))));
  std::sort(edges.begin(), edges.end(), [](const EdgeInfo &a, const EdgeInfo &b) {
    return std::make_tuple(a.midpoint.X(), a.midpoint.Y(), a.midpoint.Z(), a.length) <
           std::make_tuple(b.midpoint.X(), b.midpoint.Y(), b.midpoint.Z(), b.length);
  });
  return edges;
}

std::vector<TopoDS_Vertex> sorted_vertices(const TopoDS_Shape &shape) {
  NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> map;
  TopExp::MapShapes(shape, TopAbs_VERTEX, map);
  std::vector<TopoDS_Vertex> vertices;
  for (int i = 1; i <= map.Extent(); ++i)
    vertices.push_back(TopoDS::Vertex(map(i)));
  std::sort(vertices.begin(), vertices.end(), [](const TopoDS_Vertex &a,
                                                  const TopoDS_Vertex &b) {
    const gp_Pnt pa = BRep_Tool::Pnt(a);
    const gp_Pnt pb = BRep_Tool::Pnt(b);
    return std::make_tuple(pa.X(), pa.Y(), pa.Z()) <
           std::make_tuple(pb.X(), pb.Y(), pb.Z());
  });
  return vertices;
}

std::vector<EdgeInfo> edges_at_vertex(const std::vector<EdgeInfo> &edges,
                                      const TopoDS_Vertex &vertex) {
  std::vector<EdgeInfo> group;
  for (const EdgeInfo &edge : edges) {
    TopoDS_Vertex edge_first;
    TopoDS_Vertex edge_last;
    TopExp::Vertices(edge.edge, edge_first, edge_last);
    if ((!edge_first.IsNull() && edge_first.IsSame(vertex)) ||
        (!edge_last.IsNull() && edge_last.IsSame(vertex)))
      group.push_back(edge);
  }
  return group;
}

std::vector<EdgeInfo> role_edges(const CaseSpec &benchmark_case,
                                 const TopoDS_Shape &shape) {
  std::vector<EdgeInfo> edges = sorted_edges(shape);
  if (benchmark_case.recipe != "box")
    return edges;
  edges.erase(std::remove_if(edges.begin(), edges.end(), [](const EdgeInfo &edge) {
                const double z = std::abs(edge.chord.Z());
                return edge.length <= 0.0 || z / edge.length < 1.0 - 1.0e-9;
              }), edges.end());
  return edges;
}

bool shares_vertex(const TopoDS_Edge &a, const TopoDS_Edge &b) {
  TopoDS_Vertex af, al, bf, bl;
  TopExp::Vertices(a, af, al);
  TopExp::Vertices(b, bf, bl);
  return (!af.IsNull() && (af.IsSame(bf) || af.IsSame(bl))) ||
         (!al.IsNull() && (al.IsSame(bf) || al.IsSame(bl)));
}

// Recipe-local anchors are exact by construction (double arithmetic replayed
// from the same case generator), so a tight tolerance also catches ambiguous
// matches: a runner-up within tolerance means the anchor does not uniquely
// identify a candidate and the case/geometry pairing is untrustworthy.
constexpr double kAnchorTolerance = 1.0e-6;

double anchor_distance(const gp_Pnt &point, const std::vector<double> &anchor) {
  return point.Distance(gp_Pnt(anchor[0], anchor[1], anchor[2]));
}

bool nearest_unique(const std::vector<double> &distances, std::size_t &best,
                    std::string &error) {
  best = distances.size();
  double best_distance = std::numeric_limits<double>::infinity();
  double runner_up = std::numeric_limits<double>::infinity();
  for (std::size_t i = 0; i < distances.size(); ++i) {
    if (distances[i] < best_distance) {
      runner_up = best_distance;
      best_distance = distances[i];
      best = i;
    } else if (distances[i] < runner_up) {
      runner_up = distances[i];
    }
  }
  if (best == distances.size() || best_distance > kAnchorTolerance) {
    error = "semantic selector anchor matched no candidate within tolerance";
    return false;
  }
  if (runner_up - best_distance < kAnchorTolerance) {
    error = "semantic selector anchor matched ambiguous candidates";
    return false;
  }
  return true;
}

std::vector<TopoDS_Edge> select_corner_incident(const CaseSpec &benchmark_case,
                                                const TopoDS_Shape &shape,
                                                const std::vector<EdgeInfo> &edges,
                                                std::string &error) {
  const std::vector<std::vector<double>> &anchors = benchmark_case.selector.anchor_points;
  if (anchors.size() != 1) {
    error = "corner-incident selector requires exactly one anchor";
    return {};
  }
  const std::vector<TopoDS_Vertex> vertices = sorted_vertices(shape);
  std::vector<double> distances;
  distances.reserve(vertices.size());
  for (const TopoDS_Vertex &vertex : vertices)
    distances.push_back(anchor_distance(BRep_Tool::Pnt(vertex), anchors.front()));
  std::size_t best = 0;
  if (!nearest_unique(distances, best, error))
    return {};
  const std::size_t target_valence =
      benchmark_case.recipe == "valence4Corner" ? 4 : 3;
  const std::vector<EdgeInfo> group = edges_at_vertex(edges, vertices[best]);
  if (group.size() != target_valence) {
    error = "semantic selector anchor vertex has unexpected valence";
    return {};
  }
  std::vector<TopoDS_Edge> selected;
  for (const EdgeInfo &edge : group)
    selected.push_back(edge.edge);
  return selected;
}

std::vector<TopoDS_Edge> select_edge_anchors(const CaseSpec &benchmark_case,
                                             const std::vector<EdgeInfo> &candidates,
                                             std::string &error) {
  std::vector<bool> used(candidates.size(), false);
  std::vector<TopoDS_Edge> selected;
  for (const std::vector<double> &anchor : benchmark_case.selector.anchor_points) {
    std::vector<double> distances(candidates.size(),
                                  std::numeric_limits<double>::infinity());
    for (std::size_t i = 0; i < candidates.size(); ++i) {
      if (!used[i])
        distances[i] = anchor_distance(candidates[i].midpoint, anchor);
    }
    std::size_t best = 0;
    if (!nearest_unique(distances, best, error))
      return {};
    used[best] = true;
    selected.push_back(candidates[best].edge);
  }
  return selected;
}

std::vector<TopoDS_Edge> select_by_selector(const CaseSpec &benchmark_case,
                                            const TopoDS_Shape &shape,
                                            const std::vector<EdgeInfo> &candidates,
                                            std::string &error) {
  if (benchmark_case.selector.mode == "cornerIncident")
    return select_corner_incident(benchmark_case, shape, candidates, error);
  return select_edge_anchors(benchmark_case, candidates, error);
}

std::string curve_name(const TopoDS_Edge &edge) {
  const BRepAdaptor_Curve curve(edge);
  return curve.GetType() == GeomAbs_Line ? "line" : "analytic-curve";
}

std::vector<std::string> adjacent_surfaces(const TopoDS_Shape &shape,
                                           const TopoDS_Edge &edge) {
  NCollection_IndexedDataMap<TopoDS_Shape, NCollection_List<TopoDS_Shape>,
                             TopTools_ShapeMapHasher> map;
  TopExp::MapShapesAndAncestors(shape, TopAbs_EDGE, TopAbs_FACE, map);
  std::vector<std::string> out;
  if (!map.Contains(edge))
    return out;
  for (const TopoDS_Shape &item : map.FindFromKey(edge)) {
    const BRepAdaptor_Surface surface(TopoDS::Face(item));
    out.push_back(surface.GetType() == GeomAbs_Plane ? "plane" : "analytic-surface");
  }
  return out;
}

gp_Pnt volume_center(const TopoDS_Shape &shape) {
  GProp_GProps props;
  BRepGProp::VolumeProperties(shape, props);
  return props.CentreOfMass();
}

// Deterministic metamorphic-equivalence probes anchored on the shape's own
// centroid, scaled by the case's fillet radius so they land near the
// feature being blended rather than in arbitrary empty space.
std::vector<gp_Pnt> make_probe_points(const gp_Pnt &center, double radius) {
  const double offset = std::max(radius * 2.0, 1.0e-3);
  return {center,
          gp_Pnt(center.X() + offset, center.Y(), center.Z()),
          gp_Pnt(center.X() - offset, center.Y(), center.Z()),
          gp_Pnt(center.X(), center.Y() + offset, center.Z()),
          gp_Pnt(center.X(), center.Y() - offset, center.Z()),
          gp_Pnt(center.X(), center.Y(), center.Z() + offset),
          gp_Pnt(center.X(), center.Y(), center.Z() - offset)};
}

gp_Pnt vector_to_point(const std::vector<double> &v) {
  return gp_Pnt(v[0], v[1], v[2]);
}

gp_Trsf variant_transform(const VariantSpec &variant, const TopoDS_Shape &shape) {
  gp_Trsf transform;
  if (variant.name == "translated" || variant.name == "farOriginTranslated") {
    transform.SetTranslation(gp_Vec(variant.translation[0], variant.translation[1],
                                    variant.translation[2]));
  } else if (variant.name == "rotated") {
    const gp_Dir axis(variant.rotation_axis[0], variant.rotation_axis[1],
                      variant.rotation_axis[2]);
    constexpr double kPi = 3.141592653589793238462643383279502884;
    transform.SetRotation(gp_Ax1(volume_center(shape), axis),
                          variant.rotation_degrees * kPi / 180.0);
  } else if (variant.name == "mirrored") {
    const gp_Pnt center = variant.mirror_center.empty()
                              ? volume_center(shape)
                              : vector_to_point(variant.mirror_center);
    const gp_Dir normal(variant.mirror_normal[0], variant.mirror_normal[1],
                        variant.mirror_normal[2]);
    transform.SetMirror(gp_Ax2(center, normal));
  } else if (variant.name == "scaled") {
    const gp_Pnt center = variant.scale_center.empty()
                              ? volume_center(shape)
                              : vector_to_point(variant.scale_center);
    transform.SetScale(center, variant.scale_factor);
  }
  return transform;
}

void apply_variant(const VariantSpec &variant, GeneratedGeometry &geometry) {
  if (variant.name == "base")
    return;

  if (variant.name == "edgeOrderPermutation") {
    std::vector<TopoDS_Edge> reordered;
    reordered.reserve(variant.edge_order.size());
    for (std::size_t index : variant.edge_order) {
      if (index >= geometry.selected_edges.size()) {
        return;
      }
      reordered.push_back(geometry.selected_edges[index]);
    }
    geometry.selected_edges = std::move(reordered);
    return;
  }

  if (variant.name == "contourSeed") {
    if (!geometry.selected_edges.empty() &&
        variant.contour_seed < geometry.selected_edges.size()) {
      std::rotate(geometry.selected_edges.begin(),
                  geometry.selected_edges.begin() + variant.contour_seed,
                  geometry.selected_edges.end());
    }
    return;
  }

  const gp_Trsf transform = variant_transform(variant, geometry.shape);
  BRepBuilderAPI_Transform builder(geometry.shape, transform, true);
  geometry.shape = builder.Shape();
  for (TopoDS_Edge &edge : geometry.selected_edges)
    edge = TopoDS::Edge(builder.ModifiedShape(edge));
  for (gp_Pnt &point : geometry.probe_points)
    point.Transform(transform);
}

} // namespace

bool generate_geometry(const CaseSpec &benchmark_case,
                       const VariantSpec &variant, GeneratedGeometry &out,
                       std::string &error) {
  std::uint64_t seed = std::stoull(benchmark_case.generator.seed, nullptr, 16);
  (void)unit_double(seed);
  if (!make_shape(benchmark_case, out.shape, error))
    return false;
  if (out.shape.IsNull()) {
    error = "generator produced a null shape";
    return false;
  }
  const gp_Pnt center = volume_center(out.shape);
  out.rotation_center = {center.X(), center.Y(), center.Z()};
  out.probe_points = make_probe_points(center, benchmark_case.radius);
  std::vector<EdgeInfo> candidates = role_edges(benchmark_case, out.shape);
  out.selected_edges = select_by_selector(benchmark_case, out.shape, candidates, error);
  if (out.selected_edges.empty()) {
    if (error.empty())
      error = "semantic selector matched no edges";
    return false;
  }
  if (benchmark_case.selector.mode == "disconnected") {
    for (std::size_t i = 0; i < out.selected_edges.size(); ++i) {
      for (std::size_t j = i + 1; j < out.selected_edges.size(); ++j) {
        if (shares_vertex(out.selected_edges[i], out.selected_edges[j])) {
          error = "disconnected selector chose adjacent edges";
          return false;
        }
      }
    }
  }
  apply_variant(variant, out);
  nlohmann::json evidence = nlohmann::json::array();
  for (std::size_t i = 0; i < out.selected_edges.size(); ++i) {
    const EdgeInfo edge = describe_edge(out.selected_edges[i]);
    evidence.push_back({{"provenance",
                         "generator:" + benchmark_case.recipe + ":feature:" +
                             std::to_string(benchmark_case.parameters.feature_index)},
                        {"topologyRole", benchmark_case.selector.topology_role},
                        {"anchor", {edge.midpoint.X(), edge.midpoint.Y(), edge.midpoint.Z()}},
                        {"surface", curve_name(out.selected_edges[i])},
                        {"adjacency", adjacent_surfaces(out.shape, out.selected_edges[i])}});
  }
  out.selection_evidence = std::move(evidence);
  return true;
}

} // namespace onecad::benchmark
