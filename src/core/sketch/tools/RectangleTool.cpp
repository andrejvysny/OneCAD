#include "RectangleTool.h"
#include "../Sketch.h"
#include "../SketchRenderer.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <vector>
#include <string>

namespace onecad::core::sketch::tools {

RectangleTool::RectangleTool() = default;

void RectangleTool::onMousePress(const Vec2d& pos, Qt::MouseButton button) {
    if (button == Qt::RightButton) {
        cancel();
        return;
    }

    if (button != Qt::LeftButton) {
        return;
    }

    rectangleCreated_ = false;

    if (state_ == State::Idle) {
        // First click - record first corner
        corner1_ = pos;
        corner2_ = pos;
        state_ = State::FirstClick;
    } else if (state_ == State::FirstClick) {
        // Second click - create rectangle
        if (!sketch_) {
            return;
        }

        // Check minimum size
        double width = std::abs(pos.x - corner1_.x);
        double height = std::abs(pos.y - corner1_.y);

        if (width < 0.01 || height < 0.01) {
            // Too small, ignore
            return;
        }

        createRectangle(corner1_, pos);
        rectangleCreated_ = true;

        // Return to idle state
        state_ = State::Idle;
    }
}

void RectangleTool::onMouseMove(const Vec2d& pos) {
    if (state_ == State::FirstClick) {
        corner2_ = pos;
    }
}

void RectangleTool::onMouseRelease(const Vec2d& /*pos*/, Qt::MouseButton /*button*/) {
    // Rectangle tool uses click-click, not drag
}

void RectangleTool::onKeyPress(Qt::Key key) {
    if (key == Qt::Key_Escape) {
        cancel();
    }
}

void RectangleTool::cancel() {
    state_ = State::Idle;
    rectangleCreated_ = false;
}

void RectangleTool::render(SketchRenderer& renderer) {
    if (state_ == State::FirstClick) {
        // Show preview rectangle as 4 lines
        double width = std::abs(corner2_.x - corner1_.x);
        double height = std::abs(corner2_.y - corner1_.y);

        if (width > 0.01 || height > 0.01) {
            renderer.setPreviewRectangle(corner1_, corner2_);

            std::vector<SketchRenderer::PreviewDimension> dims;
            char buffer[32];

            // Width dimension (top edge)
            if (width > 0.01) {
                std::snprintf(buffer, sizeof(buffer), "%.2f", width);
                double midX = (corner1_.x + corner2_.x) * 0.5;
                double maxY = std::max(corner1_.y, corner2_.y);
                dims.push_back({{midX, maxY}, std::string(buffer)});
            }

            // Height dimension (right edge)
            if (height > 0.01) {
                std::snprintf(buffer, sizeof(buffer), "%.2f", height);
                double maxX = std::max(corner1_.x, corner2_.x);
                double midY = (corner1_.y + corner2_.y) * 0.5;
                dims.push_back({{maxX, midY}, std::string(buffer)});
            }

            renderer.setPreviewDimensions(dims);
        } else {
            renderer.clearPreview();
        }
    } else {
        renderer.clearPreview();
    }
}

void RectangleTool::createRectangle(const Vec2d& c1, const Vec2d& c2) {
    if (!sketch_) return;

    // Order corners (min/max)
    double minX = std::min(c1.x, c2.x);
    double maxX = std::max(c1.x, c2.x);
    double minY = std::min(c1.y, c2.y);
    double maxY = std::max(c1.y, c2.y);

    // Create 4 corner points
    EntityID p1 = sketch_->addPoint(minX, minY);  // bottom-left
    EntityID p2 = sketch_->addPoint(maxX, minY);  // bottom-right
    EntityID p3 = sketch_->addPoint(maxX, maxY);  // top-right
    EntityID p4 = sketch_->addPoint(minX, maxY);  // top-left

    if (p1.empty() || p2.empty() || p3.empty() || p4.empty()) {
        return;
    }

    // Create 4 lines connecting the points
    EntityID lineBottom = sketch_->addLine(p1, p2);  // bottom (horizontal)
    EntityID lineRight = sketch_->addLine(p2, p3);   // right (vertical)
    EntityID lineTop = sketch_->addLine(p3, p4);     // top (horizontal)
    EntityID lineLeft = sketch_->addLine(p4, p1);    // left (vertical)

    if (lineBottom.empty() || lineRight.empty() || lineTop.empty() || lineLeft.empty()) {
        return;
    }

    // Add constraints: horizontal for top/bottom, vertical for left/right
    sketch_->addHorizontal(lineBottom);
    sketch_->addHorizontal(lineTop);
    sketch_->addVertical(lineLeft);
    sketch_->addVertical(lineRight);
}

} // namespace onecad::core::sketch::tools
