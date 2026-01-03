#include "SketchToolManager.h"
#include "LineTool.h"
#include "CircleTool.h"
#include "RectangleTool.h"
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

    // Clear any preview
    if (renderer_) {
        renderer_->clearPreview();
    }

    emit toolChanged(ToolType::None);
}

void SketchToolManager::handleMousePress(const Vec2d& pos, Qt::MouseButton button) {
    if (!activeTool_) {
        return;
    }

    activeTool_->onMousePress(pos, button);

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
    }

    if (created) {
        emit geometryCreated();
    }

    emit updateRequested();
}

void SketchToolManager::handleMouseMove(const Vec2d& pos) {
    if (!activeTool_) {
        return;
    }

    activeTool_->onMouseMove(pos);
    emit updateRequested();
}

void SketchToolManager::handleMouseRelease(const Vec2d& pos, Qt::MouseButton button) {
    if (!activeTool_) {
        return;
    }

    activeTool_->onMouseRelease(pos, button);
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
    if (!activeTool_ || !renderer_) {
        return;
    }

    activeTool_->render(*renderer_);
}

std::unique_ptr<SketchTool> SketchToolManager::createTool(ToolType type) {
    switch (type) {
        case ToolType::Line:
            return std::make_unique<LineTool>();
        case ToolType::Circle:
            return std::make_unique<CircleTool>();
        case ToolType::Rectangle:
            return std::make_unique<RectangleTool>();
        case ToolType::None:
        default:
            return nullptr;
    }
}

} // namespace onecad::core::sketch::tools
