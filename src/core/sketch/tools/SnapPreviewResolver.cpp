#include "SnapPreviewResolver.h"

#include "../Sketch.h"

#include <cassert>
#include <cmath>
#include <limits>

namespace onecad::core::sketch::tools {

namespace {

bool sameResolvedSnap(const SnapResult& a, const SnapResult& b) {
    return a.snapped == b.snapped
        && a.type == b.type
        && std::abs(a.position.x - b.position.x) <= 1e-9
        && std::abs(a.position.y - b.position.y) <= 1e-9
        && a.entityId == b.entityId
        && a.secondEntityId == b.secondEntityId
        && a.pointId == b.pointId
        && a.hasGuide == b.hasGuide
        && std::abs(a.guideOrigin.x - b.guideOrigin.x) <= 1e-9
        && std::abs(a.guideOrigin.y - b.guideOrigin.y) <= 1e-9
        && a.hintText == b.hintText;
}

} // namespace

SnapResult applyGuideFirstSnapPolicy(const SnapResult& fallbackSnap,
                                     const std::vector<SnapResult>& allSnaps) {
    if (fallbackSnap.snapped &&
        (fallbackSnap.type == SnapType::Vertex || fallbackSnap.type == SnapType::Endpoint)) {
        return fallbackSnap;
    }

    // Prefer guide-guide intersection over individual H/V when crossing exists.
    for (const auto& snap : allSnaps) {
        if (snap.snapped && snap.type == SnapType::Intersection && snap.hasGuide) {
            return snap;
        }
    }

    SnapResult bestGuide;
    bestGuide.distance = std::numeric_limits<double>::max();
    for (const auto& snap : allSnaps) {
        if (snap.hasGuide && snap.snapped && snap.distance < bestGuide.distance) {
            bestGuide = snap;
        }
    }
    return bestGuide.snapped ? bestGuide : fallbackSnap;
}

std::vector<GuideSegment> buildActiveGuidesForSnap(const SnapResult& resolvedSnap,
                                                   const std::vector<SnapResult>& allSnaps) {
    std::vector<GuideSegment> guides;

    if (allSnaps.empty()) {
        return guides;
    }

    if (resolvedSnap.type == SnapType::Vertex || resolvedSnap.type == SnapType::Endpoint) {
        SnapResult bestGuide;
        bestGuide.distance = std::numeric_limits<double>::max();
        for (const auto& snap : allSnaps) {
            if (snap.hasGuide && snap.snapped && snap.distance < bestGuide.distance) {
                bestGuide = snap;
            }
        }

        if (bestGuide.snapped) {
            guides.push_back({bestGuide.guideOrigin, bestGuide.position});
        }
        return guides;
    }

    for (const auto& snap : allSnaps) {
        if (snap.hasGuide) {
            guides.push_back({snap.guideOrigin, snap.position});
        }
    }

    auto isCollinear = [](const GuideSegment& a, const GuideSegment& b) -> bool {
        double dirAx = a.target.x - a.origin.x;
        double dirAy = a.target.y - a.origin.y;
        double dirBx = b.target.x - b.origin.x;
        double dirBy = b.target.y - b.origin.y;
        double lenA = std::sqrt((dirAx * dirAx) + (dirAy * dirAy));
        double lenB = std::sqrt((dirBx * dirBx) + (dirBy * dirBy));
        if (lenA < 1e-6 || lenB < 1e-6) {
            return true;
        }
        dirAx /= lenA;
        dirAy /= lenA;
        dirBx /= lenB;
        dirBy /= lenB;
        double cross = std::abs((dirAx * dirBy) - (dirAy * dirBx));
        return cross < 0.01;
    };

    std::vector<GuideSegment> uniqueGuides;
    for (const auto& guide : guides) {
        bool duplicate = false;
        for (const auto& uniqueGuide : uniqueGuides) {
            if (isCollinear(guide, uniqueGuide)) {
                duplicate = true;
                break;
            }
        }
        if (!duplicate) {
            uniqueGuides.push_back(guide);
            if (uniqueGuides.size() >= 4) {
                break;
            }
        }
    }

    return uniqueGuides;
}

SnapInputResolution resolveSnapForInputEvent(
    const SnapManager& snapManager,
    const Vec2d& pos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeFromSnap,
    std::optional<Vec2d> referencePoint,
    bool preferGuide,
    bool collectGuideData) {
    SnapInputResolution resolution;
    resolution.bestSnap = snapManager.findBestSnap(pos, sketch, excludeFromSnap, referencePoint);
    resolution.resolvedSnap = resolution.bestSnap;

    const bool requiresAllSnaps = preferGuide || collectGuideData;
    if (!requiresAllSnaps) {
        return resolution;
    }

    resolution.allSnaps = snapManager.findAllSnaps(pos, sketch, excludeFromSnap, referencePoint);
    const SnapResult guideResolvedSnap =
        applyGuideFirstSnapPolicy(resolution.bestSnap, resolution.allSnaps);

#ifndef NDEBUG
    bool hasGuideCandidate = false;
    for (const auto& snap : resolution.allSnaps) {
        if (snap.snapped && snap.hasGuide) {
            hasGuideCandidate = true;
            break;
        }
    }
    if (!hasGuideCandidate) {
        assert(sameResolvedSnap(guideResolvedSnap, resolution.bestSnap));
    }
#endif

    resolution.resolvedSnap = preferGuide ? guideResolvedSnap : resolution.bestSnap;

    if (collectGuideData) {
        resolution.activeGuides =
            buildActiveGuidesForSnap(resolution.resolvedSnap, resolution.allSnaps);
    }

    return resolution;
}

} // namespace onecad::core::sketch::tools
