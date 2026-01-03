#include "LineTool.h"
#include "../Sketch.h"
#include "../SketchRenderer.h"

namespace onecad::core::sketch::tools {

LineTool::LineTool() = default;

void LineTool::onMousePress(const Vec2d& pos, Qt::MouseButton button) {
    if (button == Qt::RightButton) {
        // Right-click finishes polyline
        cancel();
        return;
    }

    if (button != Qt::LeftButton) {
        return;
    }

    lineCreated_ = false;

    if (state_ == State::Idle) {
        // First click - record start point
        startPoint_ = pos;
        currentPoint_ = pos;
        state_ = State::FirstClick;
    } else if (state_ == State::FirstClick) {
        // Second click - create line
        if (!sketch_) {
            return;
        }

        // Check minimum length to avoid degenerate geometry
        double dx = pos.x - startPoint_.x;
        double dy = pos.y - startPoint_.y;
        double length = std::sqrt(dx * dx + dy * dy);

        if (length < 0.01) {
            // Too short, ignore
            return;
        }

        // Create line using the convenience method that creates points
        EntityID lineId = sketch_->addLine(startPoint_.x, startPoint_.y, pos.x, pos.y);

        if (!lineId.empty()) {
            lineCreated_ = true;

            // Get the end point for polyline continuation
            // The line stores startPointId and endPointId
            auto* line = sketch_->getEntityAs<SketchLine>(lineId);
            if (line) {
                lastPointId_ = line->endPointId();
            }

            // Continue polyline: new start = old end
            startPoint_ = pos;
            currentPoint_ = pos;
            // Stay in FirstClick state for polyline mode
        }
    }
}

void LineTool::onMouseMove(const Vec2d& pos) {
    currentPoint_ = pos;
}

void LineTool::onMouseRelease(const Vec2d& /*pos*/, Qt::MouseButton /*button*/) {
    // Line tool uses click-click, not drag, so nothing on release
}

void LineTool::onKeyPress(Qt::Key key) {
    if (key == Qt::Key_Escape) {
        cancel();
    }
}

void LineTool::cancel() {
    state_ = State::Idle;
    lastPointId_.clear();
    lineCreated_ = false;
}

void LineTool::render(SketchRenderer& renderer) {
    if (state_ == State::FirstClick) {
        // Show preview line from start to current mouse position
        renderer.setPreviewLine(startPoint_, currentPoint_);
    } else {
        renderer.clearPreview();
    }
}

} // namespace onecad::core::sketch::tools
