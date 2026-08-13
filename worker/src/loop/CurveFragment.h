// Exact analytic provenance for one planarized graph edge.
#pragma once

#include "../sketch/SketchTypes.h"

#include <vector>

namespace onecad::core::loop {

namespace sk = onecad::core::sketch;

enum class CurveFragmentKind {
    Line,
    Circle,
    Arc,
    Ellipse,
};

enum class CurveRefinementPolicy {
    V2ParameterProximity,
    V3PhysicalProximity,
};

struct CurveFragment {
    sk::EntityID baseEntityId;
    CurveFragmentKind kind = CurveFragmentKind::Line;
    // Parameters remain in the base curve's unwrapped domain. This keeps
    // circle/ellipse fragments crossing the 2π seam unambiguous.
    double firstParameter = 0.0;
    double lastParameter = 0.0;
    // The base entity's authored range. Identity normalizes the fragment range
    // against these values, never against tessellation or graph-edge ordinals.
    double sourceFirstParameter = 0.0;
    double sourceLastParameter = 0.0;
    // Traversal orientation is stored with the fragment so analytic wire
    // construction does not depend on synthetic graph-edge ordering.
    bool forward = true;
    sk::Vec2d startPoint{};
    sk::Vec2d endPoint{};
    // Approximate points only support planar graph traversal and containment.
    // FaceBuilder never consumes them for BRep construction.
    std::vector<sk::Vec2d> samples;
};

}  // namespace onecad::core::loop
