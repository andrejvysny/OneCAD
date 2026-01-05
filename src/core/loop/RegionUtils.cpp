/**
 * @file RegionUtils.cpp
 * @brief Shared helpers for region IDs and hierarchy.
 */
#include "RegionUtils.h"

#include "../sketch/Sketch.h"
#include "../sketch/SketchTypes.h"

#include <algorithm>
#include <limits>
#include <unordered_set>

namespace onecad::core::loop {

namespace {
constexpr double kGeometryEpsilon = 1e-9;

bool polygonContainsPolygon(const std::vector<sk::Vec2d>& outer,
                            const std::vector<sk::Vec2d>& inner,
                            double tolerance) {
    if (outer.size() < 3 || inner.size() < 3) {
        return false;
    }

    sk::Vec2d outerMin = outer.front();
    sk::Vec2d outerMax = outer.front();
    for (const auto& p : outer) {
        outerMin.x = std::min(outerMin.x, p.x);
        outerMin.y = std::min(outerMin.y, p.y);
        outerMax.x = std::max(outerMax.x, p.x);
        outerMax.y = std::max(outerMax.y, p.y);
    }

    for (const auto& p : inner) {
        if (p.x < outerMin.x - tolerance || p.y < outerMin.y - tolerance ||
            p.x > outerMax.x + tolerance || p.y > outerMax.y + tolerance) {
            return false;
        }
    }

    for (const auto& p : inner) {
        if (!isPointInPolygon(p, outer)) {
            return false;
        }
    }

    if (polygonsIntersect(outer, inner)) {
        return false;
    }

    return true;
}

} // namespace

std::string regionKey(const Loop& loop) {
    std::vector<sk::EntityID> edges = loop.wire.edges;
    std::sort(edges.begin(), edges.end());
    std::string key;
    key.reserve(edges.size() * 40);
    for (const auto& id : edges) {
        key.append(id);
        key.push_back('|');
    }
    return key;
}

std::vector<RegionDefinition> buildRegionDefinitions(const LoopDetectionResult& result,
                                                     double tolerance) {
    std::vector<RegionDefinition> regions;
    if (!result.success) {
        return regions;
    }

    std::vector<Loop> loops;
    std::unordered_set<std::string> seen;

    auto addLoop = [&](const Loop& loop) {
        std::string key = regionKey(loop);
        if (key.empty()) {
            return;
        }
        if (seen.insert(key).second) {
            loops.push_back(loop);
        }
    };

    for (const auto& face : result.faces) {
        addLoop(face.outerLoop);
        for (const auto& hole : face.innerLoops) {
            addLoop(hole);
        }
    }

    if (loops.empty()) {
        return regions;
    }

    std::vector<size_t> order(loops.size());
    for (size_t i = 0; i < loops.size(); ++i) {
        order[i] = i;
    }
    std::sort(order.begin(), order.end(), [&](size_t a, size_t b) {
        return loops[a].area() > loops[b].area();
    });

    std::vector<int> parent(loops.size(), -1);
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
            if (!polygonContainsPolygon(loops[candidateIdx].polygon, loops[loopIdx].polygon,
                                        tolerance)) {
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
    }

    std::vector<std::vector<size_t>> children(loops.size());
    for (size_t i = 0; i < loops.size(); ++i) {
        if (parent[i] >= 0) {
            children[static_cast<size_t>(parent[i])].push_back(i);
        }
    }

    regions.reserve(loops.size());
    for (size_t i = 0; i < loops.size(); ++i) {
        if (loops[i].area() <= kGeometryEpsilon) {
            continue;
        }
        RegionDefinition region;
        region.id = regionKey(loops[i]);
        region.outerLoop = loops[i];
        for (size_t childIdx : children[i]) {
            if (loops[childIdx].area() <= kGeometryEpsilon) {
                continue;
            }
            region.holes.push_back(loops[childIdx]);
        }
        regions.push_back(std::move(region));
    }

    return regions;
}

std::optional<RegionDefinition> findRegionDefinition(const LoopDetectionResult& result,
                                                     const std::string& regionId,
                                                     double tolerance) {
    if (regionId.empty()) {
        return std::nullopt;
    }
    auto regions = buildRegionDefinitions(result, tolerance);
    for (auto& region : regions) {
        if (region.id == regionId) {
            return region;
        }
    }
    return std::nullopt;
}

LoopDetectorConfig makeRegionDetectionConfig() {
    LoopDetectorConfig config;
    config.findAllLoops = false;
    config.computeAreas = true;
    config.resolveHoles = true;
    config.validate = true;
    config.planarizeIntersections = true;
    return config;
}

std::optional<Face> resolveRegionFace(const sk::Sketch& sketch,
                                      const std::string& regionId) {
    return resolveRegionFace(sketch, regionId, makeRegionDetectionConfig());
}

std::optional<Face> resolveRegionFace(const sk::Sketch& sketch,
                                      const std::string& regionId,
                                      const LoopDetectorConfig& config) {
    LoopDetector detector;
    detector.setConfig(config);
    auto result = detector.detect(sketch);
    if (!result.success) {
        return std::nullopt;
    }

    auto region = findRegionDefinition(result, regionId, sk::constants::COINCIDENCE_TOLERANCE);
    if (!region.has_value()) {
        return std::nullopt;
    }

    Face face;
    face.outerLoop = region->outerLoop;
    face.innerLoops = std::move(region->holes);
    return face;
}

} // namespace onecad::core::loop
