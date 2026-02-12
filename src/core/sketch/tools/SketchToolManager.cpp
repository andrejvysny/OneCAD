#include "SketchToolManager.h"
#include "LineTool.h"
#include "CircleTool.h"
#include "RectangleTool.h"
#include "ArcTool.h"
#include "EllipseTool.h"
#include "TrimTool.h"
#include "MirrorTool.h"
#include "../Sketch.h"
#include "../SketchRenderer.h"
#include <QLoggingCategory>
#include <QString>
#include <cassert>
#include <cmath>
#include <limits>
#include <vector>

namespace onecad::core::sketch::tools {

Q_LOGGING_CATEGORY(logSketchToolMgr, "onecad.core.sketchtool")

namespace {

SnapResult applyGuideFirstSnapPolicy(const SnapResult& fallbackSnap,
                                     const std::vector<SnapResult>& allSnaps) {
    if (fallbackSnap.snapped &&
        (fallbackSnap.type == SnapType::Vertex || fallbackSnap.type == SnapType::Endpoint)) {
        return fallbackSnap;
    }

    // Prefer guide-guide Intersection over individual H/V when crossing exists
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

/**
 * @brief Resolve effective snap for mouse input.
 *
 * Parity contract:
 * - Commit path (`preferGuide=false`) resolves directly from `findBestSnap()`.
 * - Preview path (`preferGuide=true`) may override the fallback with the nearest
 *   guide-bearing candidate from `findAllSnaps()`.
 * - If no guide-bearing candidate exists, preview and commit resolve the same
 *   winner for identical input tuple (cursor position, sketch, exclusion set,
 *   reference point).
 */
SnapResult resolveSnapForInputEvent(const SnapManager& snapManager,
                                    const Vec2d& pos,
                                    const Sketch& sketch,
                                    const std::unordered_set<EntityID>& excludeFromSnap,
                                    std::optional<Vec2d> referencePoint,
                                    bool preferGuide,
                                    std::vector<SnapResult>* allSnapsOut = nullptr) {
    SnapResult bestSnap = snapManager.findBestSnap(pos, sketch, excludeFromSnap, referencePoint);

    // Commit fast path: callers that neither prefer guide nor need all snaps
    // should not pay the findAllSnaps() cost.
    if (!preferGuide && !allSnapsOut) {
        return bestSnap;
    }

    std::vector<SnapResult> allSnaps =
        snapManager.findAllSnaps(pos, sketch, excludeFromSnap, referencePoint);

    if (allSnapsOut) {
        *allSnapsOut = allSnaps;
    }

    SnapResult guideResolvedSnap = applyGuideFirstSnapPolicy(bestSnap, allSnaps);

#ifndef NDEBUG
    bool hasGuideCandidate = false;
    for (const auto& snap : allSnaps) {
        if (snap.snapped && snap.hasGuide) {
            hasGuideCandidate = true;
            break;
        }
    }
    if (!hasGuideCandidate) {
        assert(sameResolvedSnap(guideResolvedSnap, bestSnap));
    }
#endif

    if (preferGuide) {
        return guideResolvedSnap;
    }
    return bestSnap;
}

} // namespace

SketchToolManager::SketchToolManager(QObject* parent)
    : QObject(parent)
{
}

SketchToolManager::~SketchToolManager() = default;

void SketchToolManager::setSketch(Sketch* sketch) {
    sketch_ = sketch;
    if (activeTool_) {
        activeTool_->setSketch(sketch);
    }
}

void SketchToolManager::setRenderer(SketchRenderer* renderer) {
    renderer_ = renderer;
}

void SketchToolManager::activateTool(ToolType type) {
    qCDebug(logSketchToolMgr) << "activateTool" << static_cast<int>(type);
    if (type == currentType_ && activeTool_) {
        return; // Already active
    }

    // Deactivate current tool first
    deactivateTool();

    // Create new tool
    activeTool_ = createTool(type);
    if (activeTool_) {
        activeTool_->setSketch(sketch_);
        activeTool_->setAutoConstrainer(&autoConstrainer_);
        activeTool_->setSnapManager(&snapManager_);
        currentType_ = type;
        emit toolChanged(type);
    }
}

void SketchToolManager::deactivateTool() {
    qCDebug(logSketchToolMgr) << "deactivateTool" << static_cast<int>(currentType_);
    if (activeTool_) {
        activeTool_->cancel();
        activeTool_.reset();
    }
    currentType_ = ToolType::None;
    currentSnapResult_ = SnapResult{};
    currentInferredConstraints_.clear();

    // Clear any preview
    if (renderer_) {
        renderer_->clearPreview();
        renderer_->hideSnapIndicator();
        renderer_->clearGhostConstraints();
    }

    emit toolChanged(ToolType::None);
}

void SketchToolManager::handleMousePress(const Vec2d& pos, Qt::MouseButton button) {
    qCDebug(logSketchToolMgr) << "mousePress" << "pos=" << pos.x << pos.y << "button=" << static_cast<int>(button);
    if (!activeTool_) {
        return;
    }

    rawCursorPos_ = pos;
    if (sketch_) {
        currentSnapResult_ = resolveSnapForInputEvent(
            snapManager_,
            pos,
            *sketch_,
            excludeFromSnap_,
            activeTool_->getReferencePoint(),
            false);
    } else {
        currentSnapResult_ = SnapResult{};
    }
    activeTool_->setSnapResult(currentSnapResult_);
    qCDebug(logSketchToolMgr) << "mousePress:snap"
                              << "snapped=" << currentSnapResult_.snapped
                              << "type=" << static_cast<int>(currentSnapResult_.type)
                              << "distance=" << currentSnapResult_.distance
                              << "entity=" << QString::fromStdString(currentSnapResult_.entityId);
    activeTool_->setInferredConstraints({});

    // Use snapped position for press
    Vec2d snappedPos = currentSnapResult_.snapped ? currentSnapResult_.position : pos;

    activeTool_->onMousePress(snappedPos, button);
    currentInferredConstraints_ = activeTool_->inferredConstraints();

    // Check if geometry was created
    bool created = false;
    if (auto* lineTool = dynamic_cast<LineTool*>(activeTool_.get())) {
        if (lineTool->wasLineCreated()) {
            lineTool->clearLineCreatedFlag();
            created = true;
        }
    } else if (auto* circleTool = dynamic_cast<CircleTool*>(activeTool_.get())) {
        if (circleTool->wasCircleCreated()) {
            circleTool->clearCircleCreatedFlag();
            created = true;
        }
    } else if (auto* rectTool = dynamic_cast<RectangleTool*>(activeTool_.get())) {
        if (rectTool->wasRectangleCreated()) {
            rectTool->clearRectangleCreatedFlag();
            created = true;
        }
    } else if (auto* arcTool = dynamic_cast<ArcTool*>(activeTool_.get())) {
        if (arcTool->wasArcCreated()) {
            arcTool->clearArcCreatedFlag();
            created = true;
        }
    } else if (auto* ellipseTool = dynamic_cast<EllipseTool*>(activeTool_.get())) {
        if (ellipseTool->wasEllipseCreated()) {
            ellipseTool->clearEllipseCreatedFlag();
            created = true;
        }
    } else if (auto* trimTool = dynamic_cast<TrimTool*>(activeTool_.get())) {
        if (trimTool->wasEntityDeleted()) {
            trimTool->clearDeletedFlag();
            created = true;  // Geometry changed
        }
    } else if (auto* mirrorTool = dynamic_cast<MirrorTool*>(activeTool_.get())) {
        if (mirrorTool->wasGeometryCreated()) {
            mirrorTool->clearCreatedFlag();
            created = true;
        }
    }

    if (created) {
        qCDebug(logSketchToolMgr) << "mousePress:geometryCreated";
        emit geometryCreated();
    }

    emit updateRequested();
}

void SketchToolManager::handleMouseMove(const Vec2d& pos) {
    qCDebug(logSketchToolMgr) << "mouseMove" << pos.x << pos.y;
    rawCursorPos_ = pos;

    if (!activeTool_) {
        currentSnapResult_ = SnapResult{};
        currentInferredConstraints_.clear();
        return;
    }

    // Apply snapping
    if (sketch_) {
        std::vector<SnapResult> allSnaps;
        currentSnapResult_ = resolveSnapForInputEvent(
            snapManager_,
            pos,
            *sketch_,
            excludeFromSnap_,
            activeTool_->getReferencePoint(),
            true,
            &allSnaps);

        if (renderer_) {
            if (currentSnapResult_.type == SnapType::Vertex ||
                currentSnapResult_.type == SnapType::Endpoint) {
                SnapResult bestGuide;
                bestGuide.distance = std::numeric_limits<double>::max();
                for (const auto& snap : allSnaps) {
                    if (snap.hasGuide && snap.snapped && snap.distance < bestGuide.distance) {
                        bestGuide = snap;
                    }
                }

                if (bestGuide.snapped) {
                    renderer_->setActiveGuides({{bestGuide.guideOrigin, bestGuide.position}});
                } else {
                    renderer_->setActiveGuides({});
                }
            } else {
                // Extract all guides for multi-guide rendering.
                std::vector<SketchRenderer::GuideLineInfo> guides;
                for (const auto& snap : allSnaps) {
                    if (snap.hasGuide) {
                        guides.push_back({snap.guideOrigin, snap.position});
                    }
                }

                // Dedupe collinear guides.
                auto isCollinear = [](const SketchRenderer::GuideLineInfo& a, const SketchRenderer::GuideLineInfo& b) -> bool {
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

                std::vector<SketchRenderer::GuideLineInfo> uniqueGuides;
                for (const auto& g : guides) {
                    bool duplicate = false;
                    for (const auto& u : uniqueGuides) {
                        if (isCollinear(g, u)) {
                            duplicate = true;
                            break;
                        }
                    }
                    if (!duplicate) {
                        uniqueGuides.push_back(g);
                        if (uniqueGuides.size() >= 4) {
                            break;
                        }
                    }
                }

                renderer_->setActiveGuides(uniqueGuides);
            }
        }
    } else {
        currentSnapResult_ = SnapResult{};
        if (renderer_) {
            renderer_->setActiveGuides({});
        }
    }

    // Pass snap result to tool
    activeTool_->setSnapResult(currentSnapResult_);
    qCDebug(logSketchToolMgr) << "mouseMove:snap"
                              << "snapped=" << currentSnapResult_.snapped
                              << "type=" << static_cast<int>(currentSnapResult_.type)
                              << "distance=" << currentSnapResult_.distance
                              << "entity=" << QString::fromStdString(currentSnapResult_.entityId);
    activeTool_->setInferredConstraints({});

    // Get snapped position for tool
    Vec2d snappedPos = currentSnapResult_.snapped ? currentSnapResult_.position : pos;

    activeTool_->onMouseMove(snappedPos);
    currentInferredConstraints_ = activeTool_->inferredConstraints();
    emit updateRequested();
}

void SketchToolManager::handleMouseRelease(const Vec2d& pos, Qt::MouseButton button) {
    qCDebug(logSketchToolMgr) << "mouseRelease" << "pos=" << pos.x << pos.y << "button=" << static_cast<int>(button);
    if (!activeTool_) {
        return;
    }

    rawCursorPos_ = pos;
    if (sketch_) {
        currentSnapResult_ = resolveSnapForInputEvent(
            snapManager_,
            pos,
            *sketch_,
            excludeFromSnap_,
            activeTool_->getReferencePoint(),
            false);
    } else {
        currentSnapResult_ = SnapResult{};
    }
    activeTool_->setSnapResult(currentSnapResult_);
    qCDebug(logSketchToolMgr) << "mouseRelease:snap"
                              << "snapped=" << currentSnapResult_.snapped
                              << "type=" << static_cast<int>(currentSnapResult_.type)
                              << "distance=" << currentSnapResult_.distance
                              << "entity=" << QString::fromStdString(currentSnapResult_.entityId);
    activeTool_->setInferredConstraints({});

    // Use snapped position for release
    Vec2d snappedPos = currentSnapResult_.snapped ? currentSnapResult_.position : pos;

    activeTool_->onMouseRelease(snappedPos, button);
    currentInferredConstraints_ = activeTool_->inferredConstraints();
    emit updateRequested();
}

void SketchToolManager::handleKeyPress(Qt::Key key) {
    if (!activeTool_) {
        return;
    }

    activeTool_->onKeyPress(key);
    emit updateRequested();
}

void SketchToolManager::renderPreview() {
    if (!renderer_) {
        return;
    }

    // Show snap indicator if snapped
    if (currentSnapResult_.snapped) {
        bool showGuide = snapManager_.showGuidePoints() && currentSnapResult_.hasGuide;
        std::string hintText = snapManager_.showSnappingHints()
            ? currentSnapResult_.hintText
            : std::string();
        renderer_->showSnapIndicator(
            currentSnapResult_.position,
            currentSnapResult_.type,
            currentSnapResult_.guideOrigin,
            showGuide,
            hintText);
    } else {
        renderer_->hideSnapIndicator();
    }

    // Show ghost constraints (inferred constraints during drawing)
    renderer_->setGhostConstraints(currentInferredConstraints_);

    // Render tool preview
    if (activeTool_) {
        activeTool_->render(*renderer_);
    }
}

// NOLINTNEXTLINE(readability-convert-member-functions-to-static)
std::unique_ptr<SketchTool> SketchToolManager::createTool(ToolType type) {
    switch (type) {
        case ToolType::Line:
            return std::make_unique<LineTool>();
        case ToolType::Circle:
            return std::make_unique<CircleTool>();
        case ToolType::Rectangle:
            return std::make_unique<RectangleTool>();
        case ToolType::Arc:
            return std::make_unique<ArcTool>();
        case ToolType::Ellipse:
            return std::make_unique<EllipseTool>();
        case ToolType::Trim:
            return std::make_unique<TrimTool>();
        case ToolType::Mirror:
            return std::make_unique<MirrorTool>();
        case ToolType::None:
        default:
            return nullptr;
    }
}

} // namespace onecad::core::sketch::tools
