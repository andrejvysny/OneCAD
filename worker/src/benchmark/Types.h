#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

#include <TopoDS_Edge.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>
#include <nlohmann/json.hpp>

namespace onecad::benchmark {

struct GeneratorSpec {
  std::string name;
  int version = 0;
  std::string seed;
};

struct SelectorSpec {
  std::string mode;
  std::string topology_role;
  std::string provenance_generator;
  std::string provenance_recipe_type;
  std::size_t provenance_feature_index = 0;
  std::vector<std::string> anchor_kinds;
  std::vector<std::vector<double>> anchor_points;
  std::vector<std::string> curve_kinds;
  std::vector<std::vector<std::string>> adjacent_surface_kinds;
  std::string adjacency_relation;
};

struct CaseSpec {
  nlohmann::json canonical;
  std::string case_id;
  GeneratorSpec generator;
  std::string recipe_type;
  std::vector<double> dimensions;
  std::size_t feature_index = 0;
  std::vector<std::string> geometry_tags;
  double radius = 0.0;
  SelectorSpec selector;
  std::string expected_domain;
  nlohmann::json validators;
  nlohmann::json metamorphs;
  nlohmann::json search;
  nlohmann::json limits;
};

struct VariantSpec {
  std::string name;
  std::vector<double> translation;
  std::vector<double> rotation_axis;
  double rotation_degrees = 0.0;
};

struct Request {
  nlohmann::json canonical;
  CaseSpec benchmark_case;
  std::string backend;
  VariantSpec variant;
  std::string artifact_dir;
};

struct GeneratedGeometry {
  TopoDS_Shape shape;
  std::vector<TopoDS_Edge> selected_edges;
  nlohmann::json selection_evidence = nlohmann::json::object();
  // Deterministic metamorphic-equivalence probes: `probe_points` moves with
  // the shape under the applied variant transform (base/translated/rotated);
  // `rotation_center` is the shared pivot, computed once on the untransformed
  // shape, so it is identical across all variants of the same case and lets
  // a consumer invert a rotation without re-deriving the centroid.
  std::vector<gp_Pnt> probe_points;
  std::vector<double> rotation_center;
};

struct AdapterResult {
  bool success = false;
  bool partial = false;
  std::string operation_state = "refused";
  std::string failure_class = "operation-refused";
  std::string message;
  TopoDS_Shape output;
  TopoDS_Shape partial_shape;
  int contour_count = 0;
  int generated_face_count = 0;
  int assigned_radius_count = 0;
  double assigned_radius_max_error = 0.0;
  nlohmann::json diagnostics = nlohmann::json::array();
};

} // namespace onecad::benchmark
