#pragma once

#include <algorithm>
#include <string>

#include <TopoDS_Shape.hxx>
#include <TopAbs_ShapeEnum.hxx>

#include "nlohmann/json.hpp"

namespace onecad::kernel::validation {

enum class PublicationTier { TierA, TierB };

enum class TopLevelShapePolicy { Any, SolidLike };

enum class PublicationDisposition { Publishable, LifecycleOnly, Refused };

struct ShapeTolerances {
  double face = 0.0;
  double edge = 0.0;
  double vertex = 0.0;

  double maximum() const { return std::max({face, edge, vertex}); }
  nlohmann::json to_json() const;
};

// Geometry facts collected once and evaluated by operation policy. Keep this
// separate from a verdict: Boolean and legacy Hole intentionally have different
// lifecycle rules for the same empty result.
struct ShapeEvidence {
  bool null_shape = true;
  bool brep_valid = false;
  TopAbs_ShapeEnum top_level_shape = TopAbs_SHAPE;
  int solid_count = 0;
  double volume = 0.0;
  int self_interference_count = 0;
  bool self_interference_checked = false;
  int open_edge_count = 0;
  int non_manifold_edge_count = 0;
  bool manifold_checked = false;
  int micro_edge_count = 0;
  int sliver_face_count = 0;
  double minimum_edge_ratio = 0.0;
  double minimum_face_ratio = 0.0;
  double scale_diagonal = 0.0;
  bool micro_topology_checked = false;
  ShapeTolerances tolerances;
  double validator_duration_ms = 0.0;
  std::string error;

  bool publishable() const;
  nlohmann::json to_json() const;
};

using ShapeAuditResult = ShapeEvidence;

struct PublicationPolicy {
  std::string name;
  TopLevelShapePolicy allowed_top_level_shapes = TopLevelShapePolicy::Any;
  int min_solid_count = 1;
  int max_solid_count = 1;  // -1 permits any count above the minimum.
  bool require_positive_volume = true;
  bool require_brep_valid = true;
  PublicationTier tier = PublicationTier::TierA;
  bool allow_empty_lifecycle = false;
};

struct PublicationDecision {
  PublicationDisposition disposition = PublicationDisposition::Refused;
  std::string code;
  std::string message;
  ShapeEvidence evidence;

  struct Timings {
    double build_ms = 0.0;
    double validator_ms = 0.0;

    nlohmann::json to_json() const;
  } timings;

  bool publishable() const {
    return disposition == PublicationDisposition::Publishable;
  }
  bool lifecycle_only() const {
    return disposition == PublicationDisposition::LifecycleOnly;
  }
  nlohmann::json to_json() const;
};

PublicationPolicy single_solid_policy(std::string name, PublicationTier tier);
PublicationDecision evaluate_publication_policy(const ShapeEvidence &evidence,
                                                 const PublicationPolicy &policy);

ShapeEvidence collect_shape_evidence(const TopoDS_Shape &shape,
                                     PublicationTier tier = PublicationTier::TierB);

// Compatibility name for existing diagnostics and benchmark callers.
inline ShapeEvidence audit_shape(const TopoDS_Shape &shape) {
  return collect_shape_evidence(shape);
}

} // namespace onecad::kernel::validation
