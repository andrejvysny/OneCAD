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

namespace onecad::core::sketch::tools {

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
    if (!activeTool_) {
        return;
    }

    rawCursorPos_ = pos;
    if (sketch_) {
        currentSnapResult_ = snapManager_.findBestSnap(pos, *sketch_, excludeFromSnap_);
    } else {
        currentSnapResult_ = SnapResult{};
    }
    activeTool_->setSnapResult(currentSnapResult_);
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
        emit geometryCreated();
    }

    emit updateRequested();
}

void SketchToolManager::handleMouseMove(const Vec2d& pos) {
    rawCursorPos_ = pos;

    if (!activeTool_) {
        currentSnapResult_ = SnapResult{};
        currentInferredConstraints_.clear();
        return;
    }

    // Apply snapping
    if (sketch_) {
        currentSnapResult_ = snapManager_.findBestSnap(pos, *sketch_, excludeFromSnap_);
    } else {
        currentSnapResult_ = SnapResult{};
    }

    // Pass snap result to tool
    activeTool_->setSnapResult(currentSnapResult_);
    activeTool_->setInferredConstraints({});

    // Get snapped position for tool
    Vec2d snappedPos = currentSnapResult_.snapped ? currentSnapResult_.position : pos;

    activeTool_->onMouseMove(snappedPos);
    currentInferredConstraints_ = activeTool_->inferredConstraints();
    emit updateRequested();
}

void SketchToolManager::handleMouseRelease(const Vec2d& pos, Qt::MouseButton button) {
    if (!activeTool_) {
        return;
    }

    rawCursorPos_ = pos;
    if (sketch_) {
        currentSnapResult_ = snapManager_.findBestSnap(pos, *sketch_, excludeFromSnap_);
    } else {
        currentSnapResult_ = SnapResult{};
    }
    activeTool_->setSnapResult(currentSnapResult_);
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
        const bool showGuide = snapManager_.showGuidePoints() && currentSnapResult_.hasGuide;
        const std::string hintText = snapManager_.showSnappingHints()
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
