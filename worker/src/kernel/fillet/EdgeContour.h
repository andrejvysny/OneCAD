#pragma once

#include <string>
#include <vector>

#include <TopoDS_Edge.hxx>
#include <TopoDS_Shape.hxx>

namespace onecad::kernel::fillet {

enum class EdgeOpMode { Fillet, Chamfer };

struct EdgeContourResult {
  bool ok = false;
  std::string message;
  std::vector<int> closure_ordinals;
  std::vector<std::vector<int>> contours;
};

EdgeContourResult analyze_edge_contours(const TopoDS_Shape &body,
                                        const std::vector<TopoDS_Edge> &seeds,
                                        EdgeOpMode mode);

} // namespace onecad::kernel::fillet
