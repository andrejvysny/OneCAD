// Ported from OneCAD-CPP src/core/loop/AdjacencyGraph.h @ b4ddcccc (2026-07-16)
#ifndef ONECAD_CORE_LOOP_ADJACENCY_GRAPH_H
#define ONECAD_CORE_LOOP_ADJACENCY_GRAPH_H

#include "../sketch/SketchTypes.h"
#include "CurveFragment.h"
#include "DetectionWarning.h"

#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

namespace onecad::core::loop {

namespace sk = onecad::core::sketch;

struct GraphNode {
    sk::EntityID id;
    sk::Vec2d position;
    std::vector<int> edges;
    std::vector<sk::EntityID> pointIds;
};

struct GraphEdge {
    sk::EntityID entityId;
    int startNode = -1;
    int endNode = -1;
    bool isArc = false;
    bool isCircle = false;
    sk::Vec2d startPos{};
    sk::Vec2d endPos{};
    sk::Vec2d startTangent{};
    sk::Vec2d endTangent{};
    sk::Vec2d centerPos{};
    double radius = 0.0;
    double startAngle = 0.0;
    double endAngle = 0.0;
    std::optional<CurveFragment> fragment;
};

struct AdjacencyGraph {
    std::vector<GraphNode> nodes;
    std::vector<GraphEdge> edges;
    std::unordered_map<sk::EntityID, int> nodeByPointId;
    std::unordered_map<sk::EntityID, int> edgeByEntity;
    std::string errorMessage;
    /// Entities dropped during graph construction; never a refusal.
    std::vector<DetectionWarning> warnings;
    /// Curve pairs kept by the broad-phase cull during exact refinement.
    std::size_t curvePairs = 0;
    std::vector<CurveFragment> closedCurves;

    int findOrCreateNode(const sk::Vec2d& pos,
                         const std::optional<sk::EntityID>& pointId,
                         double tolerance);
};

} // namespace onecad::core::loop

#endif  // ONECAD_CORE_LOOP_ADJACENCY_GRAPH_H
