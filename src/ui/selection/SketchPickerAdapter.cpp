#include "SketchPickerAdapter.h"

#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/SketchRenderer.h"

namespace onecad::ui::selection {

using app::selection::PickResult;
using app::selection::SelectionItem;
using app::selection::SelectionKind;

namespace {
int priorityForSketchEntity(core::sketch::EntityType type, bool isConstruction) {
    // Lower value means higher selection priority (points > edges > regions > construction).
    if (isConstruction) {
        return 3;
    }
    switch (type) {
        case core::sketch::EntityType::Point:
            return 0;
        case core::sketch::EntityType::Line:
        case core::sketch::EntityType::Arc:
        case core::sketch::EntityType::Circle:
        case core::sketch::EntityType::Ellipse:
            return 1;
        default:
            return 2;
    }
}

SelectionKind kindForSketchEntity(core::sketch::EntityType type) {
    switch (type) {
        case core::sketch::EntityType::Point:
            return SelectionKind::SketchPoint;
        case core::sketch::EntityType::Line:
        case core::sketch::EntityType::Arc:
        case core::sketch::EntityType::Circle:
        case core::sketch::EntityType::Ellipse:
            return SelectionKind::SketchEdge;
        default:
            return SelectionKind::SketchEdge;
    }
}
} // namespace

PickResult SketchPickerAdapter::pick(const core::sketch::SketchRenderer& renderer,
                                     const core::sketch::Sketch& sketch,
                                     const core::sketch::Vec2d& sketchPos,
                                     std::string_view sketchId,
                                     double pixelScale,
                                     double tolerancePixels,
                                     Options options) const {
    (void)sketch;
    PickResult result;
    const double pixelScaleSafe = (pixelScale > 0.0) ? pixelScale : 1.0;
    const double toleranceWorld = tolerancePixels * pixelScaleSafe;
    const std::string sketchIdStr(sketchId);

    if (options.allowConstraints) {
        auto constraintId = renderer.pickConstraint(sketchPos, toleranceWorld);
        if (!constraintId.empty()) {
            SelectionItem item;
            item.kind = SelectionKind::SketchConstraint;
            item.id = {sketchIdStr, constraintId};
            item.priority = -1;
            item.screenDistance = 0.0;
            result.hits.push_back(item);
        }
    }

    auto hits = renderer.pickEntities(sketchPos, toleranceWorld);
    for (const auto& hit : hits) {
        SelectionItem item;
        item.kind = kindForSketchEntity(hit.type);
        item.id = {sketchIdStr, hit.id};
        item.isConstruction = hit.isConstruction;
        item.priority = priorityForSketchEntity(hit.type, hit.isConstruction);
        item.screenDistance = hit.distance / pixelScaleSafe;
        result.hits.push_back(item);
    }

    if (options.allowRegions) {
        auto region = renderer.pickRegion(sketchPos);
        if (region.has_value()) {
            SelectionItem item;
            item.kind = SelectionKind::SketchRegion;
            item.id = {sketchIdStr, *region};
            item.priority = 2;
            item.screenDistance = 0.0;
            result.hits.push_back(item);
        }
    }

    return result;
}

} // namespace onecad::ui::selection
