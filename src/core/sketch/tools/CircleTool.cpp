#include "CircleTool.h"
#include "../Sketch.h"
#include "../SketchRenderer.h"

#include <cmath>
#include <cstdio>
#include <string>

namespace onecad::core::sketch::tools {

CircleTool::CircleTool() = default;

void CircleTool::onMousePress(const Vec2d& pos, Qt::MouseButton button) {
    if (button == Qt::RightButton) {
        cancel();
        return;
    }

    if (button != Qt::LeftButton) {
        return;
    }

    circleCreated_ = false;

    if (state_ == State::Idle) {
        // First click - record center
        centerPoint_ = pos;
        currentPoint_ = pos;
        currentRadius_ = 0.0;
        state_ = State::FirstClick;
    } else if (state_ == State::FirstClick) {
        // Second click - create circle
        if (!sketch_) {
            return;
        }

        // Calculate radius
        double dx = pos.x - centerPoint_.x;
        double dy = pos.y - centerPoint_.y;
        double radius = std::sqrt(dx * dx + dy * dy);

        if (radius < 0.01) {
            // Too small, ignore
            return;
        }

        // Create circle
        EntityID circleId = sketch_->addCircle(centerPoint_.x, centerPoint_.y, radius);

        if (!circleId.empty()) {
            circleCreated_ = true;
        }

        // Return to idle state (circle tool doesn't chain like line tool)
        state_ = State::Idle;
        currentRadius_ = 0.0;
    }
}

void CircleTool::onMouseMove(const Vec2d& pos) {
    currentPoint_ = pos;

    if (state_ == State::FirstClick) {
        // Calculate preview radius
        double dx = pos.x - centerPoint_.x;
        double dy = pos.y - centerPoint_.y;
        currentRadius_ = std::sqrt(dx * dx + dy * dy);
    }
}

void CircleTool::onMouseRelease(const Vec2d& /*pos*/, Qt::MouseButton /*button*/) {
    // Circle tool uses click-click, not drag
}

void CircleTool::onKeyPress(Qt::Key key) {
    if (key == Qt::Key_Escape) {
        cancel();
    }
}

void CircleTool::cancel() {
    state_ = State::Idle;
    currentRadius_ = 0.0;
    circleCreated_ = false;
}

void CircleTool::render(SketchRenderer& renderer) {
    if (state_ == State::FirstClick && currentRadius_ > 0.01) {
        // Show preview circle
        renderer.setPreviewCircle(centerPoint_, currentRadius_);

        char buffer[32];
        std::snprintf(buffer, sizeof(buffer), "R: %.2f", currentRadius_);
        
        // Place label at midpoint of the radius line (from center to cursor)
        Vec2d labelPos = {
            (centerPoint_.x + currentPoint_.x) * 0.5,
            (centerPoint_.y + currentPoint_.y) * 0.5
        };
        
        renderer.setPreviewDimensions({{labelPos, std::string(buffer)}});
    } else {
        renderer.clearPreview();
    }
}

} // namespace onecad::core::sketch::tools
