#include "kernel/fillet/EdgeContour.h"

#include <algorithm>
#include <set>

#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>

#include "kernel/geometry/EdgeChainer.h"

namespace onecad::kernel::fillet {

namespace {

std::vector<int> chainer_ordinals(const TopoDS_Shape &body,
                                  const TopoDS_Edge &seed,
                                  const TopTools_IndexedMapOfShape &edges) {
  std::vector<int> result;
  const auto chain = core::modeling::EdgeChainer::buildChain(seed, body);
  for (const TopoDS_Edge &edge : chain.edges) {
    const int ordinal = edges.FindIndex(edge);
    if (ordinal > 0)
      result.push_back(ordinal);
  }
  std::sort(result.begin(), result.end());
  result.erase(std::unique(result.begin(), result.end()), result.end());
  return result;
}

template <typename Builder, typename Add>
EdgeContourResult analyze(const TopoDS_Shape &body,
                          const std::vector<TopoDS_Edge> &seeds,
                          Builder &builder, Add add) {
  EdgeContourResult result;
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(body, TopAbs_EDGE, edges);
  std::set<int> closure;
  for (const TopoDS_Edge &seed : seeds) {
    const int seed_ordinal = edges.FindIndex(seed);
    if (seed_ordinal <= 0) {
      result.message = "requested edge is not part of the target body";
      return result;
    }
    if (closure.count(seed_ordinal) > 0)
      continue;
    add(builder, seed);
    const int contour_index = builder.Contour(seed);
    if (contour_index <= 0) {
      result.message = "OCCT did not accept a requested edge contour";
      return result;
    }
    std::vector<int> contour;
    for (int index = 1; index <= builder.NbEdges(contour_index); ++index) {
      const int ordinal = edges.FindIndex(builder.Edge(contour_index, index));
      if (ordinal <= 0) {
        result.message = "OCCT contour contains an edge outside the target body";
        return result;
      }
      contour.push_back(ordinal);
    }
    std::sort(contour.begin(), contour.end());
    contour.erase(std::unique(contour.begin(), contour.end()), contour.end());
    if (contour != chainer_ordinals(body, seed, edges)) {
      result.message = "EdgeChainer disagrees with the OCCT operation contour";
      return result;
    }
    closure.insert(contour.begin(), contour.end());
    result.contours.push_back(std::move(contour));
  }
  result.closure_ordinals.assign(closure.begin(), closure.end());
  result.ok = !result.closure_ordinals.empty();
  if (!result.ok)
    result.message = "OCCT created no edge contours";
  return result;
}

} // namespace

EdgeContourResult analyze_edge_contours(const TopoDS_Shape &body,
                                        const std::vector<TopoDS_Edge> &seeds,
                                        EdgeOpMode mode) {
  try {
    if (mode == EdgeOpMode::Fillet) {
      BRepFilletAPI_MakeFillet builder(body);
      return analyze(body, seeds, builder,
                     [](auto &op, const TopoDS_Edge &edge) {
                       op.Add(1.0, edge);
                     });
    }
    BRepFilletAPI_MakeChamfer builder(body);
    return analyze(body, seeds, builder,
                   [](auto &op, const TopoDS_Edge &edge) {
                     op.Add(1.0, edge);
                   });
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    return {false, message ? message : "OCCT contour analysis failed", {}, {}};
  } catch (...) {
    return {false, "unknown OCCT contour analysis failure", {}, {}};
  }
}

} // namespace onecad::kernel::fillet
