/**
 * @file ModelingTool.h
 * @brief Base interface for 3D modeling tools.
 */
#ifndef ONECAD_UI_TOOLS_MODELINGTOOL_H
#define ONECAD_UI_TOOLS_MODELINGTOOL_H

#include "../../app/selection/SelectionTypes.h"

#include <QPoint>
#include <Qt>

namespace onecad::ui::tools {

class ModelingTool {
public:
    virtual ~ModelingTool() = default;

    virtual void begin(const app::selection::SelectionItem& selection) = 0;
    virtual void cancel() = 0;
    virtual bool isActive() const = 0;
    virtual bool isDragging() const = 0;

    virtual bool handleMousePress(const QPoint& screenPos, Qt::MouseButton button) = 0;
    virtual bool handleMouseMove(const QPoint& screenPos) = 0;
    virtual bool handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button) = 0;
};

} // namespace onecad::ui::tools

#endif // ONECAD_UI_TOOLS_MODELINGTOOL_H
