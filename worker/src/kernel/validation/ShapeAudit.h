#pragma once

#include <algorithm>
#include <string>

#include <TopoDS_Shape.hxx>
#include <TopAbs_ShapeEnum.hxx>

#include "nlohmann/json.hpp"
#include "util/Cancel.h"

namespace onecad::kernel::validation {

enum class PublicationTier { TierA, TierB };

// Structural policy for geometry admitted as a document Body. `SolidSet` permits
// transparent COMPOUND/COMPSOLID wrappers containing only solids; `SingleBody`
// additionally requires exactly one solid occurrence. Neither permits stray
// shells/faces/wires/edges/vertices beside the solid payload.
enum class TopLevelShapePolicy { Any, SolidSet, SingleBody };

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
  // Number of non-solid topological children found outside the mapped solids.
  // A compound containing one valid solid plus a stray face is not a valid Body.
  int stray_topology_count = 0;
  bool structure_checked = false;
  double volume = 0.0;
  bool volume_checked = false;
  int self_interference_count = 0;
  bool self_interference_checked = false;
  int open_edge_count = 0;
  int non_manifold_edge_count = 0;
  bool manifold_checked = false;
  int micro_edge_count = 0;
  int sliver_face_count = 0;
  // Degenerate edges (sphere poles, cone apices) counted SEPARATELY from the
  // micro count. They carry zero length by construction and are legal topology,
  // so folding them into `micro_edge_count` would refuse every sphere and every
  // cone-apex revolve.
  int degenerate_edge_count = 0;
  // Absolute, actionable measurements in mm — the yardstick `micro_edge_count`
  // and `sliver_face_count` are now taken with. `minimum_face_width` is the
  // face's minimum width `2*area/perimeter`, not its area: the pathology is a
  // THIN face, and a 1000 mm x 1e-7 mm sliver has a perfectly ordinary area.
  double minimum_edge_length = 0.0;
  double minimum_face_width = 0.0;
  // Legacy evidence, gated on by nothing. A ratio to the GLOBAL bounding-box
  // diagonal judges a 0.01 mm feature by the size of the 10 m part it sits on;
  // see the note over `collect_micro_topology`.
  double minimum_edge_ratio = 0.0;
  double minimum_face_ratio = 0.0;
  double scale_diagonal = 0.0;
  bool micro_topology_checked = false;
  ShapeTolerances tolerances;
  bool tolerances_checked = false;
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
  bool require_finite_tolerances = true;
  // Disabled when negative. Operations may set an absolute ceiling after their
  // construction/input tolerance budget has been characterized.
  double maximum_tolerance = -1.0;
  // Micro-topology bounds. ALL disabled when negative, and that default is
  // load-bearing: `PublicationPolicy` is default-constructed at sites that must
  // stay ungated (`ExtrudeOp`'s boolean policy, `PlanExecutor`'s global
  // publication invariant, `HoleOp`, `RevolveOp`, `BooleanOp`), so an enabled
  // default would silently gate every imported body. Enabling one requires the
  // census in `test_micro_topology_census.cpp` to be clean for that operation.
  //
  // Deliberately absent: a ratio-to-global-diagonal bound. The ratio is the
  // broken mechanism, not the fix.
  int max_micro_edge_count = -1;
  int max_sliver_face_count = -1;
  double minimum_edge_length = -1.0;
  double minimum_face_width = -1.0;
  PublicationTier tier = PublicationTier::TierA;
  bool require_closed_manifold = false;
  bool allow_empty_lifecycle = false;
};

struct PublicationDecision {
  PublicationDisposition disposition = PublicationDisposition::Refused;
  // The SCHEMA §8 taxonomy code (`GEOMETRY_INVALID` on every refusal today).
  std::string code;
  // SIBLING of `code`, never a replacement: the fine-grained machine-readable
  // reason (SCHEMA §7.2 `diagnostics[].reasonCode`), SCREAMING_SNAKE and 1:1
  // with the `refuse()` branches below. Empty on Publishable/LifecycleOnly.
  // `code` stays the closed §8 enum Rust parses; an unknown value there would
  // fail the whole frame, which is why the reason rides its own field.
  std::string reason_code;
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

// `cancel` (optional) makes the Tier B self-interference pass interruptible: a
// cooperative cancel mid-`BRepAlgoAPI_Check` returns evidence whose `error` names
// the cancellation, and the caller maps its op to CANCELLED (kernel-hardening WP-E).
ShapeEvidence collect_shape_evidence(const TopoDS_Shape &shape,
                                     PublicationTier tier = PublicationTier::TierB,
                                     const onecad::CancelToken *cancel = nullptr);

// Compatibility name for existing diagnostics and benchmark callers.
inline ShapeEvidence audit_shape(const TopoDS_Shape &shape) {
  return collect_shape_evidence(shape);
}

} // namespace onecad::kernel::validation
