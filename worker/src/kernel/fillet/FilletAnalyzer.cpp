#include "kernel/fillet/FilletAnalyzer.h"

#include <algorithm>
#include <map>

#include <ChFiDS_ErrorStatus.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

namespace onecad::kernel::fillet {

namespace {

std::string edge_key(const TopTools_IndexedMapOfShape &edges,
                     const TopoDS_Edge &edge) {
  const int index = edges.FindIndex(edge);
  return index > 0 ? "e:" + std::to_string(index) : "";
}

std::pair<std::string, std::string> status_info(ChFiDS_ErrorStatus status) {
  switch (status) {
  case ChFiDS_WalkingFailure:
    return {"FILLET_WALKING_FAILED", "walking-failure"};
  case ChFiDS_StartsolFailure:
    return {"FILLET_START_SOLUTION_FAILED", "start-solution-failure"};
  case ChFiDS_TwistedSurface:
    return {"FILLET_TWISTED_SURFACE", "twisted-surface"};
  case ChFiDS_Error:
    return {"FILLET_KERNEL_FAILED", "kernel-error"};
  case ChFiDS_Ok:
    return {"FILLET_BUILD_FAILED", "build-failure"};
  }
  return {"FILLET_BUILD_FAILED", "build-failure"};
}

nlohmann::json refs_json(const std::vector<ResolvedEdge> &requested) {
  nlohmann::json refs = nlohmann::json::array();
  for (const ResolvedEdge &edge : requested) {
    if (refs.size() >= 64)
      break;
    refs.push_back({{"refId", edge.ref_id},
                    {"bodyId", edge.body_id},
                    {"elementId", edge.element_id},
                    {"topoKey", edge.topo_key}});
  }
  return refs;
}

nlohmann::json metrics_json(double radius,
                            const validation::ShapeAuditResult &input,
                            const validation::ShapeAuditResult &output,
                            bool partial_available, int faulty_vertices) {
  return {{"requestedRadius", radius},
          {"solidCount", output.solid_count},
          {"volume", output.volume},
          {"inputMaxTolerance", input.tolerances.maximum()},
          {"outputMaxTolerance", output.tolerances.maximum()},
          {"inputTolerances", input.tolerances.to_json()},
          {"outputTolerances", output.tolerances.to_json()},
          {"selfInterferenceCount", output.self_interference_count},
          {"partialResultAvailable", partial_available},
          {"faultyVertexCount", faulty_vertices}};
}

struct FaultEvidence {
  std::vector<int> contours;
  int vertices = 0;
};

FaultEvidence collect_fault_evidence(BRepFilletAPI_MakeFillet &builder,
                                     const FilletAnalysis &analysis) {
  FaultEvidence evidence;
  try {
    const int count = std::min(builder.NbFaultyContours(), 16);
    for (int i = 1; i <= count; ++i)
      evidence.contours.push_back(builder.FaultyContour(i));
  } catch (...) {
  }
  try {
    evidence.vertices = std::min(builder.NbFaultyVertices(), 64);
    for (int i = 1; i <= evidence.vertices; ++i)
      (void)builder.FaultyVertex(i);
  } catch (...) {
    evidence.vertices = 0;
  }
  if (evidence.contours.empty() && !analysis.contours.empty()) {
    evidence.contours.push_back(analysis.contours.front().index);
  }
  return evidence;
}

const FilletContour *find_contour(const FilletAnalysis &analysis, int index) {
  for (const FilletContour &contour : analysis.contours) {
    if (contour.index == index)
      return &contour;
  }
  return nullptr;
}

} // namespace

FilletAnalysis
FilletAnalyzer::analyze(const BRepFilletAPI_MakeFillet &builder,
                        const TopoDS_Shape &body,
                        const std::vector<ResolvedEdge> &requested) {
  FilletAnalysis out;
  try {
    TopTools_IndexedMapOfShape body_edges;
    TopExp::MapShapes(body, TopAbs_EDGE, body_edges);
    std::map<int, FilletContour> contours;
    for (std::size_t i = 0; i < requested.size(); ++i) {
      const int contour = builder.Contour(requested[i].edge);
      if (contour <= 0) {
        out.message =
            "requested edge does not belong to an OCCT fillet contour";
        return out;
      }
      FilletContour &info = contours[contour];
      info.index = contour;
      info.requested_edges.push_back(i);
    }
    for (auto &[index, contour] : contours) {
      for (int edge_index = 1; edge_index <= builder.NbEdges(index);
           ++edge_index) {
        const std::string key =
            edge_key(body_edges, builder.Edge(index, edge_index));
        if (!key.empty() && contour.edge_topo_keys.size() < 64) {
          contour.edge_topo_keys.push_back(key);
        }
      }
      out.contours.push_back(std::move(contour));
    }
    out.ok = !out.contours.empty();
    if (!out.ok)
      out.message = "OCCT created no fillet contours";
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    out.message = message ? message : "OCCT contour analysis failed";
  } catch (...) {
    out.message = "unknown OCCT contour analysis failure";
  }
  return out;
}

PartialResultProbe
FilletAnalyzer::partial_result(BRepFilletAPI_MakeFillet &builder) {
  PartialResultProbe probe;
  try {
    probe.available = builder.HasResult();
  } catch (...) {
    return probe;
  }
  if (probe.available) {
    try {
      probe.bad_shape = builder.BadShape();
    } catch (...) {
      return probe;
    }
  }
  probe.query_ok = true;
  return probe;
}

std::vector<diagnostics::OperationDiagnostic>
FilletAnalyzer::failure_diagnostics(
    BRepFilletAPI_MakeFillet &builder,
    const std::vector<ResolvedEdge> &requested, const FilletAnalysis &analysis,
    double radius, const validation::ShapeAuditResult &input_audit,
    const validation::ShapeAuditResult &output_audit, bool partial_available,
    const std::string &fallback_message) {
  const FaultEvidence faults = collect_fault_evidence(builder, analysis);
  std::vector<diagnostics::OperationDiagnostic> out;
  for (int contour_index : faults.contours) {
    ChFiDS_ErrorStatus status = ChFiDS_Error;
    try {
      status = builder.StripeStatus(contour_index);
    } catch (...) {
    }
    const auto [code, label] = status_info(status);
    const FilletContour *contour = find_contour(analysis, contour_index);
    nlohmann::json edge_keys = contour ? nlohmann::json(contour->edge_topo_keys)
                                       : nlohmann::json::array();
    diagnostics::OperationDiagnostic diagnostic;
    diagnostic.code = code;
    diagnostic.message = code == "FILLET_WALKING_FAILED"
                             ? "Fillet failed while advancing contour"
                             : fallback_message;
    diagnostic.stage = "build";
    diagnostic.evidence = {
        {"refs", refs_json(requested)},
        {"contour",
         {{"index", contour_index},
          {"status", label},
          {"edgeTopoKeys", std::move(edge_keys)}}},
        {"metrics", metrics_json(radius, input_audit, output_audit,
                                 partial_available, faults.vertices)}};
    out.push_back(std::move(diagnostic));
  }
  if (out.empty()) {
    diagnostics::OperationDiagnostic diagnostic;
    diagnostic.code =
        partial_available ? "FILLET_PARTIAL_RESULT" : "FILLET_BUILD_FAILED";
    diagnostic.message = fallback_message;
    diagnostic.stage = "build";
    diagnostic.evidence = {
        {"refs", refs_json(requested)},
        {"metrics", metrics_json(radius, input_audit, output_audit,
                                 partial_available, faults.vertices)}};
    out.push_back(std::move(diagnostic));
  }
  return out;
}

} // namespace onecad::kernel::fillet
