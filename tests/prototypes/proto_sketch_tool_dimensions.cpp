/**
 * Prototype: Draft sketch tool dimensions
 *
 * Validates editable draft preview fields for line/rectangle/circle tools.
 */

#include "sketch/Sketch.h"
#include "sketch/SketchCircle.h"
#include "sketch/SketchLine.h"
#include "sketch/SketchPoint.h"
#include "sketch/SketchRenderer.h"
#include "sketch/tools/CircleTool.h"
#include "sketch/tools/LineTool.h"
#include "sketch/tools/RectangleTool.h"

#include <Qt>

#include <algorithm>
#include <cassert>
#include <cmath>
#include <iostream>
#include <limits>
#include <numbers>
#include <string>

using namespace onecad::core::sketch;
using namespace onecad::core::sketch::tools;

namespace {

bool approx(double a, double b, double tol = 1e-6) {
    double diff = std::abs(a - b);
    double scale = std::max(std::abs(a), std::abs(b));
    return diff <= tol || diff <= tol * scale;
}

const SketchLine* findLastLine(const Sketch& sketch) {
    const SketchLine* result = nullptr;
    for (const auto& entity : sketch.getAllEntities()) {
        if (entity && entity->type() == EntityType::Line) {
            result = static_cast<const SketchLine*>(entity.get());
        }
    }
    return result;
}

const SketchCircle* findLastCircle(const Sketch& sketch) {
    const SketchCircle* result = nullptr;
    for (const auto& entity : sketch.getAllEntities()) {
        if (entity && entity->type() == EntityType::Circle) {
            result = static_cast<const SketchCircle*>(entity.get());
        }
    }
    return result;
}

} // namespace

int main() {
    // ----- Line tool: editable length + angle -----
    {
        Sketch sketch(SketchPlane::XY());
        SketchRenderer renderer;
        LineTool tool;
        tool.setSketch(&sketch);

        tool.onMousePress({0.0, 0.0}, Qt::LeftButton);  // start
        tool.onMouseMove({10.0, 0.0});

        auto applyLength = tool.applyPreviewDimensionValue("line_length", 50.0);
        assert(applyLength.applied);
        auto applyAngle = tool.applyPreviewDimensionValue("line_angle", -45.0);
        assert(applyAngle.applied);
        auto rejectLength = tool.applyPreviewDimensionValue("line_length", 0.0);
        assert(!rejectLength.applied);

        tool.render(renderer);
        const auto& dims = renderer.getPreviewDimensions();
        assert(dims.size() >= 2);
        assert(dims[0].id == "line_length");
        assert(dims[0].value.has_value());
        assert(approx(*dims[0].value, 50.0));
        assert(dims[1].id == "line_angle");
        assert(dims[1].value.has_value());
        assert(approx(*dims[1].value, -45.0));

        tool.onMousePress({7.0, 9.0}, Qt::LeftButton);  // commit using locked draft values

        const SketchLine* line = findLastLine(sketch);
        assert(line);
        const auto* p1 = sketch.getEntityAs<SketchPoint>(line->startPointId());
        const auto* p2 = sketch.getEntityAs<SketchPoint>(line->endPointId());
        assert(p1 && p2);

        const double dx = p2->x() - p1->x();
        const double dy = p2->y() - p1->y();
        const double length = std::sqrt(dx * dx + dy * dy);
        const double angleDeg = std::atan2(dy, dx) * 180.0 / std::numbers::pi;
        assert(approx(length, 50.0));
        assert(approx(angleDeg, -45.0, 1e-4));
    }

    // ----- Rectangle tool: editable width + height -----
    {
        Sketch sketch(SketchPlane::XY());
        SketchRenderer renderer;
        RectangleTool tool;
        tool.setSketch(&sketch);

        tool.onMousePress({0.0, 0.0}, Qt::LeftButton);  // corner 1
        tool.onMouseMove({5.0, 8.0});                   // establish positive quadrant

        auto applyWidth = tool.applyPreviewDimensionValue("rect_width", 30.0);
        assert(applyWidth.applied);
        auto applyHeight = tool.applyPreviewDimensionValue("rect_height", 20.0);
        assert(applyHeight.applied);

        tool.render(renderer);
        const auto& dims = renderer.getPreviewDimensions();
        assert(dims.size() == 2);
        assert(dims[0].id == "rect_width");
        assert(dims[0].value.has_value() && approx(*dims[0].value, 30.0));
        assert(dims[1].id == "rect_height");
        assert(dims[1].value.has_value() && approx(*dims[1].value, 20.0));

        tool.onMousePress({1.0, 1.0}, Qt::LeftButton);  // commit using locked draft values

        double minX = std::numeric_limits<double>::max();
        double minY = std::numeric_limits<double>::max();
        double maxX = std::numeric_limits<double>::lowest();
        double maxY = std::numeric_limits<double>::lowest();
        int pointCount = 0;
        for (const auto& entity : sketch.getAllEntities()) {
            if (entity && entity->type() == EntityType::Point) {
                const auto* pt = static_cast<const SketchPoint*>(entity.get());
                minX = std::min(minX, pt->x());
                minY = std::min(minY, pt->y());
                maxX = std::max(maxX, pt->x());
                maxY = std::max(maxY, pt->y());
                ++pointCount;
            }
        }
        assert(pointCount == 4);
        assert(approx(maxX - minX, 30.0));
        assert(approx(maxY - minY, 20.0));
    }

    // ----- Circle tool: editable radius -----
    {
        Sketch sketch(SketchPlane::XY());
        SketchRenderer renderer;
        CircleTool tool;
        tool.setSketch(&sketch);

        tool.onMousePress({0.0, 0.0}, Qt::LeftButton);  // center
        tool.onMouseMove({5.0, 0.0});

        auto applyRadius = tool.applyPreviewDimensionValue("circle_radius", 25.0);
        assert(applyRadius.applied);
        auto rejectRadius = tool.applyPreviewDimensionValue("circle_radius", -1.0);
        assert(!rejectRadius.applied);

        tool.onMouseMove({0.0, 10.0});  // update direction only
        tool.render(renderer);
        const auto& dims = renderer.getPreviewDimensions();
        assert(dims.size() == 1);
        assert(dims[0].id == "circle_radius");
        assert(dims[0].value.has_value() && approx(*dims[0].value, 25.0));

        tool.onMousePress({0.0, 10.0}, Qt::LeftButton);  // commit using locked draft value
        const SketchCircle* circle = findLastCircle(sketch);
        assert(circle);
        assert(approx(circle->radius(), 25.0));
    }

    std::cout << "proto_sketch_tool_dimensions: OK" << std::endl;
    return 0;
}
