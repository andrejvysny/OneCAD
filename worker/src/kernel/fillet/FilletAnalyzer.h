#pragma once

#include <string>
#include <vector>

#include <BRepFilletAPI_MakeFillet.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Shape.hxx>

#include "kernel/diagnostics/OperationDiagnostic.h"
#include "kernel/validation/ShapeAudit.h"

namespace onecad::kernel::fillet {

struct ResolvedEdge {
  std::string element_id;
  std::string topo_key;
  std::string ref_id;
  std::string body_id;
  TopoDS_Edge edge;
};

struct FilletContour {
  int index = 0;
  std::vector<std::string> edge_topo_keys;
  std::vector<std::size_t> requested_edges;
};

struct FilletAnalysis {
  bool ok = false;
  std::string message;
  std::vector<FilletContour> contours;
};

struct PartialResultProbe {
  bool query_ok = false;
  bool available = false;
  TopoDS_Shape bad_shape;
};

class FilletAnalyzer {
public:
  static FilletAnalysis analyze(const BRepFilletAPI_MakeFillet &builder,
                                const TopoDS_Shape &body,
                                const std::vector<ResolvedEdge> &requested);

  static PartialResultProbe partial_result(BRepFilletAPI_MakeFillet &builder);

  static std::vector<diagnostics::OperationDiagnostic>
  failure_diagnostics(BRepFilletAPI_MakeFillet &builder,
                      const std::vector<ResolvedEdge> &requested,
                      const FilletAnalysis &analysis, double radius,
                      const validation::ShapeAuditResult &input_audit,
                      const validation::ShapeAuditResult &output_audit,
                      bool partial_available,
                      const std::string &fallback_message);
};

} // namespace onecad::kernel::fillet
