#include "benchmark/FilletRun.h"

#include <algorithm>
#include <cmath>

#include <BRepFilletAPI_MakeFillet.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_ShapeEnum.hxx>

#include "kernel/fillet/FilletBuilder.h"

namespace onecad::benchmark {
namespace {

using onecad::kernel::fillet::FilletBuilder;
using onecad::kernel::fillet::ResolvedEdge;

std::string failure_message(const Standard_Failure &failure) {
  return failure.what();
}

int generated_faces(BRepFilletAPI_MakeFillet &builder,
                    const std::vector<TopoDS_Edge> &edges) {
  int count = 0;
  for (const TopoDS_Edge &edge : edges) {
    for (const TopoDS_Shape &generated : builder.Generated(edge))
      count += generated.ShapeType() == TopAbs_FACE ? 1 : 0;
  }
  return count;
}

int contours(BRepFilletAPI_MakeFillet &builder,
             const std::vector<TopoDS_Edge> &edges) {
  std::vector<int> values;
  for (const TopoDS_Edge &edge : edges) {
    const int contour = builder.Contour(edge);
    if (contour > 0 && std::find(values.begin(), values.end(), contour) == values.end())
      values.push_back(contour);
  }
  return static_cast<int>(values.size());
}

void capture_assigned_radii(BRepFilletAPI_MakeFillet &builder,
                            const std::vector<TopoDS_Edge> &edges,
                            double radius, AdapterResult &out) {
  std::vector<int> seen;
  for (const TopoDS_Edge &edge : edges) {
    const int contour = builder.Contour(edge);
    if (contour <= 0 || std::find(seen.begin(), seen.end(), contour) != seen.end())
      continue;
    seen.push_back(contour);
    const double assigned = builder.Radius(contour);
    out.assigned_radius_max_error =
        std::max(out.assigned_radius_max_error, std::abs(assigned - radius));
    ++out.assigned_radius_count;
  }
}

nlohmann::json raw_faults(BRepFilletAPI_MakeFillet &builder) {
  nlohmann::json faulty = nlohmann::json::array();
  const int count = std::min(builder.NbFaultyContours(), 16);
  for (int i = 1; i <= count; ++i)
    faulty.push_back(builder.FaultyContour(i));
  return {{"faultyContours", faulty},
          {"faultyVertexCount", std::min(builder.NbFaultyVertices(), 64)}};
}

AdapterResult run_raw(const GeneratedGeometry &geometry, double radius) {
  AdapterResult out;
  try {
    BRepFilletAPI_MakeFillet builder(geometry.shape);
    for (const TopoDS_Edge &edge : geometry.selected_edges)
      builder.Add(radius, edge);
    out.contour_count = contours(builder, geometry.selected_edges);
    capture_assigned_radii(builder, geometry.selected_edges, radius, out);
    builder.Build();
    if (builder.IsDone()) {
      out.generated_face_count = generated_faces(builder, geometry.selected_edges);
      out.output = builder.Shape();
      out.success = !out.output.IsNull();
      out.operation_state = out.success ? "success" : "invalid-result";
      out.failure_class = out.success ? "none" : "invalid-output";
      return out;
    }
    out.partial = builder.HasResult();
    if (out.partial)
      out.partial_shape = builder.BadShape();
    out.operation_state = out.partial ? "partial" : "refused";
    out.failure_class = out.partial ? "partial-result" : "operation-refused";
    out.diagnostics.push_back(raw_faults(builder));
  } catch (const Standard_Failure &failure) {
    out.operation_state = "exception";
    out.failure_class = "kernel-exception";
    out.message = failure_message(failure);
  } catch (const std::exception &error) {
    out.operation_state = "exception";
    out.failure_class = "cpp-exception";
    out.message = error.what();
  }
  return out;
}

std::vector<ResolvedEdge> resolved_edges(const GeneratedGeometry &geometry) {
  std::vector<ResolvedEdge> out;
  for (std::size_t i = 0; i < geometry.selected_edges.size(); ++i) {
    const std::string suffix = std::to_string(i);
    out.push_back({"semantic-edge-" + suffix, "semantic-edge-" + suffix,
                   "operation.selector." + suffix, "generated-body",
                   geometry.selected_edges[i]});
  }
  return out;
}

AdapterResult run_onecad(const GeneratedGeometry &geometry, double radius) {
  AdapterResult out;
  FilletBuilder builder(geometry.shape, resolved_edges(geometry), radius);
  const auto built = builder.build();
  out.success = built.ok;
  out.output = built.shape;
  out.operation_state = built.ok ? "success" : (built.cancelled ? "cancelled" : "refused");
  out.failure_class = built.ok ? "none" : built.error_code;
  out.message = built.message;
  out.contour_count = static_cast<int>(built.analysis.contours.size());
  for (const auto &diagnostic : built.diagnostics) {
    if (out.diagnostics.size() >= 32)
      break;
    out.diagnostics.push_back(diagnostic.to_json());
  }
  if (built.ok) {
    capture_assigned_radii(builder.history(), geometry.selected_edges, radius, out);
    out.generated_face_count = generated_faces(builder.history(), geometry.selected_edges);
  } else if (built.analysis.ok) {
    try {
      out.partial = builder.history().HasResult();
      if (out.partial) {
        out.partial_shape = builder.history().BadShape();
        out.operation_state = "partial";
        out.failure_class = "partial-result";
      }
    } catch (...) {
    }
  }
  return out;
}

} // namespace

AdapterResult run_fillet(const std::string &backend,
                         const GeneratedGeometry &geometry, double radius) {
  return backend == "raw-occt" ? run_raw(geometry, radius)
                                : run_onecad(geometry, radius);
}

} // namespace onecad::benchmark
