/**
 * @file RevolveTool.h
 * @brief Tool for creating bodies by revolving a profile around an axis.
 */
#ifndef ONECAD_UI_TOOLS_REVOLVETOOL_H
#define ONECAD_UI_TOOLS_REVOLVETOOL_H

#include "ModelingTool.h"

#include "../../render/tessellation/TessellationCache.h"
#include "../../kernel/elementmap/ElementMap.h"
#include "../../app/document/OperationRecord.h" // For BooleanMode

#include <TopoDS_Face.hxx>
#include <gp_Ax1.hxx>
#include <gp_Pnt.hxx>
#include <string>

namespace onecad::app {
class Document;
}
namespace onecad::app::commands {
class CommandProcessor;
}

namespace onecad::core::sketch {
class Sketch;
}

namespace onecad::ui {
class Viewport;
}

namespace onecad::ui::tools {

class RevolveTool : public ModelingTool {
public:
    explicit RevolveTool(Viewport* viewport, app::Document* document);

    void setDocument(app::Document* document);
    void setCommandProcessor(app::commands::CommandProcessor* processor);

    void begin(const app::selection::SelectionItem& selection) override;
    void cancel() override;
    bool isActive() const override { return active_; }
    bool isDragging() const override { return dragging_; }

    bool handleMousePress(const QPoint& screenPos, Qt::MouseButton button) override;
    bool handleMouseMove(const QPoint& screenPos) override;
    bool handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button) override;
    std::optional<Indicator> indicator() const override;

    // Additional method to handle axis selection when in WaitingForAxis state
    void onSelectionChanged(const std::vector<app::selection::SelectionItem>& selection);

private:
    enum class State {
        WaitingForProfile,
        WaitingForAxis,
        Dragging
    };

    bool prepareProfile(const app::selection::SelectionItem& selection);
    bool setAxis(const app::selection::SelectionItem& selection);
    
    void updatePreview(double angle);
    void clearPreview();
    TopoDS_Shape buildRevolveShape(double angle) const;
    void detectBooleanMode(double angle);

    Viewport* viewport_ = nullptr;
    app::Document* document_ = nullptr;
    app::commands::CommandProcessor* commandProcessor_ = nullptr;
    
    app::selection::SelectionItem profileSelection_{};
    app::selection::SelectionItem axisSelection_{}; // Store axis selection
    
    core::sketch::Sketch* sketch_ = nullptr;
    std::string targetBodyId_; // For boolean ops
    
    TopoDS_Face baseFace_;
    gp_Ax1 axis_; // Revolution axis
    gp_Pnt baseCenter_; // For indicator origin
    bool axisValid_ = false;
    
    State state_ = State::WaitingForProfile;
    bool active_ = false;
    bool dragging_ = false;
    QPoint dragStart_;
    double dragStartAngle_ = 0.0;
    double currentAngle_ = 0.0; // Degrees
    app::BooleanMode booleanMode_ = app::BooleanMode::NewBody;

    render::TessellationCache previewTessellator_;
    kernel::elementmap::ElementMap previewElementMap_;
};

} // namespace onecad::ui::tools

#endif // ONECAD_UI_TOOLS_REVOLVETOOL_H
