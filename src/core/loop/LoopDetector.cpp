#include "LoopDetector.h"

#include "AdjacencyGraph.h"

#include "../sketch/Sketch.h"
#include "../sketch/SketchArc.h"
#include "../sketch/SketchCircle.h"
#include "../sketch/SketchLine.h"
#include "../sketch/SketchPoint.h"

#include <algorithm>
#include <cmath>
#include <functional>
#include <limits>
#include <numbers>
#include <optional>
#include <unordered_map>
#include <unordered_set>

namespace onecad::core::loop {

constexpr double kMinArea = 1e-6;

sk::Vec2d toVec2(const gp_Pnt2d& p) {
    return {p.X(), p.Y()};
}

double distanceSquared(const sk::Vec2d& a, const sk::Vec2d& b) {
    double dx = a.x - b.x;
    double dy = a.y - b.y;
    return dx * dx + dy * dy;
}

void appendArcSamples(std::vector<sk::Vec2d>& points,
                      const sk::Vec2d& center,
                      double radius,
                      double startAngle,
                      double endAngle,
                      bool forward) {
    double sweep = endAngle - startAngle;
    if (forward) {
        if (sweep < 0.0) {
            sweep += 2.0 * std::numbers::pi_v<double>;
        }
    } else {
        if (sweep > 0.0) {
            sweep -= 2.0 * std::numbers::pi_v<double>;
        }
    }

    double absSweep = std::abs(sweep);
    int segments = std::max(8, static_cast<int>(std::ceil(absSweep / (std::numbers::pi_v<double> / 8.0))));
    double step = sweep / static_cast<double>(segments);

    for (int i = 1; i <= segments; ++i) {
        double angle = startAngle + step * static_cast<double>(i);
        points.push_back({center.x + radius * std::cos(angle),
                          center.y + radius * std::sin(angle)});
    }
}

std::string makeCycleKey(const std::vector<sk::EntityID>& edges) {
    std::vector<sk::EntityID> sorted = edges;
    std::sort(sorted.begin(), sorted.end());
    std::string key;
    key.reserve(sorted.size() * 40);
    for (const auto& id : sorted) {
        key.append(id);
        key.push_back('|');
    }
    return key;
}

bool segmentsIntersect(const sk::Vec2d& a1, const sk::Vec2d& a2,
                       const sk::Vec2d& b1, const sk::Vec2d& b2) {
    auto cross = [](const sk::Vec2d& a, const sk::Vec2d& b, const sk::Vec2d& c) {
        return (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    };

    auto onSegment = [](const sk::Vec2d& a, const sk::Vec2d& b, const sk::Vec2d& c) {
        return std::min(a.x, b.x) <= c.x && c.x <= std::max(a.x, b.x) &&
               std::min(a.y, b.y) <= c.y && c.y <= std::max(a.y, b.y);
    };

    double d1 = cross(a1, a2, b1);
    double d2 = cross(a1, a2, b2);
    double d3 = cross(b1, b2, a1);
    double d4 = cross(b1, b2, a2);

    if (((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) &&
        ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0))) {
        return true;
    }

    if (d1 == 0.0 && onSegment(a1, a2, b1)) return true;
    if (d2 == 0.0 && onSegment(a1, a2, b2)) return true;
    if (d3 == 0.0 && onSegment(b1, b2, a1)) return true;
    if (d4 == 0.0 && onSegment(b1, b2, a2)) return true;

    return false;
}

double distanceToSegmentSquared(const sk::Vec2d& a, const sk::Vec2d& b, const sk::Vec2d& p) {
    sk::Vec2d ab{b.x - a.x, b.y - a.y};
    double denom = ab.x * ab.x + ab.y * ab.y;
    if (denom == 0.0) {
        return distanceSquared(a, p);
    }
    double t = ((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / denom;
    t = std::clamp(t, 0.0, 1.0);
    sk::Vec2d proj{a.x + t * ab.x, a.y + t * ab.y};
    return distanceSquared(p, proj);
}

bool isPointInPolygonOrOnEdge(const sk::Vec2d& point,
                              const std::vector<sk::Vec2d>& polygon,
                              double tolerance) {
    if (isPointInPolygon(point, polygon)) {
        return true;
    }
    if (polygon.size() < 2) {
        return false;
    }
    double tol2 = tolerance * tolerance;
    for (size_t i = 0; i < polygon.size(); ++i) {
        size_t next = (i + 1) % polygon.size();
        if (distanceToSegmentSquared(polygon[i], polygon[next], point) <= tol2) {
            return true;
        }
    }
    return false;
}

bool loopContainsLoop(const Loop& outer, const Loop& inner, double tolerance) {
    if (outer.polygon.size() < 3 || inner.polygon.size() < 3) {
        return false;
    }
    if (inner.boundsMin.x < outer.boundsMin.x - tolerance ||
        inner.boundsMin.y < outer.boundsMin.y - tolerance ||
        inner.boundsMax.x > outer.boundsMax.x + tolerance ||
        inner.boundsMax.y > outer.boundsMax.y + tolerance) {
        return false;
    }
    for (const auto& p : inner.polygon) {
        if (!isPointInPolygonOrOnEdge(p, outer.polygon, tolerance)) {
            return false;
        }
    }
    return true;
}

void reverseLoop(Loop& loop) {
    std::reverse(loop.wire.edges.begin(), loop.wire.edges.end());
    std::reverse(loop.wire.forward.begin(), loop.wire.forward.end());
    for (size_t i = 0; i < loop.wire.forward.size(); ++i) {
        loop.wire.forward[i] = !loop.wire.forward[i];
    }
    std::reverse(loop.polygon.begin(), loop.polygon.end());
    loop.signedArea = -loop.signedArea;
}

LoopDetector::LoopDetector()
    : config_() {
}

LoopDetector::LoopDetector(const LoopDetectorConfig& config)
    : config_(config) {
}

LoopDetectionResult LoopDetector::detect(const sk::Sketch& sketch) const {
    return detect(sketch, {});
}

LoopDetectionResult LoopDetector::detect(const sk::Sketch& sketch,
                                         const std::vector<sk::EntityID>& selectedEntities) const {
    LoopDetectionResult result;

    std::unordered_set<sk::EntityID> selection;
    if (!selectedEntities.empty()) {
        selection.insert(selectedEntities.begin(), selectedEntities.end());
    }

    auto graph = buildGraph(sketch, selection.empty() ? nullptr : &selection);
    if (!graph) {
        result.success = false;
        result.errorMessage = "Failed to build adjacency graph";
        return result;
    }

    std::vector<Loop> loops;
    std::unordered_set<sk::EntityID> edgesInLoops;

    std::vector<Wire> cycles = findCycles(*graph);
    for (auto& wire : cycles) {
        if (config_.maxLoops > 0 && loops.size() >= config_.maxLoops) {
            break;
        }

        Loop loop;
        loop.wire = wire;
        computeLoopProperties(loop, sketch);

        if (config_.validate && !validateLoop(loop, sketch)) {
            if (!config_.findAllLoops) {
                continue;
            }
        }

        if (!config_.findAllLoops && std::abs(loop.signedArea) < kMinArea) {
            continue;
        }

        loops.push_back(loop);
        for (const auto& id : loop.wire.edges) {
            edgesInLoops.insert(id);
        }
    }

    for (const auto& entity : sketch.getAllEntities()) {
        if (!entity || entity->isConstruction()) {
            continue;
        }
        if (!selection.empty() && selection.find(entity->id()) == selection.end()) {
            continue;
        }
        if (entity->type() != sk::EntityType::Circle) {
            continue;
        }
        if (edgesInLoops.find(entity->id()) != edgesInLoops.end()) {
            continue;
        }

        Loop circleLoop;
        circleLoop.wire.edges.push_back(entity->id());
        circleLoop.wire.forward.push_back(true);
        circleLoop.wire.startPoint = entity->id();
        circleLoop.wire.endPoint = entity->id();
        computeLoopProperties(circleLoop, sketch);

        if (config_.validate && !validateLoop(circleLoop, sketch)) {
            if (!config_.findAllLoops) {
                continue;
            }
        }

        loops.push_back(circleLoop);
        edgesInLoops.insert(entity->id());
    }

    result.totalLoopsFound = static_cast<int>(loops.size());

    if (config_.resolveHoles) {
        result.faces = buildFaceHierarchy(std::move(loops));
        for (const auto& face : result.faces) {
            if (!face.innerLoops.empty()) {
                result.facesWithHoles++;
            }
        }
    } else {
        for (auto& loop : loops) {
            Face face;
            face.outerLoop = std::move(loop);
            result.faces.push_back(std::move(face));
        }
    }

    std::unordered_set<int> usedEdges;
    for (const auto& face : result.faces) {
        for (const auto& edge : face.outerLoop.wire.edges) {
            auto it = graph->edgeByEntity.find(edge);
            if (it != graph->edgeByEntity.end()) {
                usedEdges.insert(it->second);
            }
        }
        for (const auto& hole : face.innerLoops) {
            for (const auto& edge : hole.wire.edges) {
                auto it = graph->edgeByEntity.find(edge);
                if (it != graph->edgeByEntity.end()) {
                    usedEdges.insert(it->second);
                }
            }
        }
    }

    std::unordered_set<int> openUsed;
    for (size_t i = 0; i < graph->nodes.size(); ++i) {
        if (graph->nodes[i].edges.size() != 1) {
            continue;
        }
        int current = static_cast<int>(i);
        int prevEdge = -1;
        Wire wire;
        while (true) {
            int nextEdge = -1;
            for (int edgeIndex : graph->nodes[current].edges) {
                if (edgeIndex == prevEdge || usedEdges.count(edgeIndex) || openUsed.count(edgeIndex)) {
                    continue;
                }
                nextEdge = edgeIndex;
                break;
            }
            if (nextEdge == -1) {
                break;
            }

            const auto& edge = graph->edges[nextEdge];
            int nextNode = (edge.startNode == current) ? edge.endNode : edge.startNode;
            wire.edges.push_back(edge.entityId);
            wire.forward.push_back(edge.startNode == current && edge.endNode == nextNode);
            openUsed.insert(nextEdge);
            prevEdge = nextEdge;
            current = nextNode;
        }

        if (!wire.edges.empty()) {
            wire.startPoint = graph->nodes[i].id;
            wire.endPoint = graph->nodes[current].id;
            result.openWires.push_back(std::move(wire));
        }
    }

    for (size_t edgeIndex = 0; edgeIndex < graph->edges.size(); ++edgeIndex) {
        if (usedEdges.count(static_cast<int>(edgeIndex)) || openUsed.count(static_cast<int>(edgeIndex))) {
            continue;
        }
        result.unusedEdges.push_back(graph->edges[edgeIndex].entityId);
    }

    for (const auto& entity : sketch.getAllEntities()) {
        if (!entity || entity->type() != sk::EntityType::Point) {
            continue;
        }
        auto* point = dynamic_cast<const sk::SketchPoint*>(entity.get());
        if (!point) {
            continue;
        }

        bool referenced = false;
        for (const auto& e : sketch.getAllEntities()) {
            if (!e || e->isConstruction()) {
                continue;
            }
            if (e->type() == sk::EntityType::Line) {
                auto* line = dynamic_cast<const sk::SketchLine*>(e.get());
                if (line && (line->startPointId() == point->id() || line->endPointId() == point->id())) {
                    referenced = true;
                    break;
                }
            } else if (e->type() == sk::EntityType::Arc) {
                auto* arc = dynamic_cast<const sk::SketchArc*>(e.get());
                if (arc && arc->centerPointId() == point->id()) {
                    referenced = true;
                    break;
                }
            } else if (e->type() == sk::EntityType::Circle) {
                auto* circle = dynamic_cast<const sk::SketchCircle*>(e.get());
                if (circle && circle->centerPointId() == point->id()) {
                    referenced = true;
                    break;
                }
            }
        }

        if (!referenced) {
            result.isolatedPoints.push_back(point->id());
        }
    }

    return result;
}

std::optional<Face> LoopDetector::findLoopAtPoint(const sk::Sketch& sketch,
                                                  const sk::Vec2d& point) const {
    LoopDetectionResult result = detect(sketch);
    double bestArea = std::numeric_limits<double>::infinity();
    std::optional<Face> bestFace;

    for (const auto& face : result.faces) {
        if (!face.outerLoop.contains(point)) {
            continue;
        }
        bool inHole = false;
        for (const auto& hole : face.innerLoops) {
            if (hole.contains(point)) {
                inHole = true;
                break;
            }
        }
        if (inHole) {
            continue;
        }
        double area = face.outerLoop.area();
        if (area < bestArea) {
            bestArea = area;
            bestFace = face;
        }
    }

    return bestFace;
}

bool LoopDetector::isClosedLoop(const sk::Sketch& sketch,
                                const std::vector<sk::EntityID>& entities) const {
    auto wire = buildWire(sketch, entities);
    return wire && wire->isClosed();
}

std::optional<Wire> LoopDetector::buildWire(const sk::Sketch& sketch,
                                            const std::vector<sk::EntityID>& entities) const {
    if (entities.empty()) {
        return std::nullopt;
    }

    std::unordered_set<sk::EntityID> selection(entities.begin(), entities.end());
    auto graph = buildGraph(sketch, &selection);
    if (!graph) {
        return std::nullopt;
    }

    std::vector<int> edgeIndices;
    edgeIndices.reserve(entities.size());
    for (const auto& id : entities) {
        auto it = graph->edgeByEntity.find(id);
        if (it != graph->edgeByEntity.end()) {
            edgeIndices.push_back(it->second);
        }
    }

    if (edgeIndices.empty()) {
        return std::nullopt;
    }

    int startNode = -1;
    for (int edgeIndex : edgeIndices) {
        const auto& edge = graph->edges[edgeIndex];
        if (graph->nodes[edge.startNode].edges.size() == 1) {
            startNode = edge.startNode;
            break;
        }
        if (graph->nodes[edge.endNode].edges.size() == 1) {
            startNode = edge.endNode;
            break;
        }
    }
    if (startNode < 0) {
        startNode = graph->edges[edgeIndices.front()].startNode;
    }

    std::unordered_set<int> allowed(edgeIndices.begin(), edgeIndices.end());
    std::unordered_set<int> visited;

    Wire wire;
    int current = startNode;
    int prevEdge = -1;
    while (true) {
        int nextEdge = -1;
        for (int edgeIndex : graph->nodes[current].edges) {
            if (edgeIndex == prevEdge || !allowed.count(edgeIndex) || visited.count(edgeIndex)) {
                continue;
            }
            nextEdge = edgeIndex;
            break;
        }
        if (nextEdge == -1) {
            break;
        }

        const auto& edge = graph->edges[nextEdge];
        int nextNode = (edge.startNode == current) ? edge.endNode : edge.startNode;
        wire.edges.push_back(edge.entityId);
        wire.forward.push_back(edge.startNode == current && edge.endNode == nextNode);
        visited.insert(nextEdge);
        prevEdge = nextEdge;
        current = nextNode;
    }

    if (visited.size() != allowed.size()) {
        return std::nullopt;
    }

    wire.startPoint = graph->nodes[startNode].id;
    wire.endPoint = graph->nodes[current].id;
    return wire;
}

std::unique_ptr<AdjacencyGraph> LoopDetector::buildGraph(
    const sk::Sketch& sketch,
    const std::unordered_set<sk::EntityID>* selection) const {
    auto graph = std::make_unique<AdjacencyGraph>();
    double tolerance = config_.coincidenceTolerance;

    for (const auto& entity : sketch.getAllEntities()) {
        if (!entity || entity->isConstruction()) {
            continue;
        }
        if (selection && !selection->empty() && selection->find(entity->id()) == selection->end()) {
            continue;
        }

        if (entity->type() == sk::EntityType::Line) {
            auto* line = dynamic_cast<const sk::SketchLine*>(entity.get());
            if (!line) {
                continue;
            }
            auto* start = sketch.getEntityAs<sk::SketchPoint>(line->startPointId());
            auto* end = sketch.getEntityAs<sk::SketchPoint>(line->endPointId());
            if (!start || !end) {
                continue;
            }

            sk::Vec2d startPos = toVec2(start->position());
            sk::Vec2d endPos = toVec2(end->position());

            int startNode = graph->findOrCreateNode(startPos, line->startPointId(), tolerance);
            int endNode = graph->findOrCreateNode(endPos, line->endPointId(), tolerance);

            GraphEdge edge;
            edge.entityId = line->id();
            edge.startNode = startNode;
            edge.endNode = endNode;
            edge.startPos = startPos;
            edge.endPos = endPos;
            graph->edgeByEntity[edge.entityId] = static_cast<int>(graph->edges.size());
            graph->edges.push_back(std::move(edge));

            graph->nodes[startNode].edges.push_back(static_cast<int>(graph->edges.size() - 1));
            graph->nodes[endNode].edges.push_back(static_cast<int>(graph->edges.size() - 1));
        } else if (entity->type() == sk::EntityType::Arc) {
            auto* arc = dynamic_cast<const sk::SketchArc*>(entity.get());
            if (!arc) {
                continue;
            }
            auto* centerPoint = sketch.getEntityAs<sk::SketchPoint>(arc->centerPointId());
            if (!centerPoint) {
                continue;
            }

            gp_Pnt2d centerPos = centerPoint->position();
            gp_Pnt2d arcStart = arc->startPoint(centerPos);
            gp_Pnt2d arcEnd = arc->endPoint(centerPos);

            sk::Vec2d startPos = toVec2(arcStart);
            sk::Vec2d endPos = toVec2(arcEnd);

            int startNode = graph->findOrCreateNode(startPos, std::nullopt, tolerance);
            int endNode = graph->findOrCreateNode(endPos, std::nullopt, tolerance);

            GraphEdge edge;
            edge.entityId = arc->id();
            edge.isArc = true;
            edge.startNode = startNode;
            edge.endNode = endNode;
            edge.startPos = startPos;
            edge.endPos = endPos;
            edge.centerPos = toVec2(centerPos);
            edge.radius = arc->radius();
            edge.startAngle = arc->startAngle();
            edge.endAngle = arc->endAngle();
            graph->edgeByEntity[edge.entityId] = static_cast<int>(graph->edges.size());
            graph->edges.push_back(std::move(edge));

            graph->nodes[startNode].edges.push_back(static_cast<int>(graph->edges.size() - 1));
            graph->nodes[endNode].edges.push_back(static_cast<int>(graph->edges.size() - 1));
        }
    }

    return graph;
}

std::vector<Wire> LoopDetector::findCycles(const AdjacencyGraph& graphBase) const {
    const auto& graph = graphBase;
    std::vector<Wire> cycles;
    std::unordered_set<std::string> seen;

    std::vector<int> pathNodes;
    std::vector<int> pathEdges;
    std::unordered_set<int> visitedNodes;

    std::function<void(int, int, int)> dfs = [&](int start, int current, int parentEdge) {
        for (int edgeIndex : graph.nodes[current].edges) {
            if (edgeIndex == parentEdge) {
                continue;
            }
            const auto& edge = graph.edges[edgeIndex];
            int next = (edge.startNode == current) ? edge.endNode : edge.startNode;
            if (next == start && pathEdges.size() >= 1) {
                std::vector<int> cycleEdges = pathEdges;
                cycleEdges.push_back(edgeIndex);
                std::vector<sk::EntityID> cycleEdgeIds;
                cycleEdgeIds.reserve(cycleEdges.size());
                for (int idx : cycleEdges) {
                    cycleEdgeIds.push_back(graph.edges[idx].entityId);
                }
                std::string key = makeCycleKey(cycleEdgeIds);
                if (seen.insert(key).second) {
                    Wire wire;
                    std::vector<int> cycleNodes = pathNodes;
                    cycleNodes.push_back(start);
                    for (size_t i = 0; i < cycleEdges.size(); ++i) {
                        int edgeIdx = cycleEdges[i];
                        int from = cycleNodes[i];
                        int to = cycleNodes[i + 1];
                        const auto& edgeRef = graph.edges[edgeIdx];
                        wire.edges.push_back(edgeRef.entityId);
                        wire.forward.push_back(edgeRef.startNode == from && edgeRef.endNode == to);
                    }
                    wire.startPoint = graph.nodes[start].id;
                    wire.endPoint = graph.nodes[start].id;
                    cycles.push_back(std::move(wire));
                }
                continue;
            }

            if (visitedNodes.count(next)) {
                continue;
            }

            visitedNodes.insert(next);
            pathNodes.push_back(next);
            pathEdges.push_back(edgeIndex);
            dfs(start, next, edgeIndex);
            pathEdges.pop_back();
            pathNodes.pop_back();
            visitedNodes.erase(next);
        }
    };

    for (size_t i = 0; i < graph.nodes.size(); ++i) {
        pathNodes.clear();
        pathEdges.clear();
        visitedNodes.clear();
        visitedNodes.insert(static_cast<int>(i));
        pathNodes.push_back(static_cast<int>(i));
        dfs(static_cast<int>(i), static_cast<int>(i), -1);
    }

    return cycles;
}

void LoopDetector::computeLoopProperties(Loop& loop, const sk::Sketch& sketch) const {
    loop.polygon.clear();

    if (loop.wire.edges.empty()) {
        loop.signedArea = 0.0;
        loop.boundsMin = {0.0, 0.0};
        loop.boundsMax = {0.0, 0.0};
        loop.centroid = {0.0, 0.0};
        return;
    }

    bool hasCurrent = false;

    for (size_t i = 0; i < loop.wire.edges.size(); ++i) {
        const auto& edgeId = loop.wire.edges[i];
        bool forward = (i < loop.wire.forward.size()) ? loop.wire.forward[i] : true;

        const auto* entity = sketch.getEntity(edgeId);
        if (!entity) {
            continue;
        }

        if (entity->type() == sk::EntityType::Line) {
            auto* line = sketch.getEntityAs<sk::SketchLine>(edgeId);
            if (!line) {
                continue;
            }
            auto* start = sketch.getEntityAs<sk::SketchPoint>(line->startPointId());
            auto* end = sketch.getEntityAs<sk::SketchPoint>(line->endPointId());
            if (!start || !end) {
                continue;
            }

            sk::Vec2d startPos = toVec2(start->position());
            sk::Vec2d endPos = toVec2(end->position());
            sk::Vec2d from = forward ? startPos : endPos;
            sk::Vec2d to = forward ? endPos : startPos;
            if (!hasCurrent) {
                loop.polygon.push_back(from);
                hasCurrent = true;
            }
            loop.polygon.push_back(to);
        } else if (entity->type() == sk::EntityType::Arc) {
            auto* arc = sketch.getEntityAs<sk::SketchArc>(edgeId);
            if (!arc) {
                continue;
            }
            auto* centerPoint = sketch.getEntityAs<sk::SketchPoint>(arc->centerPointId());
            if (!centerPoint) {
                continue;
            }
            sk::Vec2d centerPos = toVec2(centerPoint->position());
            gp_Pnt2d arcStart = arc->startPoint(centerPoint->position());
            gp_Pnt2d arcEnd = arc->endPoint(centerPoint->position());
            sk::Vec2d from = forward ? toVec2(arcStart) : toVec2(arcEnd);
            sk::Vec2d to = forward ? toVec2(arcEnd) : toVec2(arcStart);
            if (!hasCurrent) {
                loop.polygon.push_back(from);
                hasCurrent = true;
            }
            appendArcSamples(loop.polygon, centerPos, arc->radius(),
                             forward ? arc->startAngle() : arc->endAngle(),
                             forward ? arc->endAngle() : arc->startAngle(),
                             forward);
        } else if (entity->type() == sk::EntityType::Circle) {
            auto* circle = sketch.getEntityAs<sk::SketchCircle>(edgeId);
            if (!circle) {
                continue;
            }
            auto* centerPoint = sketch.getEntityAs<sk::SketchPoint>(circle->centerPointId());
            if (!centerPoint) {
                continue;
            }
            sk::Vec2d centerPos = toVec2(centerPoint->position());
            int segments = 32;
            for (int s = 0; s <= segments; ++s) {
                double angle = (2.0 * std::numbers::pi_v<double> * s) / static_cast<double>(segments);
                loop.polygon.push_back({centerPos.x + circle->radius() * std::cos(angle),
                                        centerPos.y + circle->radius() * std::sin(angle)});
            }
            hasCurrent = true;
        }
    }

    if (!loop.polygon.empty()) {
        std::vector<sk::Vec2d> filtered;
        filtered.reserve(loop.polygon.size());
        filtered.push_back(loop.polygon.front());
        double tol2 = config_.coincidenceTolerance * config_.coincidenceTolerance;
        for (size_t i = 1; i < loop.polygon.size(); ++i) {
            if (distanceSquared(filtered.back(), loop.polygon[i]) > tol2) {
                filtered.push_back(loop.polygon[i]);
            }
        }
        loop.polygon.swap(filtered);
    }

    if (loop.polygon.size() >= 3) {
        loop.signedArea = computeSignedArea(loop.polygon);
        loop.centroid = computeCentroid(loop.polygon);
        loop.boundsMin = {loop.polygon.front().x, loop.polygon.front().y};
        loop.boundsMax = loop.boundsMin;
        for (const auto& p : loop.polygon) {
            loop.boundsMin.x = std::min(loop.boundsMin.x, p.x);
            loop.boundsMin.y = std::min(loop.boundsMin.y, p.y);
            loop.boundsMax.x = std::max(loop.boundsMax.x, p.x);
            loop.boundsMax.y = std::max(loop.boundsMax.y, p.y);
        }
    } else {
        loop.signedArea = 0.0;
        loop.centroid = {0.0, 0.0};
        loop.boundsMin = {0.0, 0.0};
        loop.boundsMax = {0.0, 0.0};
    }
}

std::vector<Face> LoopDetector::buildFaceHierarchy(std::vector<Loop> loops) const {
    std::vector<Face> faces;
    if (loops.empty()) {
        return faces;
    }

    std::vector<size_t> order(loops.size());
    for (size_t i = 0; i < loops.size(); ++i) {
        order[i] = i;
    }
    std::sort(order.begin(), order.end(), [&](size_t a, size_t b) {
        return loops[a].area() > loops[b].area();
    });

    std::vector<int> parent(loops.size(), -1);
    std::vector<int> depth(loops.size(), 0);

    for (size_t i = 0; i < loops.size(); ++i) {
        size_t loopIdx = order[i];
        double bestArea = std::numeric_limits<double>::infinity();
        int bestParent = -1;

        for (size_t j = 0; j < loops.size(); ++j) {
            size_t candidateIdx = order[j];
            if (candidateIdx == loopIdx) {
                continue;
            }
            if (loops[candidateIdx].area() <= loops[loopIdx].area()) {
                continue;
            }
            if (!loopContainsLoop(loops[candidateIdx], loops[loopIdx], config_.coincidenceTolerance)) {
                continue;
            }
            if (polygonsIntersect(loops[candidateIdx].polygon, loops[loopIdx].polygon)) {
                continue;
            }
            double area = loops[candidateIdx].area();
            if (area < bestArea) {
                bestArea = area;
                bestParent = static_cast<int>(candidateIdx);
            }
        }

        parent[loopIdx] = bestParent;
        if (bestParent >= 0) {
            depth[loopIdx] = depth[bestParent] + 1;
        }
    }

    for (size_t i = 0; i < loops.size(); ++i) {
        if (loops[i].polygon.size() < 3) {
            continue;
        }
        bool shouldBeCCW = (depth[i] % 2 == 0);
        if (loops[i].isCCW() != shouldBeCCW) {
            reverseLoop(loops[i]);
        }
    }

    std::unordered_map<int, size_t> faceByLoop;
    for (size_t i = 0; i < loops.size(); ++i) {
        if (depth[i] % 2 != 0) {
            continue;
        }
        Face face;
        face.outerLoop = std::move(loops[i]);
        faceByLoop[static_cast<int>(i)] = faces.size();
        faces.push_back(std::move(face));
    }

    for (size_t i = 0; i < loops.size(); ++i) {
        if (depth[i] % 2 == 0) {
            continue;
        }
        int ancestor = parent[i];
        while (ancestor >= 0 && depth[ancestor] % 2 != 0) {
            ancestor = parent[ancestor];
        }
        if (ancestor < 0) {
            continue;
        }
        auto faceIt = faceByLoop.find(ancestor);
        if (faceIt == faceByLoop.end()) {
            continue;
        }
        faces[faceIt->second].innerLoops.push_back(std::move(loops[i]));
    }

    return faces;
}

bool LoopDetector::validateLoop(const Loop& loop, const sk::Sketch& /*sketch*/) const {
    if (!loop.wire.isClosed()) {
        return false;
    }
    if (loop.polygon.size() < 3) {
        return false;
    }
    if (std::abs(loop.signedArea) < kMinArea) {
        return false;
    }

    size_t n = loop.polygon.size();
    if (n < 4) {
        return true;
    }

    for (size_t i = 0; i < n; ++i) {
        size_t iNext = (i + 1) % n;
        for (size_t j = i + 1; j < n; ++j) {
            size_t jNext = (j + 1) % n;
            if (i == j || iNext == j || jNext == i) {
                continue;
            }
            if (i == 0 && jNext == n - 1) {
                continue;
            }
            if (segmentsIntersect(loop.polygon[i], loop.polygon[iNext],
                                  loop.polygon[j], loop.polygon[jNext])) {
                return false;
            }
        }
    }

    return true;
}

double computeSignedArea(const std::vector<sk::Vec2d>& polygon) {
    if (polygon.size() < 3) {
        return 0.0;
    }
    double area = 0.0;
    for (size_t i = 0; i < polygon.size(); ++i) {
        const auto& p1 = polygon[i];
        const auto& p2 = polygon[(i + 1) % polygon.size()];
        area += p1.x * p2.y - p2.x * p1.y;
    }
    return 0.5 * area;
}

bool isPointInPolygon(const sk::Vec2d& point, const std::vector<sk::Vec2d>& polygon) {
    if (polygon.size() < 3) {
        return false;
    }
    bool inside = false;
    for (size_t i = 0, j = polygon.size() - 1; i < polygon.size(); j = i++) {
        const auto& pi = polygon[i];
        const auto& pj = polygon[j];
        bool intersect = ((pi.y > point.y) != (pj.y > point.y)) &&
                         (point.x < (pj.x - pi.x) * (point.y - pi.y) / (pj.y - pi.y) + pi.x);
        if (intersect) {
            inside = !inside;
        }
    }
    return inside;
}

sk::Vec2d computeCentroid(const std::vector<sk::Vec2d>& polygon) {
    sk::Vec2d centroid{0.0, 0.0};
    if (polygon.empty()) {
        return centroid;
    }

    double area = computeSignedArea(polygon);
    if (std::abs(area) < kMinArea) {
        for (const auto& p : polygon) {
            centroid.x += p.x;
            centroid.y += p.y;
        }
        centroid.x /= static_cast<double>(polygon.size());
        centroid.y /= static_cast<double>(polygon.size());
        return centroid;
    }

    double factor = 0.0;
    for (size_t i = 0; i < polygon.size(); ++i) {
        const auto& p1 = polygon[i];
        const auto& p2 = polygon[(i + 1) % polygon.size()];
        double cross = p1.x * p2.y - p2.x * p1.y;
        centroid.x += (p1.x + p2.x) * cross;
        centroid.y += (p1.y + p2.y) * cross;
        factor += cross;
    }

    factor = 1.0 / (3.0 * factor);
    centroid.x *= factor;
    centroid.y *= factor;
    return centroid;
}

bool polygonsIntersect(const std::vector<sk::Vec2d>& poly1,
                       const std::vector<sk::Vec2d>& poly2) {
    if (poly1.empty() || poly2.empty()) {
        return false;
    }
    size_t n1 = poly1.size();
    size_t n2 = poly2.size();

    for (size_t i = 0; i < n1; ++i) {
        size_t iNext = (i + 1) % n1;
        for (size_t j = 0; j < n2; ++j) {
            size_t jNext = (j + 1) % n2;
            if (segmentsIntersect(poly1[i], poly1[iNext], poly2[j], poly2[jNext])) {
                return true;
            }
        }
    }
    return false;
}

bool Loop::contains(const sk::Vec2d& point) const {
    return isPointInPolygon(point, polygon);
}

bool Loop::contains(const Loop& other) const {
    if (polygon.empty()) {
        return false;
    }
    return isPointInPolygon(other.centroid, polygon);
}

bool Face::isValid() const {
    if (!outerLoop.isCCW()) {
        return false;
    }
    for (const auto& hole : innerLoops) {
        if (hole.isCCW()) {
            return false;
        }
    }
    return true;
}

} // namespace onecad::core::loop
