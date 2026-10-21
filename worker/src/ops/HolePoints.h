// HolePoints.h — ordered multi-point Hole placement (SCHEMA §7.3 `Hole.points`,
// `resultPolicyVersion: 3`, ONECAD-361).
//
// A V3 record carries `points: [{id, at:[x,y,z]}, ...]`; `point` mirrors
// `points[0].at` for old readers and is otherwise ignored. Every placement is
// pre-flighted in index order (shape, projection, fence, face classification,
// pairwise spacing) before ANY tool is built, so a bad point refuses the whole
// feature and the predecessor stays published. The per-point tools are disjoint
// by the spacing rule, so one compound cut drills them all and every born
// element lies within exactly one tool radius of one origin — which is how its
// topology-ledger claim gets the point `id` as its slot.
#pragma once

#include <optional>
#include <string>
#include <vector>

#include <TopoDS_Face.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>

#include "nlohmann/json.hpp"
#include "ops/HoleTool.h"
#include "ops/OpTypes.h"
#include "session/TopologyOrigins.h"

namespace onecad::ops {

inline constexpr std::size_t kHoleMaxPoints = 64;
// Coincidence threshold and spacing margin on PROJECTED origins (SCHEMA §7.3).
inline constexpr double kHolePointGap = 1e-3;
inline constexpr std::size_t kHolePointIdMaxBytes = 64;

struct HolePointSpec {
    std::string id;  // empty for a legacy/V2 single point
    gp_Pnt frozen;
};

// A V3 refusal: top-level recoverable OP_FAILED plus the WP-T1 diagnostic
// `{severity, code:"OP_FAILED", message, stage, reasonCode, evidence}`.
OpOutcome hole_point_refusal(const char* reason_code, const std::string& message,
                             nlohmann::json evidence);

// V3 preflight of `points` and the `point` mirror. `nullopt` = valid, `out`
// filled in index order.
std::optional<OpOutcome> read_hole_points(const nlohmann::json& params,
                                          std::vector<HolePointSpec>& out);

// Re-project `frozen` onto `face`, fence it at 1e-3 mm and require it inside
// the face boundary. Empty return = `origin` set. Otherwise the legacy message,
// with `reason_code` naming the V3 reasonCode.
std::string place_hole_point(const TopoDS_Face& face, const gp_Pnt& frozen, gp_Pnt& origin,
                             const char*& reason_code);

// The widest radius of one hole's tool (`HoleTool.cpp`): drill, counterbore, or
// the countersink frustum at its overshoot start.
double hole_tool_radius(const HoleDims& dims);

// Pairwise spacing on projected origins: refusal or `nullopt`.
std::optional<OpOutcome> check_hole_spacing(const std::vector<gp_Pnt>& origins, double radius);

// Stamp each birth whose representative point lies within `radius` of exactly
// one hole axis with that point's id.
void attribute_hole_births(session::TopologyBodyHistory& history,
                           const std::vector<gp_Pnt>& origins, const gp_Dir& axis,
                           double radius, const std::vector<HolePointSpec>& specs);

}  // namespace onecad::ops
