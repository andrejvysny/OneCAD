/**
 * @file ExtrudeTool.h
 * @brief Sketch region extrusion tool (modeling mode).
 */
#ifndef ONECAD_UI_TOOLS_EXTRUDETOOL_H
#define ONECAD_UI_TOOLS_EXTRUDETOOL_H

#include "ModelingTool.h"

#include "../../render/tessellation/TessellationCache.h"
#include "../../kernel/elementmap/ElementMap.h"
#include "../../app/document/OperationRecord.h"

#include <TopoDS_Face.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Pln.hxx>
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

class ExtrudeTool : public ModelingTool {
public:
    explicit ExtrudeTool(Viewport* viewport, app::Document* document);

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

private:
    bool prepareInput(const app::selection::SelectionItem& selection);
    bool isPlanarFace(const TopoDS_Face& face) const;
    void updatePreview(double distance);
    void clearPreview();
    TopoDS_Shape buildExtrudeShape(double distance) const;
    void detectBooleanMode(double distance);

    Viewport* viewport_ = nullptr;
    app::Document* document_ = nullptr;
    app::commands::CommandProcessor* commandProcessor_ = nullptr;
    app::selection::SelectionItem selection_{};
    core::sketch::Sketch* sketch_ = nullptr; // Null if extruding a body face
    std::string targetBodyId_; // ID of the body owning the selected face (if any)
    TopoDS_Shape targetShape_; // The original shape of the body being modified (for boolean ops)
    TopoDS_Face baseFace_;
    gp_Pnt baseCenter_;
    gp_Dir direction_;
    gp_Pln neutralPlane_;

    bool active_ = false;
    bool dragging_ = false;
    QPoint dragStart_;
    double currentDistance_ = 0.0;
    double draftAngleDeg_ = 0.0;
    app::BooleanMode booleanMode_ = app::BooleanMode::NewBody;

    render::TessellationCache previewTessellator_;
    kernel::elementmap::ElementMap previewElementMap_;
};

} // namespace onecad::ui::tools

#endif // ONECAD_UI_TOOLS_EXTRUDETOOL_H
