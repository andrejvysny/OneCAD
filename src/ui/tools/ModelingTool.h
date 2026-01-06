/**
 * @file ModelingTool.h
 * @brief Base interface for 3D modeling tools.
 */
#ifndef ONECAD_UI_TOOLS_MODELINGTOOL_H
#define ONECAD_UI_TOOLS_MODELINGTOOL_H

#include "../../app/selection/SelectionTypes.h"
#include "../../app/document/OperationRecord.h"

#include <QPoint>
#include <QVector3D>
#include <Qt>
#include <optional>

namespace onecad::ui::tools {

class ModelingTool {
public:
    struct Indicator {
        QVector3D origin;
        QVector3D direction;
        double distance = 0.0;
        bool showDistance = false;
        app::BooleanMode booleanMode = app::BooleanMode::NewBody;
    };

    virtual ~ModelingTool() = default;

    virtual void begin(const app::selection::SelectionItem& selection) = 0;
    virtual void cancel() = 0;
    virtual bool isActive() const = 0;
    virtual bool isDragging() const = 0;

    virtual bool handleMousePress(const QPoint& screenPos, Qt::MouseButton button) = 0;
    virtual bool handleMouseMove(const QPoint& screenPos) = 0;
    virtual bool handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button) = 0;

    virtual std::optional<Indicator> indicator() const { return std::nullopt; }
};

} // namespace onecad::ui::tools

#endif // ONECAD_UI_TOOLS_MODELINGTOOL_H
