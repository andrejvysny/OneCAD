#pragma once

#include <cstddef>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include <TopoDS_Edge.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>
#include <nlohmann/json.hpp>

namespace onecad::benchmark {

struct GeneratorSpec {
  // Absent in case-v1, which predates the operation-family axis; the v1 parse
  // fills it with the only family that existed.
  std::string family = "fillet";
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
  bool has_provenance_feature_index = false;
  std::vector<std::string> anchor_kinds;
  std::vector<std::vector<double>> anchor_points;
  std::vector<std::string> curve_kinds;
  std::vector<std::vector<std::string>> adjacent_surface_kinds;
  std::string adjacency_relation;
  // v2 only; empty / absent for every v1 case.
  std::string convexity;
  std::optional<int> vertex_valence;
};

/// One support surface of a `supportPair` recipe. Which optionals are populated
/// is a function of `kind` and is enforced at parse time, mirroring
/// `SupportSurface::validate` on the Rust side.
struct SupportSpec {
  std::string kind;
  std::optional<double> radius;
  std::optional<double> minor_radius;
  std::optional<double> half_angle_degrees;
  std::optional<int> degree;
  std::optional<int> control_points;
};

/// The typed per-recipe parameter union. A free-form bag would let a typo
/// silently generate different geometry, which is the one failure mode a
/// fuzzing corpus cannot survive, so every field is named and range-checked.
struct RecipeParameters {
  std::vector<double> dimensions; // box / valence / wedge recipes
  std::size_t feature_index = 0;
  bool has_feature_index = false;
  std::optional<SupportSpec> support_a;
  std::optional<SupportSpec> support_b;
  std::optional<double> dihedral_degrees;
  std::optional<int> valence;
  std::string convexity;
  std::optional<double> edge_length;
  std::optional<double> neighbor_size;
  std::optional<double> seam_offset;
  std::optional<double> consumption_ratio;
  std::optional<double> separation;
};

/// The requested radius law. Only `constant` is executable today — variable
/// radius arrives with a validator that can prove the produced blend follows
/// the law, because an option is not a capability.
struct RadiusLaw {
  std::string mode = "constant";
  double radius = 0.0;
  double start_radius = 0.0;
  double end_radius = 0.0;
  std::vector<std::pair<double, double>> control_points;

  double max_radius() const {
    if (mode == "constant")
      return radius;
    if (mode == "linear")
      return start_radius > end_radius ? start_radius : end_radius;
    double peak = 0.0;
    for (const auto &point : control_points)
      peak = point.second > peak ? point.second : peak;
    return peak;
  }
};

struct CaseSpec {
  nlohmann::json canonical;
  int schema_version = 1;
  std::string case_id;
  GeneratorSpec generator;
  std::string recipe;
  RecipeParameters parameters;
  std::vector<std::string> geometry_tags;
  // Reporting label only. The band never scales anything here: the case file
  // carries already-scaled lengths, so a scaled case is fully explicit and the
  // generator cannot disagree with the numbers the selector anchors on.
  std::string scale_band;
  RadiusLaw radius_law;
  // The executable radius. Equal to the constant law's radius; a non-constant
  // law is refused at parse time, so consumers can read this unconditionally.
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
  std::vector<double> mirror_normal;
  std::vector<double> mirror_center;
  double scale_factor = 1.0;
  std::vector<double> scale_center;
  std::string epsilon_parameter;
  double epsilon_relative_delta = 0.0;
  std::vector<std::size_t> edge_order;
  std::size_t contour_seed = 0;
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
